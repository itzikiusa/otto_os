<script lang="ts">
  // The refs sidebar's lower sections (arch-1-09: split out of GraphView.svelte):
  // STASHES (`git stash list`), WORKTREES (`git worktree list`; a click opens
  // the tree as a git tab, never a branch switch) and SUBMODULES. Rows and
  // their ⋯ buttons open the menus in graphMenus.ts. Row styling is shared
  // with the branch sections through RefSidebar's `.refs-panel` rules.
  import type { StashInfo, SubmoduleInfo, WorktreeInfo } from '../../../lib/api/types';
  import { rowMenu } from '../../../lib/rowMenu';
  import { sentenceCase } from '../../../lib/labels';
  import Icon from '../../../lib/components/Icon.svelte';
  import { normPath, stashShortMsg } from '../ref-menus';
  import type { GraphActions } from './graphActions.svelte';
  import type { GraphMenus } from './graphMenus';
  import type { GraphSelection } from './graphSelection.svelte';

  interface Props {
    stashes: StashInfo[];
    worktrees: WorktreeInfo[];
    submodules: SubmoduleInfo[];
    /** Absolute path of the open repo tab (main or linked worktree). */
    currentRepoPath: string;
    openWtBusy: string;
    actions: GraphActions;
    menus: GraphMenus;
    selection: GraphSelection;
  }
  let { stashes, worktrees, submodules, currentRepoPath, openWtBusy, actions, menus, selection }: Props = $props();

  let stashesOpen = $state(false);
  let worktreesOpen = $state(false);
  let submodulesOpen = $state(false);
</script>

<!-- STASHES — read-only `git stash list`; right-click for Apply / Drop. -->
<div class="ref-section">
  <button class="ref-header" onclick={() => (stashesOpen = !stashesOpen)} aria-expanded={stashesOpen}>
    <Icon name={stashesOpen ? 'chevronDown' : 'chevronRight'} size={12} />
    <Icon name="stash" size={12} />
    <span>STASHES</span>
    <span class="ref-count">{stashes.length}</span>
  </button>
  {#if stashesOpen}
    {#each stashes as s (s.ref)}
      <div class="ref-action-row">
      <button use:rowMenu
        class="ref-row stash-row"
        title={`${s.ref} · ${s.message}`}
        onclick={() => selection.selectStash(s)}
        oncontextmenu={(e) => menus.stashMenu(e, s)}
      >
        <Icon name="stash" size={12} />
        <span class="ref-name stash-msg">{stashShortMsg(s)}</span>
        {#if s.branch}<span class="stash-branch mono dim">{s.branch}</span>{/if}
      </button>
      <button class="icon-btn ref-action" title="Actions for stash {s.ref}" aria-label="Actions for stash {s.ref}" aria-haspopup="menu" onclick={(e) => menus.stashMenu(e, s)}><Icon name="more" size={13} /></button>
      </div>
    {:else}
      <div class="dim ref-empty">No stashes</div>
    {/each}
  {/if}
</div>

<!-- WORKTREES — `git worktree list`; click opens as a git tab (never
     branch-switch). Right-click to remove/prune. Shows agent-created
     trees (swarm, Run-with-Otto, goal loops) too. -->
<div class="ref-section">
  <button class="ref-header" onclick={() => (worktreesOpen = !worktreesOpen)} aria-expanded={worktreesOpen}>
    <Icon name={worktreesOpen ? 'chevronDown' : 'chevronRight'} size={12} />
    <Icon name="worktree" size={12} />
    <span>WORKTREES</span>
    <span class="ref-count">{worktrees.length}</span>
  </button>
  {#if worktreesOpen}
    {#each worktrees as w (w.path)}
      {@const isHere = currentRepoPath !== '' && normPath(w.path) === currentRepoPath}
      <div class="ref-action-row">
      <button use:rowMenu
        type="button"
        class="ref-row stash-row is-worktree"
        class:current={isHere}
        disabled={w.prunable || openWtBusy !== ''}
        title={w.prunable
          ? `${w.path} · stale (folder gone)`
          : isHere
            ? `Current worktree · ${w.path}`
            : `Open worktree · ${w.path}${w.branch ? ` · ${w.branch}` : ' · detached'}${w.dirty ? ' · uncommitted changes' : ''}`}
        onclick={() => {
          if (!w.prunable && !isHere) void actions.openWorktree(w);
        }}
        oncontextmenu={(e) => {
          e.preventDefault();
          menus.wtMenu(e, w);
        }}
      >
        <Icon name="worktree" size={12} />
        <span class="ref-name stash-msg" class:dim={w.prunable}>{actions.wtName(w)}</span>
        {#if !w.dirty_known}<span class="wt-dirty" title="Changes unknown — status check unavailable">?</span>
        {:else if w.dirty}<span class="wt-dirty" title="Uncommitted changes">●</span>{/if}
        {#if w.locked}<Icon name="lock" size={12} />{/if}
        {#if w.prunable}
          <span class="wt-flag mono dim">stale</span>
        {:else if isHere}
          <span class="wt-flag mono dim">current</span>
        {:else if w.is_main}
          <span class="wt-open-hint">open worktree</span>
          <span class="wt-flag mono dim">main</span>
        {:else}
          <span class="wt-open-hint">open worktree</span>
          {#if w.branch}
            <span class="stash-branch mono dim">{w.branch}</span>
          {:else}
            <span class="wt-flag mono dim">detached</span>
          {/if}
        {/if}
        {#if openWtBusy === w.path}<span class="dim">…</span>{/if}
      </button>
      <button class="icon-btn ref-action" title="Actions for worktree {w.path}" aria-label="Actions for worktree {w.path}" aria-haspopup="menu" onclick={(e) => menus.wtMenu(e, w)}><Icon name="more" size={13} /></button>
      </div>
    {:else}
      <div class="dim ref-empty">No worktrees</div>
    {/each}
    {#if worktrees.some((w) => w.prunable)}
      <button class="ref-row stash-row wt-prune" onclick={() => void actions.wtPrune()}>
        <Icon name="trash" size={12} />
        <span class="ref-name">Prune stale entries</span>
      </button>
    {/if}
  {/if}
</div>

<!-- SUBMODULES — `git submodule status`; right-click to init/update. -->
{#if submodules.length > 0}
  <div class="ref-section">
    <button class="ref-header" onclick={() => (submodulesOpen = !submodulesOpen)} aria-expanded={submodulesOpen}>
      <Icon name={submodulesOpen ? 'chevronDown' : 'chevronRight'} size={12} />
      <Icon name="shapes" size={12} />
      <span>SUBMODULES</span>
      <span class="ref-count">{submodules.length}</span>
    </button>
    {#if submodulesOpen}
      {#each submodules as sub (sub.path)}
        <button use:rowMenu
          class="ref-row stash-row"
          title={`${sub.path} @ ${sub.sha.slice(0, 10)}${sub.url ? ` · ${sub.url}` : ''}${sub.state !== 'ok' ? ` · ${sub.state}` : ''}`}
          oncontextmenu={(e) => menus.subMenu(e, sub)}
          aria-label="Actions for submodule {sub.path}"
          aria-haspopup="menu"
          onclick={(e) => menus.subMenu(e, sub)}
        >
          <Icon name="shapes" size={12} />
          <span class="ref-name stash-msg">{sub.path}</span>
          {#if sub.state !== 'ok'}
            <span class="wt-flag mono" class:sub-warn={sub.state !== 'uninitialized'}>{sentenceCase(sub.state)}</span>
          {/if}
          <Icon name="more" size={13} />
        </button>
      {/each}
    {/if}
  </div>
{/if}

<style>
  /* ── STASHES / WORKTREES / SUBMODULES rows (the base row is RefSidebar's) ── */
  .stash-row {
    cursor: pointer;
    color: var(--text-dim);
  }
  .stash-row:hover {
    background: var(--hover);
    color: var(--text);
  }
  .stash-msg {
    font-size: var(--fs-xs);
  }
  .stash-branch {
    flex-shrink: 0;
    font-size: var(--fs-xs);
    max-width: 80px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .wt-dirty {
    flex-shrink: 0;
    color: var(--warning);
    font-size: var(--fs-xs);
    line-height: 1;
  }
  .wt-flag {
    flex-shrink: 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .wt-flag.sub-warn {
    color: var(--warning);
  }
  /* Out-ranks the shared `.refs-panel .ref-row` base (RefSidebar). */
  .ref-section .wt-prune {
    width: 100%;
    text-align: start;
    background: none;
    border: none;
    cursor: pointer;
    color: var(--text-dim);
    font-size: var(--fs-xs);
  }
  .dim {
    color: var(--text-dim);
  }
  .mono {
    font-family: var(--font-mono);
  }
</style>
