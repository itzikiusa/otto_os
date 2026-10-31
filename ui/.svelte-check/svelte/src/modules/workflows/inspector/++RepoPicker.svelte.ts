///<reference types="svelte" />
;// A node's `repo_id` param as a picker over the workspace's repositories (it
// used to be a free-text "Repo ID" to copy from the Git tab). A value that
// isn't in the list — another workspace's repo, or a stale id — stays
// selectable so opening the node never silently rewrites it.

import { untrack } from 'svelte';
import LoadState from '../../../lib/components/LoadState.svelte';
import type { WorkflowNode } from '../../../lib/api/types';
import type { InspectorOptions } from './options.svelte';
import { paramStr, type SetParam } from './util';

;type $$ComponentProps =  {
    id: string; emptyLabel: string; node: WorkflowNode; set: SetParam; wsId: string; pickers: InspectorOptions;
  };function $$render() {

  
  
  
  
  
  
  
  
  

  let { id, emptyLabel, node, set, wsId, pickers }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();
  // Fetched once per workspace, only when a node with a repo picker is open.
  $effect(() => {
    const w = wsId;
    untrack(() => pickers.repos.ensure(w));
  });
  const value = $derived(paramStr(node, 'repo_id'));
  const repos = $derived(pickers.repos.rows(wsId) ?? []);
  const loading = $derived(pickers.repos.loading(wsId));
  const error = $derived(pickers.repos.error(wsId));
;
async () => {

 { svelteHTML.createElement("select", {     id,value,"disabled":loading && repos.length === 0,"onchange":(e) => set('repo_id', e.currentTarget.value),});
   { svelteHTML.createElement("option", {"value":"",});loading && repos.length === 0 ? 'Loading repositories…' : emptyLabel; }
     for(let r of __sveltets_2_ensureArray(repos)){r.id;
     { svelteHTML.createElement("option", {   "value":r.id,"title":r.path,});r.name; }
  }
  if(value && !repos.some((r) => r.id === value)){
     { svelteHTML.createElement("option", {value,});value;     }
  }
 }
if(error){
   { const $$_etatSdaoL0C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL0C({ target: __sveltets_2_any(), props: {       "what":`repositories`,"variant":`compact`,error,"empty":true,"onretry":() => pickers.repos.retry(wsId),}});}
}
};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const RepoPicker__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type RepoPicker__SvelteComponent_ = ReturnType<typeof RepoPicker__SvelteComponent_>;
/*Ωignore_endΩ*/export default RepoPicker__SvelteComponent_;