<script lang="ts">
  // The Branches pane's lower sections (arch-1-09: split out of GraphView.svelte):
  // STASHES (`git stash list`), WORKTREES (`git worktree list`; a click opens
  // the tree as a git tab, never a branch switch) and SUBMODULES. Rows and
  // their ⋯ buttons open the menus in graphMenus.ts. Row styling is shared
  // with the branch sections (the .side-row classes + RefSidebar's
  // `.refs-panel` rules).
  import type { StashInfo, SubmoduleInfo, WorktreeInfo } from '../../../lib/api/types';
  import { rowMenu } from '../../../lib/rowMenu';
  import { sentenceCase } from '../../../lib/labels';
  import Icon from '../../../lib/components/Icon.svelte';
  import SideSection from '../../../lib/components/side/SideSection.svelte';
  import { normPath, stashShortMsg } from '../ref-menus';
  import { relativeAge, worktreeFacts, worktreeSummary } from '../gitopsPlan';
  import { plural } from '../../../lib/plural';
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
    /** The pane's picked row (RefSidebar). */
    picked: string;
  }
  let { stashes, worktrees, submodules, currentRepoPath, openWtBusy, actions, menus, selection, picked = $bindable() }: Props = $props();

  let stashesOpen = $state(false);
  let worktreesOpen = $state(false);
  let submodulesOpen = $state(false);
</script>

<!-- STASHES — `git stash list`; a click opens the stash browser (files +
     patch, Apply / Pop / Drop); right-click for the quick actions. -->
<SideSection title="Stashes" count={stashes.length} bind:open={stashesOpen}>
  {#if stashes.length === 0}
    <div class="dim ref-empty">No stashes</div>
  {:else}
    <div role="tree" aria-label="Stashes">
      {#each stashes as s (s.ref)}
        {@const key = `stash:${s.ref}`}
        <div class="side-row ref-action-row" class:selected={picked === key}>
          <button use:rowMenu
            class="side-row-main ref-row stash-row"
            data-side-row
            role="treeitem"
            aria-selected={picked === key}
            aria-level={1}
            title={`${s.ref} · ${s.message} · ${relativeAge(s.date)}${s.files != null ? ` · ${plural(s.files, 'file')}` : ''} — click to view its changes`}
            onclick={() => {
              picked = key;
              void selection.selectStash(s);
              actions.openStashBrowser(s);
            }}
            oncontextmenu={(e) => menus.stashMenu(e, s)}
          >
            <Icon name="stash" size={12} />
            <span class="ref-name side-row-title stash-msg">{stashShortMsg(s)}</span>
            {#if s.files != null}<span class="wt-flag mono" title={plural(s.files, 'file')}>{s.files}f</span>{/if}
            {#if s.branch}<span class="stash-branch mono dim">{s.branch}</span>{/if}
          </button>
          <button class="icon-btn side-row-more ref-action" tabindex="-1" title="Actions for stash {s.ref}" aria-label="Actions for stash {s.ref}" aria-haspopup="menu" onclick={(e) => menus.stashMenu(e, s)}><Icon name="more" size={14} /></button>
        </div>
      {/each}
    </div>
  {/if}
</SideSection>

<!-- WORKTREES — `git worktree list`; click opens as a git tab (never
     branch-switch). Right-click to remove/prune. Shows agent-created
     trees (swarm, Run-with-Otto, goal loops) too. -->
<SideSection title="Worktrees" count={worktrees.length} bind:open={worktreesOpen}>
  {#if worktrees.length === 0}
    <div class="dim ref-empty">No worktrees</div>
  {:else}
    <div role="tree" aria-label="Worktrees">
      {#each worktrees as w (w.path)}
        {@const isHere = currentRepoPath !== '' && normPath(w.path) === currentRepoPath}
        {@const f = worktreeFacts(w)}
        <div class="side-row ref-action-row">
          <button use:rowMenu
            type="button"
            class="side-row-main ref-row stash-row is-worktree"
            data-side-row
            role="treeitem"
            aria-selected={false}
            aria-level={1}
            class:current={isHere}
            disabled={w.prunable || openWtBusy !== ''}
            title={w.prunable
              ? `${w.path} · stale (folder gone)`
              : isHere
                ? `Current worktree · ${w.path}`
                : `Open worktree · ${w.path}${w.branch ? ` · ${w.branch}` : ' · detached'} — ${worktreeSummary(w)}`}
            onclick={() => {
              if (!w.prunable && !isHere) void actions.openWorktree(w);
            }}
            oncontextmenu={(e) => {
              e.preventDefault();
              menus.wtMenu(e, w);
            }}
          >
            <Icon name="worktree" size={12} />
            <span class="ref-name side-row-title stash-msg" class:dim={w.prunable}>{actions.wtName(w)}</span>
            {#if f.unknown}<span class="wt-dirty" title="Changes unknown — status check unavailable">?</span>
            {:else if f.changes + f.untracked > 0}<span class="wt-dirty" title={`${plural(f.changes, 'uncommitted change')}, ${plural(f.untracked, 'untracked file')}`}>●{f.changes + f.untracked}</span>{/if}
            {#if f.unpushed > 0}<span class="wt-dirty" title={`${plural(f.unpushed, 'commit')} on no remote and not in ${w.base ?? 'the default branch'}`}>↑{f.unpushed}</span>{/if}
            {#if f.localOnly}<span class="wt-flag mono" title="This branch is not on any remote">local</span>{/if}
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
          <button class="icon-btn side-row-more ref-action" tabindex="-1" title="Actions for worktree {w.path}" aria-label="Actions for worktree {w.path}" aria-haspopup="menu" onclick={(e) => menus.wtMenu(e, w)}><Icon name="more" size={14} /></button>
        </div>
      {/each}
      {#if worktrees.some((w) => w.prunable)}
        <div class="side-row">
          <button class="side-row-main ref-row stash-row wt-prune" data-side-row role="treeitem" aria-selected={false} aria-level={1} onclick={() => void actions.wtPrune()}>
            <Icon name="trash" size={12} />
            <span class="ref-name side-row-title">Prune stale entries</span>
          </button>
        </div>
      {/if}
    </div>
  {/if}
</SideSection>

<!-- SUBMODULES — `git submodule status`; a click or right-click opens init / update. -->
{#if submodules.length > 0}
  <SideSection title="Submodules" count={submodules.length} bind:open={submodulesOpen}>
    <div role="tree" aria-label="Submodules">
      {#each submodules as sub (sub.path)}
        <div class="side-row">
          <button use:rowMenu
            class="side-row-main ref-row stash-row"
            data-side-row
            role="treeitem"
            aria-selected={false}
            aria-level={1}
            title={`${sub.path} @ ${sub.sha.slice(0, 10)}${sub.url ? ` · ${sub.url}` : ''}${sub.state !== 'ok' ? ` · ${sub.state}` : ''}`}
            oncontextmenu={(e) => menus.subMenu(e, sub)}
            aria-label="Actions for submodule {sub.path}"
            aria-haspopup="menu"
            onclick={(e) => menus.subMenu(e, sub)}
          >
            <Icon name="shapes" size={12} />
            <span class="ref-name side-row-title stash-msg">{sub.path}</span>
            {#if sub.state !== 'ok'}
              <span class="wt-flag mono" class:sub-warn={sub.state !== 'uninitialized'}>{sentenceCase(sub.state)}</span>
            {/if}
            <Icon name="more" size={14} />
          </button>
        </div>
      {/each}
    </div>
  </SideSection>
{/if}

<style>
  /* ── STASHES / WORKTREES / SUBMODULES rows (the base row is the shared .side-row) ── */
  .stash-row {
    color: var(--text-dim);
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
  .dim {
    color: var(--text-dim);
  }
  .mono {
    font-family: var(--font-mono);
  }
</style>
