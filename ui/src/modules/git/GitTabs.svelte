<script lang="ts">
  // GitKraken-style top-level repo tab strip for the Git page. Each tab is one
  // OPEN repo (name + current branch + a status dot + a close ✕). Click to
  // activate, middle-click to close, drag to reorder (mirrors shell/TabBar). The
  // trailing + button opens a picker of repos not already open. Styled like
  // ApiPage's `.req-tabs`. Workspace-independent — driven entirely by the git
  // store's open-tabs state, which persists across restarts.
  import { api } from '../../lib/api/client';
  import type { Repo, RepoStatusResp } from '../../lib/api/types';
  import { git } from '../../lib/stores/git.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import Icon from '../../lib/components/Icon.svelte';

  interface Props {
    /** Open a repo as a tab (parent loads/wires the active RepoView). */
    onopen: (repoId: string) => void;
    /** Open the "Add repository" flow in a given mode (parent owns the modal). */
    onadd: (mode: 'register' | 'clone' | 'browse') => void;
  }
  let { onopen, onadd }: Props = $props();

  const byId = $derived(new Map(git.allRepos.map((r) => [r.id, r])));
  const openRepos = $derived(
    git.openRepoIds.map((id) => byId.get(id)).filter((r): r is Repo => r != null),
  );
  const closedRepos = $derived(git.allRepos.filter((r) => !git.openRepoIds.includes(r.id)));

  // ── Per-repo status (branch + dirty dot) ─────────────────────────────────
  // Fetched lazily once per open repo, cached here so the strip shows the live
  // branch + a clean/dirty indicator without re-querying on every render.
  let statusCache = $state<Record<string, RepoStatusResp | null>>({});

  $effect(() => {
    for (const id of git.openRepoIds) {
      if (id in statusCache) continue;
      statusCache[id] = null; // mark in-flight so we don't refetch
      void api
        .get<RepoStatusResp>(`/repos/${id}/status`)
        .then((s) => (statusCache[id] = s))
        .catch(() => (statusCache[id] = null));
    }
  });

  function branchOf(id: string): string | null {
    return statusCache[id]?.branch ?? null;
  }
  function isDirty(id: string): boolean {
    return (statusCache[id]?.changes.length ?? 0) > 0;
  }

  function openPicker(e: MouseEvent): void {
    // ALWAYS offer ways to add a new repo (clone or local) — even when every
    // registered repo is already open — then list any not-yet-open repos to
    // open. Picking one that's already open just activates it (openRepoTab is
    // idempotent), so there's no duplicate-tab risk.
    ctxMenu.show(e, [
      { label: 'Clone a repository…', icon: 'download', action: () => onadd('clone') },
      { label: 'Browse remote to clone…', icon: 'globe', action: () => onadd('browse') },
      { label: 'Add a local repository…', icon: 'folder', action: () => onadd('register') },
      ...(closedRepos.length > 0 ? [{ separator: true }] : []),
      ...closedRepos.map((r) => ({
        label: r.name,
        icon: 'branch',
        action: () => onopen(r.id),
      })),
    ]);
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
</script>

<div class="git-tabs">
  <!-- The tablist must contain ONLY role="tab" children (ARIA
       aria-required-children); `display:contents` keeps the flex layout
       identical while moving the "new repo" button out of the tablist. -->
  <div class="git-tablist" role="tablist">
  {#each openRepos as r (r.id)}
    <div
      class="git-tab"
      class:active={git.activeRepoId === r.id}
      class:drag-over={dragOverId === r.id}
      role="tab"
      tabindex="0"
      aria-selected={git.activeRepoId === r.id}
      draggable="true"
      title={r.path}
      onclick={() => onopen(r.id)}
      onkeydown={(e) => e.key === 'Enter' && onopen(r.id)}
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
          <Icon name="branch" size={9} />{branchOf(r.id)}
        </span>
      {/if}
      <button
        class="git-tab-close"
        title="Close tab"
        aria-label="Close tab"
        onclick={(e) => {
          e.stopPropagation();
          git.closeRepoTab(r.id);
        }}>×</button
      >
    </div>
  {/each}
  </div>
  <button class="git-tab-new" title="Open a repository" aria-label="Open a repository" onclick={openPicker}>
    +
  </button>
</div>

<style>
  .git-tabs {
    display: flex;
    align-items: stretch;
    gap: 2px;
    padding: 6px 8px 0;
    border-bottom: 1px solid var(--border);
    overflow-x: auto;
    flex-shrink: 0;
    scrollbar-width: none;
  }
  /* Layout-neutral wrapper: groups the tabs under role="tablist" without
     introducing a box (so the flex row above is unchanged). */
  .git-tablist {
    display: contents;
  }
  .git-tabs::-webkit-scrollbar {
    display: none;
  }
  .git-tab {
    display: flex;
    align-items: center;
    gap: 6px;
    max-width: 230px;
    padding: 6px 8px 6px 10px;
    border: 1px solid transparent;
    border-bottom: none;
    border-radius: var(--radius-s) var(--radius-s) 0 0;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    font-size: 12px;
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
    background: #febc2e;
    opacity: 1;
  }
  .git-tab-name {
    overflow: hidden;
    text-overflow: ellipsis;
    font-weight: 500;
  }
  .git-tab-branch {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    flex-shrink: 0;
    font-size: 10px;
    color: var(--text-dim);
    max-width: 110px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .git-tab.active .git-tab-branch {
    color: var(--accent);
  }
  .git-tab-close {
    border: none;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    font-size: 15px;
    line-height: 1;
    padding: 0 2px;
    border-radius: 4px;
  }
  .git-tab-close:hover {
    background: var(--border);
    color: var(--text);
  }
  .git-tab-new {
    border: none;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    font-size: 18px;
    padding: 0 10px;
    border-radius: var(--radius-s);
    flex-shrink: 0;
  }
  .git-tab-new:hover {
    background: var(--surface-2);
    color: var(--accent);
  }
  .mono {
    font-family: var(--font-mono);
  }
</style>
