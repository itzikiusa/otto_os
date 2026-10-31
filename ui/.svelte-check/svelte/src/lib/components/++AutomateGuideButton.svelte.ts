///<reference types="svelte" />
;// The Automate chooser outside empty states (S20-18): a header action that
// collapses into the page's ⋯ first (lowest `data-overflow`) and opens the
// guide expanded in a sheet.

import Icon from './Icon.svelte';
import Modal from './Modal.svelte';
import AutomateGuide, { type AutomateModule } from './AutomateGuide.svelte';

;type $$ComponentProps =  { current: AutomateModule };function $$render() {

  
  
  
  
  
  

  let { current }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();
  let open = $state(false);
;
async () => {

  { svelteHTML.createElement("button", {                  "type":`button`,"class":`icon-btn`,...__sveltets_2_empty({"data-overflow":`-20`}),...__sveltets_2_empty({"data-icon":`info`}),...__sveltets_2_empty({"data-label":`Which automation should I use?`}),"aria-label":`Which automation should I use?`,"title":`Which automation should I use?`,...__sveltets_2_empty({"data-testid":`automate-guide-open`}),"onclick":() => (open = true),});
   { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`info`,"size":14,}});}
 }

if(open){
   { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); new $$_ladoM0C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"title":`Which automation should I use?`,"width":560,"onclose":() => (open = false),}});
     { const $$_ediuGetamotuA1C = __sveltets_2_ensureComponent(AutomateGuide); new $$_ediuGetamotuA1C({ target: __sveltets_2_any(), props: {   current,"open":true,"onnavigate":() => (open = false),}});}
   Modal}
}
};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const AutomateGuideButton__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type AutomateGuideButton__SvelteComponent_ = ReturnType<typeof AutomateGuideButton__SvelteComponent_>;
/*Ωignore_endΩ*/export default AutomateGuideButton__SvelteComponent_;