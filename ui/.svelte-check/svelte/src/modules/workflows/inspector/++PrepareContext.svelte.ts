///<reference types="svelte" />
;// prepare_context: fetch the input's Jira ticket into a context file, then
// (with a prompt) run an agent to consolidate a brief.

import type { WorkflowNode } from '../../../lib/api/types';
import IdPicker from './IdPicker.svelte';
import ProviderModel from './ProviderModel.svelte';
import type { InspectorOptions } from './options.svelte';
import { jiraAccountOptions, listValue, paramBool, paramList, paramStr, type SetParam } from './util';

;type $$ComponentProps =  { node: WorkflowNode; set: SetParam; pickers: InspectorOptions };function $$render() {

  
  
  
  
  
  
  

  let { node, set, pickers }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();
;
async () => {

 { svelteHTML.createElement("p", { "class":`insp-note`,});
               
                 
       
 }
 { svelteHTML.createElement("label", { "for":`np-pc-prompt`,});        }
  { svelteHTML.createElement("textarea", {             "dir":`auto`,"id":`np-pc-prompt`,"class":`np-prompt np-prompt-lg`,"rows":14,"placeholder":`e.g. Read the ticket + repos and produce a consolidated brief…`,"value":paramStr(node, 'prompt'),"oninput":(e) => set('prompt', e.currentTarget.value),}); }
 { const $$_ledoMredivorP0C = __sveltets_2_ensureComponent(ProviderModel); new $$_ledoMredivorP0C({ target: __sveltets_2_any(), props: {  node,set,}});}
 { svelteHTML.createElement("label", { "for":`np-pc-skills`,});  }
 { svelteHTML.createElement("input", {            "dir":`auto`,"id":`np-pc-skills`,"type":`text`,"placeholder":`e.g. golang-testing`,"value":paramList(node, 'skills'),"oninput":(e) => set('skills', listValue(e.currentTarget.value)),});}
 { svelteHTML.createElement("label", { "for":`np-pc-account`,});       }
 { const $$_rekciPdI0C = __sveltets_2_ensureComponent(IdPicker); new $$_rekciPdI0C({ target: __sveltets_2_any(), props: {                    "id":`np-pc-account`,"value":paramStr(node, 'account_id'),"cache":pickers.accounts,"key":`me`,"toOptions":jiraAccountOptions,"noun":`Jira account`,"emptyLabel":`The default Jira account`,"placeholder":`inherits the default Jira account`,"what":`Jira accounts`,"onchange":(v) => set('account_id', v || undefined),}});}
 { svelteHTML.createElement("label", { "class":`np-chk`,});
   { svelteHTML.createElement("input", {       "type":`checkbox`,"checked":paramBool(node, 'require'),"onchange":(e) => set('require', e.currentTarget.checked || undefined),});}        
 }
};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const PrepareContext__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type PrepareContext__SvelteComponent_ = ReturnType<typeof PrepareContext__SvelteComponent_>;
/*Ωignore_endΩ*/export default PrepareContext__SvelteComponent_;