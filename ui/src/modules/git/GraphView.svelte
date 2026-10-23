<script lang="ts">
  // Three panes: LEFT = refs tree (graph/RefSidebar), MIDDLE = the commit graph
  // (this file), RIGHT = commit detail / diff or the WIP panel
  // (graph/CommitDetailPane). On a phone they stack as accordion sections.
  // The state lives in graph/*.svelte.ts behind narrow hosts: the data and
  // its (re)loading (graphData), what the refs mean (graphRefIndex), the
  // row window and lane layout (graphViewport), the selection and reveal
  // (graphSelection), the actions (graphActions), the menus (graphMenus) and
  // the ref popover (refPopoverState). This file wires them and renders rows.
  import { rowMenu } from '../../lib/rowMenu';
  import { plural } from '../../lib/plural';
  import { splitter } from '../../lib/paneResizer';
  import type { RepoStatusResp } from '../../lib/api/types';
  import { ui } from '../../lib/stores/ui.svelte';
  import { startMouseDrag } from '../../lib/dragCursor';
  import { git } from '../../lib/stores/git.svelte';
  import { rel } from '../../lib/stores/now.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import CreatePr from './CreatePr.svelte';
  import { LANE_W, NODE_R, type LaneRow } from './graph-lanes';
  import { branchTracking } from './refTracking';
  import { normPath, primaryChip, shouldCollapseRow, stashShortMsg, type RefChip as Chip } from './ref-menus';
  import { createGraphActions } from './graph/graphActions.svelte';
  import { createGraphMenus } from './graph/graphMenus';
  import { createGraphData } from './graph/graphData.svelte';
  import { createGraphRefIndex } from './graph/graphRefIndex.svelte';
  import { createGraphViewport } from './graph/graphViewport.svelte';
  import { createGraphSelection } from './graph/graphSelection.svelte';
  import { createRefPopover } from './graph/refPopoverState.svelte';
  import { resizerKey } from './graph/resizerKey';
  import RefSidebar from './graph/RefSidebar.svelte';
  import RefChip from './graph/RefChip.svelte';
  import LazyMount from '../../lib/components/LazyMount.svelte';
  import { lazyComponent } from '../../lib/lazy-component.svelte';
  import CommitDetailPane from './graph/CommitDetailPane.svelte';

  interface Props {
    repoId: string;
    /** Absolute path of this repo tab (main or linked worktree). Used to tell
     *  "branch lives in another worktree" vs "this checkout". */
    repoPath?: string;
    /** Workspace that owns this repo — used when registering a worktree path. */
    workspaceId?: string;
    status: RepoStatusResp;
    onstatus: (s: RepoStatusResp) => void;
    /** Drag-to-merge: dropping branch `source` onto a different local branch
     *  `target` asks the parent to open the merge approval modal. Nothing merges
     *  here — the parent owns the modal + conflict-resolver routing. */
    onmergerequest?: (source: string, target: string) => void;
    /** Open the parent's conflict-resolver view (WIP panel's conflicted rows). */
    onresolveconflicts?: () => void;
  }
  let {
    repoId,
    repoPath = '',
    workspaceId = '',
    status,
    onstatus,
    onmergerequest,
    onresolveconflicts,
  }: Props = $props();

  // Which phone accordion sections are expanded. The refs (branches) tree
  // starts collapsed — the commit graph is what users want first — and the
  // diff opens on select.
  let secRefsOpen = $state(false);
  let secCommitsOpen = $state(true);

  let checkoutBusy = $state('');
  /** Path of the worktree currently being opened as a git tab (busy guard). */
  let openWtBusy = $state('');

  /** Absolute path of the repo tab we're viewing (main or linked worktree). */
  const currentRepoPath = $derived(
    normPath(repoPath || git.allRepos.find((r) => r.id === repoId)?.path || ''),
  );

  const data = createGraphData({
    get repoId() { return repoId; },
    onstatus: (s) => onstatus(s),
    graphVisible: () => vp.visible(),
  });
  const index = createGraphRefIndex({
    get refs() { return data.refs; },
    get commits() { return data.commits; },
    get stashes() { return data.stashes; },
    get worktrees() { return data.worktrees; },
    get status() { return status; },
    isForeignWorktree: (w) => actions.isForeignWorktree(w),
  });
  const vp = createGraphViewport({
    get commits() { return data.commits; },
    get laneNames() { return index.laneNames; },
    get loadingMore() { return data.loadingMore; },
    get hasMore() { return data.hasMore; },
    loadMore: () => void data.loadMore(),
    flushResync: () => data.flushResync(),
  });
  const sel = createGraphSelection({
    get repoId() { return repoId; },
    get status() { return status; },
    get isMobile() { return vp.isMobile; },
    get commitsOpen() { return secCommitsOpen; },
    set commitsOpen(v) { secCommitsOpen = v; },
    get refs() { return data.refs; },
    get commits() { return data.commits; },
    get commitsLoading() { return data.commitsLoading; },
    get bySha() { return index.bySha; },
    get firstParentChild() { return index.firstParentChild; },
    has: (sha) => data.has(sha),
    loadUntil: (sha) => data.loadUntil(sha),
    chipsFor: (c) => index.chipsFor(c),
    branchTipCommit: (b) => index.branchTipCommit(b),
    scrollToCommit: (sha) => vp.scrollToCommit(sha),
    focusRow: (sha) => void vp.focusRow(sha),
  });

  // ── Actions and context menus (graph/graphActions, graph/graphMenus) ──────
  const actions = createGraphActions({
    get repoId() { return repoId; },
    get workspaceId() { return workspaceId; },
    get onstatus() { return onstatus; },
    get checkoutBusy() { return checkoutBusy; },
    set checkoutBusy(v) { checkoutBusy = v; },
    get openWtBusy() { return openWtBusy; },
    set openWtBusy(v) { openWtBusy = v; },
    get worktrees() { return data.worktrees; },
    set worktrees(v) { data.worktrees = v; },
    get submodules() { return data.submodules; },
    set submodules(v) { data.submodules = v; },
    get currentRepoPath() { return currentRepoPath; },
    get refKnowledge() { return index.refKnowledge; },
    normPath,
    adoptRefsRev: () => data.adoptRefsRev(),
    refreshAfter: (s) => data.refreshAfter(s),
    refreshAfterCheckout: (s, stashed) => data.refreshAfterCheckout(s, stashed),
    mutate: (path, body, okTitle, okDetail) => data.mutate(path, body, okTitle, okDetail),
    holdDiff: () => sel.holdDiff(),
    stashShortMsg,
  });
  const menus = createGraphMenus(
    {
      get repoId() { return repoId; },
      get onmergerequest() { return onmergerequest; },
      get refs() { return data.refs; },
      get selectedCommit() { return sel.selectedCommit; },
      get worktreeByBranch() { return index.worktreeByBranch; },
      get worktreeBranches() { return index.worktreeBranches; },
      get refKnowledge() { return index.refKnowledge; },
      get baseBranch() { return index.baseBranch; },
      openCreatePr,
      chipsFor: (c) => index.chipsFor(c),
      setCleanupBase: (name) => data.setCleanupBase(name),
    },
    actions,
  );
  const { commitMenu, fileToolsMenu } = menus;

  // The "▾ +N" popover (graph/refPopoverState); its view loads on first open.
  const RefPopover = lazyComponent(() => import('./graph/RefPopover.svelte'));
  const refPop = createRefPopover({
    get refs() { return data.refs; },
    get selectedSha() { return sel.selectedSha; },
    get worktreeByBranch() { return index.worktreeByBranch; },
    selectCommit: (c, opts) => sel.selectCommit(c, opts),
    holdDiff: () => sel.holdDiff(),
    checkout: (name, create) => actions.checkout(name, create),
    checkoutRemoteRef: (label) => actions.checkoutRemoteRef(label),
    openWorktree: (w) => actions.openWorktree(w),
    branchMenu: (e, b) => menus.branchMenu(e, b),
    tagMenu: (e, t) => menus.tagMenu(e, t),
  });

  // ── Create-PR sheet (opened from branch / commit menus with a pre-filled
  //    source). Reuses CreatePr.svelte; `initialSource` defaults the dropdown. ──
  let createPrOpen = $state(false);
  let createPrSource = $state<string | undefined>(undefined);
  function openCreatePr(source: string): void {
    createPrSource = source;
    createPrOpen = true;
  }

  // WIP row: sits on the HEAD commit's lane so the dashed stub visually hangs
  // off the checked-out branch (falls back to lane 0 while the log loads).
  const wipCol = $derived(vp.laneRows.find((r) => index.isHeadCommit(r.commit))?.col ?? 0);
  const wipStagedCount = $derived(status.changes.filter((c) => c.staged).length);
  const wipConflictCount = $derived(status.changes.filter((c) => c.kind === 'conflicted').length);

  /** The shared relative-time label (content.md §7), compact for dense rows. */
  function fmtDate(iso: string): string {
    return rel(iso, { compact: true });
  }
  function fmtDateTitle(iso: string): string {
    const d = new Date(iso);
    return Number.isNaN(d.getTime()) ? '' : d.toLocaleString();
  }

  // Drag-to-resize the commit-list column while the detail panel is open
  // (desktop only; persisted via ui.gitGraphListWidth).
  let listResizing = $state(false); // suspends the panel's flex transition
  function startListResize(e: MouseEvent): void {
    listResizing = true;
    const startX = e.clientX;
    const startW = ui.gitGraphListWidth;
    startMouseDrag(e, {
      cursor: 'col-resize',
      onMove: (ev) => ui.setGitGraphListWidth(startW + (ev.clientX - startX), false),
      onEnd: () => {
        listResizing = false;
        ui.setGitGraphListWidth(ui.gitGraphListWidth);
      },
    });
  }
</script>

<!-- One commit-row ref chip — shared by the inline (single ref) and collapsed
     (primary ref) renderings so they never drift apart. -->
{#snippet chipView(chip: Chip, label: string, laneColor: string)}
  <RefChip
    {chip}
    {label}
    lane={laneColor}
    tracking={branchTracking(chip.kind === 'head' || chip.kind === 'local' ? index.localBranchByName.get(chip.label) : undefined, status)}
    title={chip.kind === 'stash'
      ? `Stash · ${label}`
      : chip.current
        ? `Checked out · ${chip.label}`
        : chip.worktree
          ? `Worktree · ${chip.label} — open worktree (do not switch branch)`
          : chip.label}
  />
{/snippet}

<div class="graphview" class:mobile={vp.isMobile}>
  <!-- ── LEFT: refs tree ───────────────────────────────────────────────────── -->
  {#if vp.isMobile}
    <!-- Mobile accordion header for the branches/refs tree. -->
    <button
      class="mob-sec-head"
      class:open={secRefsOpen}
      onclick={() => (secRefsOpen = !secRefsOpen)}
      aria-expanded={secRefsOpen}
    >
      <Icon name={secRefsOpen ? 'chevronDown' : 'chevronRight'} size={13} />
      <Icon name="branch" size={13} />
      <span>Branches &amp; Tags</span>
      <span class="grow"></span>
      {#if data.refs}<span class="mob-sec-count">{data.refs.local.length + data.refs.remote.length + data.refs.tags.length}</span>{/if}
    </button>
  {/if}
  <RefSidebar
    refs={data.refs}
    refsLoading={data.refsLoading}
    refsError={data.refsError}
    onretry={() => data.loadRefs(repoId)}
    {status}
    stashes={data.stashes}
    worktrees={data.worktrees}
    submodules={data.submodules}
    {currentRepoPath}
    isMobile={vp.isMobile}
    collapsed={!secRefsOpen}
    {checkoutBusy}
    {openWtBusy}
    {index}
    {actions}
    {menus}
    selection={sel}
    {onmergerequest}
  />

  <!-- ── MIDDLE: commit graph ─────────────────────────────────────────────── -->
  {#if vp.isMobile}
    <button
      class="mob-sec-head"
      class:open={secCommitsOpen}
      onclick={() => (secCommitsOpen = !secCommitsOpen)}
      aria-expanded={secCommitsOpen}
    >
      <Icon name={secCommitsOpen ? 'chevronDown' : 'chevronRight'} size={13} />
      <Icon name="commit" size={13} />
      <span>Commits</span>
      <span class="grow"></span>
      {#if !data.commitsLoading}<span class="mob-sec-count">{data.commits.length}</span>{/if}
    </button>
  {/if}
  <div
    class="graph-panel"
    class:panel-shrunk={sel.detailOpen && !vp.isMobile}
    class:mob-collapsed={vp.isMobile && !secCommitsOpen}
    class:resizing={listResizing}
    style:flex-basis={sel.detailOpen && !vp.isMobile ? `${ui.gitGraphListWidth}px` : null}
    bind:this={vp.panelEl}
    onscroll={vp.onScroll}
  >
    {#if data.commitsError && data.commits.length === 0}
      <LoadState what="commits" error={data.commitsError} empty loading={data.commitsLoading} onretry={() => data.retryCommits()} />
    {:else if data.commitsLoading}
      <div style="padding: 10px"><Skeleton rows={12} height={vp.rowH} /></div>
    {:else if data.commits.length === 0}
      <div class="dim" style="padding: 18px; font-size: var(--fs-s)">No commits yet — changes you commit from the WIP row appear here.</div>
    {:else}
      {#if data.refreshError}
        <div class="graph-stale">
          <LoadState what="the graph" error={data.refreshError} onretry={() => void data.reloadGraph()} />
        </div>
      {:else if data.refsStaleError && data.commits.length > 0}
        <!-- Two background ref reads in a row failed: branches may be stale. -->
        <div class="graph-stale">
          <LoadState what="branches" error={data.refsStaleError} onretry={() => void data.refreshAfter().catch(data.noteBackgroundRead)} />
        </div>
      {/if}
      <div class="graph-list">
        <!-- Column header — orients the three zones (which column holds refs,
             which holds the graph) so the ref gutter isn't read as part of the
             commit text. Sticky so it survives scrolling a long history. -->
        <div class="graph-head" aria-hidden="true">
          <span class="gh-branch" title="Branch / tag">BRANCH / TAG</span>
          <!-- The label is hidden (not clipped to "GRA") when a 1-lane gutter is
               too narrow to hold it at the readable size. -->
          <span class="gh-graph" class:gh-label-hidden={vp.gutterWidth < 40} style="width: {vp.gutterWidth}px" title="Graph">GRAPH</span>
          <span class="gh-msg">COMMIT MESSAGE</span>
        </div>
        <!-- WIP row (GitKraken-style): uncommitted changes pinned above the
             graph; selecting it opens the staging/commit panel on the right. -->
        {#if status.changes.length > 0}
          {@const cx = wipCol * LANE_W + LANE_W / 2}
          {@const rowH = vp.rowH}
          <button
            class="graph-row wip-row"
            style:height="{rowH}px"
            class:graph-row-selected={sel.wipSelected}
            onclick={sel.selectWip}
            title="Uncommitted changes — click to stage & commit"
            aria-pressed={sel.wipSelected}
          >
            <div class="branch-cell">
              <span class="wip-chip mono">WIP</span>
            </div>
            <svg class="gutter" width={vp.gutterWidth} height={rowH} style="flex-shrink: 0; width: {vp.gutterWidth}px;">
              <!-- dashed stub hanging toward the HEAD commit's lane -->
              <line x1={cx} y1={rowH / 2 + NODE_R} x2={cx} y2={rowH} stroke="var(--accent)" stroke-width="1.5" stroke-dasharray="2 2" opacity="0.7" />
              {#if sel.wipSelected}
                <circle cx={cx} cy={rowH / 2} r={NODE_R + 3} fill="none" stroke="var(--accent)" stroke-width="1.5" opacity="0.55" />
              {/if}
              <circle cx={cx} cy={rowH / 2} r={NODE_R} fill="var(--surface)" stroke="var(--accent)" stroke-width="1.5" stroke-dasharray="2.5 1.8" />
            </svg>
            <div class="commit-info">
              <div class="ci-top">
                <span class="ci-subject wip-subject mono">// WIP</span>
              </div>
              <div class="ci-meta">
                <span class="dim">
                  {plural(status.changes.length, 'file')} changed{wipStagedCount > 0 ? ` · ${wipStagedCount} staged` : ''}
                </span>
                {#if wipConflictCount > 0}
                  <span class="wip-conflicts" title="{plural(wipConflictCount, 'conflicted file')} — open the WIP panel to resolve">
                    <Icon name="warning" size={12} /> {wipConflictCount} conflicted
                  </span>
                {/if}
              </div>
            </div>
          </button>
        {/if}
        <!-- Windowed rows: the spacers stand in for everything scrolled past /
             not yet reached, so the scrollbar reflects the FULL history while
             only ~a screenful of rows exists in the DOM. -->
        <div bind:this={vp.topSpacerEl} class="row-spacer" style="height: {vp.padTop}px" aria-hidden="true"></div>
        {#each vp.visibleRows as row (row.commit.sha)}
          {@render commitRow(row)}
        {/each}
        <div class="row-spacer" style="height: {vp.padBottom}px" aria-hidden="true"></div>
        <!-- History footer: more pages stream in on scroll, so this is a status
             line (and a manual fallback for anyone who can't scroll-trigger it),
             never a cap. -->
        <div class="graph-more">
          {#if data.loadingMore}
            <span class="spinner" role="img" aria-label="Loading more history" title="Loading more history"></span>
          {:else if data.hasMore}
            <button class="btn small" onclick={() => void data.loadMore()}>
              Load older commits
            </button>
          {:else}
            <span class="dim">{plural(data.commits.length, 'commit')} · beginning of history</span>
          {/if}
        </div>
      </div>
    {/if}
  </div>

  <!-- Create-PR sheet, opened from a branch/commit context menu. -->
  {#if createPrOpen}
    <CreatePr
      {repoId}
      initialSource={createPrSource}
      onclose={() => (createPrOpen = false)}
      oncreated={() => {
        createPrOpen = false;
        void data.refreshAfter();
      }}
    />
  {/if}

  <!-- ── RIGHT: commit detail + diff / WIP staging panel ─────────────────── -->
  {#if sel.detailOpen && !vp.isMobile}
    <div
      class="graph-resizer"
      role="separator"
      aria-orientation="vertical"
      aria-label="Resize the commit list"
      aria-valuenow={ui.gitGraphListWidth}
      use:splitter={{
        onkeydown: (e) => resizerKey(e, () => ui.gitGraphListWidth, (w) => ui.setGitGraphListWidth(w), 420),
        onmousedown: startListResize,
        ondblclick: () => ui.setGitGraphListWidth(420),
      }}
      title="Drag or use ←/→ to resize · double-click to reset"
    ></div>
  {/if}

  {#if vp.isMobile && sel.detailOpen}
    <!-- Mobile: a sticky section header for the open commit's diff (or the WIP
         panel), with a tap target to close it (re-opening the commit list). -->
    <button class="mob-sec-head mob-diff-head" onclick={sel.closeDetail} aria-expanded="true">
      <Icon name="chevronDown" size={13} />
      <Icon name="file" size={13} />
      <span class="mob-diff-title mono">{sel.wipSelected ? '// WIP' : (sel.selectedCommit?.short_sha ?? 'Diff')}</span>
      <span class="grow"></span>
      {#if sel.wipSelected}
        <span class="mob-sec-count">{plural(status.changes.length, 'file')}</span>
      {:else if sel.diffFileCount !== null}
        <span class="mob-sec-count">{plural(sel.diffFileCount, 'file')}</span>
      {/if}
      <span class="mob-close" aria-hidden="true"><Icon name="x" size={14} /></span>
    </button>
  {/if}
  <CommitDetailPane
    {repoId}
    {status}
    {onstatus}
    onresolve={onresolveconflicts}
    open={sel.detailOpen}
    isMobile={vp.isMobile}
    wipSelected={sel.wipSelected}
    selectedSha={sel.selectedSha}
    selectedCommit={sel.selectedCommit}
    chips={sel.selectedCommit ? index.chipsFor(sel.selectedCommit) : []}
    stashLabel={index.stashLabel}
    selectedBranch={sel.selectedBranch}
    diffSha={sel.diffSha}
    diffHeld={sel.diffHeld}
    onresume={() => sel.selectedSha && sel.scheduleDiff(sel.selectedSha, 0)}
    onfiletools={fileToolsMenu}
    bind:fileCount={sel.diffFileCount}
    onclose={sel.closeDetail}
    oncommitted={() => void data.refreshAfter()}
  />
</div>

<!-- Collapsed-row ref popover: rendered at the component root (position:fixed)
     so it clears the scroll container. -->
{#if refPop.menu}
  <LazyMount lazy={RefPopover} what="the ref list" quiet props={{ pop: refPop, menu: refPop.menu }} onerror={refPop.close} />
{/if}

<!-- One windowed commit row: the BRANCH / TAG cell, the SVG lane gutter and
     the subject / sha / author / date. -->
{#snippet commitRow(row: LaneRow)}
  {@const rowH = vp.rowH}
  {@const svgW = vp.gutterWidth}
  {@const cx = row.col * LANE_W + LANE_W / 2}
  {@const cy = rowH / 2}
  {@const isSelected = sel.selectedSha === row.commit.sha}
  {@const isHead = index.isHeadCommit(row.commit)}
  {@const isStash = index.isStashNode(row.commit.sha)}
  {@const chips = index.chipsFor(row.commit)}
  {@const spine = sel.highlightSpine}
  <div
    class="graph-row"
    style:height="{rowH}px"
    class:graph-row-selected={isSelected}
    class:graph-row-head={isHead}
    class:dimmed={spine !== null && !spine.has(row.commit.sha)}
    class:on-spine={spine !== null && spine.has(row.commit.sha)}
    class:stash-row-commit={isStash}
    class:row-pulse={vp.pulseSha === row.commit.sha}
    data-sha={row.commit.sha}
    title={isHead ? `${row.commit.subject} — you are here (HEAD)` : row.commit.subject}
  >
    <!-- BRANCH / TAG column: refs for this row, right-aligned so labels
         line up vertically and butt against the graph node (GitKraken). -->
    <div class="branch-cell">
      {#if shouldCollapseRow(chips)}
        <!-- Keep collapsed refs on a separate keyboard-accessible control. -->
        {@const primary = primaryChip(chips)}
        <button use:rowMenu class="ref-select" tabindex="-1" title={primary.label} aria-label={`Select ${primary.label}`} onclick={() => sel.selectCommit(row.commit)} oncontextmenu={(e) => commitMenu(e, row.commit)}>{@render chipView(primary, primary.label, row.color)}</button>
        <button
          class="ref-expander"
          tabindex="0"
          aria-label={`Show ${chips.length} refs for ${row.commit.subject}`}
          aria-haspopup="dialog"
          disabled={data.refsLoading || !data.refs}
          aria-expanded={refPop.menu?.commit.sha === row.commit.sha}
          title="{chips.length} refs on this commit — click to list"
          onclick={(e) => refPop.open(e, row.commit, chips)}
        ><Icon name="chevronDown" size={12} />+{chips.length - 1}</button>
      {:else}
        {#each chips as chip (chip.kind + chip.label)}
          {@const label = chip.kind === 'stash' ? index.stashLabel(row.commit.sha) : chip.label}
          <button use:rowMenu class="ref-select" tabindex="-1" title={label} aria-label={`Select ${label}`} onclick={() => sel.selectCommit(row.commit)} oncontextmenu={(e) => commitMenu(e, row.commit)}>{@render chipView(chip, label, row.color)}</button>
        {/each}
      {/if}
      <!-- HEAD marker ("you are here") — rightmost so it stays visible as
           the cell clips on the left. -->
      {#if isHead}
        <span class="head-badge" title="HEAD — checked-out commit">HEAD</span>
      {/if}
    </div>

    <button use:rowMenu class="graph-select" title={row.commit.subject} aria-label={row.commit.subject} aria-pressed={isSelected} onclick={() => sel.selectCommit(row.commit)} oncontextmenu={(e) => commitMenu(e, row.commit)}>
    <svg class="gutter" width={svgW} height={rowH} style="flex-shrink: 0; width: {svgW}px;">
      <!-- Lane lines (behind the node). Each segment gets an invisible WIDE
           companion stroke carrying the <title>: the real line is 1.5px, far
           too thin to hover, and a nested SVG <title> wins over the row's own
           title — so pointing at a line names its branch. -->
      {#each row.lines as line}
        {@const x1 = line.fromCol * LANE_W + LANE_W / 2}
        {@const x2 = line.toCol * LANE_W + LANE_W / 2}
        {@const d =
          line.kind === 'vert'
            ? `M${x1},0 L${x1},${rowH}`
            : line.kind === 'converge'
              ? `M${x1},0 Q${x1},${cy - 10} ${cx},${cy}`
              : `M${cx},${cy} Q${cx},${cy + 10} ${x2},${rowH}`}
        <path d={d} stroke={line.color} stroke-width="1.5" fill="none" />
        {#if line.branch}
          <path class="lane-hit" d={d} fill="none">
            <title>{line.branch}</title>
          </path>
        {/if}
      {/each}
      <!-- Leader line tying this row's ref chip(s) to their node, so a label
           visibly belongs to ONE lane. Faint: a connector, never a lane. -->
      {#if chips.length > 0 && cx > NODE_R}
        <line x1={0} y1={cy} x2={cx - NODE_R} y2={cy} stroke={row.color} stroke-width="1" opacity="0.45" />
      {/if}
      <!-- The node's OWN lane, split around the node: the half above (only
           when a lane above feeds into it — a tip has nothing above) and the
           half below (only when it has a parent — a root has nothing below). -->
      {#if row.hasAbove}
        <line x1={cx} y1={0} x2={cx} y2={cy - NODE_R} stroke={row.color} stroke-width="1.5" />
      {/if}
      {#if row.hasBelow}
        <line x1={cx} y1={cy + NODE_R} x2={cx} y2={rowH} stroke={row.color} stroke-width="1.5" />
      {/if}
      <!-- node: dashed + muted for stash plumbing; a halo ring when selected -->
      {#if isStash}
        <circle cx={cx} cy={cy} r={NODE_R} fill="var(--surface)" stroke="var(--text-dim)" stroke-width="1.3" stroke-dasharray="2 1.6" />
      {:else}
        {#if isSelected}
          <circle cx={cx} cy={cy} r={NODE_R + 3} fill="none" stroke={row.color} stroke-width="1.5" opacity="0.55" />
        {/if}
        <circle cx={cx} cy={cy} r={NODE_R} fill={row.color} />
      {/if}
      <!-- The node's own column: one hit target spanning the full row height,
           so hovering this commit's dot OR its line names the lane's branch.
           Last in the SVG so it sits above the painted marks. -->
      {#if row.branch}
        <path class="lane-hit" d="M{cx},0 L{cx},{rowH}" fill="none">
          <title>{row.branch}</title>
        </path>
      {/if}
    </svg>

    <div class="commit-info">
      <div class="ci-top">
        <span class="ci-subject">{row.commit.subject}</span>
      </div>
      <div class="ci-meta">
        <span class="mono ci-sha">{row.commit.short_sha}</span>
        <span class="dim ci-author">{row.commit.author}</span>
        <span class="grow"></span>
        <span class="dim ci-date" title={fmtDateTitle(row.commit.date)}>{fmtDate(row.commit.date)}</span>
      </div>
    </div>
    </button>
  </div>
{/snippet}

<style>
  .graphview {
    display: flex;
    height: 100%;
    min-height: 0;
  }

  /* ── graph panel ── */
  .graph-panel {
    /* Width of the BRANCH / TAG column on every row (kept equal so labels align
       vertically). Narrows when the detail panel takes the page width. */
    --branch-col-w: 168px;
    flex: 1;
    min-width: 0;
    overflow-y: auto;
    overflow-x: auto;
    transition: flex var(--dur-fast) ease-out; /* user-driven: the detail pane opening */
  }
  /* When detail is open, the commit list becomes a fixed-width column and the
     detail panel flexes to fill the rest of the page (see .detail-visible). */
  .graph-panel.panel-shrunk {
    --branch-col-w: 108px;
    flex: 0 0 420px; /* basis overridden by the inline ui.gitGraphListWidth */
    width: auto;
    min-width: 300px;
    border-inline-end: 1px solid var(--border);
  }
  .graph-panel.resizing {
    transition: none;
  }
  .graph-resizer {
    flex: 0 0 6px;
    margin-inline-start: -2px;
    cursor: col-resize;
  }
  .graph-resizer:hover {
    background: var(--accent-soft-strong);
  }
  /* Repeated refresh failures: the stale bar rides above the sticky column
     header and stays put while the history scrolls. */
  .graph-stale {
    position: sticky;
    top: 0;
    z-index: 4;
  }
  .graph-list {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  /* Column header. Mirrors the row layout exactly (branch cell width, then the
     gutter width, then the message column) so the labels sit over what they
     name. Sticky against the scrolling history. */
  .graph-head {
    position: sticky;
    top: 0;
    z-index: 3;
    display: flex;
    align-items: center;
    height: 22px;
    flex-shrink: 0;
    padding-inline-end: 12px;
    background: var(--surface);
    border-bottom: 1px solid var(--border);
    font-size: var(--fs-xs);
    font-weight: 600;
    letter-spacing: .06em;
    color: var(--text-dim);
    user-select: none;
  }
  .graph-head .gh-branch {
    width: var(--branch-col-w, 168px);
    flex-shrink: 0;
    text-align: end;
    padding-inline: 6px;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .graph-head .gh-graph {
    flex-shrink: 0;
    overflow: hidden;
    white-space: nowrap;
    text-align: center;
  }
  .graph-head .gh-label-hidden {
    visibility: hidden;
  }
  .graph-head .gh-msg {
    flex: 1;
    min-width: 0;
    padding: 0 8px;
    overflow: hidden;
    white-space: nowrap;
  }
  .graph-select, .ref-select {
    display: flex;
    align-items: center;
    background: transparent;
    border: 0;
    padding: 0;
    color: inherit;
    text-align: inherit;
    min-width: 0;
    cursor: pointer;
  }
  .graph-select { flex: 1; height: 100%; }
  .ref-select { flex-shrink: 1; overflow: hidden; }
  .graph-row:has(.graph-select:focus-visible) {
    outline: 2px solid var(--accent-text);
    outline-offset: -2px;
  }
  .graph-row {
    position: relative;
    display: flex;
    align-items: center;
    width: 100%;
    height: 32px; /* overridden inline by ROW_H — keep them equal */
    padding-inline-end: 12px;
    border: none;
    border-bottom: 1px solid color-mix(in srgb, var(--border) 50%, transparent);
    background: transparent;
    cursor: pointer;
    text-align: start;
    transition: background var(--dur-fast) ease-out;
  }
  .graph-row:hover {
    background: var(--hover);
  }
  .graph-row-selected,
  .graph-row-selected:hover {
    /* Clear selected state: accent wash + inset accent bar (inset avoids a
       layout shift that a left border would cause on the selected row only). */
    background: var(--accent-soft-strong);
    box-shadow: inset 2px 0 0 0 var(--accent);
  }
  /* Mirror the inline-start accent rail to the right edge under RTL. */
  :global([dir='rtl']) .graph-row-selected,
  :global([dir='rtl']) .graph-row-selected:hover {
    box-shadow: inset -2px 0 0 0 var(--accent);
  }
  .graph-row-selected .ci-subject {
    color: var(--accent-text);
    font-weight: 600;
  }
  /* Jump-to-ref landing flash: after scrolling hundreds of rows the selection
     wash alone is easy to lose, so the destination row pulses once. */
  .row-pulse {
    animation: row-pulse 1.2s ease-out 1;
  }
  /* A one-shot highlight decay on the jumped-to row — not a live pulse. */
  @keyframes row-pulse { /* ui-guards: allow */
    0%,
    55% {
      background: var(--accent-line);
    }
    100% {
      background: var(--accent-soft-strong);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .row-pulse {
      animation: none;
    }
  }
  /* Invisible hover target for a lane. Transparent + a fat stroke so a 1.5px
     line is actually pointable; `pointer-events: stroke` keeps the hit area on
     the stroke itself (never the curve's implied fill), so neighbouring lanes
     stay independently hoverable. */
  .lane-hit {
    stroke: transparent;
    stroke-width: 11;
    pointer-events: stroke;
  }
  /* Windowing spacers: they only reserve height, so they must never be squeezed
     by the list's flex layout or the scroll geometry drifts off the row pitch. */
  .row-spacer {
    flex: 0 0 auto;
    width: 100%;
  }
  /* ── History footer (paging status / manual "load older") ── */
  .graph-more {
    display: flex;
    justify-content: center;
    padding: 10px 12px 16px;
    font-size: var(--fs-xs);
  }
  /* ── WIP row (uncommitted changes, GitKraken-style) ── */
  .wip-row:not(.graph-row-selected) {
    background: var(--accent-soft);
  }
  .wip-row .wip-subject {
    color: var(--accent-text);
    font-weight: 600;
    font-style: italic;
  }
  .wip-chip {
    display: inline-flex;
    align-items: center;
    flex-shrink: 0;
    font-size: var(--fs-xs);
    font-weight: 600;
    padding: 1px 6px;
    border-radius: var(--radius-s);
    border: 1px dashed var(--accent-line-strong);
    color: var(--accent-text);
    background: var(--accent-soft);
  }
  /* Conflicted-files chip on the WIP row — conflicts must be visible from the
     graph itself, not only after opening the WIP panel. */
  .wip-conflicts {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    flex-shrink: 0;
    font-size: var(--fs-xs);
    font-weight: 600;
    padding: 0 6px;
    border-radius: var(--radius-s);
    color: var(--warning);
    background: var(--warning-soft);
  }

  /* The HEAD commit ("you are here") — a leading accent rail + faint wash so the
     checked-out tip is obvious at a glance, even when not selected. */
  .graph-row-head:not(.graph-row-selected) {
    background: var(--accent-soft);
    box-shadow: inset 2px 0 0 0 var(--accent);
  }
  :global([dir='rtl']) .graph-row-head:not(.graph-row-selected) {
    box-shadow: inset -2px 0 0 0 var(--accent);
  }
  /* "HEAD" badge on the checked-out commit row — a filled accent pill next to the
     branch chip so the HEAD commit is unmistakable. */
  .head-badge {
    flex-shrink: 0;
    font-size: var(--fs-xs);
    font-weight: 600;
    letter-spacing: 0.06em;
    line-height: 1;
    padding: 2px 6px;
    border-radius: var(--radius-s);
    /* Quiet marker: the checked-out branch chip beside it already carries the
       one solid accent fill in the row. */
    background: var(--accent-soft);
    color: var(--accent-text);
    box-shadow: inset 0 0 0 1px var(--accent-line);
    white-space: nowrap;
  }
  .gutter {
    display: block;
    flex-shrink: 0;
    overflow: visible;
  }
  .commit-info {
    flex: 1;
    min-width: 0;
    padding: 0 8px;
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: 2px;
    line-height: 1.2;
  }
  .ci-top {
    display: flex;
    align-items: center;
    gap: 4px;
    overflow: hidden;
  }
  .ci-subject {
    font-size: var(--fs-s);
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1;
    min-width: 0;
  }
  .ci-meta {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--fs-xs);
  }
  .ci-sha {
    color: var(--accent-text);
    font-size: var(--fs-xs);
  }
  .ci-author {
    max-width: 140px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ci-date {
    font-size: var(--fs-xs);
    white-space: nowrap;
  }

  /* ── BRANCH / TAG column ──────────────────────────────────────────────────
     A fixed-width, right-aligned cell at the start of every row. Equal width
     across rows → labels line up vertically; right-alignment butts them against
     the graph node (GitKraken layout). Overflowing chips clip on the left so the
     chip nearest the graph stays visible. */
  .branch-cell {
    width: var(--branch-col-w, 168px);
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 4px;
    overflow: hidden;
    padding-inline: 6px;
  }
  /* In the fixed-width column a chip SHRINKS to fit and ellipsizes its own
     label at the end. Left rigid (flex-shrink:0, max-width 160px) a long chip
     overflowed a 156px (or 76px with the detail open) cell and, being
     end-aligned, got clipped at its START — "gin/CS3-15178-goo…" lost both
     ends. The HEAD badge and the +N expander stay rigid. */
  .branch-cell :global(.ref-chip) {
    flex-shrink: 1;
    min-width: 0;
  }

  /* ── Multi-ref collapse: the "▾ +N" expander + its grouped popover ──────────── */
  .ref-expander {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    flex-shrink: 0;
    font-size: var(--fs-xs);
    font-weight: 600;
    line-height: 1;
    padding: 2px 6px;
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text-dim);
    border: 1px solid var(--border);
    cursor: pointer;
    white-space: nowrap;
    user-select: none;
  }
  .ref-expander:hover {
    /* Active highlight: the accent tint every selection uses. */
    background: var(--accent-soft);
    color: var(--accent-text);
    border-color: var(--accent-line);
  }
  @media (max-width: 1024px) {
    .ref-expander { min-width: 36px; min-height: 36px; }
  }

  /* ── Branch-line highlight (press a commit → see its branch) ───────────────
     Off-spine rows fade; spine rows get a faint wash. The selected row keeps its
     own accent wash (.graph-row-selected wins via later/!specificity). */
  .graph-row.dimmed {
    opacity: 0.34;
  }
  .graph-row.dimmed:hover {
    opacity: 0.62;
  }
  .graph-row.on-spine:not(.graph-row-selected):not(.graph-row-head) {
    /* A third of the soft tint: many rows sit on the spine at once. */
    background: color-mix(in srgb, var(--accent-soft) 33%, transparent);
  }
  /* A stash commit row: muted, italic subject (it's a WIP snapshot, not history). */
  .graph-row.stash-row-commit .ci-subject {
    color: var(--text-dim);
    font-style: italic;
  }

  /* Shared utilities */
  .grow {
    flex: 1;
  }
  .dim {
    color: var(--text-dim);
  }
  .mono {
    font-family: var(--font-mono);
  }

  /* ── Mobile accordion section headers (phones) ─────────────────────────────
     Each pane (Branches / Commits / Diff) gets a tappable header; the body
     below collapses or expands and scrolls independently. */
  .mob-sec-head {
    display: none;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 12px 14px;
    border: none;
    border-bottom: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text);
    font-size: var(--fs-l);
    font-weight: 600;
    cursor: pointer;
    text-align: start;
    flex-shrink: 0;
    -webkit-tap-highlight-color: transparent;
  }
  .mob-sec-head:active {
    background: linear-gradient(var(--accent-soft), var(--accent-soft)) var(--surface-2);
  }
  .mob-sec-count {
    font-size: var(--fs-xs);
    font-weight: 600;
    padding: 1px 8px;
    border-radius: 999px;
    background: var(--surface);
    color: var(--text-dim);
  }
  .mob-diff-head {
    background: linear-gradient(var(--accent-soft), var(--accent-soft)) var(--surface-2);
    position: sticky;
    top: 0;
    z-index: 3;
  }
  .mob-diff-title {
    font-size: var(--fs-m);
    color: var(--accent-text);
    font-weight: 600;
  }
  .mob-close {
    display: inline-flex;
    color: var(--text-dim);
    padding-inline-start: 6px;
  }

  /* ── Mobile + tablet (≤1024px) layout: vertical accordion ── */
  @media (max-width: 1024px) {
    .graphview.mobile {
      flex-direction: column;
      overflow-y: auto;
      -webkit-overflow-scrolling: touch;
    }
    .mobile .mob-sec-head { display: flex; }

    /* Commit list: takes the remaining room, scrolls on its own. */
    .mobile .graph-panel,
    .mobile .graph-panel.panel-shrunk {
      flex: 1 1 auto;
      min-width: 0;
      width: auto;
      min-height: 180px;
      overflow-y: auto;
      overflow-x: auto;
      overscroll-behavior: contain;
      border-inline-end: none;
    }

    /* A collapsed section hides its body entirely (header stays tappable). */
    .mobile .mob-collapsed {
      display: none !important;
    }

    /* Bigger touch targets + legible text on the commit rows. */
    /* Row height (46px here) is ROW_H, applied inline — see "Row windowing". */
    .mobile .ci-subject { font-size: var(--fs-l); }
    .mobile .ci-meta { font-size: var(--fs-s); }
    .mobile .ci-sha { font-size: var(--fs-s); }
    .mobile .ci-author { font-size: var(--fs-s); max-width: 110px; }
    .mobile .ci-date { font-size: var(--fs-s); }
    .mobile :global(.ref-chip) { font-size: var(--fs-xs); max-width: 120px; }
    /* Branch/tag column stays a compact, aligned column on phones; chips clip on
       the left and the full refs are available in the detail header on tap. */
    .mobile .graph-panel { --branch-col-w: 108px; }
    .mobile .branch-cell { gap: 2px; padding-inline: 4px; }
    .mobile .branch-cell :global(.ref-chip) { max-width: 96px; }
  }
</style>
