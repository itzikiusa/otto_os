///<reference types="svelte" />
;// canvas: a diagram prompt, the mode (Mermaid / Excalidraw) and the agent.

import type { WorkflowNode } from '../../../lib/api/types';
import ProviderModel from './ProviderModel.svelte';
import { paramStr, type SetParam } from './util';

;type $$ComponentProps =  { node: WorkflowNode; set: SetParam };function $$render() {

  
  
  
  

  let { node, set }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();
;
async () => {

 { svelteHTML.createElement("label", { "for":`np-cprompt`,});  }
  { svelteHTML.createElement("textarea", {             "dir":`auto`,"id":`np-cprompt`,"class":`np-prompt`,"rows":7,"placeholder":`Diagram the request flow described in the input…`,"value":paramStr(node, 'prompt'),"oninput":(e) => set('prompt', e.currentTarget.value),}); }
 { svelteHTML.createElement("label", { "for":`np-cmode`,});  }
  { svelteHTML.createElement("select", {      "id":`np-cmode`,"value":paramStr(node, 'mode') || 'mermaid',"onchange":(e) => set('mode', e.currentTarget.value),});
   { svelteHTML.createElement("option", { "value":`mermaid`,});  }
   { svelteHTML.createElement("option", { "value":`excalidraw`,});  }
 }
 { const $$_ledoMredivorP0C = __sveltets_2_ensureComponent(ProviderModel); new $$_ledoMredivorP0C({ target: __sveltets_2_any(), props: {  node,set,}});}
};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const Canvas__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type Canvas__SvelteComponent_ = ReturnType<typeof Canvas__SvelteComponent_>;
/*Ωignore_endΩ*/export default Canvas__SvelteComponent_;