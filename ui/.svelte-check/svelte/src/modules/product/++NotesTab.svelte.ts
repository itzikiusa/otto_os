///<reference types="svelte" />
;// Notes tab — list, add, edit, delete internal notes for the selected story.

import { rel } from '../../lib/stores/now.svelte';
import { toastError } from '../../lib/toastError';
import { product } from '../../lib/stores/product.svelte';
import { toasts } from '../../lib/toast.svelte';
import { renderMarkdown } from '../../lib/md';
import { confirmer } from '../../lib/confirm.svelte';
import Modal from '../../lib/components/Modal.svelte';
import Icon from '../../lib/components/Icon.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../lib/loadError';
import { focusFirstField, refocusEditTrigger } from '../../lib/inlineEdit';
import type { ProductNote, NewNoteReq } from './types';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  

  // ── Load notes when story is selected / changes ────────────────────────────
  // A failed load shows inline with Retry — never as "No notes yet".
  let loadError = $state<string | null>(null);
  async function load(): Promise<void> {
    try {
      await product.loadNotes();
      loadError = null;
    } catch (e) {
      loadError = loadErrorText(e);
    }
  }
  $effect(() => {
    product.selectedId;
    if (product.selectedId) {
      void load();
    }
  });

  // ── Add note form ──────────────────────────────────────────────────────────
  let addOpen = $state(false);
  let newBody = $state('');
  let newSection = $state('');
  let addWorking = $state(false);

  function openAdd(): void {
    addOpen = true;
    newBody = '';
    newSection = '';
  }
  function closeAdd(): void {
    addOpen = false;
  }

  // ── Inline edit state ──────────────────────────────────────────────────────
  let editingId = $state<string | null>(null);
  let editBody = $state('');

  function startEdit(n: ProductNote): void {
    editingId = n.id;
    editBody = n.body;
  }
  // Inline-edit focus contract (lib/inlineEdit.ts): the editor takes focus on
  // open; Cancel and a successful save hand it back to the note's Edit button.
  function cancelEdit(): void {
    const id = editingId;
    editingId = null;
    if (id) void refocusEditTrigger(`note-${id}`);
  }

  // ── Busy flags ─────────────────────────────────────────────────────────────
  let savingId = $state<string | null>(null);
  let deletingId = $state<string | null>(null);

  // ── Actions ─────────────────────────────────────────────────────────────────

  async function addNote(): Promise<void> {
    const body = newBody.trim();
    if (!body) return;
    addWorking = true;
    try {
      const req: NewNoteReq = {
        body,
        section: newSection.trim() || null,
      };
      await product.addNote(req);
      addOpen = false;
      toasts.success('Note added');
    } catch (e) {
      toastError('Couldn’t add note', e);
    } finally {
      addWorking = false;
    }
  }

  async function saveEdit(nid: string): Promise<void> {
    const body = editBody.trim();
    if (!body) return;
    savingId = nid;
    try {
      await product.updateNote(nid, { body });
      editingId = null;
      void refocusEditTrigger(`note-${nid}`);
    } catch (e) {
      toastError('Couldn’t save note', e);
    } finally {
      savingId = null;
    }
  }

  async function deleteNote(n: ProductNote): Promise<void> {
    const preview = n.body.length > 80 ? n.body.slice(0, 80) + '…' : n.body;
    if (!(await confirmer.ask(`Delete this note?\n\n“${preview}”`, { title: 'Delete note', confirmLabel: 'Delete', danger: true }))) return;
    deletingId = n.id;
    try {
      await product.deleteNote(n.id);
    } catch (e) {
      toastError('Couldn’t delete note', e);
    } finally {
      deletingId = null;
    }
  }

  // ── Helpers ────────────────────────────────────────────────────────────────

  function fmtDate(s: string): string {
    try { return rel(s); } catch { return s; }
  }
;
async () => {

if(!product.selectedId){
   { svelteHTML.createElement("div", { "class":`muted`,});   }
}else{
   { svelteHTML.createElement("div", { "class":`ntab`,});

    
     { svelteHTML.createElement("div", { "class":`toolbar`,});
       { svelteHTML.createElement("span", { "class":`grow`,}); }
       { svelteHTML.createElement("button", {   "class":`btn small primary`,"onclick":openAdd,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}   }
     }

    
    
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL1 = new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {         children:() => { return __sveltets_2_any(0); },"what":`notes`,"loading":product.loadingNotes,"error":loadError,"empty":product.notes.length === 0,"onretry":() => void load(),emptyView:() => { async ()/*Ωignore_positionΩ*/ => {
         { svelteHTML.createElement("div", { "class":`muted`,});          }
      };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL1.$$prop_def;/*Ωignore_endΩ*/
      
         { svelteHTML.createElement("div", { "class":`n-list`,});
             for(let n of __sveltets_2_ensureArray(product.notes)){n.id;
             { svelteHTML.createElement("div", { "class":`n-card`,});

              
               { svelteHTML.createElement("div", { "class":`n-header`,});
                 { svelteHTML.createElement("div", { "class":`n-meta-row`,});
                  if(n.section){
                     { svelteHTML.createElement("span", { "class":`section-chip`,});n.section; }
                  }
                   { svelteHTML.createElement("span", {   "class":`n-meta`,"title":n.author_id ? `Author id: ${n.author_id}` : undefined,});fmtDate(n.created_at); }
                 }
                if(editingId !== n.id){
                   { svelteHTML.createElement("div", { "class":`n-actions`,});
                     { svelteHTML.createElement("button", {             "class":`btn small ghost`,"onclick":() => startEdit(n),"disabled":savingId === n.id || deletingId === n.id,...__sveltets_2_empty({"data-edit-for":`note-${n.id}`}),"aria-label":`Edit note${n.section ? ` ${n.section}` : ''} from ${fmtDate(n.created_at)}`,"title":`Edit`,});  }
                     { svelteHTML.createElement("button", {         "class":`btn small danger`,"onclick":() => deleteNote(n),"disabled":savingId === n.id || deletingId === n.id,"title":`Delete`,});
                      deletingId === n.id ? 'Deleting…' : 'Delete';
                     }
                   }
                }
               }

              
              if(editingId === n.id){
                 {const $$action_0 = __sveltets_2_ensureAction(focusFirstField(svelteHTML.mapElementTag('div')));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {  "class":`edit-wrap`,});
                   { svelteHTML.createElement("textarea", {              "dir":`auto`,"class":`edit-text`,"bind:value":editBody,"rows":4,"aria-label":`Note (Markdown)`,"placeholder":`e.g. **Decision:** limits are per brand, not per player`,"disabled":savingId === n.id,});/*Ωignore_startΩ*/() => editBody = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
                   { svelteHTML.createElement("div", { "class":`edit-actions`,});
                     { svelteHTML.createElement("button", {       "class":`btn small primary`,"onclick":() => saveEdit(n.id),"disabled":savingId === n.id || !editBody.trim(),});
                      savingId === n.id ? 'Saving…' : 'Save';
                     }
                     { svelteHTML.createElement("button", {       "class":`btn small ghost`,"onclick":cancelEdit,"disabled":savingId === n.id,});  }
                   }
                 }}
              }else{
                 { svelteHTML.createElement("div", { "class":`n-body md-body`,}); renderMarkdown(n.body); }
              }
             }
          }
         }
    
     LoadState}

    
    if(addOpen){
       { const $$_ladoM1C = __sveltets_2_ensureComponent(Modal); const $$_ladoM1 = new $$_ladoM1C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"title":`Add note`,"width":480,"onclose":closeAdd,footer:() => { async ()/*Ωignore_positionΩ*/ => {
           { svelteHTML.createElement("button", {     "class":`btn`,"onclick":closeAdd,"disabled":addWorking,});  }
           { svelteHTML.createElement("button", {       "class":`btn primary`,"onclick":addNote,"disabled":addWorking || !newBody.trim(),});
            addWorking ? 'Adding…' : 'Add note';
           }
        };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM1.$$prop_def;/*Ωignore_endΩ*/
         { svelteHTML.createElement("div", { "class":`nt-add-body`,});
           { svelteHTML.createElement("label", { "class":`form-label`,});  { svelteHTML.createElement("span", { "class":`req`,});  }
             { svelteHTML.createElement("textarea", {            "dir":`auto`,"class":`form-textarea`,"bind:value":newBody,"rows":5,"placeholder":`e.g. **Decision:** limits are per brand, not per player`,"disabled":addWorking,});/*Ωignore_startΩ*/() => newBody = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
           }
           { svelteHTML.createElement("label", { "class":`form-label`,}); 
             { svelteHTML.createElement("input", {          "dir":`auto`,"class":`form-input`,"bind:value":newSection,"placeholder":`e.g. scope, edge-cases, decisions`,"disabled":addWorking,});/*Ωignore_startΩ*/() => newSection = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           }
         }
        
       Modal}
    }
   }
}


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const NotesTab__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type NotesTab__SvelteComponent_ = ReturnType<typeof NotesTab__SvelteComponent_>;
/*Ωignore_endΩ*/export default NotesTab__SvelteComponent_;