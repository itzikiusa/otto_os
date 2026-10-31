///<reference types="svelte" />
;// Whole-document editor (JSON / Vertical views): the full row as one JSON
// object; Save builds a Mongo updateOne (or a per-changed-column SQL UPDATE)
// and opens the normal review modal. With `rowIdx === -1` it is the INSERT
// editor (insertOne / INSERT from the typed JSON). Mounted by ResultsGrid
// while `flow.docEditor` is set, inside the shared Modal. The draft lives in
// a CodeEditor (JSON mode) so a large document is searchable: ⌘F and the
// Find button open its search panel; Esc there closes the panel, not the modal.

import { untrack } from 'svelte';
import Icon from '../../lib/components/Icon.svelte';
import Modal from '../../lib/components/Modal.svelte';
import CodeEditor from '../../lib/components/CodeEditor.svelte';
import type { EditFlow } from './EditFlow.svelte';
import { claimFind, editorEscGuard } from './doc-modal';
import { confirmer } from '../../lib/confirm.svelte';
;

  interface Props {
    flow: EditFlow;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { flow }: Props = $props();
  const inserting = $derived((flow.docEditor?.rowIdx ?? 0) < 0);

  let editor = $state<ReturnType<typeof CodeEditor> | null>(null);
  let wrap = $state(false);
  // The editor seeds from the draft once; later `draft` writes are its own
  // edits echoing back (a reopen remounts this component).
  const initial = untrack(() => flow.docEditor?.draft ?? '');

  // Esc / ✕ / backdrop / Cancel all land here: an edited draft asks first, so
  // a stray Esc (or a misclick on the backdrop) never silently drops the edit.
  let closing = false;
  async function close() {
    const open = flow.docEditor;
    if (!open || closing) return;
    if (open.draft !== initial) {
      closing = true;
      const discard = await confirmer.ask('Discard your changes to this document?', {
        title: 'Discard changes',
        confirmLabel: 'Discard',
        danger: true,
      });
      closing = false;
      if (!discard) {
        editor?.focus();
        return;
      }
    }
    if (flow.docEditor === open) flow.docEditor = null;
  }

  $effect(() => claimFind(() => editor?.openSearch()));
  // After the Modal's own initial focus (it only knows form controls).
  $effect(() => {
    if (!editor) return;
    const raf = requestAnimationFrame(() => editor?.focus());
    return () => cancelAnimationFrame(raf);
  });
;
async () => {

if(flow.docEditor){
   { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); new $$_ladoM0C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"title":inserting ? 'Insert document' : 'Edit document',"width":720,"onclose":close,}});
     { svelteHTML.createElement("div", { "class":`cell-viewer`,});
       { svelteHTML.createElement("div", { "class":`cv-tools`,});
         { svelteHTML.createElement("p", { "class":`cv-hint`,});
          if(inserting){
            flow.engine === 'mongodb' ? 'insertOne' : 'INSERT';     }else{        }
         }
         { svelteHTML.createElement("span", { "class":`grow`,}); }
         { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => editor?.openSearch(),"aria-label":`Find in document`,"title":`Find in document (⌘F)`,});
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`search`,"size":13,}});}
         }
         { svelteHTML.createElement("button", {         "class":`icon-btn wrap-toggle`,"aria-pressed":wrap,"onclick":() => (wrap = !wrap),"aria-label":`Wrap long lines`,"title":`Wrap long lines`,});
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`text`,"size":13,}});}
         }
       }
       {const $$action_0 = __sveltets_2_ensureAction(editorEscGuard(svelteHTML.mapElementTag('div')));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {  "class":`cv-edit`,});
         { const $$_rotidEedoC3C = __sveltets_2_ensureComponent(CodeEditor); const $$_rotidEedoC3 = new $$_rotidEedoC3C({ target: __sveltets_2_any(), props: {                  "findOwner":false,"path":`document.json`,"root":"","content":initial,"readOnly":false,wrap,"highlightLineLimit":20000,"onchange":(v) => { if (flow.docEditor) flow.docEditor.draft = v; },"onsubmit":() => flow.saveDocEdit(),}});editor = $$_rotidEedoC3;}
       }}
       { svelteHTML.createElement("div", { "class":`cv-foot`,});
        if(flow.docEditor.err){ { svelteHTML.createElement("span", { "class":`cv-err`,});flow.docEditor.err; }}
         { svelteHTML.createElement("span", { "class":`grow`,}); }
         { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":close,});  }
         { svelteHTML.createElement("button", {     "class":`btn small primary`,"onclick":() => flow.saveDocEdit(),"title":`Validate and review the statement (⌘⏎)`,});  }
       }
     }
   Modal}
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const DocEditor__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type DocEditor__SvelteComponent_ = ReturnType<typeof DocEditor__SvelteComponent_>;
/*Ωignore_endΩ*/export default DocEditor__SvelteComponent_;