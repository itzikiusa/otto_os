///<reference types="svelte" />
;// Inline preview failure: the message (and source position when known) in
// the pane itself — never a toast — plus Retry when re-running can help.
// It is inserted with its text, so it carries no role: `announceShown`
// speaks the title once when it appears (politely — it re-renders while the
// author types, and the line/column would re-announce on every keystroke).

import Icon from '../../../lib/components/Icon.svelte';
import { announceShown } from '../../../lib/announce.svelte';
;

  interface Props {
    title: string;
    message: string;
    line?: number;
    col?: number;
    onretry?: () => void;
  };function $$render() {

  
  
  
  
  
  
  

  let { title, message, line, col, onretry }: Props = $props();
;
async () => {

 {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('div'),(title)));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {     "class":`perr`,...__sveltets_2_empty({"data-testid":`wb-preview-error`}),});
   { svelteHTML.createElement("div", { "class":`head`,});
     { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":14,}});}
     { svelteHTML.createElement("span", { "class":`title`,});title; }
    if(line !== undefined){
       { svelteHTML.createElement("span", { "class":`pos`,}); line;col !== undefined ? `, column ${col}` : ''; }
    }
   }
   { svelteHTML.createElement("pre", { "class":`msg`,});message; }
  if(onretry){
     { svelteHTML.createElement("button", {     "type":`button`,"class":`btn small`,"onclick":onretry,});  }
  }
 }}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const PreviewError__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type PreviewError__SvelteComponent_ = ReturnType<typeof PreviewError__SvelteComponent_>;
/*Ωignore_endΩ*/export default PreviewError__SvelteComponent_;