///<reference types="svelte" />
;// The refs sidebar's lower sections (arch-1-09: split out of GraphView.svelte):
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
;

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
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  

  let { stashes, worktrees, submodules, currentRepoPath, openWtBusy, actions, menus, selection }: Props = $props();

  let stashesOpen = $state(false);
  let worktreesOpen = $state(false);
  let submodulesOpen = $state(false);
;
async () => {


 { svelteHTML.createElement("div", { "class":`ref-section`,});
   { svelteHTML.createElement("button", {     "class":`ref-header`,"onclick":() => (stashesOpen = !stashesOpen),"aria-expanded":stashesOpen,});
     { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":stashesOpen ? 'chevronDown' : 'chevronRight',"size":12,}});}
     { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`stash`,"size":12,}});}
     { svelteHTML.createElement("span", {});  }
     { svelteHTML.createElement("span", { "class":`ref-count`,});stashes.length; }
   }
  if(stashesOpen){
       for(let s of __sveltets_2_ensureArray(stashes)){s.ref;
       { svelteHTML.createElement("div", { "class":`ref-action-row`,});
       {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('button')));{ svelteHTML.createElement("button", __sveltets_2_union($$action_0), {          "class":`ref-row stash-row`,"title":`${s.ref} · ${s.message}`,"onclick":() => selection.selectStash(s),"oncontextmenu":(e) => menus.stashMenu(e, s),});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`stash`,"size":12,}});}
         { svelteHTML.createElement("span", { "class":`ref-name stash-msg`,});stashShortMsg(s); }
        if(s.branch){ { svelteHTML.createElement("span", { "class":`stash-branch mono dim`,});s.branch; }}
       }}
       { svelteHTML.createElement("button", {         "class":`icon-btn ref-action`,"title":`Actions for stash ${s.ref}`,"aria-label":`Actions for stash ${s.ref}`,"aria-haspopup":`menu`,"onclick":(e) => menus.stashMenu(e, s),}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":13,}});} }
       }
    }
       { svelteHTML.createElement("div", { "class":`dim ref-empty`,});  }
    
  }
 }


 { svelteHTML.createElement("div", { "class":`ref-section`,});
   { svelteHTML.createElement("button", {     "class":`ref-header`,"onclick":() => (worktreesOpen = !worktreesOpen),"aria-expanded":worktreesOpen,});
     { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":worktreesOpen ? 'chevronDown' : 'chevronRight',"size":12,}});}
     { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`worktree`,"size":12,}});}
     { svelteHTML.createElement("span", {});  }
     { svelteHTML.createElement("span", { "class":`ref-count`,});worktrees.length; }
   }
  if(worktreesOpen){
       for(let w of __sveltets_2_ensureArray(worktrees)){w.path;
      const isHere = currentRepoPath !== '' && normPath(w.path) === currentRepoPath;
       { svelteHTML.createElement("div", { "class":`ref-action-row`,});
       {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('button')));{ svelteHTML.createElement("button", __sveltets_2_union($$action_0), {               "type":`button`,"class":`ref-row stash-row is-worktree`,"disabled":w.prunable || openWtBusy !== '',"title":w.prunable
          ? `${w.path} · stale (folder gone)`
          : isHere
            ? `Current worktree · ${w.path}`
            : `Open worktree · ${w.path}${w.branch ? ` · ${w.branch}` : ' · detached'}${w.dirty ? ' · uncommitted changes' : ''}`,"onclick":() => {
          if (!w.prunable && !isHere) void actions.openWorktree(w);
        },"oncontextmenu":(e) => {
          e.preventDefault();
          menus.wtMenu(e, w);
        },});isHere;
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`worktree`,"size":12,}});}
         { svelteHTML.createElement("span", {  "class":`ref-name stash-msg`,});w.prunable;actions.wtName(w); }
        if(!w.dirty_known){ { svelteHTML.createElement("span", {   "class":`wt-dirty`,"title":`Changes unknown — status check unavailable`,});  }
        } else if (w.dirty){ { svelteHTML.createElement("span", {   "class":`wt-dirty`,"title":`Uncommitted changes`,});  }}
        if(w.locked){ { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`lock`,"size":12,}});}}
        if(w.prunable){
           { svelteHTML.createElement("span", { "class":`wt-flag mono dim`,});  }
        } else if (isHere){
           { svelteHTML.createElement("span", { "class":`wt-flag mono dim`,});  }
        } else if (w.is_main){
           { svelteHTML.createElement("span", { "class":`wt-open-hint`,});  }
           { svelteHTML.createElement("span", { "class":`wt-flag mono dim`,});  }
        }else{
           { svelteHTML.createElement("span", { "class":`wt-open-hint`,});  }
          if(w.branch){
             { svelteHTML.createElement("span", { "class":`stash-branch mono dim`,});w.branch; }
          }else{
             { svelteHTML.createElement("span", { "class":`wt-flag mono dim`,});  }
          }
        }
        if(openWtBusy === w.path){ { svelteHTML.createElement("span", { "class":`dim`,});  }}
       }}
       { svelteHTML.createElement("button", {         "class":`icon-btn ref-action`,"title":`Actions for worktree ${w.path}`,"aria-label":`Actions for worktree ${w.path}`,"aria-haspopup":`menu`,"onclick":(e) => menus.wtMenu(e, w),}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":13,}});} }
       }
    }
       { svelteHTML.createElement("div", { "class":`dim ref-empty`,});  }
    
    if(worktrees.some((w) => w.prunable)){
       { svelteHTML.createElement("button", {   "class":`ref-row stash-row wt-prune`,"onclick":() => void actions.wtPrune(),});
         { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":12,}});}
         { svelteHTML.createElement("span", { "class":`ref-name`,});   }
       }
    }
  }
 }


if(submodules.length > 0){
   { svelteHTML.createElement("div", { "class":`ref-section`,});
     { svelteHTML.createElement("button", {     "class":`ref-header`,"onclick":() => (submodulesOpen = !submodulesOpen),"aria-expanded":submodulesOpen,});
       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":submodulesOpen ? 'chevronDown' : 'chevronRight',"size":12,}});}
       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`shapes`,"size":12,}});}
       { svelteHTML.createElement("span", {});  }
       { svelteHTML.createElement("span", { "class":`ref-count`,});submodules.length; }
     }
    if(submodulesOpen){
         for(let sub of __sveltets_2_ensureArray(submodules)){sub.path;
         {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('button')));{ svelteHTML.createElement("button", __sveltets_2_union($$action_0), {              "class":`ref-row stash-row`,"title":`${sub.path} @ ${sub.sha.slice(0, 10)}${sub.url ? ` · ${sub.url}` : ''}${sub.state !== 'ok' ? ` · ${sub.state}` : ''}`,"oncontextmenu":(e) => menus.subMenu(e, sub),"aria-label":`Actions for submodule ${sub.path}`,"aria-haspopup":`menu`,"onclick":(e) => menus.subMenu(e, sub),});
           { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`shapes`,"size":12,}});}
           { svelteHTML.createElement("span", { "class":`ref-name stash-msg`,});sub.path; }
          if(sub.state !== 'ok'){
             { svelteHTML.createElement("span", {  "class":`wt-flag mono`,});sub.state !== 'uninitialized';sentenceCase(sub.state); }
          }
           { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":13,}});}
         }}
      }
    }
   }
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const RefSideLists__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type RefSideLists__SvelteComponent_ = ReturnType<typeof RefSideLists__SvelteComponent_>;
/*Ωignore_endΩ*/export default RefSideLists__SvelteComponent_;