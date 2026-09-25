<script lang="ts">
  // GitKraken-style top-level repo tab strip for the Git page. Each tab is one
  // OPEN repo (name + current branch + a status dot + a close ✕). Click to
  // activate, middle-click to close, drag to reorder (mirrors shell/TabBar). The
  // trailing + button opens a picker of repos not already open. Styled like
  // ApiPage's `.req-tabs`. Workspace-independent — driven entirely by the git
  // store's open-tabs state, which persists across restarts.
  import { untrack } from 'svelte';
  import type { Repo } from '../../lib/api/types';
  import { git } from '../../lib/stores/git.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import Icon from '../../lib/components/Icon.svelte';

  interface Props {
    /** Open a repo as a tab (parent loads/wires the active RepoView). */
    onopen: (repoId: string) => void;
    /** Open the "Add repository" flow in a given mode (parent owns the modal). */
    onadd: (mode: 'register' | 'clone' | 'browse') => void;
    /** Rendered inside the PageHeader bar (pill tabs, no own border row). */
    embedded?: boolean;
  }
  let { onopen, onadd, embedded = false }: Props = $props();

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
      { filter: true, filterPlaceholder: 'Search repositories…', maxVisible: 12 },
    );
  }

  /** Tablist keys: Enter/Space open, ←/→ (Home/End) move + open, Delete or
   *  Backspace closes (focus moves to the neighbour). RTL flips the arrows. */
  function onTabKey(e: KeyboardEvent, id: string): void {
    const ids = openRepos.map((r) => r.id);
    const i = ids.indexOf(id);
    const focusTab = (tid: string | undefined) => {
      if (!tid) return;
      onopen(tid);
      queueMicrotask(() =>
        listEl?.querySelector<HTMLElement>(`[data-repo-id="${CSS.escape(tid)}"]`)?.focus(),
      );
    };
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      onopen(id);
    } else if (e.key === 'ArrowLeft' || e.key === 'ArrowRight') {
      e.preventDefault();
      const rtl = listEl ? getComputedStyle(listEl).direction === 'rtl' : false;
      const fwd = e.key === (rtl ? 'ArrowLeft' : 'ArrowRight');
      focusTab(ids[(i + (fwd ? 1 : -1) + ids.length) % ids.length]);
    } else if (e.key === 'Home' || e.key === 'End') {
      e.preventDefault();
      focusTab(e.key === 'Home' ? ids[0] : ids[ids.length - 1]);
    } else if (e.key === 'Delete' || e.key === 'Backspace') {
      e.preventDefault();
      const next = ids[i + 1] ?? ids[i - 1];
      git.closeRepoTab(id);
      if (next) focusTab(next);
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
  <!-- The tablist must contain ONLY role="tab" children (ARIA
       aria-required-children), and it is the only part that scrolls: the
       auto-fetch toggle and + stay pinned after it, fully visible however
       many repos are open. -->
  <div class="git-tablist" role="tablist" bind:this={listEl}>
  {#each openRepos as r (r.id)}
    <div
      class="git-tab"
      class:active={git.activeRepoId === r.id}
      class:drag-over={dragOverId === r.id}
      role="tab"
      tabindex={git.activeRepoId === r.id || (!git.activeRepoId && r.id === openRepos[0]?.id) ? 0 : -1}
      aria-selected={git.activeRepoId === r.id}
      data-repo-id={r.id}
      draggable="true"
      title={branchOf(r.id) ? `${r.path}\n${branchOf(r.id)}` : r.path}
      onclick={() => onopen(r.id)}
      onkeydown={(e) => onTabKey(e, r.id)}
      ondragstart={(e) => onDragStart(e, r.id)}
      ondragover={(e) => onDragOver(e, r.id)}
      ondragleave={() => onDragLeave(r.id)}
      ondrop={(e) => onDrop(e, r.id)}
      ondragend={onDragEnd}
      onauxclick={(e) => {
        if (e.button === 1) {
          e.preventDefault();
          git.closeRepoTab(r.id);
        }
      }}
    >
      <span class="git-tab-dot" class:dirty={isDirty(r.id)} aria-hidden="true"></span>
      <span class="git-tab-name">{r.name}</span>
      {#if branchOf(r.id)}
        <span class="git-tab-branch mono">
          <Icon name="branch" size={9} /><span class="git-tab-branch-name">{branchOf(r.id)}</span>
        </span>
      {/if}
      <button
        class="git-tab-close"
        tabindex="-1"
        title="Close {r.name}"
        aria-label="Close {r.name}"
        onclick={(e) => {
          e.stopPropagation();
          git.closeRepoTab(r.id);
        }}><Icon name="x" size={9} /></button
      >
    </div>
  {/each}
  </div>
  <button
    class="git-autofetch"
    class:on={git.autoFetchEnabled}
    title={git.autoFetchEnabled
      ? `Auto-fetch on — selected repo every 30s, other open repos every ${git.autoFetchIntervalSec}s while this window is focused. Click to pause.`
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
    max-width: 230px;
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
    background: var(--surface-2);
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
    overflow: hidden;
    text-overflow: ellipsis;
    font-weight: 500;
  }
  /* A short branch ("main") keeps its full width; the repo name gives way
     first. Long branches still ellipsize at the cap. */
  .git-tab-branch {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    min-width: 0;
    flex-shrink: 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    max-width: 120px;
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
    padding: 0 7px;
    border-radius: var(--radius-s);
    flex-shrink: 0;
  }
  .git-tab-new:hover {
    background: var(--surface-2);
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
    background: var(--surface-2);
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
    gap: 3px;
  }
  .embedded .git-tablist {
    align-items: center;
  }
  .embedded .git-tab {
    border: 1px solid transparent;
    border-radius: var(--radius-s);
    padding: 4px 6px 4px 9px;
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
      padding: 8px 6px 8px 12px;
      font-size: var(--fs-m);
      flex-shrink: 0;
    }
    /* ≥40px touch hit area for the close ✕ — its onclick already
       stopPropagation()s, so tapping it never also activates the tab. */
    .git-tab-close {
      width: auto;
      height: auto;
      min-width: 40px;
      min-height: 40px;
    }
    .git-autofetch,
    .git-tab-new {
      min-width: 40px;
      min-height: 40px;
    }
  }
</style>
