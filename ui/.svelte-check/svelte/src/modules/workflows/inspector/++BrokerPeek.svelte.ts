///<reference types="svelte" />
;// broker_peek: pick a cluster, then one of its topics (listed once the
// cluster is a known one — never per keystroke of a typed id).

import type { WorkflowNode } from '../../../lib/api/types';
import IdPicker from './IdPicker.svelte';
import { isUlid, type InspectorOptions } from './options.svelte';
import { clusterOptions, paramNum, paramStr, topicOptions, type SetParam } from './util';

;type $$ComponentProps =  { node: WorkflowNode; set: SetParam; wsId: string; pickers: InspectorOptions };function $$render() {

  
  
  
  
  
  

  let { node, set, wsId, pickers }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();
  const cid = $derived(paramStr(node, 'cluster_id'));
  // Only a COMPLETE id: one the list knows, or (free text) a full ULID.
  const clusterKnown = $derived.by(() => {
    const rows = pickers.brokerClusters.rows(wsId);
    return rows ? rows.some((c) => c.id === cid) : isUlid(cid);
  });
;
async () => {

 { svelteHTML.createElement("label", { "for":`np-cid`,});  }
 { const $$_rekciPdI0C = __sveltets_2_ensureComponent(IdPicker); new $$_rekciPdI0C({ target: __sveltets_2_any(), props: {                      "id":`np-cid`,"dir":`ltr`,"value":cid,"cache":pickers.brokerClusters,"key":wsId,"toOptions":clusterOptions,"noun":`cluster`,"placeholder":`Broker cluster id`,"what":`broker clusters`,"emptyHint":`No broker clusters in this workspace yet — add one in Message Brokers.`,"onchange":(v) => set('cluster_id', v),}});}
 { svelteHTML.createElement("label", { "for":`np-topic`,});  }
if(cid && clusterKnown){
   { const $$_rekciPdI0C = __sveltets_2_ensureComponent(IdPicker); new $$_rekciPdI0C({ target: __sveltets_2_any(), props: {                      "id":`np-topic`,"dir":`ltr`,"value":paramStr(node, 'topic'),"cache":pickers.brokerTopics,"key":cid,"toOptions":topicOptions,"noun":`topic`,"placeholder":`my-topic`,"what":`topics`,"emptyHint":`This cluster has no topics yet.`,"onchange":(v) => set('topic', v),}});}
}else{
   { svelteHTML.createElement("input", {            "dir":`ltr`,"id":`np-topic`,"type":`text`,"placeholder":cid ? 'my-topic' : 'Choose a cluster first',"value":paramStr(node, 'topic'),"oninput":(e) => set('topic', e.currentTarget.value),});}
}
 { svelteHTML.createElement("label", { "for":`np-limit`,});   }
 { svelteHTML.createElement("input", {             "id":`np-limit`,"type":`number`,"min":`1`,"max":`50`,"value":paramNum(node, 'limit', 20),"oninput":(e) => set('limit', Number(e.currentTarget.value)),});}
};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const BrokerPeek__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type BrokerPeek__SvelteComponent_ = ReturnType<typeof BrokerPeek__SvelteComponent_>;
/*Ωignore_endΩ*/export default BrokerPeek__SvelteComponent_;