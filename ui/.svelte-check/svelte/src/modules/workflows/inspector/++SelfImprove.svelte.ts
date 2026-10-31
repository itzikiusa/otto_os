///<reference types="svelte" />
;// self_improve: which agents reflect (empty = the Self-Improvement settings).

import type { WorkflowNode } from '../../../lib/api/types';
import { agentProviders } from '../../../lib/providers';
import { selfImproveProviders as providersOf, toggled, type SetParam } from './util';

;type $$ComponentProps =  { node: WorkflowNode; set: SetParam };function $$render() {

  
  
  
  

  let { node, set }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();
  const selfImproveProviders = (): string[] => providersOf(node);
  function toggleSelfImproveProvider(prov: string): void {
    const next = toggled(selfImproveProviders(), prov);
    set('providers', next.length ? next : undefined);
  }
;
async () => {

 { svelteHTML.createElement("p", { "class":`insp-note`,});
           { svelteHTML.createElement("strong", {});  }
       { svelteHTML.createElement("strong", {});   } 
              
    
 }
 { svelteHTML.createElement("span", { "class":`np-label`,});         }
 { svelteHTML.createElement("div", { "class":`rv-provs`,});
     for(let prov of __sveltets_2_ensureArray(agentProviders())){prov;
     { svelteHTML.createElement("label", {  "class":`rv-chip`,});selfImproveProviders().includes(prov);
       { svelteHTML.createElement("input", {       "type":`checkbox`,"checked":selfImproveProviders().includes(prov),"onchange":() => toggleSelfImproveProvider(prov),});}
      prov;
     }
  }
 }
 { svelteHTML.createElement("p", { "class":`node-hint`,});
          
   { svelteHTML.createElement("strong", {});   }
 }
};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const SelfImprove__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type SelfImprove__SvelteComponent_ = ReturnType<typeof SelfImprove__SvelteComponent_>;
/*Ωignore_endΩ*/export default SelfImprove__SvelteComponent_;