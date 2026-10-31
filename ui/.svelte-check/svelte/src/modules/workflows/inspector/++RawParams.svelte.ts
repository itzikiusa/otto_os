///<reference types="svelte" />
;// Fallback raw-JSON params editor for unrecognised or future node kinds.

import type { WorkflowNode } from '../../../lib/api/types';

;type $$ComponentProps =  { node: WorkflowNode; setParams: (params: unknown) => void };function $$render() {

  
  

  let { node, setParams }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();
;
async () => {


 { svelteHTML.createElement("label", { "for":`np-raw`,});  }
  { svelteHTML.createElement("textarea", {           "dir":`ltr`,"id":`np-raw`,"rows":5,"placeholder":`&#123;&#125;`,"value":node.params != null ? JSON.stringify(node.params, null, 2) : '',"oninput":(e) => {
    try {
      setParams(JSON.parse(e.currentTarget.value));
    } catch { /* keep typing */ }
  },}); }
};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const RawParams__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type RawParams__SvelteComponent_ = ReturnType<typeof RawParams__SvelteComponent_>;
/*Ωignore_endΩ*/export default RawParams__SvelteComponent_;