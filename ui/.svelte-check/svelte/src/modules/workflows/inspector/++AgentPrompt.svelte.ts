///<reference types="svelte" />
;// agent_prompt: the prompt, provider/model and skills.

import type { WorkflowNode } from '../../../lib/api/types';
import ProviderModel from './ProviderModel.svelte';
import { listValue, paramList, paramStr, type SetParam } from './util';

;type $$ComponentProps =  { node: WorkflowNode; set: SetParam };function $$render() {

  
  
  
  

  let { node, set }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();
;
async () => {

 { svelteHTML.createElement("label", { "for":`np-prompt`,});  }
  { svelteHTML.createElement("textarea", {           "dir":`auto`,"id":`np-prompt`,"class":`np-prompt np-prompt-lg`,"rows":14,"value":paramStr(node, 'prompt'),"oninput":(e) => set('prompt', e.currentTarget.value),}); }
 { const $$_ledoMredivorP0C = __sveltets_2_ensureComponent(ProviderModel); new $$_ledoMredivorP0C({ target: __sveltets_2_any(), props: {  node,set,}});}
 { svelteHTML.createElement("label", { "for":`np-skills`,});  }
 { svelteHTML.createElement("input", {            "dir":`auto`,"id":`np-skills`,"type":`text`,"placeholder":`e.g. golang-testing, golang-code-review`,"value":paramList(node, 'skills'),"oninput":(e) => set('skills', listValue(e.currentTarget.value)),});}
};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const AgentPrompt__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type AgentPrompt__SvelteComponent_ = ReturnType<typeof AgentPrompt__SvelteComponent_>;
/*Ωignore_endΩ*/export default AgentPrompt__SvelteComponent_;