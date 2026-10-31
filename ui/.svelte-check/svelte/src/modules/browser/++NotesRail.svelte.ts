///<reference types="svelte" />
;// Notes rail: one row per DOM annotation ("mark") on the currently-open
// page, newest last. A thin renderer over `browser.annotations` — mutation
// (edit comment / delete / send-to-session) goes through the `browser`
// store, never the API module directly (matches the scheduledTasks /
// runWithOtto convention). Live rows arrive via the `browser_annotation_added`
// WS event, which the store applies in place — no polling here.

import type { BrowserAnnotation } from '../../lib/api/types';
import { toastError } from '../../lib/toastError';
import { browser } from '../../lib/stores/browser.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import Icon from '../../lib/components/Icon.svelte';
import SendToSession from './SendToSession.svelte';

;type $$ComponentProps =  { annotations: BrowserAnnotation[] };function $$render() {

  
  
  
  
  
  

  
  
  
  
  
  

  let { annotations }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();

  let editingId = $state<string | null>(null);
  let editText = $state('');

  function startEdit(a: BrowserAnnotation): void {
    editingId = a.id;
    editText = a.comment;
  }

  async function saveEdit(id: string): Promise<void> {
    try {
      await browser.updateAnnotationComment(id, editText);
      editingId = null;
    } catch (e) {
      toastError('Couldn’t update note', e);
    }
  }

  function cancelEdit(): void {
    editingId = null;
  }

  async function remove(id: string): Promise<void> {
    const mark = annotations.find(a => a.id === id);
    if (!mark || !await confirmer.ask(`Delete this mark and its note? “${mark.text.slice(0, 180)}”`, { title: 'Delete mark', confirmLabel: 'Delete mark', danger: true })) return;
    if (!annotations.some(a => a.id === id)) return;
    try {
      await browser.deleteAnnotation(id);
    } catch (e) {
      toastError('Couldn’t delete mark', e);
    }
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`notes-rail`,});
   { svelteHTML.createElement("div", { "class":`rail-head`,});
     { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`note`,"size":13,}});}
     { svelteHTML.createElement("span", {});  }
     { svelteHTML.createElement("span", { "class":`count`,});annotations.length; }
   }

  if(annotations.length === 0){
     { svelteHTML.createElement("p", { "class":`empty`,});
                      
     }
  }else{
     { svelteHTML.createElement("ul", { "class":`list`,});
         for(let a of __sveltets_2_ensureArray(annotations)){a.id;
         { svelteHTML.createElement("li", { "class":`row`,});
           { svelteHTML.createElement("span", {  "class":`swatch`,});__sveltets_2_ensureType(String, Number, a.color || 'var(--warning)'); }
           { svelteHTML.createElement("div", { "class":`body`,});
             { svelteHTML.createElement("p", { "class":`excerpt`,});a.text; }
            if(editingId === a.id){
               { svelteHTML.createElement("div", { "class":`edit`,});
                 { svelteHTML.createElement("textarea", {              "dir":`auto`,"bind:value":editText,"placeholder":`Check this claim against the spec`,"aria-label":`Note for this mark`,"rows":2,"spellcheck":`false`,"onkeydown":(e) => {
                    if (e.key === 'Escape') { e.preventDefault(); cancelEdit(); }
                    else if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) { e.preventDefault(); void saveEdit(a.id); }
                  },});/*Ωignore_startΩ*/() => editText = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
                 { svelteHTML.createElement("div", { "class":`edit-actions`,});
                   { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":cancelEdit,});  }
                   { svelteHTML.createElement("button", {     "class":`btn small primary`,"onclick":() => saveEdit(a.id),"title":`Save note (⌘↩)`,});  }
                 }
               }
            } else if (a.comment){
               { svelteHTML.createElement("p", { "class":`comment`,});a.comment; }
            }
           }
           { svelteHTML.createElement("div", { "class":`actions`,});
            if(editingId !== a.id){
               { svelteHTML.createElement("button", {       "class":`icon-btn`,"title":`Edit note`,"aria-label":`Edit note`,"onclick":() => startEdit(a),});
                 { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`edit`,"size":12,}});}
               }
            }
             { const $$_noisseSoTdneS4C = __sveltets_2_ensureComponent(SendToSession); new $$_noisseSoTdneS4C({ target: __sveltets_2_any(), props: {  "annotationId":a.id,}});}
             { svelteHTML.createElement("button", {       "class":`icon-btn`,"title":`Delete mark`,"aria-label":`Delete mark`,"onclick":() => remove(a.id),});
               { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":12,}});}
             }
           }
         }
      }
     }
  }
 }


};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const NotesRail__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type NotesRail__SvelteComponent_ = ReturnType<typeof NotesRail__SvelteComponent_>;
/*Ωignore_endΩ*/export default NotesRail__SvelteComponent_;