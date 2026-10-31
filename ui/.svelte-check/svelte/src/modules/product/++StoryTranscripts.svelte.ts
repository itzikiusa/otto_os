///<reference types="svelte" />
;// The draft story's transcripts (Overview, right column): the paged list
// with lazily loaded bodies, ⌘F over every transcript (collapsed ones too),
// download, remove and the add form. Split out of OverviewTab
// (architecture-5-04); OverviewTab is keyed by story, so a fresh instance —
// and fresh form/expansion state — per story.

import { announceShown } from '../../lib/announce.svelte';
import { tick, untrack } from 'svelte';
import { rel } from '../../lib/stores/now.svelte';
import { toastError } from '../../lib/toastError';
import Icon from '../../lib/components/Icon.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { product } from '../../lib/stores/product.svelte';
import { registerFindProvider } from '../../lib/findProviders';
import { toasts } from '../../lib/toast.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import type { ProductTranscriptSummary } from './types';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  // ── Transcript state ──────────────────────────────────────────────────────
  let newTranscriptTitle = $state('');
  let newTranscriptBody = $state('');
  let addingTranscript = $state(false);
  let expandedTranscripts = $state<Record<string, boolean>>({});

  // First page once per mounted story.
  $effect(() => {
    untrack(() => void product.loadTranscripts());
  });

  async function doAddTranscript(): Promise<void> {
    if (!newTranscriptBody.trim()) return;
    addingTranscript = true;
    try {
      await product.addTranscript({
        title: newTranscriptTitle.trim() || null,
        body: newTranscriptBody.trim(),
      });
      newTranscriptTitle = '';
      newTranscriptBody = '';
      toasts.success('Transcript added');
    } catch (e) {
      toastError('Couldn’t add transcript', e);
    } finally {
      addingTranscript = false;
    }
  }

  async function doDeleteTranscript(t: ProductTranscriptSummary): Promise<void> {
    const ok = await confirmer.ask(
      `Remove transcript “${t.title || 'untitled'}”?`,
      { title: 'Remove transcript', confirmLabel: 'Remove', danger: true },
    );
    if (!ok) return;
    try {
      await product.deleteTranscript(t.id);
      toasts.info('Transcript removed');
    } catch (e) {
      toastError('Couldn’t remove the transcript', e);
    }
  }

  async function toggleTranscript(id: string): Promise<void> {
    expandedTranscripts = { ...expandedTranscripts, [id]: !expandedTranscripts[id] };
    if (expandedTranscripts[id]) await product.loadTranscriptBody(id);
  }

  async function downloadTranscript(t: ProductTranscriptSummary): Promise<void> {
    try {
      const full = await product.getTranscript(t.id);
      const url = URL.createObjectURL(new Blob([full.body], { type: 'text/plain;charset=utf-8' }));
      const anchor = document.createElement('a');
      anchor.href = url; anchor.download = `${t.title || 'transcript'}.txt`; anchor.click();
      setTimeout(() => URL.revokeObjectURL(url), 0);
    } catch (e) { toasts.error('Couldn’t download transcript', product.errMsg(e)); }
  }

  // ⌘F reaches COLLAPSED transcripts too: their bodies aren't mounted, so the
  // page-wide find overlay's DOM walk never saw them. This provider
  // (lib/findProviders.ts) searches title + body of every transcript; going to
  // a match in a collapsed one expands it. The date is `data-find-skip` in the
  // row, so the row's text order is title → body, matching `text(i)`.
  let transcriptListEl = $state<HTMLElement | undefined>();
  $effect(() => {
    if (!transcriptListEl) return;
    const list = transcriptListEl;
    let revealGeneration = 0;
    return registerFindProvider({
      root: () => list,
      count: () => product.transcriptSearchRows.length,
      search: (query, limit, signal) => { revealGeneration++; return product.searchTranscripts(query, limit, signal); },
      text: (i) => {
        const t = product.transcriptSearchRows[i];
        return t ? `${t.title || 'Untitled transcript'}\n${product.transcriptBodies[t.id] ?? ''}` : '';
      },
      reveal: async (i, signal) => {
        const t = product.transcriptSearchRows[i], owner = product.selectedId;
        const generation = revealGeneration;
        if (!t || signal?.aborted) return;
        await product.revealTranscript(t);
        if (signal?.aborted || generation !== revealGeneration || product.selectedId !== owner) return;
        expandedTranscripts = { ...expandedTranscripts, [t.id]: true };
        await tick();
      },
      // Only an expanded row has its body mounted; a collapsed one → reveal.
      rowElement: (i) => {
        const t = product.transcriptSearchRows[i];
        return t && expandedTranscripts[t.id] && product.transcriptBodies[t.id] !== undefined
          ? list.querySelector(`[data-transcript-id="${CSS.escape(t.id)}"]`) : null;
      },
      release: () => { revealGeneration++; product.releaseTranscriptSearch(); },
    });
  });
;
async () => {

 { svelteHTML.createElement("div", { "class":`transcripts-section`,});
   { svelteHTML.createElement("div", { "class":`transcripts-header`,});
     { svelteHTML.createElement("span", { "class":`section-title`,});  }
   }
   { svelteHTML.createElement("div", {  "class":`muted`,...__sveltets_2_empty({"data-find-skip":true}),});              }

  if(product.transcriptsError){
     {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('div'),({ text: `Couldn’t load transcripts. ${product.transcriptsError}`, assertive: true })));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {  });   product.transcriptsError; }}
     { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => void product.loadTranscripts(),});  }
  }
  if(product.loadingTranscripts){
      { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {    "what":`transcripts`,"variant":`compact`,"loading":true,"empty":true,}});}
  } else if (product.transcripts.length === 0 && !product.transcriptsError){
     { svelteHTML.createElement("div", { "class":`muted`,});       }
  }else{
     { const $$_div1 = svelteHTML.createElement("div", {  "class":`transcript-list`,});transcriptListEl = $$_div1;
         for(let t of __sveltets_2_ensureArray(product.transcripts)){t.id;
         { svelteHTML.createElement("div", {   "class":`transcript-item`,...__sveltets_2_empty({"data-transcript-id":t.id}),});
           { svelteHTML.createElement("div", { "class":`transcript-header`,});
             { svelteHTML.createElement("button", {       "class":`transcript-toggle`,"onclick":() => toggleTranscript(t.id),"aria-expanded":expandedTranscripts[t.id] ?? false,});
               { svelteHTML.createElement("span", {   "class":`coll-arrow`,"aria-hidden":`true`,}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":expandedTranscripts[t.id] ? 'chevronDown' : 'chevronRight',"size":12,}});} }
               { svelteHTML.createElement("span", { "class":`transcript-title`,});t.title || 'Untitled transcript'; }
               { svelteHTML.createElement("span", {   "class":`transcript-date`,...__sveltets_2_empty({"data-find-skip":true}),"title":new Date(t.created_at).toLocaleString(),});rel(t.created_at); }
             }
             { svelteHTML.createElement("button", {         "class":`del-transcript-btn`,"onclick":() => doDeleteTranscript(t),"title":`Remove transcript`,"aria-label":`Remove transcript`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});} }
             { svelteHTML.createElement("button", {   "class":`btn small`,...__sveltets_2_empty({"data-find-skip":true}),"onclick":() => void downloadTranscript(t),});  }
           }
          if(expandedTranscripts[t.id]){
            if(product.transcriptBodies[t.id] !== undefined){
               { svelteHTML.createElement("div", { "class":`transcript-body`,});product.transcriptBodies[t.id]; }
            } else if (product.transcriptBodyLoading[t.id]){
               { svelteHTML.createElement("div", {...__sveltets_2_empty({"data-find-skip":true}),});  { const $$_etatSdaoL4C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL4C({ target: __sveltets_2_any(), props: {    "what":`this transcript`,"variant":`compact`,"loading":true,"empty":true,}});} }
            }else{
              if(product.transcriptBodyErrors[t.id]){ {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('div'),({ text: product.transcriptBodyErrors[t.id], assertive: true })));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {  ...__sveltets_2_empty({"data-find-skip":true}),});product.transcriptBodyErrors[t.id]; }}}
               { svelteHTML.createElement("button", {   "class":`btn small`,...__sveltets_2_empty({"data-find-skip":true}),"onclick":() => void product.loadTranscriptBody(t.id),});product.transcriptBodyErrors[t.id] ? 'Retry' : 'Load transcript'; }
            }
          }
         }
      }
     }
  }
  if(product.transcriptPreviousCursors.length || product.transcriptNextCursor){
     { svelteHTML.createElement("div", {...__sveltets_2_empty({"data-find-skip":true}),});
       { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":product.loadingTranscripts || !product.transcriptPreviousCursors.length,"onclick":() => void product.previousTranscriptPage(),});  }
       { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":product.loadingTranscripts || !product.transcriptNextCursor,"onclick":() => void product.loadTranscripts(product.transcriptNextCursor),});  }
     }
  }

  
   { svelteHTML.createElement("div", { "class":`add-transcript-form`,});
     { svelteHTML.createElement("input", {            "dir":`auto`,"class":`input wide-input`,"bind:value":newTranscriptTitle,"placeholder":`e.g. Kickoff call, 12 Oct`,"aria-label":`Transcript title (optional)`,"spellcheck":`false`,});/*Ωignore_startΩ*/() => newTranscriptTitle = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     { svelteHTML.createElement("textarea", {              "dir":`auto`,"aria-label":`Transcript`,"class":`textarea`,"bind:value":newTranscriptBody,"rows":5,"placeholder":`e.g. PO: the limit resets every Monday at midnight…`,"spellcheck":`false`,});/*Ωignore_startΩ*/() => newTranscriptBody = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
     { svelteHTML.createElement("button", {       "class":`btn small`,"onclick":doAddTranscript,"disabled":addingTranscript || !newTranscriptBody.trim(),});
      addingTranscript ? 'Adding…' : 'Add transcript';
     }
   }
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const StoryTranscripts__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type StoryTranscripts__SvelteComponent_ = ReturnType<typeof StoryTranscripts__SvelteComponent_>;
/*Ωignore_endΩ*/export default StoryTranscripts__SvelteComponent_;