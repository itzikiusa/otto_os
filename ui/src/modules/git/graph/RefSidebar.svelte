<script lang="ts">
  // History's list pane, "Branches" (arch-1-09: split out of GraphView.svelte):
  // LOCAL / REMOTE branches grouped into collapsible prefix folders under a
  // per-section row budget, TAGS, then the stash / worktree / submodule lists
  // (RefSideLists) — a ListPane of SideSections whose rows draw their own rich
  // content on the shared .side-row classes. A branch or tag click jumps the
  // graph to it (graphSelection; on a phone it pushes the graph), a
  // double-click checks it out (or opens the worktree holding it), right-click
  // and ⋯ open its menu, and dragging a branch onto a local branch asks the
  // parent to merge (refDrag). It renders no graph state of its own.
  import type { RefBranch, RefsResp, RepoStatusResp, StashInfo, SubmoduleInfo, WorktreeInfo } from '../../../lib/api/types';
  import { rowMenu } from '../../../lib/rowMenu';
  import Icon from '../../../lib/components/Icon.svelte';
  import ListPane from '../../../lib/components/side/ListPane.svelte';
  import SideSection from '../../../lib/components/side/SideSection.svelte';
  import { gitPhoneMenu } from '../gitPhoneMenu.svelte';
  import { branchTracking } from '../refTracking';
  import { planSection, type SectionPlan as Plan } from '../ref-budget';
  import { groupBranches, midParts, type BranchFolder, type BranchLeaf } from '../ref-menus';
  import RefSideLists from './RefSideLists.svelte';
  import RefDialogs from './RefDialogs.svelte';
  import { createRefDrag } from './refDrag.svelte';
  import type { GraphActions } from './graphActions.svelte';
  import type { GraphMenus } from './graphMenus';
  import type { GraphRefIndex } from './graphRefIndex.svelte';
  import type { GraphSelection } from './graphSelection.svelte';

  interface Props {
    repoId: string;
    refs: RefsResp | null;
    refsLoading: boolean;
    refsError: string | null;
    onretry: () => void;
    status: RepoStatusResp;
    stashes: StashInfo[];
    worktrees: WorktreeInfo[];
    submodules: SubmoduleInfo[];
    currentRepoPath: string;
    checkoutBusy: string;
    openWtBusy: string;
    index: GraphRefIndex;
    actions: GraphActions;
    menus: GraphMenus;
    selection: GraphSelection;
    onmergerequest?: (source: string, target: string) => void;
  }
  let {
    repoId,
    refs,
    refsLoading,
    refsError,
    onretry,
    status,
    stashes,
    worktrees,
    submodules,
    currentRepoPath,
    checkoutBusy,
    openWtBusy,
    index,
    actions,
    menus,
    selection,
    onmergerequest,
  }: Props = $props();

  const drag = createRefDrag({ get onmergerequest() { return onmergerequest; } });

  /** The row last picked ("local:main", "tag:v1", "stash:<ref>"): the list's
   *  selection, still marked after the phone's Back. */
  let picked = $state('');

  // Section collapse state
  let localOpen = $state(true);
  let remoteOpen = $state(true);
  let tagsOpen = $state(false);
  // Collapsed branch FOLDERS (e.g. "local:feature", "remote:release"). Default
  // expanded; a folder key here is collapsed. See groupBranches()/folderKey().
  let collapsedFolders = $state(new Set<string>());
  function folderKey(section: 'local' | 'remote', name: string): string {
    return `${section}:${name}`;
  }
  function toggleFolder(key: string): void {
    const next = new Set(collapsedFolders);
    if (next.has(key)) next.delete(key);
    else next.add(key);
    collapsedFolders = next;
  }

  // Grouped once per `refs` change (not re-sorted inside the template on every
  // render of the section).
  type Grouped = { loose: BranchLeaf[]; folders: BranchFolder[] };
  type SectionPlan = Plan<BranchLeaf>;
  const EMPTY_GROUPS: Grouped = { loose: [], folders: [] };
  const groupedLocal = $derived(refs ? groupBranches(refs.local) : EMPTY_GROUPS);
  const groupedRemote = $derived(refs ? groupBranches(refs.remote) : EMPTY_GROUPS);

  // A repo with thousands of `origin/*` branches must not mount thousands of
  // rows. The budget is per SECTION (LOCAL / REMOTE / TAGS), shared by loose
  // leaves, folder headers and expanded folders' leaves — a per-folder cap
  // bounded nothing when the branches sit under many prefix folders
  // (r3-03-02). Past the budget one "Show more" row grows it.
  const LEAF_PAGE = 150;
  let leafLimits = $state<Record<string, number>>({});
  function leafLimit(key: string): number {
    return leafLimits[key] ?? LEAF_PAGE;
  }
  function showMoreLeaves(key: string): void {
    leafLimits = { ...leafLimits, [key]: leafLimit(key) + LEAF_PAGE * 4 };
  }
  const localPlan = $derived(
    planSection(groupedLocal.loose, groupedLocal.folders, leafLimit('local:'), (n) =>
      collapsedFolders.has(folderKey('local', n)),
    ),
  );
  const remotePlan = $derived(
    planSection(groupedRemote.loose, groupedRemote.folders, leafLimit('remote:'), (n) =>
      collapsedFolders.has(folderKey('remote', n)),
    ),
  );

  /** Local branch DOUBLE-click: open foreign worktree, else checkout. Never try
   *  to checkout a branch already held by another worktree (git errors hard). */
  function activateLocalBranch(b: RefBranch): void {
    if (b.is_current) return;
    selection.holdDiff();
    const wt = index.worktreeByBranch.get(b.name);
    if (wt) {
      void actions.openWorktree(wt);
      return;
    }
    void actions.checkout(b.name, false);
  }
</script>

<!-- Branch leaf name with MIDDLE ellipsis so the distinguishing suffix
     (e.g. "…-retry-fix") stays visible; short names render plain. -->
{#snippet refName(label: string)}
  {@const mp = midParts(label)}
  {#if mp}
    <span class="mono ref-name side-row-title mid"><span class="mid-head">{mp.head}</span><span class="mid-tail">{mp.tail}</span></span>
  {:else}
    <span class="mono ref-name side-row-title">{label}</span>
  {/if}
{/snippet}

<!-- "Already in develop / main" mark: names every integration branch that
     holds the tip (git-flow lands in develop long before main). Never shown
     on an integration branch itself (see landedIn). -->
{#snippet mergedMark(b: RefBranch, into: string[])}
  <span class="merged-mark" title={`Already in ${into.join(', ')}${b.is_current ? '' : ' — safe to delete'}`}>
    <Icon name="check" size={12} /><span class="merged-into">{into.join(' · ')}</span>
  </span>
{/snippet}

<!-- A row's ⋯ (on hover / focus; right-click, long-press and the
     ContextMenu key open the same menu through use:rowMenu). -->
{#snippet more(label: string, open: (e: MouseEvent) => void, disabled = false)}
  <button class="icon-btn side-row-more ref-action" tabindex="-1" aria-label={label} title={label} aria-haspopup="menu" {disabled} onclick={open}><Icon name="more" size={14} /></button>
{/snippet}

<!-- One branch row, reused for loose branches and folder children. `label`
     is the short leaf name; `b.name` stays the full ref for every action. -->
{#snippet localRow(leaf: BranchLeaf, nested: boolean)}
  {@const b = leaf.b}
  {@const into = index.landedIn(b)}
  {@const tracking = branchTracking(b, status)}
  {@const wtElsewhere = index.worktreeByBranch.get(b.name)}
  {@const key = `local:${b.name}`}
  <div class="side-row ref-action-row" class:selected={picked === key} style:--side-depth={nested ? 1 : 0}>
  <button use:rowMenu
    class="side-row-main ref-row"
    data-side-row
    role="treeitem"
    aria-selected={picked === key}
    aria-level={nested ? 2 : 1}
    class:current={b.is_current}
    class:is-worktree={!!wtElsewhere}
    class:drag-target={drag.overTarget === b.name}
    class:dragging={drag.source?.name === b.name && !drag.source.remote}
    draggable={checkoutBusy === '' && openWtBusy === ''}
    ondragstart={(e) => drag.start(e, b.name, false)}
    ondragend={drag.end}
    ondragover={(e) => drag.over(e, b.name)}
    ondragleave={() => drag.leave(b.name)}
    ondrop={(e) => drag.drop(e, b.name)}
    disabled={checkoutBusy !== '' || openWtBusy !== ''}
    onclick={() => { picked = key; void selection.selectBranchRow(b); }}
    ondblclick={() => activateLocalBranch(b)}
    oncontextmenu={(e) => menus.branchMenu(e, b)}
    title={drag.source && drag.isValidTarget(b.name)
      ? `Merge ${drag.sourceName()} → ${b.name}`
      : wtElsewhere
        ? `Double-click to open worktree · ${wtElsewhere.path}${wtElsewhere.branch ? ` · ${wtElsewhere.branch}` : ''}`
        : `${b.name} — click to highlight, double-click to checkout`}
  >
    {#if b.is_current}
      <span class="cur-pip" title="Checked out"><Icon name="check" size={12} /></span>
    {:else if wtElsewhere}
      <span class="cur-pip wt-pip" title="Worktree — click to open"><Icon name="worktree" size={12} /></span>
    {:else}
      <Icon name="dot" size={12} />
    {/if}
    {@render refName(leaf.label)}
    {#if into.length > 0}{@render mergedMark(b, into)}{/if}
    {#if wtElsewhere}
      <span class="wt-open-hint" title="Open worktree (do not switch branch)">open worktree</span>
    {/if}
    {#if tracking.ahead > 0 || tracking.behind > 0}
      <span class="ref-ab" title="{tracking.ahead} ahead · {tracking.behind} behind upstream">
        {#if tracking.ahead > 0}<span class="ab-ahead">↑{tracking.ahead}</span>{/if}
        {#if tracking.behind > 0}<span class="ab-behind">↓{tracking.behind}</span>{/if}
      </span>
    {/if}
    {#if b.upstream}
      {#if b.upstream.endsWith(`/${b.name}`)}
        <!-- Default same-named tracking (e.g. origin/<branch>): redundant,
             so collapse to a small "tracked" glyph and leave the row's
             width for the branch NAME instead of a duplicate string. -->
        <span class="ref-track" title={`tracks ${b.upstream}`}><Icon name="globe" size={12} /></span>
      {:else}
        <!-- Non-default upstream (tracks a differently-named branch) → show it. -->
        <span class="ref-upstream mono dim" title={`upstream: ${b.upstream}`}>{b.upstream}</span>
      {/if}
    {/if}
    {#if checkoutBusy === b.name || (wtElsewhere && openWtBusy === wtElsewhere.path)}<span class="dim" title="Working…">…</span>{/if}
  </button>
  {@render more(`Actions for branch ${b.name}`, (e) => menus.branchMenu(e, b), checkoutBusy !== '' || openWtBusy !== '')}
  </div>
{/snippet}

{#snippet remoteRow(leaf: BranchLeaf, nested: boolean)}
  {@const b = leaf.b}
  {@const into = index.landedIn(b)}
  {@const key = `remote:${b.name}`}
  <div class="side-row ref-action-row" class:selected={picked === key} style:--side-depth={nested ? 1 : 0}>
  <button use:rowMenu
    class="side-row-main ref-row remote"
    data-side-row
    role="treeitem"
    aria-selected={picked === key}
    aria-level={nested ? 2 : 1}
    class:dragging={drag.source?.name === b.name && drag.source.remote}
    draggable={checkoutBusy === ''}
    ondragstart={(e) => drag.start(e, b.name, true)}
    ondragend={drag.end}
    disabled={checkoutBusy !== ''}
    onclick={() => { picked = key; void selection.selectBranchRow(b); }}
    ondblclick={() => actions.checkoutRemote(b)}
    oncontextmenu={(e) => menus.branchMenu(e, b)}
    title="{b.name} — click to highlight · double-click to checkout as a local tracking branch · drag onto a local branch to merge"
  >
    <Icon name="dot" size={12} />
    {@render refName(leaf.label)}
    {#if into.length > 0}{@render mergedMark(b, into)}{/if}
    {#if checkoutBusy === b.name.replace(/^[^/]+\//, '')}<span class="dim">…</span>{/if}
  </button>
  {@render more(`Actions for branch ${b.name}`, (e) => menus.branchMenu(e, b), checkoutBusy !== '' || openWtBusy !== '')}
  </div>
{/snippet}

<!-- A collapsible branch FOLDER (feature/, release/, …): a tree node. -->
{#snippet folderHeader(section: 'local' | 'remote', folder: BranchFolder)}
  {@const key = folderKey(section, folder.name)}
  {@const folded = collapsedFolders.has(key)}
  <div class="side-row">
    <button class="side-row-main ref-folder" data-side-row role="treeitem" aria-selected={false} aria-level={1} aria-expanded={!folded} onclick={() => toggleFolder(key)} title={folder.name}>
      <Icon name={folded ? 'chevronRight' : 'chevronDown'} size={12} />
      <Icon name="folder" size={12} />
      <span class="side-row-title folder-name">{folder.name}</span>
      <span class="side-row-trailing">{folder.leaves.length}</span>
    </button>
  </div>
{/snippet}

<!-- "Show more" for a section past its row budget. -->
{#snippet moreLeaves(key: string, hidden: number)}
  {#if hidden > 0}
    <div class="side-row">
      <button class="side-row-main ref-more-leaves" data-side-row role="treeitem" aria-selected={false} aria-level={1} onclick={() => showMoreLeaves(key)}>
        <span class="dim">Show {Math.min(hidden, LEAF_PAGE * 4)} more ({hidden} hidden)</span>
      </button>
    </div>
  {/if}
{/snippet}

<!-- Bulk cleanup: the section's own action. -->
{#snippet cleanup(section: 'local' | 'remote', list: RefBranch[])}
  <button class="icon-btn" title="Delete several {section} branches…" aria-label="Delete several {section} branches"
    disabled={list.length === 0 || checkoutBusy !== ''} onclick={() => actions.openBranchCleanup([], section)}><Icon name="trash" size={12} /></button>
{/snippet}

{#snippet branchRows(section: 'local' | 'remote', list: RefBranch[], grouped: Grouped, plan: SectionPlan)}
  {#if list.length === 0}
    <div class="dim ref-empty">{section === 'local' ? 'No local branches' : 'No remote branches'}</div>
  {:else}
    <div role="tree" aria-label={section === 'local' ? 'Local branches' : 'Remote branches'}>
      {#each grouped.loose.slice(0, plan.loose) as leaf (leaf.b.name)}
        {@render (section === 'local' ? localRow : remoteRow)(leaf, false)}
      {/each}
      {#each plan.folders as { folder, shown } (folder.name)}
        {@render folderHeader(section, folder)}
        {#if !collapsedFolders.has(folderKey(section, folder.name)) && shown > 0}
          {#each folder.leaves.slice(0, shown) as leaf (leaf.b.name)}
            {@render (section === 'local' ? localRow : remoteRow)(leaf, true)}
          {/each}
        {/if}
      {/each}
      {@render moreLeaves(`${section}:`, plan.hidden)}
    </div>
  {/if}
{/snippet}

<ListPane
  count={refs ? refs.local.length + refs.remote.length + refs.tags.length : null}
  menu={() => gitPhoneMenu.items()}
  loading={refsLoading}
  error={refsError}
  {onretry}
  testid="git-refs"
>
  {#if refs}
    <div class="refs-panel">
      <SideSection title="Local" count={refs.local.length} bind:open={localOpen}>
        {#snippet actions()}{@render cleanup('local', refs.local)}{/snippet}
        {@render branchRows('local', refs.local, groupedLocal, localPlan)}
      </SideSection>
      <SideSection title="Remote" count={refs.remote.length} bind:open={remoteOpen}>
        {#snippet actions()}{@render cleanup('remote', refs.remote)}{/snippet}
        {@render branchRows('remote', refs.remote, groupedRemote, remotePlan)}
      </SideSection>
      <SideSection title="Tags" count={refs.tags.length} bind:open={tagsOpen}>
        {#if refs.tags.length === 0}
          <div class="dim ref-empty">No tags</div>
        {:else}
          <div role="tree" aria-label="Tags">
            {#each refs.tags.slice(0, leafLimit('tags:')) as t (t.name)}
              {@const key = `tag:${t.name}`}
              <div class="side-row ref-action-row" class:selected={picked === key}>
                <button use:rowMenu
                  class="side-row-main ref-row tag"
                  data-side-row
                  role="treeitem"
                  aria-selected={picked === key}
                  aria-level={1}
                  class:ref-row-busy={selection.revealBusy === t.name}
                  title="{t.name} — click to show it on the graph, right-click for actions"
                  onclick={() => { picked = key; void selection.selectTagRow(t); }}
                  oncontextmenu={(e) => menus.tagMenu(e, t)}
                >
                  <Icon name="tag" size={12} />
                  <span class="mono ref-name side-row-title">{t.name}</span>
                </button>
                {@render more(`Actions for tag ${t.name}`, (e) => menus.tagMenu(e, t))}
              </div>
            {/each}
            {@render moreLeaves('tags:', refs.tags.length - Math.min(refs.tags.length, leafLimit('tags:')))}
          </div>
        {/if}
      </SideSection>

      <RefSideLists {stashes} {worktrees} {submodules} {currentRepoPath} {openWtBusy} {actions} {menus} {selection} bind:picked />
    </div>
  {/if}
</ListPane>

<RefDialogs {repoId} {refs} {status} {index} {actions} />

<style>
  /* The rows are the shared .side-row ones (app.css "Side surfaces"); these
     rules only draw what a ref row adds. Written under `.refs-panel :global(…)`
     because RefSideLists (stashes / worktrees / submodules) renders the same
     rows inside this panel. */
  /* Middle-ellipsized branch name: the head span shrinks + ellipsizes while the
     tail (distinguishing suffix) stays pinned and fully visible. */
  .refs-panel :global(.ref-name.mid) {
    display: inline-flex;
    align-items: baseline;
    text-overflow: clip;
  }
  .mid-head {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
    flex: 0 1 auto;
  }
  .mid-tail {
    flex: 0 0 auto;
    white-space: nowrap;
  }
  .refs-panel :global(.ref-name) {
    flex: 1;
    font-size: var(--fs-s);
  }
  /* "Already in develop / main" mark: a small, muted success check + names. */
  .merged-mark {
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    gap: 2px;
    color: var(--success);
    opacity: 0.85;
  }
  .merged-into {
    font-size: var(--fs-xs);
    white-space: nowrap;
  }
  .refs-panel :global(.side-row:hover .merged-mark) {
    opacity: 1;
  }
  /* The folder glyph picks up the accent so the grouping reads at a glance. */
  .ref-folder :global(svg:nth-of-type(2)) {
    color: var(--accent-text);
    opacity: 0.8;
  }
  .folder-name {
    font-weight: 600;
  }
  .refs-panel :global(.ref-row:disabled) {
    cursor: default;
  }
  /* Deep jump in flight (paging history to reach the ref's commit). */
  .refs-panel :global(.ref-row-busy) {
    opacity: 0.6;
    cursor: progress;
  }
  /* The checked-out branch: accent text and the check pip (the tint is the
     list's selection). */
  .refs-panel :global(.ref-row.current) {
    color: var(--accent-text);
  }
  .refs-panel :global(.ref-row[draggable='true']) {
    cursor: grab;
  }
  .refs-panel :global(.ref-row.dragging) {
    opacity: 0.45;
  }
  /* Accent outline while a valid merge source hovers this local branch. */
  .refs-panel :global(.ref-row.drag-target) {
    background: var(--accent-soft);
    outline: 1.5px solid var(--accent-text);
    outline-offset: -1.5px;
  }
  .refs-panel :global(.ref-row.remote),
  .refs-panel :global(.ref-row.tag) {
    color: var(--text-dim);
  }
  /* Compact "also tracked on remote" glyph that replaces a redundant
     origin/<same-name> upstream string, so the branch NAME keeps the row width. */
  .ref-track {
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    color: var(--text-dim);
    opacity: 0.65;
  }
  /* Current-branch marker: a filled accent check pip. */
  .cur-pip {
    display: grid;
    place-items: center;
    flex-shrink: 0;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: var(--accent-solid);
    color: var(--accent-contrast);
  }
  /* Worktree pip: violet folder so it never reads as "checked out here". */
  .cur-pip.wt-pip {
    background: color-mix(in srgb, var(--worktree) 55%, transparent);
    color: var(--worktree-text);
    border: 1px solid color-mix(in srgb, var(--worktree) 50%, transparent);
  }
  /* Branch held by another worktree — distinct from a regular local branch. */
  .refs-panel :global(.ref-row.is-worktree) {
    color: var(--worktree-text);
  }
  /* Inline "open worktree" affordance next to the branch / worktree name. */
  /* Yields first (ellipsized) so a narrow pane keeps the BRANCH name, not 0 px of it. */
  .refs-panel :global(.wt-open-hint) {
    flex: 0 100 auto;
    min-width: 0;
    overflow: hidden; text-overflow: ellipsis;
    font-size: var(--fs-xs);
    font-weight: 600;
    letter-spacing: .06em;
    color: var(--worktree-text);
    opacity: 0.85;
    white-space: nowrap;
  }
  .refs-panel :global(.side-row:hover .wt-open-hint) {
    opacity: 1;
  }
  .ref-ab {
    display: inline-flex;
    gap: 2px;
    flex-shrink: 0;
    font-size: var(--fs-xs);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }
  .ab-ahead {
    color: var(--success);
  }
  .ab-behind {
    color: var(--danger);
  }
  .ref-upstream {
    flex-shrink: 0;
    font-size: var(--fs-xs);
    max-width: 90px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .refs-panel :global(.ref-empty) {
    padding-block: 4px 6px;
    padding-inline: 8px;
    font-size: var(--fs-xs);
  }
  .dim {
    color: var(--text-dim);
  }
  .mono {
    font-family: var(--font-mono);
  }
</style>
