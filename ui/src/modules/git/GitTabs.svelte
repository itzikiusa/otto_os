<script lang="ts">
  // GitKraken-style top-level repo tab strip for the Git page. Each tab is one
  // OPEN repo (name + a status dot + a close ✕, and its current branch when
  // the strip has room — see `fitBranches`; the branch is always in the
  // tab's tooltip and accessible name). Click to
  // activate, middle-click to close, drag to reorder (mirrors shell/TabBar). The
  // trailing + button opens a picker of repos not already open. Styled like
  // ApiPage's `.req-tabs`. Workspace-independent — driven entirely by the git
  // store's open-tabs state, which persists across restarts.
  import { untrack } from 'svelte';
  import type { Repo } from '../../lib/api/types';
  import { git } from '../../lib/stores/git.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { onTabKey } from '../../lib/tabKeys';
  import { branchMode, repoTabLabel } from './repoTabFit';

  interface Props {
    /** Open a repo as a tab (parent loads/wires the active RepoView). */
    onopen: (repoId: string) => void;
    /** Open the "Add repository" flow in a given mode (parent owns the modal). */
    onadd: (mode: 'register' | 'clone' | 'browse') => void;
    /** Rendered inside the PageHeader bar (pill tabs, no own border row). */
    embedded?: boolean;
    /** The page-level Focus view is showing: its entry is pressed and no
     *  repo tab reads as selected. */
    focus?: boolean;
    /** Toggle Focus. Without it the Focus entry isn't rendered. */
    ontogglefocus?: () => void;
  }
  let { onopen, onadd, embedded = false, focus = false, ontogglefocus }: Props = $props();
  const selectedId = $derived(focus ? null : git.activeRepoId);

  const byId = $derived(new Map(git.allRepos.map((r) => [r.id, r])));
  const openRepos = $derived(
    git.openRepoIds.map((id) => byId.get(id)).filter((r): r is Repo => r != null),
  );
  const closedRepos = $derived(git.allRepos.filter((r) => !git.openRepoIds.includes(r.id)));

  // ── Per-repo status (branch + dirty dot) ─────────────────────────────────
  // Status is owned by the git store (shared with RepoView + the auto-fetch
  // loop). Lazily load it once per open repo so the strip shows the live branch
  // + a clean/dirty indicator; the auto-fetch loop keeps it fresh thereafter.
  // `untrack` the loads so this effect re-runs only when the OPEN set changes,
  // not on every status write the loop makes.
  $effect(() => {
    const ids = git.openRepoIds;
    untrack(() => {
      for (const id of ids) git.ensureStatus(id);
    });
  });

  function branchOf(id: string): string | null {
    return git.statusById[id]?.branch ?? null;
  }
  function isDirty(id: string): boolean {
    return (git.statusById[id]?.changes.length ?? 0) > 0;
  }

  function openPicker(e: MouseEvent): void {
    // ALWAYS offer ways to add a new repo (clone or local) — even when every
    // registered repo is already open — then list any not-yet-open repos to
    // open. Picking one that's already open just activates it (openRepoTab is
    // idempotent), so there's no duplicate-tab risk.
    // The repo list is data-driven and can be long — searchable, and capped at
    // a dozen visible rows (the search narrows into the rest). The three add
    // actions stay pinned above the list.
    ctxMenu.show(
      e,
      [
        { label: 'Clone a repository…', icon: 'download', pinned: true, action: () => onadd('clone') },
        { label: 'Browse remote to clone…', icon: 'globe', pinned: true, action: () => onadd('browse') },
        { label: 'Add a local repository…', icon: 'folder', pinned: true, action: () => onadd('register') },
        ...(closedRepos.length > 0 ? [{ separator: true }] : []),
        ...closedRepos.map((r) => ({
          label: r.name,
          icon: 'branch',
          action: () => onopen(r.id),
        })),
      ],
      { filter: true, filterPlaceholder: 'Filter repositories…', maxVisible: 12 },
    );
  }

  /** Tablist keys: Enter/Space open, Delete or Backspace closes (focus moves to
   *  the neighbour); ←/→ (Home/End, RTL-aware) move + open via the shared
   *  roving-tabindex handler. */
  function onRepoTabKey(e: KeyboardEvent, id: string): void {
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      onopen(id);
    } else if (e.key === 'Delete' || e.key === 'Backspace') {
      e.preventDefault();
      const ids = openRepos.map((r) => r.id);
      const i = ids.indexOf(id);
      const next = ids[i + 1] ?? ids[i - 1];
      git.closeRepoTab(id);
      if (next) {
        onopen(next);
        queueMicrotask(() => listEl?.querySelector<HTMLElement>(`[data-repo-id="${CSS.escape(next)}"]`)?.focus());
      }
    } else {
      onTabKey(e);
    }
  }

  // ── Drag-to-reorder (mirrors shell/TabBar) ───────────────────────────────
  let dragId = $state<string | null>(null);
  let dragOverId = $state<string | null>(null);

  function onDragStart(e: DragEvent, id: string): void {
    dragId = id;
    e.dataTransfer?.setData('text/plain', id);
    if (e.dataTransfer) e.dataTransfer.effectAllowed = 'move';
  }
  function onDragOver(e: DragEvent, id: string): void {
    if (!dragId || id === dragId) return;
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
    dragOverId = id;
  }
  function onDragLeave(id: string): void {
    if (dragOverId === id) dragOverId = null;
  }
  function onDrop(e: DragEvent, id: string): void {
    e.preventDefault();
    if (!dragId || id === dragId) return;
    git.reorderRepoTab(dragId, git.openRepoIds.indexOf(id));
    dragId = null;
    dragOverId = null;
  }
  function onDragEnd(): void {
    dragId = null;
    dragOverId = null;
  }

  // ── Branch chips only while they fit ─────────────────────────────────────
  // Measured, not counted: show every branch, read the widths, and if the row
  // would overflow keep only the active tab's branch (if THAT fits) or none
  // (repoTabFit.ts). Written straight to `data-branches` on the list so the
  // measure → apply round trip happens inside one frame, without a re-render.
  // Re-measured when the strip or its header resizes, and when the tabs,
  // their branches or the active tab change.
  const GAP = 2;
  function fitBranches(): void {
    const el = listEl;
    if (!el) return;
    el.removeAttribute('data-branches');
    const tabs = Array.from(el.querySelectorAll<HTMLElement>('.git-tab'));
    const measures = tabs.map((t) => {
      const chip = t.querySelector<HTMLElement>('.git-tab-branch');
      return { full: t.getBoundingClientRect().width + GAP, branch: chip ? chip.getBoundingClientRect().width + 6 : 0 };
    });
    const overflowing = el.scrollWidth > el.clientWidth + 0.5;
    const active = tabs.findIndex((t) => t.classList.contains('active'));
    el.setAttribute('data-branches', overflowing ? branchMode(measures, el.clientWidth + GAP, active) : 'all');
  }
  let fitFrame = 0;
  function scheduleFit(): void {
    if (fitFrame) return;
    fitFrame = requestAnimationFrame(() => {
      fitFrame = 0;
      fitBranches();
    });
  }
  $effect(() => {
    const el = listEl;
    if (!el) return;
    const ro = new ResizeObserver(scheduleFit);
    ro.observe(el);
    // The row is content-sized, so its own size never says "there is room
    // again": watch what bounds it too — the header row and its siblings (the
    // actions shrinking frees room) and the header's style, where PageHeader
    // writes the tab cap (--ph-tabs-max).
    const header = el.closest('header');
    const row = el.closest('.ph-tabs-inline')?.parentElement ?? el.parentElement;
    if (row) {
      ro.observe(row);
      for (const child of Array.from(row.children)) ro.observe(child);
    }
    const mo = new MutationObserver(scheduleFit);
    if (header) mo.observe(header, { attributes: true, attributeFilter: ['style'] });
    scheduleFit();
    return () => {
      ro.disconnect();
      mo.disconnect();
      if (fitFrame) cancelAnimationFrame(fitFrame);
      fitFrame = 0;
    };
  });
  $effect(() => {
    void openRepos.map((r) => `${r.name}:${branchOf(r.id)}`).join();
    void selectedId;
    untrack(scheduleFit);
  });

  // Only the tab row scrolls; + and auto-fetch sit outside it so they are
  // never clipped. Keep the active tab in view when it changes (opened from
  // the picker, restored on load) — it may be off the scroller's edge.
  let listEl = $state<HTMLDivElement | null>(null);
  $effect(() => {
    const id = git.activeRepoId;
    if (!listEl || !id) return;
    const el = listEl.querySelector<HTMLElement>(`[data-repo-id="${CSS.escape(id)}"]`);
    el?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
  });
</script>

<div class="git-tabs" class:embedded>
  {#if ontogglefocus && git.allRepos.length > 0}
    <!-- Focus is cross-repo (my PRs + my Jira work), so it is its own entry
         pinned before the repo tabs, never one of a repo's sub-tabs. -->
    <button
      class="git-focus"
      class:on={focus}
      aria-pressed={focus}
      title={focus ? 'Back to the open repository' : 'Focus: your pull requests and Jira work across repositories'}
      onclick={ontogglefocus}
    >
      <Icon name="zap" size={13} />Focus
    </button>
    <span class="git-focus-sep" aria-hidden="true"></span>
  {/if}
  <!-- The tablist must contain ONLY role="tab" children (ARIA
       aria-required-children), and it is the only part that scrolls: the
       auto-fetch toggle and + stay pinned after it, fully visible however
       many repos are open. -->
  <div class="git-tablist" role="tablist" aria-label="Open repositories" bind:this={listEl}>
  {#each openRepos as r (r.id)}
    {@const branch = branchOf(r.id)}
    {@const label = repoTabLabel(r.name, branch)}
    {@const dirty = isDirty(r.id)}
    <!-- A tab's children are presentational, so the close button can't live
         inside role="tab" (axe nested-interactive): a presentation wrapper
         holds the tab button and the close button as siblings. The wrapper
         carries the drag-to-reorder handlers. -->
    <div
      class="git-tab"
      class:active={selectedId === r.id}
      class:drag-over={dragOverId === r.id}
      role="presentation"
      draggable="true"
      ondragstart={(e) => onDragStart(e, r.id)}
      ondragover={(e) => onDragOver(e, r.id)}
      ondragleave={() => onDragLeave(r.id)}
      ondrop={(e) => onDrop(e, r.id)}
      ondragend={onDragEnd}
    >
      <button
        type="button"
        class="git-tab-main"
        role="tab"
        tabindex={git.activeRepoId === r.id || (!git.activeRepoId && r.id === openRepos[0]?.id) ? 0 : -1}
        aria-selected={selectedId === r.id}
        aria-label={dirty ? `${label}, uncommitted changes` : label}
        data-repo-id={r.id}
        title={`${label}${dirty ? ' · uncommitted changes' : ''}\n${r.path}`}
        onclick={() => onopen(r.id)}
        onkeydown={(e) => onRepoTabKey(e, r.id)}
        onauxclick={(e) => {
          if (e.button === 1) {
            e.preventDefault();
            git.closeRepoTab(r.id);
          }
        }}
      >
        <span class="git-tab-dot" class:dirty aria-hidden="true"></span>
        <span class="git-tab-name">{r.name}</span>
        {#if branch}
          <span class="git-tab-branch mono" aria-hidden="true">
            <Icon name="branch" size={12} /><span class="git-tab-branch-name">{branch}</span>
          </span>
        {/if}
      </button>
      <button
        type="button"
        class="git-tab-close"
        tabindex="-1"
        title="Close {r.name}"
        aria-label="Close {r.name}"
        onclick={(e) => {
          e.stopPropagation();
          git.closeRepoTab(r.id);
        }}><Icon name="x" size={12} /></button
      >
    </div>
  {/each}
  </div>
  <button
    class="git-autofetch"
    class:on={git.autoFetchEnabled}
    title={git.autoFetchEnabled
      ? `Auto-fetch on — selected repo every 30s, other open repos every ${git.autoFetchIntervalSec}s while Git is open and this window is focused. Click to pause.`
      : 'Auto-fetch paused. Click to fetch open repos automatically.'}
    aria-label="Toggle auto-fetch for open repositories"
    aria-pressed={git.autoFetchEnabled}
    onclick={() => git.setAutoFetch(!git.autoFetchEnabled)}
  >
    <Icon name="fetch" size={13} />
  </button>
  <button class="git-tab-new" title="Open a repository" aria-label="Open a repository" onclick={openPicker}>
    <Icon name="plus" size={14} />
  </button>
</div>

<style>
  .git-tabs {
    display: flex;
    align-items: stretch;
    gap: 2px;
    padding: 6px 8px 0;
    border-bottom: 1px solid var(--border);
    /* The strip itself never scrolls or clips its buttons — only the tablist
       below does (it shrinks first: min-width 0). */
    min-width: 0;
    flex-shrink: 0;
  }
  .git-tablist {
    display: flex;
    align-items: stretch;
    gap: inherit;
    flex: 0 1 auto;
    min-width: 0;
    overflow-x: auto;
    scrollbar-width: none;
  }
  .git-tablist::-webkit-scrollbar {
    display: none;
  }
  .git-tab {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-shrink: 0;
    /* Keep natural widths on desktop: a fixed cap truncated identities even
       when the header had hundreds of unused pixels. Overflow stays in the
       tablist, and short identities never stretch just to fill the row. */
    max-width: none;
    padding: 6px 8px;
    padding-inline-start: 10px;
    border: 1px solid transparent;
    border-bottom: none;
    border-radius: var(--radius-s) var(--radius-s) 0 0;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    font-size: var(--fs-s);
    white-space: nowrap;
  }
  .git-tab:hover {
    background: var(--hover);
  }
  /* The role="tab" button: the wrapper keeps the tab chrome; this only lays
     out the dot, name and branch and takes the focus ring. */
  .git-tab-main {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    padding: 0;
    border: none;
    background: transparent;
    color: inherit;
    font: inherit;
    cursor: pointer;
    white-space: nowrap;
  }
  .git-tab-main:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 2px;
    border-radius: var(--radius-s);
  }
  .git-tab.active {
    background: var(--surface-2);
    border-color: var(--border);
    color: var(--text);
  }
  /* Drop target while reordering — faint left beacon (matches TabBar). */
  .git-tab.drag-over {
    border-inline-start: 2px solid var(--accent);
  }
  .git-tab-dot {
    flex-shrink: 0;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--text-dim);
    opacity: 0.55;
  }
  /* Uncommitted changes present → amber dot (a quiet "dirty" beacon). */
  .git-tab-dot.dirty {
    background: var(--warning);
    opacity: 1;
  }
  .git-tab-name {
    /* min-width:0 lets this flex item shrink so the ellipsis actually engages
       (flex default min-width:auto would otherwise refuse to clip the name). */
    min-width: 0;
    flex-shrink: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    font-weight: 500;
  }
  /* The repo name is the tab's identity, the branch is context: when a tab
     is constrained on mobile the branch gives way first (shrinks 4× faster), so "bo_common_ui" stays readable instead of
     "bo_co…" next to a full "feature/kyb-co…". Short branches ("main") are
     untouched. The full name + branch are in the tab's tooltip. */
  .git-tab-branch {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    min-width: 0;
    flex-shrink: 4;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    max-width: none;
  }
  /* Ellipsis needs a block-level text box: on the inline-flex chip itself the
     bare text node was cut mid-character ("release/5.02.4("). */
  .git-tab-branch-name {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .git-tab.active .git-tab-branch {
    color: var(--accent-text);
  }
  /* fitBranches(): the row would overflow — name-only tabs (the active one
     keeps its branch when that alone fits). */
  .git-tablist:global([data-branches='none']) .git-tab-branch,
  .git-tablist:global([data-branches='active']) .git-tab:not(.active) .git-tab-branch {
    display: none;
  }
  .git-focus {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    flex-shrink: 0;
    padding: 4px 8px;
    border: 1px solid transparent;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font-size: var(--fs-s);
    font-weight: 500;
    cursor: pointer;
    white-space: nowrap;
  }
  .git-focus:hover {
    background: var(--hover);
    color: var(--text);
  }
  .git-focus.on {
    background: var(--accent-soft);
    border-color: var(--border);
    color: var(--accent-text);
  }
  .git-focus:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: -2px;
  }
  .git-focus-sep {
    align-self: center;
    flex-shrink: 0;
    width: 1px;
    height: 16px;
    margin-inline: 4px;
    background: var(--border);
  }
  /* Same look as the session TabBar's close (Icon x in a 16px box). */
  .git-tab-close {
    display: grid;
    place-items: center;
    flex-shrink: 0;
    width: 16px;
    height: 16px;
    border: none;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    padding: 0;
    border-radius: var(--radius-s);
  }
  .git-tab-close:hover {
    background: var(--border);
    color: var(--text);
  }
  .git-tab-new {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 28px;
    border: none;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    padding: 0 6px;
    border-radius: var(--radius-s);
    flex-shrink: 0;
  }
  .git-tab-new:hover {
    background: var(--hover);
    color: var(--accent-text);
  }
  /* Auto-fetch toggle: dim when paused, accent when on. */
  .git-autofetch {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border: 1px solid transparent;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    padding: 0 8px;
    border-radius: var(--radius-s);
    flex-shrink: 0;
    opacity: 0.55;
  }
  .git-autofetch:hover {
    background: var(--hover);
    color: var(--text);
    opacity: 1;
  }
  .git-autofetch.on {
    color: var(--accent-text);
    opacity: 1;
  }
  .mono {
    font-family: var(--font-mono);
  }

  /* Embedded in the PageHeader bar: pill tabs centred in the 46px row instead
     of folder tabs sitting on their own border. */
  .git-tabs.embedded {
    align-items: center;
    padding: 0;
    border-bottom: none;
    min-width: 0;
    max-width: 100%;
    gap: 2px;
  }
  .embedded .git-tablist {
    align-items: center;
  }
  .embedded .git-tab {
    border: 1px solid transparent;
    border-radius: var(--radius-s);
    padding-block: 4px; padding-inline: 8px 6px;
  }
  .embedded .git-tab.active {
    background: var(--surface-2);
    border-color: var(--border);
  }
  .embedded .git-autofetch,
  .embedded .git-tab-new {
    height: 28px;
    min-width: 28px;
  }

  /* ── Mobile + tablet (≤1024px): the tab row scrolls horizontally with
     momentum — bump tap targets so tabs + the close ✕ + the + button are
     comfortable (+ is already pinned outside the scroller). ── */
  @media (max-width: 1024px) {
    .git-tabs {
      gap: 4px;
      padding: 6px 6px 0;
    }
    .git-tablist {
      -webkit-overflow-scrolling: touch;
      overscroll-behavior-x: contain;
    }
    .git-tab {
      max-width: 200px;
      padding-block: 8px; padding-inline: 12px 6px;
      font-size: var(--fs-m);
      flex-shrink: 0;
    }
    .git-tab-branch { max-width: 120px; }
    /* ≥40px touch hit area for the close ✕ — its onclick already
       stopPropagation()s, so tapping it never also activates the tab. */
    .git-tab-close {
      width: auto;
      height: auto;
      min-width: 40px;
      min-height: 40px;
    }
    .git-autofetch,
    .git-tab-new,
    .git-focus {
      min-width: 40px;
      min-height: 40px;
    }
  }
</style>
