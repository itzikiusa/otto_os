///<reference types="svelte" />
;
import { plural } from '../../lib/plural';
// Sync collections with a git repo connected to this workspace, as Postman
// collection files under collections/. Pull imports them; Commit & push
// exports every top-level collection. Outward-facing (a commit + push), so
// the sheet says where it goes and what is written before the button.
import Icon from '../../lib/components/Icon.svelte';
import Modal from '../../lib/components/Modal.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../lib/loadError';
import { apiClient } from '../../lib/stores/apiClient.svelte';
import { api } from '../../lib/api/client';
import { ws } from '../../lib/stores/workspace.svelte';
import type { Repo } from '../../lib/api/types';
;

  interface Props { onclose: () => void };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  

  let { onclose }: Props = $props();

  let repos = $state<Repo[]>([]);
  let loading = $state(true);
  let loadError = $state('');
  let repoId = $state('');
  let commitMsg = $state('Update API collections');
  let branch = $state('');
  let busy = $state(false);
  let pullError = $state('');

  const roots = $derived(apiClient.collections.filter((c) => (c.parent_id ?? null) === null));
  const repo = $derived(repos.find((r) => r.id === repoId) ?? null);

  async function load(): Promise<void> {
    if (!ws.currentId) return;
    loading = true;
    loadError = '';
    try {
      repos = await api.get<Repo[]>(`/workspaces/${ws.currentId}/repos`);
      if (repos.length && !repoId) repoId = repos[0].id;
    } catch (e) {
      loadError = loadErrorText(e);
    } finally {
      loading = false;
    }
  }
  $effect(() => { void load(); });

  async function pull(): Promise<void> {
    busy = true;
    pullError = '';
    try {
      await apiClient.gitPullCollections(repoId);
      onclose();
    } catch (e) {
      pullError = loadErrorText(e);
    } finally {
      busy = false;
    }
  }
  async function push(): Promise<void> {
    busy = true;
    const ok = await apiClient.gitPushCollections(repoId, commitMsg, branch.trim() || null);
    busy = false;
    if (ok) onclose();
  }
;
async () => {

 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {    children:() => { return __sveltets_2_any(0); },"title":`Sync collections with Git`,"width":500,onclose,footer:() => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("button", {   "class":`btn`,"onclick":onclose,});  }
    if(repos.length > 0){
       { svelteHTML.createElement("button", {     "class":`btn`,"onclick":pull,"disabled":busy || !repoId,}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`download`,"size":13,}});}  }
       { svelteHTML.createElement("button", {       "class":`btn primary`,"onclick":push,"disabled":busy || !repoId || roots.length === 0,"title":roots.length === 0 ? 'No collections to push yet — save a request into a collection first' : undefined,});busy ? 'Working…' : 'Commit & push'; }
    }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
  if(loading || loadError){
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {         "what":`this workspace’s repositories`,loading,"error":loadError,"empty":true,"onretry":load,}});}
  } else if (repos.length === 0){
     { svelteHTML.createElement("p", { "class":`dim`,});                 }
  }else{
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`git-repo`,});  }
       { svelteHTML.createElement("select", {     "id":`git-repo`,"class":`input`,"bind:value":repoId,});/*Ωignore_startΩ*/() => repoId = __sveltets_2_any(null);/*Ωignore_endΩ*/
           for(let r of __sveltets_2_ensureArray(repos)){r.id; { svelteHTML.createElement("option", { "value":r.id,});r.name; }}
       }
     }
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`git-msg`,});  }
       { svelteHTML.createElement("input", {        "dir":`auto`,"id":`git-msg`,"class":`input`,"bind:value":commitMsg,});/*Ωignore_startΩ*/() => commitMsg = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     }
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`git-branch`,});  }
       { svelteHTML.createElement("input", {          "dir":`ltr`,"id":`git-branch`,"class":`input mono`,"bind:value":branch,"placeholder":`api-collections-update`,});/*Ωignore_startΩ*/() => branch = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       { svelteHTML.createElement("span", { "class":`hint`,});               }
     }
     { svelteHTML.createElement("p", { "class":`summary`,});
       { svelteHTML.createElement("strong", {});   }  plural(roots.length, 'collection file');
        { svelteHTML.createElement("code", {});  }   { svelteHTML.createElement("strong", {});repo?.name; }branch.trim() ? ` on ${branch.trim()}` : '';     
                      { svelteHTML.createElement("code", {});  }
       { svelteHTML.createElement("strong", {});  }         
              
     }
  }
  if(pullError){
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {        "what":`collection files`,"error":pullError,"empty":true,"onretry":pull,}});}
  }
  
 Modal}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const GitSyncDialog__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type GitSyncDialog__SvelteComponent_ = ReturnType<typeof GitSyncDialog__SvelteComponent_>;
/*Ωignore_endΩ*/export default GitSyncDialog__SvelteComponent_;