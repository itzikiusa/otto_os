///<reference types="svelte" />
;// The small utility kinds: delay, transform, game_engine, channel_notify,
// budget_gate, human_approval and condition.

import type { WorkflowNode } from '../../../lib/api/types';
import { agentProviders, defaultAgentProvider } from '../../../lib/providers';
import JsonLabel from './JsonLabel.svelte';
import { jsonValue, paramJson, paramNum, paramStr, type SetParam } from './util';

;type $$ComponentProps =  { node: WorkflowNode; set: SetParam; onzoom: (field: string, label: string) => void };function $$render() {

  
  
  
  
  
  

  let { node, set, onzoom }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();
;
async () => {

if(node.kind === 'delay'){
   { svelteHTML.createElement("label", { "for":`np-ms`,});  }
   { svelteHTML.createElement("input", {               "id":`np-ms`,"type":`number`,"min":`0`,"max":`10000`,"step":`100`,"value":paramNum(node, 'ms', 0),"oninput":(e) => set('ms', Number(e.currentTarget.value)),});}
} else if (node.kind === 'transform'){
   { const $$_lebaLnosJ0C = __sveltets_2_ensureComponent(JsonLabel); new $$_lebaLnosJ0C({ target: __sveltets_2_any(), props: {      "forId":`np-json`,"label":`Merge JSON (object)`,"onzoom":() => onzoom('json', 'Merge JSON (object)'),}});}
   { svelteHTML.createElement("textarea", {            "dir":`ltr`,"id":`np-json`,"rows":4,"placeholder":`&#123;&#125;`,"value":paramJson(node, 'json'),"oninput":(e) => set('json', jsonValue(e.currentTarget.value)),}); }
} else if (node.kind === 'game_engine'){
   { svelteHTML.createElement("label", { "for":`np-game`,});  }
   { svelteHTML.createElement("select", {       "id":`np-game`,"value":paramStr(node, 'game') || 'slots',"onchange":(e) => set('game', e.currentTarget.value),});
     { svelteHTML.createElement("option", { "value":`slots`,});  }
     { svelteHTML.createElement("option", { "value":`crash`,});  }
     { svelteHTML.createElement("option", { "value":`scratch`,});  }
   }
} else if (node.kind === 'channel_notify'){
   { svelteHTML.createElement("label", { "for":`np-msg`,});  }
   { svelteHTML.createElement("textarea", {            "dir":`auto`,"id":`np-msg`,"rows":3,"placeholder":`Workflow step completed: &#123;reply&#125;`,"value":paramStr(node, 'message'),"oninput":(e) => set('message', e.currentTarget.value),}); }
   { svelteHTML.createElement("label", { "for":`np-ch`,});  }
   { svelteHTML.createElement("select", {       "id":`np-ch`,"value":paramStr(node, 'channel') || '',"onchange":(e) => set('channel', e.currentTarget.value || undefined),});
     { svelteHTML.createElement("option", {"value":"",});  }
     { svelteHTML.createElement("option", { "value":`slack`,});  }
     { svelteHTML.createElement("option", { "value":`telegram`,});  }
   }
} else if (node.kind === 'budget_gate'){
   { svelteHTML.createElement("label", { "for":`np-provider`,});      }
   { svelteHTML.createElement("select", {       "id":`np-provider`,"value":paramStr(node, 'provider') || defaultAgentProvider(),"onchange":(e) => set('provider', e.currentTarget.value),});
       for(let pv of __sveltets_2_ensureArray(agentProviders())){pv; { svelteHTML.createElement("option", { "value":pv,});pv; }}
   }
   { svelteHTML.createElement("p", { "class":`node-hint`,});             }
} else if (node.kind === 'human_approval'){
   { svelteHTML.createElement("label", { "for":`np-aprompt`,});  }
   { svelteHTML.createElement("input", {            "dir":`auto`,"id":`np-aprompt`,"type":`text`,"placeholder":`Please review and approve to continue`,"value":paramStr(node, 'prompt'),"oninput":(e) => set('prompt', e.currentTarget.value),});}
   { svelteHTML.createElement("p", { "class":`node-hint`,});              }
} else if (node.kind === 'condition'){
   { svelteHTML.createElement("label", { "for":`np-expr`,});  }
   { svelteHTML.createElement("input", {            "dir":`ltr`,"id":`np-expr`,"type":`text`,"placeholder":`e.g. score >= 80`,"value":paramStr(node, 'expr'),"oninput":(e) => set('expr', e.currentTarget.value),});}
   { svelteHTML.createElement("p", { "class":`node-hint`,});         }
}
};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const Utility__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type Utility__SvelteComponent_ = ReturnType<typeof Utility__SvelteComponent_>;
/*Ωignore_endΩ*/export default Utility__SvelteComponent_;