///<reference types="svelte" />
;// "Save request" sheet: a name and WHERE it goes (collection / folder picked
// from a tree-ordered list, or a new collection created on the spot).
// Replaces the old two-prompt flow with its numbered collection list.

import { announceShown } from '../../lib/announce.svelte';
import { onDestroy } from 'svelte';
import Modal from '../../lib/components/Modal.svelte';
import { apiClient } from '../../lib/stores/apiClient.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { collectionPaths } from '../../lib/api/apiVars';
import type { Id } from '../../lib/api/types';
;

  interface Props {
    title?: string;
    initialName: string;
    initialCollection: Id | null;
    onclose: () => void;
    /** Resolves once the request is saved (the sheet stays open on failure). */
    onsave: (name: string, collectionId: Id | null) => Promise<boolean>;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  

  let { title = 'Save request', initialName, initialCollection, onclose, onsave }: Props = $props();

  let alive = true;
  onDestroy(() => { alive = false; });
  function close(): void {
    alive = false;
    onclose();
  }

  const NEW = '__new__';
  const paths = $derived(collectionPaths(apiClient.collections));
  // svelte-ignore state_referenced_locally
  let name = $state(initialName);
  // svelte-ignore state_referenced_locally
  let target = $state<string>(initialCollection ?? '');
  let newCollection = $state('');
  let busy = $state(false);
  let error = $state('');

  const valid = $derived(name.trim() !== '' && (target !== NEW || newCollection.trim() !== ''));

  async function submit(e?: Event): Promise<void> {
    e?.preventDefault();
    if (!valid || busy) return;
    busy = true;
    error = '';
    const workspaceId = ws.currentId;
    const tabId = apiClient.draft?.tabId;
    const current = () => alive && ws.currentId === workspaceId && apiClient.draft?.tabId === tabId;
    try {
      let collectionId: Id | null = target === '' ? null : target;
      if (target === NEW) {
        const created = await apiClient.saveCollection({ name: newCollection.trim(), parent_id: null });
        if (!current()) return;
        if (!created) { error = 'Couldn’t create the collection. Try again, or pick an existing one.'; return; }
        collectionId = created.id;
        // Keep the completed first step if request persistence needs a retry.
        target = created.id;
      }
      if (await onsave(name.trim(), collectionId) && current()) close();
    } finally {
      busy = false;
    }
  }
;
async () => {

 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {    children:() => { return __sveltets_2_any(0); },title,"width":440,"onclose":close,footer:() => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("button", {   "class":`btn`,"onclick":close,});  }
     { svelteHTML.createElement("button", {     "class":`btn primary`,"onclick":() => void submit(),"disabled":!valid || busy,});busy ? 'Saving…' : 'Save'; }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
   { svelteHTML.createElement("form", {   "class":`save-form`,"onsubmit":submit,});
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`save-req-name`,});  }
       { svelteHTML.createElement("input", {            "dir":`auto`,"id":`save-req-name`,"class":`input`,"bind:value":name,"placeholder":`List customers`,"autocomplete":`off`,});/*Ωignore_startΩ*/() => name = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     }
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`save-req-where`,});  }
       { svelteHTML.createElement("select", {     "id":`save-req-where`,"class":`input`,"bind:value":target,});/*Ωignore_startΩ*/() => target = __sveltets_2_any(null);/*Ωignore_endΩ*/
         { svelteHTML.createElement("option", {"value":"",});   }
           for(let p of __sveltets_2_ensureArray(paths)){p.id;
           { svelteHTML.createElement("option", { "value":p.id,});p.path; }
        }
         { svelteHTML.createElement("option", { "value":NEW,});  }
       }
      if(target === NEW){
         { svelteHTML.createElement("input", {          "dir":`auto`,"class":`input new-col`,"bind:value":newCollection,"placeholder":`Payments API`,"aria-label":`New collection name`,});/*Ωignore_startΩ*/() => newCollection = __sveltets_2_any(null);/*Ωignore_endΩ*/}
      }
       { svelteHTML.createElement("span", { "class":`hint`,});                      }
     }
    if(error){ {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('p'),({ text: error, assertive: true })));{ svelteHTML.createElement("p", __sveltets_2_union($$action_0), {   "class":`err`,});error; }}}
     { svelteHTML.createElement("button", {     "type":`submit`,"hidden":true,"aria-hidden":`true`,"tabindex":-1,}); }
   }
  
 Modal}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const SaveRequestDialog__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type SaveRequestDialog__SvelteComponent_ = ReturnType<typeof SaveRequestDialog__SvelteComponent_>;
/*Ωignore_endΩ*/export default SaveRequestDialog__SvelteComponent_;