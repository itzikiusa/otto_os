///<reference types="svelte" />
;// Full Git view for the Side panel. Auto-detects the repo for the panel's
// folder — the focused session's cwd in an agent session, else the
// workspace folder or the one picked in the panel (registers it
// idempotently), then embeds the same
// RepoView the Git module uses — branch + push/pull/fetch toolbar and the
// Graph / Changes / History / Pull Requests tabs — with panel-local tab state
// so it never navigates away from the page it sits beside. No Focus tab
// here: that view is cross-repo, it lives on the Git page.

import { git } from '../../lib/stores/git.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { router } from '../../lib/router.svelte';
import { rightPanel } from '../../lib/stores/rightPanel.svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import RepoView from './RepoView.svelte';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let panelTab = $state('changes');

  // When the panel's folder changes (focused session, picked folder), detect
  // the repo at it.
  $effect(() => {
    const cwd = rightPanel.folder;
    const wsId = ws.currentId;
    if (cwd && wsId) void git.detectFor(wsId, cwd);
  });
;
async () => {

 { svelteHTML.createElement("div", { "class":`gpanel`,});
  if(git.detecting && !git.primary){
     { svelteHTML.createElement("div", { "class":`gp-pad`,}); { const $$_notelekS2C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS2C({ target: __sveltets_2_any(), props: {    "rows":4,"height":26,}});} }
  } else if (!git.primary){
     { svelteHTML.createElement("div", { "class":`gp-empty`,});
      if(git.reposError){
         { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {          "what":`this workspace’s repositories`,"variant":`compact`,"error":git.reposError,"empty":true,"onretry":() => ws.currentId && void git.retryRepos(ws.currentId, rightPanel.folder),}});}
      } else if (git.notARepo){
         { svelteHTML.createElement("p", { "class":`dim`,});rightPanel.session ? 'This session’s folder' : 'This folder';     }
      }else{
         { svelteHTML.createElement("p", { "class":`dim`,});   rightPanel.session ? 'this session' : 'this workspace';  }
      }
       { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => router.go('git'),});   }
     }
  }else{
    const primary = git.primary;
    if(primary){
      
      primary.id; {
         { const $$_weiVopeR1C = __sveltets_2_ensureComponent(RepoView); new $$_weiVopeR1C({ target: __sveltets_2_any(), props: {      "repo":primary,"tab":panelTab,"embedded":true,"onTab":(t) => (panelTab = t),}});}
      }
    }
  }
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const GitPanel__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type GitPanel__SvelteComponent_ = ReturnType<typeof GitPanel__SvelteComponent_>;
/*Ωignore_endΩ*/export default GitPanel__SvelteComponent_;