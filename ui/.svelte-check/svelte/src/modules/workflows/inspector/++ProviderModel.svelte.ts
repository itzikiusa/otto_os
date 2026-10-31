///<reference types="svelte" />
;// Shared Provider + Model editor for every agent-running node — sourced from
// the live registry (built-ins + custom, e.g. grok) so no node is hardcoded
// to claude. Empty provider = default.

import ModelPicker from '../../../lib/components/ModelPicker.svelte';
import { agentProviders, defaultAgentProvider } from '../../../lib/providers';
import type { WorkflowNode } from '../../../lib/api/types';
import { paramStr, type SetParam } from './util';

;type $$ComponentProps =  { node: WorkflowNode; set: SetParam };function $$render() {

  
  
  
  
  
  
  

  let { node, set }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();
;
async () => {

 { svelteHTML.createElement("label", { "for":`np-provider`,});  }
 { svelteHTML.createElement("select", {     "id":`np-provider`,"value":paramStr(node, 'provider') || '',"onchange":(e) => set('provider', e.currentTarget.value || undefined),});
   { svelteHTML.createElement("option", {"value":"",}); defaultAgentProvider();  }
     for(let p of __sveltets_2_ensureArray(agentProviders())){p; { svelteHTML.createElement("option", { "value":p,});p; }}
 }

 { svelteHTML.createElement("div", { "class":`np-mp`,});
   { const $$_rekciPledoM1C = __sveltets_2_ensureComponent(ModelPicker); new $$_rekciPledoM1C({ target: __sveltets_2_any(), props: {      "provider":paramStr(node, 'provider') || defaultAgentProvider(),"value":paramStr(node, 'model'),"onchange":(m) => set('model', m),}});}
 }
};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ProviderModel__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ProviderModel__SvelteComponent_ = ReturnType<typeof ProviderModel__SvelteComponent_>;
/*Ωignore_endΩ*/export default ProviderModel__SvelteComponent_;