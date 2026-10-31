///<reference types="svelte" />
;// Modal to search and attach a product story to a running session.
// Mirrors AttachIssue.svelte but calls POST /sessions/{id}/attach-product
// which also injects the full refined context bundle into the live PTY.

import type { ProductStory } from '../product/types';
import { toastError } from '../../lib/toastError';
import { ws } from '../../lib/stores/workspace.svelte';
import { api } from '../../lib/api/client';
import { toasts } from '../../lib/toast.svelte';
import Modal from '../../lib/components/Modal.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import Badge from '../../lib/components/Badge.svelte';
import { loadErrorText } from '../../lib/loadError';
import { sentenceCase } from '../../lib/labels';
;

  interface Props {
    sessionId: string;
    onclose: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { sessionId, onclose }: Props = $props();

  let stories = $state<ProductStory[]>([]);
  let loading = $state(true);
  let query = $state('');
  let attaching = $state(false);

  // The story list is a global library, but `attach-product` only accepts a
  // story created in the SESSION's workspace (404 otherwise —
  // api-contracts-4-01), so the picker lists just those.
  const sessionWsId = $derived(ws.getSession(sessionId)?.workspace_id ?? ws.currentId);

  // A failed load used to fall through to "No product stories in this
  // workspace." — say it failed, with Retry.
  let loadError = $state<string | null>(null);
  function load(wsId: string): void {
    loading = true;
    loadError = null;
    void api
      .get<ProductStory[]>(`/workspaces/${wsId}/product/stories`)
      .then((s) => { stories = s.filter((x) => x.workspace_id === wsId); loading = false; })
      .catch((e) => {
        loadError = loadErrorText(e);
        loading = false;
      });
  }

  $effect(() => {
    const wsId = sessionWsId;
    if (!wsId) { loading = false; return; }
    load(wsId);
  });

  const filtered = $derived(
    query.trim() === ''
      ? stories
      : stories.filter(
          (s) =>
            s.title.toLowerCase().includes(query.toLowerCase()) ||
            s.source_key.toLowerCase().includes(query.toLowerCase()),
        ),
  );

  async function attach(story: ProductStory): Promise<void> {
    attaching = true;
    try {
      await ws.attachProductStory(sessionId, story.id);
      toasts.success('Context attached', story.title);
      onclose();
    } catch (e) {
      toastError('Couldn’t attach the story', e);
    } finally {
      attaching = false;
    }
  }
;
async () => {

 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {    children:() => { return __sveltets_2_any(0); },"title":`Attach product story`,"width":540,onclose,footer:() => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("button", {   "class":`btn`,"onclick":onclose,});  }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
   { svelteHTML.createElement("div", { "class":`hint`,});
                 
        
   }

   { svelteHTML.createElement("input", {             "dir":`auto`,"aria-label":`Filter product stories`,"class":`search-input`,"type":`search`,"placeholder":`Filter by title or key…`,"bind:value":query,...__sveltets_2_empty({"data-autofocus":true}),});/*Ωignore_startΩ*/() => query = __sveltets_2_any(null);/*Ωignore_endΩ*/}

   { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL1 = new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {            children:() => { return __sveltets_2_any(0); },"what":`product stories`,loading,"error":loadError,"empty":filtered.length === 0,"onretry":() => sessionWsId && load(sessionWsId),"rows":3,emptyView:() => { async ()/*Ωignore_positionΩ*/ => {
       { const $$_etatSytpmE0C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE0C({ target: __sveltets_2_any(), props: {       "icon":stories.length === 0 ? 'box' : 'search',"title":stories.length === 0 ? 'No product stories in this workspace' : 'No stories match your filter',"body":stories.length === 0 ? 'Refine a story in Product first, then attach it here.' : undefined,}});}
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL1.$$prop_def;/*Ωignore_endΩ*/
    
     { svelteHTML.createElement("ul", { "class":`story-list`,});
         for(let story of __sveltets_2_ensureArray(filtered)){story.id;
         { svelteHTML.createElement("li", {});
           { svelteHTML.createElement("button", {       "class":`story-row`,"disabled":attaching,"onclick":() => { void attach(story); },});
             { svelteHTML.createElement("span", { "class":`story-key`,});story.source_key; }
             { svelteHTML.createElement("span", { "class":`story-title`,});story.title; }
             { const $$_egdaB5C = __sveltets_2_ensureComponent(Badge); new $$_egdaB5C({ target: __sveltets_2_any(), props: {  "label":sentenceCase(story.stage),}});}
           }
         }
      }
     }
   LoadState}

  
 Modal}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const AttachProductStory__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type AttachProductStory__SvelteComponent_ = ReturnType<typeof AttachProductStory__SvelteComponent_>;
/*Ωignore_endΩ*/export default AttachProductStory__SvelteComponent_;