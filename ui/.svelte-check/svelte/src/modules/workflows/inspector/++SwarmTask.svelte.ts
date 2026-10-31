///<reference types="svelte" />
;// swarm_task: pick the swarm / project instead of pasting raw ids. Loaded on
// demand (null = not loaded / failed — the inputs fall back to free text so a
// saved id is still editable); each lookup is tried ONCE per workspace / swarm.

import type { WorkflowNode } from '../../../lib/api/types';
import { untrack } from 'svelte';
import { isUlid, type InspectorOptions } from './options.svelte';
import { paramStr, type SetParam } from './util';

;type $$ComponentProps =  { node: WorkflowNode; set: SetParam; wsId: string; pickers: InspectorOptions };function $$render() {

  
  
  
  
  
  
  

  let { node, set, wsId, pickers }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();
  const sid = $derived(paramStr(node, 'swarm_id'));
  const swarmOpts = $derived(pickers.swarms.rows(wsId));
  const projOpts = $derived(sid ? (pickers.swarmProjects.rows(sid) ?? undefined) : undefined);
  $effect(() => {
    const w = wsId;
    const s = sid;
    const opts = pickers.swarms.rows(w); // re-checked once the list lands
    untrack(() => {
      pickers.swarms.ensure(w);
      // Only a COMPLETE id: one the list knows, or (free text) a full ULID.
      const complete = opts ? opts.some((o) => o.id === s) : isUlid(s);
      if (s && complete) pickers.swarmProjects.ensure(s);
    });
  });
;
async () => {

 { svelteHTML.createElement("label", { "for":`np-swarm`,});  }
if(swarmOpts){
   { svelteHTML.createElement("select", {       "id":`np-swarm`,"value":sid,"onchange":(e) => { set('swarm_id', e.currentTarget.value); set('project_id', ''); },});
     { svelteHTML.createElement("option", {"value":"",});   }
       for(let o of __sveltets_2_ensureArray(swarmOpts)){o.id; { svelteHTML.createElement("option", { "value":o.id,});o.name; }}
    if(sid && !swarmOpts.some((o) => o.id === sid)){ { svelteHTML.createElement("option", { "value":sid,});  sid;  }}
   }
  if(swarmOpts.length === 0){ { svelteHTML.createElement("p", { "class":`node-hint`,});             }}
}else{
   { svelteHTML.createElement("input", {            "dir":`auto`,"id":`np-swarm`,"type":`text`,"placeholder":`Swarm id`,"value":sid,"oninput":(e) => set('swarm_id', e.currentTarget.value),});}
}
 { svelteHTML.createElement("label", { "for":`np-proj`,});  }
if(projOpts){
   { svelteHTML.createElement("select", {       "id":`np-proj`,"value":paramStr(node, 'project_id'),"onchange":(e) => set('project_id', e.currentTarget.value),});
     { svelteHTML.createElement("option", {"value":"",});   }
       for(let o of __sveltets_2_ensureArray(projOpts)){o.id; { svelteHTML.createElement("option", { "value":o.id,});o.name; }}
    if(paramStr(node, 'project_id') && !projOpts.some((o) => o.id === paramStr(node, 'project_id'))){
       { svelteHTML.createElement("option", { "value":paramStr(node, 'project_id'),});  paramStr(node, 'project_id');  }
    }
   }
}else{
   { svelteHTML.createElement("input", {            "dir":`auto`,"id":`np-proj`,"type":`text`,"placeholder":sid ? 'Swarm project id' : 'Choose a swarm first',"value":paramStr(node, 'project_id'),"oninput":(e) => set('project_id', e.currentTarget.value),});}
}
 { svelteHTML.createElement("label", { "for":`np-title`,});  }
 { svelteHTML.createElement("input", {            "dir":`auto`,"id":`np-title`,"type":`text`,"placeholder":`Workflow-generated task title`,"value":paramStr(node, 'title'),"oninput":(e) => set('title', e.currentTarget.value),});}
 { svelteHTML.createElement("label", { "for":`np-desc`,});  }
  { svelteHTML.createElement("textarea", {             "dir":`auto`,"id":`np-desc`,"class":`np-prompt`,"rows":6,"placeholder":`Task details…`,"value":paramStr(node, 'description'),"oninput":(e) => set('description', e.currentTarget.value),}); }
};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const SwarmTask__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type SwarmTask__SvelteComponent_ = ReturnType<typeof SwarmTask__SvelteComponent_>;
/*Ωignore_endΩ*/export default SwarmTask__SvelteComponent_;