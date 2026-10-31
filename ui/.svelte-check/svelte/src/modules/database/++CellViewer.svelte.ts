///<reference types="svelte" />
;
import { formatBytes } from '../../lib/metric-format';
import { focusOnMount } from '../../lib/focusOnMount';
// The expandable cell viewer: full value in a modal, SQL formatting toggle,
// Copy, and — when the cell belongs to an editable column — an in-viewer
// editor whose Save hands the draft to the normal cell-edit review flow.
// Mounted by ResultsGrid while `flow.viewer` is set; the shared Modal owns
// the sheet, focus trap, Esc and the pushModal registration.
import Icon from '../../lib/components/Icon.svelte';
import Modal from '../../lib/components/Modal.svelte';
import type { EditFlow } from './EditFlow.svelte';
import { copyWithToast } from './results-format';
;

  interface Props {
    flow: EditFlow;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  

  let { flow }: Props = $props();
  // A path-level change parked on this column (Vertical view): saving a
  // whole-cell draft from here replaces it (EditFlow's conflict rule).
  const nestedPending = $derived.by(() => {
    const e = flow.viewer?.edit;
    const name = e ? flow.result?.columns[e.colIdx]?.name : undefined;
    return !!e && !!name && flow.pendingValue(e.rowIdx, e.colIdx) === undefined && flow.hasPendingUnder(e.rowIdx, name);
  });

  // A multi-MB value (a big Mongo document, a blob) laid out in one wrapping
  // <pre> took seconds to open. Show the head; "Show all" renders the rest on
  // request. Copy (and the editor) always use the FULL value.
  const VIEW_MAX = 256 * 1024;
  /** The viewer object the user asked to see in full (resets per cell). */
  let expandedFor = $state<object | null>(null);
  const clipped = $derived(
    !!flow.viewer && expandedFor !== flow.viewer && flow.viewerText.length > VIEW_MAX,
  );
  const shownText = $derived(clipped ? flow.viewerText.slice(0, VIEW_MAX) : flow.viewerText);
;
async () => {

if(flow.viewer){
   { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); new $$_ladoM0C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"title":`Cell value`,"width":720,"onclose":() => (flow.viewer = null),}});
     { svelteHTML.createElement("div", { "class":`cell-viewer`,});
       { svelteHTML.createElement("div", { "class":`cv-tools`,});
        if(flow.viewer.sql){
           { svelteHTML.createElement("button", {          "class":`btn small cv-toggle`,"aria-pressed":!!flow.viewer.formatted,"onclick":() => (flow.viewer && (flow.viewer.formatted = !flow.viewer.formatted)),"title":`Toggle SQL formatting`,});flow.viewer.formatted;
             { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`grid`,"size":12,}});}flow.viewer.formatted ? 'Formatted' : 'Raw';
           }
        }
        if(flow.viewer.edit && !flow.viewerEditing){
           { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":() => flow.startViewerEdit(),"title":`Edit this cell value`,});
             { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`edit`,"size":12,}});}
           }
        }
         { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":() => copyWithToast(flow.viewerText, ['Copied', 'Full cell value copied']),"title":`Copy full value`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`file`,"size":12,}});}  }
       }
      if(nestedPending){
         { svelteHTML.createElement("div", { "class":`cv-pending`,});              }
      }
      if(flow.viewerEditing){
         {const $$action_0 = __sveltets_2_ensureAction(focusOnMount(svelteHTML.mapElementTag('textarea')));{ svelteHTML.createElement("textarea", __sveltets_2_union($$action_0), {             "dir":`ltr`,"aria-label":`Cell value`,"class":`cv-edit mono`,"bind:value":flow.viewerDraft,"spellcheck":`false`,"onkeydown":(e) => {
            if (e.key === 'Escape') { e.stopPropagation(); flow.viewerEditing = false; flow.viewerErr = null; }
            if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) flow.saveViewerEdit();
          },});/*Ωignore_startΩ*/() => flow.viewerDraft = __sveltets_2_any(null);/*Ωignore_endΩ*/ }}
         { svelteHTML.createElement("div", { "class":`cv-foot`,});
          if(flow.viewerErr){ { svelteHTML.createElement("span", { "class":`cv-err`,});flow.viewerErr; }}
           { svelteHTML.createElement("span", { "class":`grow`,}); }
           { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":() => { flow.viewerEditing = false; flow.viewerErr = null; },});  }
           { svelteHTML.createElement("button", {     "class":`btn small primary`,"onclick":() => flow.saveViewerEdit(),"title":`Validate and review the update (⌘⏎)`,});  }
         }
      }else{
         { svelteHTML.createElement("pre", { "class":`cv-body mono`,});shownText; }
        if(clipped){
           { svelteHTML.createElement("div", { "class":`cv-more`,});
             { svelteHTML.createElement("span", {});   formatBytes(VIEW_MAX);  formatBytes(flow.viewerText.length);       }
             { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":() => (expandedFor = flow.viewer),});  }
           }
        }
      }
     }
   Modal}
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const CellViewer__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type CellViewer__SvelteComponent_ = ReturnType<typeof CellViewer__SvelteComponent_>;
/*Ωignore_endΩ*/export default CellViewer__SvelteComponent_;