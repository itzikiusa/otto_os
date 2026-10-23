<script lang="ts">
  // The git graph's refs sidebar (arch-1-09: split out of GraphView.svelte):
  // LOCAL / REMOTE branches grouped into collapsible prefix folders under a
  // per-section row budget, TAGS, then the stash / worktree / submodule lists
  // (RefSideLists), plus its drag-to-resize edge. A branch or tag click jumps
  // the graph to it (graphSelection), a double-click checks it out (or opens
  // the worktree holding it), right-click and ⋯ open its menu, and dragging a
  // branch onto a local branch asks the parent to merge (refDrag). It renders
  // no graph state of its own.
  import type { RefBranch, RefsResp, RepoStatusResp, StashInfo, SubmoduleInfo, WorktreeInfo } from '../../../lib/api/types';
  import { rowMenu } from '../../../lib/rowMenu';
  import { splitter } from '../../../lib/paneResizer';
  import { startMouseDrag } from '../../../lib/dragCursor';
  import { ui } from '../../../lib/stores/ui.svelte';
  import Skeleton from '../../../lib/components/Skeleton.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import { branchTracking } from '../refTracking';
  import { planSection, type SectionPlan as Plan } from '../ref-budget';
  import { groupBranches, midParts, type BranchFolder, type BranchLeaf } from '../ref-menus';
  import RefSideLists from './RefSideLists.svelte';
  import { createRefDrag } from './refDrag.svelte';
  import { resizerKey } from './resizerKey';
  import type { GraphActions } from './graphActions.svelte';
  import type { GraphMenus } from './graphMenus';
  import type { GraphRefIndex } from './graphRefIndex.svelte';
  import type { GraphSelection } from './graphSelection.svelte';

  interface Props {
    refs: RefsResp | null;
    refsLoading: boolean;
    refsError: string | null;
    onretry: () => void;
    status: RepoStatusResp;
    stashes: StashInfo[];
    worktrees: WorktreeInfo[];
    submodules: SubmoduleInfo[];
    currentRepoPath: string;
    isMobile: boolean;
    /** The phone accordion's Branches section is folded. */
    collapsed: boolean;
    checkoutBusy: string;
    openWtBusy: string;
    index: GraphRefIndex;
    actions: GraphActions;
    menus: GraphMenus;
    selection: GraphSelection;
    onmergerequest?: (source: string, target: string) => void;
  }
  let {
    refs,
    refsLoading,
    refsError,
    onretry,
    status,
    stashes,
    worktrees,
    submodules,
    currentRepoPath,
    isMobile,
    collapsed,
    checkoutBusy,
    openWtBusy,
    index,
    actions,
    menus,
    selection,
    onmergerequest,
  }: Props = $props();

  const drag = createRefDrag({ get onmergerequest() { return onmergerequest; } });

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

  // ── Drag-resize (desktop; persisted via gitGraphSideWidth) ─────────────────
  // The sidebar holds long branch names; a drag handle on its inline-end edge
  // lets the user widen it to read them.
  let resizing = $state(false);
  function startResize(e: MouseEvent): void {
    resizing = true;
    const startX = e.clientX;
    const startW = ui.gitGraphSideWidth;
    // Under RTL the sidebar sits on the right, so dragging left widens it.
    const dir = document.dir === 'rtl' ? -1 : 1;
    // lib/dragCursor (r3-03-08): overlay cursor instead of an inherited
    // body.style write, one width write per frame, one persist on release.
    startMouseDrag(e, {
      cursor: 'col-resize',
      onMove: (ev) => ui.setGitGraphSideWidth(startW + dir * (ev.clientX - startX), false),
      onEnd: () => {
        resizing = false;
        ui.setGitGraphSideWidth(ui.gitGraphSideWidth);
      },
    });
  }
</script>

<!-- Branch leaf name with MIDDLE ellipsis so the distinguishing suffix
     (e.g. "…-retry-fix") stays visible; short names render plain. -->
{#snippet refName(label: string)}
  {@const mp = midParts(label)}
  {#if mp}
    <span class="mono ref-name mid"><span class="mid-head">{mp.head}</span><span class="mid-tail">{mp.tail}</span></span>
  {:else}
    <span class="mono ref-name">{label}</span>
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

<!-- One branch row, reused for loose branches and folder children. `label`
     is the short leaf name; `b.name` stays the full ref for every action. -->
{#snippet localRow(leaf: BranchLeaf, nested: boolean)}
  {@const b = leaf.b}
  {@const into = index.landedIn(b)}
  {@const tracking = branchTracking(b, status)}
  {@const wtElsewhere = index.worktreeByBranch.get(b.name)}
  <div class="ref-action-row">
  <button use:rowMenu
    class="ref-row"
    class:nested
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
    onclick={() => selection.selectBranchRow(b)}
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
  <button
    class="icon-btn ref-action"
    aria-label="Actions for branch {b.name}"
    title="Actions for branch {b.name}"
    aria-haspopup="menu"
    disabled={checkoutBusy !== '' || openWtBusy !== ''}
    onclick={(e) => menus.branchMenu(e, b)}
  ><Icon name="more" size={13} /></button>
  </div>
{/snippet}

{#snippet remoteRow(leaf: BranchLeaf, nested: boolean)}
  {@const b = leaf.b}
  {@const into = index.landedIn(b)}
  <div class="ref-action-row">
  <button use:rowMenu
    class="ref-row remote"
    class:nested
    class:dragging={drag.source?.name === b.name && drag.source.remote}
    draggable={checkoutBusy === ''}
    ondragstart={(e) => drag.start(e, b.name, true)}
    ondragend={drag.end}
    disabled={checkoutBusy !== ''}
    onclick={() => selection.selectBranchRow(b)}
    ondblclick={() => actions.checkoutRemote(b)}
    oncontextmenu={(e) => menus.branchMenu(e, b)}
    title="{b.name} — click to highlight · double-click to checkout as a local tracking branch · drag onto a local branch to merge"
  >
    <Icon name="dot" size={12} />
    {@render refName(leaf.label)}
    {#if into.length > 0}{@render mergedMark(b, into)}{/if}
    {#if checkoutBusy === b.name.replace(/^[^/]+\//, '')}<span class="dim">…</span>{/if}
  </button>
  <button
    class="icon-btn ref-action"
    aria-label="Actions for branch {b.name}"
    title="Actions for branch {b.name}"
    aria-haspopup="menu"
    disabled={checkoutBusy !== '' || openWtBusy !== ''}
    onclick={(e) => menus.branchMenu(e, b)}
  ><Icon name="more" size={13} /></button>
  </div>
{/snippet}

<!-- A collapsible branch FOLDER header (feature/, release/, …). -->
{#snippet folderHeader(section: 'local' | 'remote', folder: BranchFolder)}
  {@const key = folderKey(section, folder.name)}
  {@const folded = collapsedFolders.has(key)}
  <button class="ref-folder" class:collapsed={folded} onclick={() => toggleFolder(key)} title={folder.name} aria-expanded={!folded}>
    <Icon name={folded ? 'chevronRight' : 'chevronDown'} size={12} />
    <Icon name="folder" size={12} />
    <span class="folder-name">{folder.name}</span>
    <span class="ref-count">{folder.leaves.length}</span>
  </button>
{/snippet}

<!-- "Show more" for a section past its row budget. -->
{#snippet moreLeaves(key: string, hidden: number)}
  {#if hidden > 0}
    <button class="ref-more-leaves" onclick={() => showMoreLeaves(key)}>
      <span class="dim">Show {Math.min(hidden, LEAF_PAGE * 4)} more ({hidden} hidden)</span>
    </button>
  {/if}
{/snippet}

{#snippet branchSection(section: 'local' | 'remote', list: RefBranch[], grouped: Grouped, plan: SectionPlan)}
  {@const open = section === 'local' ? localOpen : remoteOpen}
  <div class="ref-section">
    <button
      class="ref-header"
      onclick={() => (section === 'local' ? (localOpen = !localOpen) : (remoteOpen = !remoteOpen))}
      aria-expanded={open}
    >
      <Icon name={open ? 'chevronDown' : 'chevronRight'} size={12} />
      <Icon name={section === 'local' ? 'branch' : 'globe'} size={12} />
      <span>{section === 'local' ? 'LOCAL' : 'REMOTE'}</span>
      <span class="ref-count">{list.length}</span>
    </button>
    {#if open}
      {#if list.length === 0}
        <div class="dim ref-empty">{section === 'local' ? 'No local branches' : 'No remote branches'}</div>
      {/if}
      {#each grouped.loose.slice(0, plan.loose) as leaf (leaf.b.name)}
        {@render (section === 'local' ? localRow : remoteRow)(leaf, false)}
      {/each}
      {#each plan.folders as { folder, shown } (folder.name)}
        {@render folderHeader(section, folder)}
        {#if !collapsedFolders.has(folderKey(section, folder.name)) && shown > 0}
          <div class="folder-children">
            {#each folder.leaves.slice(0, shown) as leaf (leaf.b.name)}
              {@render (section === 'local' ? localRow : remoteRow)(leaf, true)}
            {/each}
          </div>
        {/if}
      {/each}
      {@render moreLeaves(`${section}:`, plan.hidden)}
    {/if}
  </div>
{/snippet}

<aside
  class="refs-panel"
  class:mob={isMobile}
  class:mob-collapsed={isMobile && collapsed}
  class:resizing
  style:width={!isMobile ? `${ui.gitGraphSideWidth}px` : null}
>
  {#if refsLoading}
    <div style="padding: 10px"><Skeleton rows={6} height={22} /></div>
  {:else if refsError}
    <div class="refs-error">
      <LoadState what="branches" error={refsError} empty variant="compact" {onretry} />
    </div>
  {:else if refs}
    {@render branchSection('local', refs.local, groupedLocal, localPlan)}
    {@render branchSection('remote', refs.remote, groupedRemote, remotePlan)}

    <!-- TAGS -->
    <div class="ref-section">
      <button class="ref-header" onclick={() => (tagsOpen = !tagsOpen)} aria-expanded={tagsOpen}>
        <Icon name={tagsOpen ? 'chevronDown' : 'chevronRight'} size={12} />
        <Icon name="tag" size={12} />
        <span>TAGS</span>
        <span class="ref-count">{refs.tags.length}</span>
      </button>
      {#if tagsOpen}
        {#each refs.tags.slice(0, leafLimit('tags:')) as t (t.name)}
          <div class="ref-action-row">
          <button use:rowMenu
            class="ref-row tag"
            class:ref-row-busy={selection.revealBusy === t.name}
            title="{t.name} — click to show it on the graph, right-click for actions"
            onclick={() => selection.selectTagRow(t)}
            oncontextmenu={(e) => menus.tagMenu(e, t)}
          >
            <Icon name="tag" size={12} />
            <span class="mono ref-name">{t.name}</span>
          </button>
          <!-- Actions stay reachable without a right-click (trackpad/touch),
               since left-click jumps to the tag instead of opening them: a
               real sibling button (never a control nested in the row). -->
          <button
            class="icon-btn ref-action ref-more"
            title="Actions for tag {t.name}"
            aria-label="Actions for tag {t.name}"
            aria-haspopup="menu"
            onclick={(e) => menus.tagMenu(e, t)}
          ><Icon name="more" size={13} /></button>
          </div>
        {:else}
          <div class="dim ref-empty">No tags</div>
        {/each}
        {@render moreLeaves('tags:', refs.tags.length - Math.min(refs.tags.length, leafLimit('tags:')))}
      {/if}
    </div>

    <RefSideLists {stashes} {worktrees} {submodules} {currentRepoPath} {openWtBusy} {actions} {menus} {selection} />
  {/if}
</aside>

<!-- Drag handle on the sidebar's inline-end edge (desktop): widen it to read
     long branch names. Double-click resets to the default width. A focusable
     separator is the ARIA window-splitter widget; `splitter` makes it
     focusable and wires its keys and drag (lib/paneResizer). -->
{#if !isMobile}
  <div
    class="refs-resizer"
    role="separator"
    aria-orientation="vertical"
    aria-label="Resize the branch list"
    aria-valuenow={ui.gitGraphSideWidth}
    use:splitter={{
      onkeydown: (e) => resizerKey(e, () => ui.gitGraphSideWidth, (w) => ui.setGitGraphSideWidth(w), 220, true),
      onmousedown: startResize,
      ondblclick: () => ui.setGitGraphSideWidth(220),
    }}
    title="Drag or use ←/→ to resize · double-click to reset"
  ></div>
{/if}

<style>
  /* Row rules are written under `.refs-panel :global(…)` because RefSideLists
     (stashes / worktrees / submodules) renders the same rows inside this
     panel. Each keeps its original order, so the cascade is unchanged. */
  .refs-panel {
    width: 220px;
    flex-shrink: 0;
    overflow-y: auto;
    border-inline-end: 1px solid var(--border);
    padding: 6px 0;
  }
  /* Drag handle on the sidebar's edge (desktop). Mirrors .graph-resizer:
     a thin hit-area straddling the border that lights up on hover. */
  .refs-resizer {
    flex: 0 0 6px;
    margin-inline-start: -2px;
    cursor: col-resize;
    z-index: 1;
  }
  .refs-resizer:hover {
    background: var(--accent-soft-strong);
  }
  .refs-error {
    padding: 8px;
  }
  /* Middle-ellipsized branch name: the head span shrinks + ellipsizes while the
     tail (distinguishing suffix) stays pinned and fully visible. */
  .refs-panel :global(.ref-name.mid) {
    display: inline-flex;
    align-items: baseline;
    flex: 1;
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
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
  .refs-panel :global(.ref-row:hover .merged-mark) {
    opacity: 1;
  }
  .refs-panel :global(.ref-section) {
    margin-bottom: 2px;
  }
  .refs-panel :global(.ref-header) {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    padding: 6px 10px;
    border: none;
    background: transparent;
    color: var(--text-dim);
    font-size: var(--fs-xs);
    font-weight: 600;
    letter-spacing: 0.06em;
    cursor: pointer;
    text-transform: uppercase;
    text-align: start;
    transition: color var(--dur-fast) ease-out;
  }
  .refs-panel :global(.ref-header:hover) {
    color: var(--text);
  }
  .refs-panel :global(.ref-count) {
    margin-inline-start: auto;
    background: var(--surface-2);
    border-radius: 999px;
    font-size: var(--fs-xs);
    padding: 1px 6px;
    font-weight: 600;
    letter-spacing: 0;
  }
  /* Collapsible branch-folder header (feature/, release/, …). The folder icon +
     twisty + medium-weight name set it apart from the leaf branch rows so the
     nesting reads as a hierarchy, not a flat sibling list. */
  .ref-folder {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    /* Starts on the SAME indent as a loose branch row (22px): a folder is a
       sibling of the loose branches, not of the LOCAL/REMOTE section header. */
    padding-block: 4px;
    padding-inline: 22px 10px;
    border: none;
    background: transparent;
    color: var(--text);
    font-size: var(--fs-xs);
    cursor: pointer;
    text-align: start;
    transition: background var(--dur-fast) ease-out, color var(--dur-fast) ease-out;
  }
  .ref-folder:hover {
    background: var(--hover);
  }
  /* The count pill is surface-2 too — lift it so it doesn't vanish on hover. */
  .refs-panel .ref-folder:hover :global(.ref-count) {
    background: var(--surface-3);
  }
  /* The folder glyph picks up the accent so the grouping is obvious at a glance. */
  .ref-folder :global(svg:nth-of-type(2)) {
    color: var(--accent-text);
    opacity: 0.8;
  }
  .folder-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-weight: 600;
  }
  /* Children of a folder: indented under a vertical tree guide so the nesting is
     visually unmistakable. The guide drops from the centre of the folder's
     11px chevron (22 + 5.5), and nested rows' dots line up under its icon. */
  .folder-children {
    margin-inline-start: 28px;
    border-inline-start: 1.5px solid var(--border);
  }
  .refs-panel :global(.ref-action-row) {
    display: flex;
    align-items: stretch;
    min-width: 0;
  }
  .refs-panel :global(.ref-action-row .ref-row) {
    flex: 1;
    min-width: 0;
    width: 0;
  }
  .refs-panel :global(.ref-action) {
    flex-shrink: 0;
    align-self: center;
  }
  @media (max-width: 1024px) {
    .refs-panel :global(.ref-action) { min-width: 36px; min-height: 36px; }
  }
  .refs-panel :global(.ref-row) {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    height: 24px;
    padding-block: 0;
    padding-inline: 22px 10px;
    border: none;
    background: transparent;
    color: var(--text-dim);
    font-size: var(--fs-s);
    cursor: pointer;
    text-align: start;
    overflow: hidden;
    transition: background var(--dur-fast) ease-out, color var(--dur-fast) ease-out;
  }
  .ref-more-leaves {
    display: flex;
    align-items: center;
    width: 100%;
    height: 24px;
    padding-block: 0;
    padding-inline: 22px 10px;
    border: none;
    background: transparent;
    font-size: var(--fs-xs);
    cursor: pointer;
    text-align: start;
  }
  .ref-more-leaves:hover {
    background: var(--hover);
  }
  .refs-panel :global(.ref-row:hover:not(:disabled)) {
    background: var(--hover);
    color: var(--text);
  }
  .refs-panel :global(.ref-row:disabled) {
    cursor: default;
  }
  /* Deep jump in flight (paging history to reach the ref's commit). */
  .refs-panel :global(.ref-row-busy) {
    opacity: 0.6;
    cursor: progress;
  }
  /* Tag actions: revealed on hover/focus so the row stays clean, but always
     present for pointers that have no right-click. */
  .refs-panel :global(.ref-more) {
    opacity: 0;
    transition: opacity var(--dur-fast) ease-out;
  }
  .refs-panel :global(.ref-action-row:hover .ref-more),
  .refs-panel :global(.ref-action-row:focus-within .ref-more),
  .refs-panel :global(.ref-more:focus-visible) {
    opacity: 1;
  }
  /* A pointer that cannot hover keeps the tag actions visible. */
  @media (hover: none) {
    .refs-panel :global(.ref-more) {
      opacity: 1;
    }
  }
  /* Folder children sit just inside the tree guide of `.folder-children`. */
  .refs-panel :global(.ref-row.nested) {
    padding-inline-start: 10px;
  }
  /* The checked-out branch: accent text + a leading accent rail and faint wash so
     the row itself is unmistakable, not just the check pip. */
  .refs-panel :global(.ref-row.current) {
    color: var(--accent-text);
    font-weight: 600;
    background: var(--accent-soft);
    box-shadow: inset 2px 0 0 0 var(--accent);
  }
  .refs-panel :global(.ref-row.current:hover:not(:disabled)) {
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  :global([dir='rtl']) .refs-panel :global(.ref-row.current) {
    box-shadow: inset -2px 0 0 0 var(--accent);
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
    color: var(--text);
    outline: 1.5px solid var(--accent-text);
    outline-offset: -1.5px;
    border-radius: var(--radius-s);
  }
  .refs-panel :global(.ref-row.tag) {
    cursor: default;
    color: var(--text-dim);
  }
  .refs-panel :global(.ref-row.remote) {
    color: var(--text-dim);
  }
  .refs-panel :global(.ref-name) {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--fs-xs);
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
  .refs-panel :global(.ref-row.is-worktree:hover:not(:disabled)) {
    background: color-mix(in srgb, var(--worktree) 12%, transparent);
  }
  /* Inline "open worktree" affordance next to the branch / worktree name. */
  .refs-panel :global(.wt-open-hint) {
    flex-shrink: 0;
    font-size: var(--fs-xs);
    font-weight: 600;
    letter-spacing: .06em;
    color: var(--worktree-text);
    opacity: 0.85;
    white-space: nowrap;
  }
  .refs-panel :global(.ref-row:hover .wt-open-hint) {
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
    padding: 4px 22px 6px;
    font-size: var(--fs-xs);
  }
  .dim {
    color: var(--text-dim);
  }
  .mono {
    font-family: var(--font-mono);
  }

  /* ── Mobile + tablet (≤1024px): the accordion's Branches section ── */
  @media (max-width: 1024px) {
    /* Capped height, scrolls on its own when expanded. */
    .refs-panel.mob {
      width: 100%;
      flex: 0 0 auto;
      max-height: 38vh;
      overflow-y: auto;
      overscroll-behavior: contain;
      border-inline-end: none;
      border-bottom: 1px solid var(--border);
    }
    /* A collapsed section hides its body entirely (header stays tappable). */
    .refs-panel.mob.mob-collapsed {
      display: none !important;
    }
    /* Bigger tap targets + legible text. */
    .refs-panel.mob :global(.ref-row) { height: 36px; font-size: var(--fs-l); }
    .refs-panel.mob :global(.ref-name) { font-size: var(--fs-m); }
    .refs-panel.mob :global(.ref-header) { font-size: var(--fs-s); padding: 8px 12px; }
  }
</style>
