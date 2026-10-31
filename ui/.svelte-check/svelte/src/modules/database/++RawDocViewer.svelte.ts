///<reference types="svelte" />
;// Read-only "View full document" (Mongo) / "View raw JSON" (any engine): the
// complete row pretty-printed — the same Extended JSON the document editor
// seeds from ($oid, $date, …) — in a read-only CodeEditor, so every nested
// value is on the page and ⌘F / Find searches all of it (the JSON tree only
// renders its open branches). CodeMirror virtualizes, so a 100 KB+ document
// opens instantly. Mounted by ResultsGrid while `flow.rawDoc` is set.

import { formatBytes } from '../../lib/metric-format';
import Icon from '../../lib/components/Icon.svelte';
import Modal from '../../lib/components/Modal.svelte';
import CodeEditor from '../../lib/components/CodeEditor.svelte';
import type { EditFlow } from './EditFlow.svelte';
import { copyWithToast } from './results-format';
import { claimFind, editorEscGuard } from './doc-modal';
;

  interface Props {
    flow: EditFlow;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  

  let { flow }: Props = $props();

  let editor = $state<ReturnType<typeof CodeEditor> | null>(null);
  let wrap = $state(true);

  $effect(() => claimFind(() => editor?.openSearch()));
  $effect(() => {
    if (!editor) return;
    const raf = requestAnimationFrame(() => editor?.focus());
    return () => cancelAnimationFrame(raf);
  });
;
async () => {

if(flow.rawDoc){
  const doc = flow.rawDoc;
   { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); new $$_ladoM0C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"title":flow.engine === 'mongodb' ? 'Full document' : 'Raw JSON',"width":760,"onclose":() => (flow.rawDoc = null),}});
     { svelteHTML.createElement("div", { "class":`raw-doc`,});
       { svelteHTML.createElement("div", { "class":`rd-tools`,});
         { svelteHTML.createElement("span", { "class":`rd-meta`,});formatBytes(doc.text.length);   }
         { svelteHTML.createElement("span", { "class":`grow`,}); }
         { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => editor?.openSearch(),"aria-label":`Find in document`,"title":`Find in document (⌘F)`,});
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`search`,"size":13,}});}
         }
         { svelteHTML.createElement("button", {         "class":`icon-btn wrap-toggle`,"aria-pressed":wrap,"onclick":() => (wrap = !wrap),"aria-label":`Wrap long lines`,"title":`Wrap long lines`,});
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`text`,"size":13,}});}
         }
         { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":() => copyWithToast(doc.text, ['Copied', 'Full document copied']),"title":`Copy the full document`,});
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`copy`,"size":12,}});}
         }
       }
       {const $$action_0 = __sveltets_2_ensureAction(editorEscGuard(svelteHTML.mapElementTag('div')));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {  "class":`rd-body`,});
         { const $$_rotidEedoC3C = __sveltets_2_ensureComponent(CodeEditor); const $$_rotidEedoC3 = new $$_rotidEedoC3C({ target: __sveltets_2_any(), props: {             "findOwner":false,"path":`document.json`,"root":"","content":doc.text,"readOnly":true,wrap,"highlightLineLimit":20000,}});editor = $$_rotidEedoC3;}
       }}
     }
   Modal}
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const RawDocViewer__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type RawDocViewer__SvelteComponent_ = ReturnType<typeof RawDocViewer__SvelteComponent_>;
/*Ωignore_endΩ*/export default RawDocViewer__SvelteComponent_;