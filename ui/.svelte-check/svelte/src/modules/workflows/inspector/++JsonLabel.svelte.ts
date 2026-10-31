///<reference types="svelte" />
;// Label + zoom button for a cramped node-form JSON field (R10); the big
// editor is the page's modal.

import Icon from '../../../lib/components/Icon.svelte';

;type $$ComponentProps =  { forId: string; label: string; onzoom: () => void };function $$render() {

  
  
  

  let { forId, label, onzoom }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();
;
async () => {

 { svelteHTML.createElement("div", { "class":`np-jsonlabel`,});
   { svelteHTML.createElement("label", { "for":forId,});label; }
   { svelteHTML.createElement("button", {         "type":`button`,"class":`np-zoom`,"title":`Zoom — edit in a big editor`,"aria-label":`Zoom JSON editor`,"onclick":onzoom,});
     { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`maximize`,"size":12,}});}
   }
 }
};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const JsonLabel__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type JsonLabel__SvelteComponent_ = ReturnType<typeof JsonLabel__SvelteComponent_>;
/*Ωignore_endΩ*/export default JsonLabel__SvelteComponent_;