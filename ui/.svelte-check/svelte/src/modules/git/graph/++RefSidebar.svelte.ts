///<reference types="svelte" />
;// The git graph's refs sidebar (arch-1-09: split out of GraphView.svelte):
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
;

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
  };;

  // Grouped once per `refs` change (not re-sorted inside the template on every
  // render of the section).
  type Grouped = { loose: BranchLeaf[]; folders: BranchFolder[] };;;
  type SectionPlan = Plan<BranchLeaf>;;function $$render() {
 const refName/*Ωignore_positionΩ*/ = (label: string)/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
  const mp = midParts(label);
  if(mp){
     { svelteHTML.createElement("span", { "class":`mono ref-name mid`,}); { svelteHTML.createElement("span", { "class":`mid-head`,});mp.head; } { svelteHTML.createElement("span", { "class":`mid-tail`,});mp.tail; } }
  }else{
     { svelteHTML.createElement("span", { "class":`mono ref-name`,});label; }
  }
};return __sveltets_2_any(0)}; const mergedMark/*Ωignore_positionΩ*/ = (b: RefBranch, into: string[])/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
   { svelteHTML.createElement("span", {   "class":`merged-mark`,"title":`Already in ${into.join(', ')}${b.is_current ? '' : ' — safe to delete'}`,});
     { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});} { svelteHTML.createElement("span", { "class":`merged-into`,});into.join(' · '); }
   }
};return __sveltets_2_any(0)}; const localRow/*Ωignore_positionΩ*/ = (leaf: BranchLeaf, nested: boolean)/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
  const b = leaf.b;
  const into = index.landedIn(b);
  const tracking = branchTracking(b, status);
  const wtElsewhere = index.worktreeByBranch.get(b.name);
   { svelteHTML.createElement("div", { "class":`ref-action-row`,});
   {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('button')));{ svelteHTML.createElement("button", __sveltets_2_union($$action_0), {                               "class":`ref-row`,"draggable":checkoutBusy === '' && openWtBusy === '',"ondragstart":(e) => drag.start(e, b.name, false),"ondragend":drag.end,"ondragover":(e) => drag.over(e, b.name),"ondragleave":() => drag.leave(b.name),"ondrop":(e) => drag.drop(e, b.name),"disabled":checkoutBusy !== '' || openWtBusy !== '',"onclick":() => selection.selectBranchRow(b),"ondblclick":() => activateLocalBranch(b),"oncontextmenu":(e) => menus.branchMenu(e, b),"title":drag.source && drag.isValidTarget(b.name)
      ? `Merge ${drag.sourceName()} → ${b.name}`
      : wtElsewhere
        ? `Double-click to open worktree · ${wtElsewhere.path}${wtElsewhere.branch ? ` · ${wtElsewhere.branch}` : ''}`
        : `${b.name} — click to highlight, double-click to checkout`,});nested;b.is_current;!!wtElsewhere;drag.overTarget === b.name;drag.source?.name === b.name && !drag.source.remote;
    if(b.is_current){
       { svelteHTML.createElement("span", {   "class":`cur-pip`,"title":`Checked out`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});} }
    } else if (wtElsewhere){
       { svelteHTML.createElement("span", {   "class":`cur-pip wt-pip`,"title":`Worktree — click to open`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`worktree`,"size":12,}});} }
    }else{
       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`dot`,"size":12,}});}
    }
    ;__sveltets_2_ensureSnippet(refName(leaf.label));
    if(into.length > 0){;__sveltets_2_ensureSnippet(mergedMark(b, into));}
    if(wtElsewhere){
       { svelteHTML.createElement("span", {   "class":`wt-open-hint`,"title":`Open worktree (do not switch branch)`,});  }
    }
    if(tracking.ahead > 0 || tracking.behind > 0){
       { svelteHTML.createElement("span", {   "class":`ref-ab`,"title":`${tracking.ahead} ahead · ${tracking.behind} behind upstream`,});
        if(tracking.ahead > 0){ { svelteHTML.createElement("span", { "class":`ab-ahead`,}); tracking.ahead; }}
        if(tracking.behind > 0){ { svelteHTML.createElement("span", { "class":`ab-behind`,}); tracking.behind; }}
       }
    }
    if(b.upstream){
      if(b.upstream.endsWith(`/${b.name}`)){
        
         { svelteHTML.createElement("span", {   "class":`ref-track`,"title":`tracks ${b.upstream}`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`globe`,"size":12,}});} }
      }else{
        
         { svelteHTML.createElement("span", {   "class":`ref-upstream mono dim`,"title":`upstream: ${b.upstream}`,});b.upstream; }
      }
    }
    if(checkoutBusy === b.name || (wtElsewhere && openWtBusy === wtElsewhere.path)){ { svelteHTML.createElement("span", {   "class":`dim`,"title":`Working…`,});  }}
   }}
   { svelteHTML.createElement("button", {             "class":`icon-btn ref-action`,"aria-label":`Actions for branch ${b.name}`,"title":`Actions for branch ${b.name}`,"aria-haspopup":`menu`,"disabled":checkoutBusy !== '' || openWtBusy !== '',"onclick":(e) => menus.branchMenu(e, b),}); { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":13,}});} }
   }
};return __sveltets_2_any(0)}; const remoteRow/*Ωignore_positionΩ*/ = (leaf: BranchLeaf, nested: boolean)/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
  const b = leaf.b;
  const into = index.landedIn(b);
   { svelteHTML.createElement("div", { "class":`ref-action-row`,});
   {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('button')));{ svelteHTML.createElement("button", __sveltets_2_union($$action_0), {                      "class":`ref-row remote`,"draggable":checkoutBusy === '',"ondragstart":(e) => drag.start(e, b.name, true),"ondragend":drag.end,"disabled":checkoutBusy !== '',"onclick":() => selection.selectBranchRow(b),"ondblclick":() => actions.checkoutRemote(b),"oncontextmenu":(e) => menus.branchMenu(e, b),"title":`${b.name} — click to highlight · double-click to checkout as a local tracking branch · drag onto a local branch to merge`,});nested;drag.source?.name === b.name && drag.source.remote;
     { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`dot`,"size":12,}});}
    ;__sveltets_2_ensureSnippet(refName(leaf.label));
    if(into.length > 0){;__sveltets_2_ensureSnippet(mergedMark(b, into));}
    if(checkoutBusy === b.name.replace(/^[^/]+\//, '')){ { svelteHTML.createElement("span", { "class":`dim`,});  }}
   }}
   { svelteHTML.createElement("button", {             "class":`icon-btn ref-action`,"aria-label":`Actions for branch ${b.name}`,"title":`Actions for branch ${b.name}`,"aria-haspopup":`menu`,"disabled":checkoutBusy !== '' || openWtBusy !== '',"onclick":(e) => menus.branchMenu(e, b),}); { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":13,}});} }
   }
};return __sveltets_2_any(0)}; const folderHeader/*Ωignore_positionΩ*/ = (section: 'local' | 'remote', folder: BranchFolder)/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
  const key = folderKey(section, folder.name);
  const folded = collapsedFolders.has(key);
   { svelteHTML.createElement("button", {        "class":`ref-folder`,"onclick":() => toggleFolder(key),"title":folder.name,"aria-expanded":!folded,});folded;
     { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":folded ? 'chevronRight' : 'chevronDown',"size":12,}});}
     { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`folder`,"size":12,}});}
     { svelteHTML.createElement("span", { "class":`folder-name`,});folder.name; }
     { svelteHTML.createElement("span", { "class":`ref-count`,});folder.leaves.length; }
   }
};return __sveltets_2_any(0)}; const moreLeaves/*Ωignore_positionΩ*/ = (key: string, hidden: number)/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
  if(hidden > 0){
     { svelteHTML.createElement("button", {   "class":`ref-more-leaves`,"onclick":() => showMoreLeaves(key),});
       { svelteHTML.createElement("span", { "class":`dim`,}); Math.min(hidden, LEAF_PAGE * 4);  hidden;  }
     }
  }
};return __sveltets_2_any(0)}; const branchSection/*Ωignore_positionΩ*/ = (section: 'local' | 'remote', list: RefBranch[], grouped: Grouped, plan: SectionPlan)/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
  const open = section === 'local' ? localOpen : remoteOpen;
   { svelteHTML.createElement("div", { "class":`ref-section`,});
     { svelteHTML.createElement("button", {       "class":`ref-header`,"onclick":() => (section === 'local' ? (localOpen = !localOpen) : (remoteOpen = !remoteOpen)),"aria-expanded":open,});
       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":open ? 'chevronDown' : 'chevronRight',"size":12,}});}
       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":section === 'local' ? 'branch' : 'globe',"size":12,}});}
       { svelteHTML.createElement("span", {});section === 'local' ? 'LOCAL' : 'REMOTE'; }
       { svelteHTML.createElement("span", { "class":`ref-count`,});list.length; }
     }
    if(open){
      if(list.length === 0){
         { svelteHTML.createElement("div", { "class":`dim ref-empty`,});section === 'local' ? 'No local branches' : 'No remote branches'; }
      }
         for(let leaf of __sveltets_2_ensureArray((grouped.loose.slice(0, plan.loose)))){leaf.b.name;
        ;__sveltets_2_ensureSnippet((section === 'local' ? localRow : remoteRow)(leaf, false));
      }
         for(let { folder, shown } of __sveltets_2_ensureArray(plan.folders)){folder.name;
        ;__sveltets_2_ensureSnippet(folderHeader(section, folder));
        if(!collapsedFolders.has(folderKey(section, folder.name)) && shown > 0){
           { svelteHTML.createElement("div", { "class":`folder-children`,});
               for(let leaf of __sveltets_2_ensureArray((folder.leaves.slice(0, shown)))){leaf.b.name;
              ;__sveltets_2_ensureSnippet((section === 'local' ? localRow : remoteRow)(leaf, true));
            }
           }
        }
      }
      ;__sveltets_2_ensureSnippet(moreLeaves(`${section}:`, plan.hidden));
    }
   }
};return __sveltets_2_any(0)};
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

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
;
async () => {




















  { svelteHTML.createElement("aside", {      "class":`refs-panel`,});isMobile;isMobile && collapsed;resizing;__sveltets_2_ensureType(String, Number, !isMobile ? `${ui.gitGraphSideWidth}px` : null);
  if(refsLoading){
     { svelteHTML.createElement("div", { "style":`padding: 10px`,}); { const $$_notelekS2C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS2C({ target: __sveltets_2_any(), props: {    "rows":6,"height":22,}});} }
  } else if (refsError){
     { svelteHTML.createElement("div", { "class":`refs-error`,});
       { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {       "what":`branches`,"error":refsError,"empty":true,"variant":`compact`,onretry,}});}
     }
  } else if (refs){
    ;__sveltets_2_ensureSnippet(branchSection('local', refs.local, groupedLocal, localPlan));
    ;__sveltets_2_ensureSnippet(branchSection('remote', refs.remote, groupedRemote, remotePlan));

    
     { svelteHTML.createElement("div", { "class":`ref-section`,});
       { svelteHTML.createElement("button", {     "class":`ref-header`,"onclick":() => (tagsOpen = !tagsOpen),"aria-expanded":tagsOpen,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":tagsOpen ? 'chevronDown' : 'chevronRight',"size":12,}});}
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`tag`,"size":12,}});}
         { svelteHTML.createElement("span", {});  }
         { svelteHTML.createElement("span", { "class":`ref-count`,});refs.tags.length; }
       }
      if(tagsOpen){
           for(let t of __sveltets_2_ensureArray((refs.tags.slice(0, leafLimit('tags:'))))){t.name;
           { svelteHTML.createElement("div", { "class":`ref-action-row`,});
           {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('button')));{ svelteHTML.createElement("button", __sveltets_2_union($$action_0), {           "class":`ref-row tag`,"title":`${t.name} — click to show it on the graph, right-click for actions`,"onclick":() => selection.selectTagRow(t),"oncontextmenu":(e) => menus.tagMenu(e, t),});selection.revealBusy === t.name;
             { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`tag`,"size":12,}});}
             { svelteHTML.createElement("span", { "class":`mono ref-name`,});t.name; }
           }}
          
           { svelteHTML.createElement("button", {           "class":`icon-btn ref-action ref-more`,"title":`Actions for tag ${t.name}`,"aria-label":`Actions for tag ${t.name}`,"aria-haspopup":`menu`,"onclick":(e) => menus.tagMenu(e, t),}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":13,}});} }
           }
        }
           { svelteHTML.createElement("div", { "class":`dim ref-empty`,});  }
        
        ;__sveltets_2_ensureSnippet(moreLeaves('tags:', refs.tags.length - Math.min(refs.tags.length, leafLimit('tags:'))));
      }
     }

     { const $$_stsiLediSfeR1C = __sveltets_2_ensureComponent(RefSideLists); new $$_stsiLediSfeR1C({ target: __sveltets_2_any(), props: {        stashes,worktrees,submodules,currentRepoPath,openWtBusy,actions,menus,selection,}});}
  }
 }


if(!isMobile){
   {const $$action_0 = __sveltets_2_ensureAction(splitter(svelteHTML.mapElementTag('div'),({
      onkeydown: (e) => resizerKey(e, () => ui.gitGraphSideWidth, (w) => ui.setGitGraphSideWidth(w), 220, true),
      onmousedown: startResize,
      ondblclick: () => ui.setGitGraphSideWidth(220),
    })));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {               "class":`refs-resizer`,"role":`separator`,"aria-orientation":`vertical`,"aria-label":`Resize the branch list`,"aria-valuenow":ui.gitGraphSideWidth,"title":`Drag or use ←/→ to resize · double-click to reset`,}); }}
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const RefSidebar__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type RefSidebar__SvelteComponent_ = ReturnType<typeof RefSidebar__SvelteComponent_>;
/*Ωignore_endΩ*/export default RefSidebar__SvelteComponent_;