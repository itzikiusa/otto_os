///<reference types="svelte" />
;// The selected node's per-kind param form (split out of WorkflowsPage.svelte).
// Each kind exposes only its meaningful params; unrecognised kinds fall
// through to a raw JSON editor. Every edit goes through the page's `set`
// (the selected node's `onParam`), which marks the canvas dirty.

import type { WorkflowNode } from '../../../lib/api/types';
import AgentPrompt from './AgentPrompt.svelte';
import BrokerPeek from './BrokerPeek.svelte';
import Canvas from './Canvas.svelte';
import DbQuery from './DbQuery.svelte';
import GitPr from './GitPr.svelte';
import HttpRequest from './HttpRequest.svelte';
import Loop from './Loop.svelte';
import ManualTrigger from './ManualTrigger.svelte';
import PrepareContext from './PrepareContext.svelte';
import ProductStep from './ProductStep.svelte';
import ProviderModel from './ProviderModel.svelte';
import RawParams from './RawParams.svelte';
import ReviewRun from './ReviewRun.svelte';
import SelfImprove from './SelfImprove.svelte';
import SwarmTask from './SwarmTask.svelte';
import Utility from './Utility.svelte';
import type { InspectorOptions } from './options.svelte';
import LazyMount from '../../../lib/components/LazyMount.svelte';
import { lazyComponent } from '../../../lib/lazy-component.svelte';
import { KINDS_WITH_OWN_PROVIDER, type SetParam } from './util';
;

  interface Props {
    node: WorkflowNode;
    set: SetParam;
    /** Replace the node's params wholesale (the raw-JSON fallback). */
    setParams: (params: unknown) => void;
    /** The workflow's workspace — what the pickers list. */
    wsId: string;
    pickers: InspectorOptions;
    workflowName: string | undefined;
    /** Open the page's big JSON editor for `field`. */
    onzoom: (field: string, label: string) => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  // Only a product_publish node needs the Confluence space/page pickers: keep
  // them (and their issues API) off the Workflows page's first load.
  const ProductPublish = lazyComponent(() => import('./ProductPublish.svelte'));
  

  let { node, set, setParams, wsId, pickers, workflowName, onzoom }: Props = $props();
  const UTILITY_KINDS = ['delay', 'transform', 'game_engine', 'channel_notify', 'budget_gate', 'human_approval', 'condition'];
  const PRODUCT_STEP_KINDS = ['product_analyze', 'product_rewrite', 'product_plan'];
;
async () => {


 { svelteHTML.createElement("div", { "class":`np-form`,});
  if(node.kind === 'manual_trigger'){
     { const $$_reggirTlaunaM1C = __sveltets_2_ensureComponent(ManualTrigger); new $$_reggirTlaunaM1C({ target: __sveltets_2_any(), props: {     node,set,wsId,pickers,workflowName,}});}
  } else if (node.kind === 'agent_prompt'){
     { const $$_tpmorPtnegA1C = __sveltets_2_ensureComponent(AgentPrompt); new $$_tpmorPtnegA1C({ target: __sveltets_2_any(), props: {  node,set,}});}
  } else if (node.kind === 'prepare_context'){
     { const $$_txetnoCeraperP1C = __sveltets_2_ensureComponent(PrepareContext); new $$_txetnoCeraperP1C({ target: __sveltets_2_any(), props: {   node,set,pickers,}});}
  } else if (node.kind === 'http_request' || node.kind === 'api_run'){
     { const $$_tseuqeRpttH1C = __sveltets_2_ensureComponent(HttpRequest); new $$_tseuqeRpttH1C({ target: __sveltets_2_any(), props: {   node,set,onzoom,}});}
  } else if (UTILITY_KINDS.includes(node.kind)){
     { const $$_ytilitU1C = __sveltets_2_ensureComponent(Utility); new $$_ytilitU1C({ target: __sveltets_2_any(), props: {   node,set,onzoom,}});}
  } else if (node.kind === 'db_query'){
     { const $$_yreuQbD1C = __sveltets_2_ensureComponent(DbQuery); new $$_yreuQbD1C({ target: __sveltets_2_any(), props: {    node,set,wsId,pickers,}});}
  } else if (node.kind === 'broker_peek'){
     { const $$_keePrekorB1C = __sveltets_2_ensureComponent(BrokerPeek); new $$_keePrekorB1C({ target: __sveltets_2_any(), props: {    node,set,wsId,pickers,}});}
  } else if (node.kind === 'swarm_task'){
     { const $$_ksaTmrawS1C = __sveltets_2_ensureComponent(SwarmTask); new $$_ksaTmrawS1C({ target: __sveltets_2_any(), props: {    node,set,wsId,pickers,}});}
  } else if (node.kind === 'loop'){
     { const $$_pooL1C = __sveltets_2_ensureComponent(Loop); new $$_pooL1C({ target: __sveltets_2_any(), props: {   node,set,onzoom,}});}
  } else if (node.kind === 'review_run'){
     { const $$_nuRweiveR1C = __sveltets_2_ensureComponent(ReviewRun); new $$_nuRweiveR1C({ target: __sveltets_2_any(), props: {    node,set,wsId,pickers,}});}
  } else if (PRODUCT_STEP_KINDS.includes(node.kind)){
     { const $$_petStcudorP1C = __sveltets_2_ensureComponent(ProductStep); new $$_petStcudorP1C({ target: __sveltets_2_any(), props: {    node,set,wsId,pickers,}});}
  } else if (node.kind === 'product_publish'){
     { const $$_tnuoMyzaL1C = __sveltets_2_ensureComponent(LazyMount); new $$_tnuoMyzaL1C({ target: __sveltets_2_any(), props: {        "lazy":ProductPublish,"what":`the publish settings`,"variant":`compact`,"props":{ node, set, wsId, pickers },}});}
  } else if (node.kind === 'canvas'){
     { const $$_savnaC1C = __sveltets_2_ensureComponent(Canvas); new $$_savnaC1C({ target: __sveltets_2_any(), props: {  node,set,}});}
  } else if (node.kind === 'git_pr'){
     { const $$_rPtiG1C = __sveltets_2_ensureComponent(GitPr); new $$_rPtiG1C({ target: __sveltets_2_any(), props: {    node,set,wsId,pickers,}});}
  } else if (node.kind === 'self_improve'){
     { const $$_evorpmIfleS1C = __sveltets_2_ensureComponent(SelfImprove); new $$_evorpmIfleS1C({ target: __sveltets_2_any(), props: {  node,set,}});}
  } else if (node.kind !== 'log' && node.kind !== 'verifier'){
     { const $$_smaraPwaR1C = __sveltets_2_ensureComponent(RawParams); new $$_smaraPwaR1C({ target: __sveltets_2_any(), props: {  node,setParams,}});}
  }

  
  if(!KINDS_WITH_OWN_PROVIDER.includes(node.kind) && node.kind !== 'manual_trigger'){
     { const $$_ledoMredivorP1C = __sveltets_2_ensureComponent(ProviderModel); new $$_ledoMredivorP1C({ target: __sveltets_2_any(), props: {  node,set,}});}
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const NodeParams__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type NodeParams__SvelteComponent_ = ReturnType<typeof NodeParams__SvelteComponent_>;
/*Ωignore_endΩ*/export default NodeParams__SvelteComponent_;