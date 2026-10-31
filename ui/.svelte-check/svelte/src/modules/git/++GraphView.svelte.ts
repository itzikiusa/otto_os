///<reference types="svelte" />
;// Three panes: LEFT = refs tree (graph/RefSidebar), MIDDLE = the commit graph
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
;

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
  };function $$render() {
 const chipView/*Ωignore_positionΩ*/ = (chip: Chip, label: string, laneColor: string)/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
   { const $$_pihCfeR0C = __sveltets_2_ensureComponent(RefChip); new $$_pihCfeR0C({ target: __sveltets_2_any(), props: {         chip,label,"lane":laneColor,"tracking":branchTracking(chip.kind === 'head' || chip.kind === 'local' ? index.localBranchByName.get(chip.label) : undefined, status),"title":chip.kind === 'stash'
      ? `Stash · ${label}`
      : chip.current
        ? `Checked out · ${chip.label}`
        : chip.worktree
          ? `Worktree · ${chip.label} — open worktree (do not switch branch)`
          : chip.label,}});}
};return __sveltets_2_any(0)}; const commitRow/*Ωignore_positionΩ*/ = (row: LaneRow)/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
  const rowH = vp.rowH;
  const svgW = vp.gutterWidth;
  const cx = row.col * LANE_W + LANE_W / 2;
  const cy = rowH / 2;
  const isSelected = sel.selectedSha === row.commit.sha;
  const isHead = index.isHeadCommit(row.commit);
  const isStash = index.isStashNode(row.commit.sha);
  const chips = index.chipsFor(row.commit);
  const spine = sel.highlightSpine;
   { svelteHTML.createElement("div", {              "class":`graph-row`,...__sveltets_2_empty({"data-sha":row.commit.sha}),"title":isHead ? `${row.commit.subject} — you are here (HEAD)` : row.commit.subject,});__sveltets_2_ensureType(String, Number, `${rowH} `);isSelected;isHead;spine !== null && !spine.has(row.commit.sha);spine !== null && spine.has(row.commit.sha);isStash;vp.pulseSha === row.commit.sha;
    
     { svelteHTML.createElement("div", { "class":`branch-cell`,});
      if(shouldCollapseRow(chips)){
        
        const primary = primaryChip(chips);
         {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('button')));{ svelteHTML.createElement("button", __sveltets_2_union($$action_0), {            "class":`ref-select`,"tabindex":-1,"title":primary.label,"aria-label":`Select ${primary.label}`,"onclick":() => sel.selectCommit(row.commit),"oncontextmenu":(e) => commitMenu(e, row.commit),});;__sveltets_2_ensureSnippet(chipView(primary, primary.label, row.color)); }}
         { svelteHTML.createElement("button", {                 "class":`ref-expander`,"tabindex":0,"aria-label":`Show ${chips.length} refs for ${row.commit.subject}`,"aria-haspopup":`dialog`,"disabled":data.refsLoading || !data.refs,"aria-expanded":refPop.menu?.commit.sha === row.commit.sha,"title":`${chips.length} refs on this commit — click to list`,"onclick":(e) => refPop.open(e, row.commit, chips),}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`chevronDown`,"size":12,}});} chips.length - 1; }
      }else{
           for(let chip of __sveltets_2_ensureArray(chips)){chip.kind + chip.label;
          const label = chip.kind === 'stash' ? index.stashLabel(row.commit.sha) : chip.label;
           {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('button')));{ svelteHTML.createElement("button", __sveltets_2_union($$action_0), {            "class":`ref-select`,"tabindex":-1,"title":label,"aria-label":`Select ${label}`,"onclick":() => sel.selectCommit(row.commit),"oncontextmenu":(e) => commitMenu(e, row.commit),});;__sveltets_2_ensureSnippet(chipView(chip, label, row.color)); }}
        }
      }
      
      if(isHead){
         { svelteHTML.createElement("span", {   "class":`head-badge`,"title":`HEAD — checked-out commit`,});  }
      }
     }

     {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('button')));{ svelteHTML.createElement("button", __sveltets_2_union($$action_0), {            "class":`graph-select`,"title":row.commit.subject,"aria-label":row.commit.subject,"aria-pressed":isSelected,"onclick":() => sel.selectCommit(row.commit),"oncontextmenu":(e) => commitMenu(e, row.commit),});
     { svelteHTML.createElement("svg", {       "class":`gutter`,"width":svgW,"height":rowH,"style":`flex-shrink: 0; width: ${svgW}px;`,});
      
        for(let line of __sveltets_2_ensureArray(row.lines)){
        const x1 = line.fromCol * LANE_W + LANE_W / 2;
        const x2 = line.toCol * LANE_W + LANE_W / 2;
        const d =
          line.kind === 'vert'
            ? `M${x1},0 L${x1},${rowH}`
            : line.kind === 'converge'
              ? `M${x1},0 Q${x1},${cy - 10} ${cx},${cy}`
              : `M${cx},${cy} Q${cx},${cy + 10} ${x2},${rowH}`;
         { svelteHTML.createElement("path", {        "d":d,"stroke":line.color,"stroke-width":`1.5`,"fill":`none`,});}
        if(line.branch){
           { svelteHTML.createElement("path", {     "class":`lane-hit`,"d":d,"fill":`none`,});
             { svelteHTML.createElement("title", {});line.branch; }
           }
        }
      }
      
      if(chips.length > 0 && cx > NODE_R){
         { svelteHTML.createElement("line", {              "x1":0,"y1":cy,"x2":cx - NODE_R,"y2":cy,"stroke":row.color,"stroke-width":`1`,"opacity":`0.45`,});}
      }
      
      if(row.hasAbove){
         { svelteHTML.createElement("line", {            "x1":cx,"y1":0,"x2":cx,"y2":cy - NODE_R,"stroke":row.color,"stroke-width":`1.5`,});}
      }
      if(row.hasBelow){
         { svelteHTML.createElement("line", {            "x1":cx,"y1":cy + NODE_R,"x2":cx,"y2":rowH,"stroke":row.color,"stroke-width":`1.5`,});}
      }
      
      if(isStash){
         { svelteHTML.createElement("circle", {              "cx":cx,"cy":cy,"r":NODE_R,"fill":`var(--surface)`,"stroke":`var(--text-dim)`,"stroke-width":`1.3`,"stroke-dasharray":`2 1.6`,});}
      }else{
        if(isSelected){
           { svelteHTML.createElement("circle", {              "cx":cx,"cy":cy,"r":NODE_R + 3,"fill":`none`,"stroke":row.color,"stroke-width":`1.5`,"opacity":`0.55`,});}
        }
         { svelteHTML.createElement("circle", {        "cx":cx,"cy":cy,"r":NODE_R,"fill":row.color,});}
      }
      
      if(row.branch){
         { svelteHTML.createElement("path", {     "class":`lane-hit`,"d":`M${cx},0 L${cx},${rowH}`,"fill":`none`,});
           { svelteHTML.createElement("title", {});row.branch; }
         }
      }
     }

     { svelteHTML.createElement("div", { "class":`commit-info`,});
       { svelteHTML.createElement("div", { "class":`ci-top`,});
         { svelteHTML.createElement("span", { "class":`ci-subject`,});row.commit.subject; }
       }
       { svelteHTML.createElement("div", { "class":`ci-meta`,});
         { svelteHTML.createElement("span", { "class":`mono ci-sha`,});row.commit.short_sha; }
         { svelteHTML.createElement("span", { "class":`dim ci-author`,});row.commit.author; }
         { svelteHTML.createElement("span", { "class":`grow`,}); }
         { svelteHTML.createElement("span", {   "class":`dim ci-date`,"title":fmtDateTitle(row.commit.date),});fmtDate(row.commit.date); }
       }
     }
     }}
   }
};return __sveltets_2_any(0)};
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

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
;
async () => {




 { svelteHTML.createElement("div", {  "class":`graphview`,});vp.isMobile;
  
  if(vp.isMobile){
    
     { svelteHTML.createElement("button", {        "class":`mob-sec-head`,"onclick":() => (secRefsOpen = !secRefsOpen),"aria-expanded":secRefsOpen,});secRefsOpen;
       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":secRefsOpen ? 'chevronDown' : 'chevronRight',"size":13,}});}
       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`branch`,"size":13,}});}
       { svelteHTML.createElement("span", {});   }
       { svelteHTML.createElement("span", { "class":`grow`,}); }
      if(data.refs){ { svelteHTML.createElement("span", { "class":`mob-sec-count`,});data.refs.local.length + data.refs.remote.length + data.refs.tags.length; }}
     }
  }
   { const $$_rabediSfeR1C = __sveltets_2_ensureComponent(RefSidebar); new $$_rabediSfeR1C({ target: __sveltets_2_any(), props: {                             "refs":data.refs,"refsLoading":data.refsLoading,"refsError":data.refsError,"onretry":() => data.loadRefs(repoId),status,"stashes":data.stashes,"worktrees":data.worktrees,"submodules":data.submodules,currentRepoPath,"isMobile":vp.isMobile,"collapsed":!secRefsOpen,checkoutBusy,openWtBusy,index,actions,menus,"selection":sel,onmergerequest,}});}

  
  if(vp.isMobile){
     { svelteHTML.createElement("button", {        "class":`mob-sec-head`,"onclick":() => (secCommitsOpen = !secCommitsOpen),"aria-expanded":secCommitsOpen,});secCommitsOpen;
       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":secCommitsOpen ? 'chevronDown' : 'chevronRight',"size":13,}});}
       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`commit`,"size":13,}});}
       { svelteHTML.createElement("span", {});  }
       { svelteHTML.createElement("span", { "class":`grow`,}); }
      if(!data.commitsLoading){ { svelteHTML.createElement("span", { "class":`mob-sec-count`,});data.commits.length; }}
     }
  }
   { const $$_div1 = svelteHTML.createElement("div", {          "class":`graph-panel`,"onscroll":vp.onScroll,});sel.detailOpen && !vp.isMobile;vp.isMobile && !secCommitsOpen;listResizing;__sveltets_2_ensureType(String, Number, sel.detailOpen && !vp.isMobile ? `${ui.gitGraphListWidth}px` : null);vp.panelEl = $$_div1;
    if(data.commitsError && data.commits.length === 0){
       { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {        "what":`commits`,"error":data.commitsError,"empty":true,"loading":data.commitsLoading,"onretry":() => data.retryCommits(),}});}
    } else if (data.commitsLoading){
       { svelteHTML.createElement("div", { "style":`padding: 10px`,}); { const $$_notelekS3C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS3C({ target: __sveltets_2_any(), props: {    "rows":12,"height":vp.rowH,}});} }
    } else if (data.commits.length === 0){
       { svelteHTML.createElement("div", {   "class":`dim`,"style":`padding: 18px; font-size: var(--fs-s)`,});             }
    }else{
      if(data.refreshError){
         { svelteHTML.createElement("div", { "class":`graph-stale`,});
           { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {      "what":`the graph`,"error":data.refreshError,"onretry":() => void data.reloadGraph(),}});}
         }
      } else if (data.refsStaleError && data.commits.length > 0){
        
         { svelteHTML.createElement("div", { "class":`graph-stale`,});
           { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {      "what":`branches`,"error":data.refsStaleError,"onretry":() => void data.refreshAfter().catch(data.noteBackgroundRead),}});}
         }
      }
       { svelteHTML.createElement("div", { "class":`graph-list`,});
        
         { svelteHTML.createElement("div", {   "class":`graph-head`,"aria-hidden":`true`,});
           { svelteHTML.createElement("span", {   "class":`gh-branch`,"title":`Branch / tag`,});   }
          
           { svelteHTML.createElement("span", {      "class":`gh-graph`,"style":`width: ${vp.gutterWidth}px`,"title":`Graph`,});vp.gutterWidth < 40;  }
           { svelteHTML.createElement("span", { "class":`gh-msg`,});  }
         }
        
        if(status.changes.length > 0){
          const cx = wipCol * LANE_W + LANE_W / 2;
          const rowH = vp.rowH;
           { svelteHTML.createElement("button", {           "class":`graph-row wip-row`,"onclick":sel.selectWip,"title":`Uncommitted changes — click to stage & commit`,"aria-pressed":sel.wipSelected,});__sveltets_2_ensureType(String, Number, `${rowH} `);sel.wipSelected;
             { svelteHTML.createElement("div", { "class":`branch-cell`,});
               { svelteHTML.createElement("span", { "class":`wip-chip mono`,});  }
             }
             { svelteHTML.createElement("svg", {       "class":`gutter`,"width":vp.gutterWidth,"height":rowH,"style":`flex-shrink: 0; width: ${vp.gutterWidth}px;`,});
              
               { svelteHTML.createElement("line", {                "x1":cx,"y1":rowH / 2 + NODE_R,"x2":cx,"y2":rowH,"stroke":`var(--accent)`,"stroke-width":`1.5`,"stroke-dasharray":`2 2`,"opacity":`0.7`,});}
              if(sel.wipSelected){
                 { svelteHTML.createElement("circle", {              "cx":cx,"cy":rowH / 2,"r":NODE_R + 3,"fill":`none`,"stroke":`var(--accent)`,"stroke-width":`1.5`,"opacity":`0.55`,});}
              }
               { svelteHTML.createElement("circle", {              "cx":cx,"cy":rowH / 2,"r":NODE_R,"fill":`var(--surface)`,"stroke":`var(--accent)`,"stroke-width":`1.5`,"stroke-dasharray":`2.5 1.8`,});}
             }
             { svelteHTML.createElement("div", { "class":`commit-info`,});
               { svelteHTML.createElement("div", { "class":`ci-top`,});
                 { svelteHTML.createElement("span", { "class":`ci-subject wip-subject mono`,});  }
               }
               { svelteHTML.createElement("div", { "class":`ci-meta`,});
                 { svelteHTML.createElement("span", { "class":`dim`,});
                  plural(status.changes.length, 'file'); wipStagedCount > 0 ? ` · ${wipStagedCount} staged` : '';
                 }
                if(wipConflictCount > 0){
                   { svelteHTML.createElement("span", {   "class":`wip-conflicts`,"title":`${plural(wipConflictCount, 'conflicted file')} — open the WIP panel to resolve`,});
                     { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});} wipConflictCount; 
                   }
                }
               }
             }
           }
        }
        
         { const $$_div3 = svelteHTML.createElement("div", {       "class":`row-spacer`,"style":`height: ${vp.padTop}px`,"aria-hidden":`true`,});vp.topSpacerEl = $$_div3; }
           for(let row of __sveltets_2_ensureArray(vp.visibleRows)){row.commit.sha;
          ;__sveltets_2_ensureSnippet(commitRow(row));
        }
         { svelteHTML.createElement("div", {     "class":`row-spacer`,"style":`height: ${vp.padBottom}px`,"aria-hidden":`true`,}); }
        
         { svelteHTML.createElement("div", { "class":`graph-more`,});
          if(data.loadingMore){
             { svelteHTML.createElement("span", {       "class":`spinner`,"role":`img`,"aria-label":`Loading more history`,"title":`Loading more history`,}); }
          } else if (data.hasMore){
             { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => void data.loadMore(),});
                
             }
          }else{
             { svelteHTML.createElement("span", { "class":`dim`,});plural(data.commits.length, 'commit');     }
          }
         }
       }
    }
   }

  
  if(createPrOpen){
     { const $$_rPetaerC1C = __sveltets_2_ensureComponent(CreatePr); new $$_rPetaerC1C({ target: __sveltets_2_any(), props: {        repoId,"initialSource":createPrSource,"onclose":() => (createPrOpen = false),"oncreated":() => {
        createPrOpen = false;
        void data.refreshAfter();
      },}});}
  }

  
  if(sel.detailOpen && !vp.isMobile){
     {const $$action_0 = __sveltets_2_ensureAction(splitter(svelteHTML.mapElementTag('div'),({
        onkeydown: (e) => resizerKey(e, () => ui.gitGraphListWidth, (w) => ui.setGitGraphListWidth(w), 420),
        onmousedown: startListResize,
        ondblclick: () => ui.setGitGraphListWidth(420),
      })));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {               "class":`graph-resizer`,"role":`separator`,"aria-orientation":`vertical`,"aria-label":`Resize the commit list`,"aria-valuenow":ui.gitGraphListWidth,"title":`Drag or use ←/→ to resize · double-click to reset`,}); }}
  }

  if(vp.isMobile && sel.detailOpen){
    
     { svelteHTML.createElement("button", {     "class":`mob-sec-head mob-diff-head`,"onclick":sel.closeDetail,"aria-expanded":`true`,});
       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`chevronDown`,"size":13,}});}
       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`file`,"size":13,}});}
       { svelteHTML.createElement("span", { "class":`mob-diff-title mono`,});sel.wipSelected ? '// WIP' : (sel.selectedCommit?.short_sha ?? 'Diff'); }
       { svelteHTML.createElement("span", { "class":`grow`,}); }
      if(sel.wipSelected){
         { svelteHTML.createElement("span", { "class":`mob-sec-count`,});plural(status.changes.length, 'file'); }
      } else if (sel.diffFileCount !== null){
         { svelteHTML.createElement("span", { "class":`mob-sec-count`,});plural(sel.diffFileCount, 'file'); }
      }
       { svelteHTML.createElement("span", {   "class":`mob-close`,"aria-hidden":`true`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":14,}});} }
     }
  }
   { const $$_enaPliateDtimmoC1C = __sveltets_2_ensureComponent(CommitDetailPane); const $$_enaPliateDtimmoC1 = new $$_enaPliateDtimmoC1C({ target: __sveltets_2_any(), props: {                                    repoId,status,onstatus,"onresolve":onresolveconflicts,"open":sel.detailOpen,"isMobile":vp.isMobile,"wipSelected":sel.wipSelected,"selectedSha":sel.selectedSha,"selectedCommit":sel.selectedCommit,"chips":sel.selectedCommit ? index.chipsFor(sel.selectedCommit) : [],"stashLabel":index.stashLabel,"selectedBranch":sel.selectedBranch,"diffSha":sel.diffSha,"diffHeld":sel.diffHeld,"onresume":() => sel.selectedSha && sel.scheduleDiff(sel.selectedSha, 0),"onfiletools":fileToolsMenu,fileCount:sel.diffFileCount,"onclose":sel.closeDetail,"oncommitted":() => void data.refreshAfter(),}});/*Ωignore_startΩ*/() => sel.diffFileCount = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_enaPliateDtimmoC1.$$bindings = 'fileCount';}
 }


if(refPop.menu){
   { const $$_tnuoMyzaL0C = __sveltets_2_ensureComponent(LazyMount); new $$_tnuoMyzaL0C({ target: __sveltets_2_any(), props: {        "lazy":RefPopover,"what":`the ref list`,"quiet":true,"props":{ pop: refPop, menu: refPop.menu },"onerror":refPop.close,}});}
}





};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const GraphView__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type GraphView__SvelteComponent_ = ReturnType<typeof GraphView__SvelteComponent_>;
/*Ωignore_endΩ*/export default GraphView__SvelteComponent_;