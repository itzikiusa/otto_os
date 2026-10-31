///<reference types="svelte" />
;// product_analyze / product_rewrite / product_plan: the story, an extra
// instruction and the agent; rewrite and plan can persist a product version.

import type { WorkflowNode } from '../../../lib/api/types';
import IdPicker from './IdPicker.svelte';
import ProviderModel from './ProviderModel.svelte';
import type { InspectorOptions } from './options.svelte';
import { paramBool, paramStr, storyOptions, type SetParam } from './util';

;type $$ComponentProps =  { node: WorkflowNode; set: SetParam; wsId: string; pickers: InspectorOptions };function $$render() {

  
  
  
  
  
  
  

  let { node, set, wsId, pickers }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();
;
async () => {

 { svelteHTML.createElement("label", { "for":`np-story`,});  }
 { const $$_rekciPdI0C = __sveltets_2_ensureComponent(IdPicker); new $$_rekciPdI0C({ target: __sveltets_2_any(), props: {                    "id":`np-story`,"value":paramStr(node, 'story_id'),"cache":pickers.stories,"key":wsId,"toOptions":storyOptions,"noun":`story`,"placeholder":`product story id`,"what":`product stories`,"emptyHint":`No product stories in this workspace yet — import one on the Product page.`,"onchange":(v) => set('story_id', v),}});}
 { svelteHTML.createElement("label", { "for":`np-instruction`,});   }
 { svelteHTML.createElement("input", {            "dir":`auto`,"id":`np-instruction`,"type":`text`,"placeholder":`Focus on…`,"value":paramStr(node, 'instruction'),"oninput":(e) => set('instruction', e.currentTarget.value),});}
 { const $$_ledoMredivorP0C = __sveltets_2_ensureComponent(ProviderModel); new $$_ledoMredivorP0C({ target: __sveltets_2_any(), props: {  node,set,}});}
if(node.kind !== 'product_analyze'){
   { svelteHTML.createElement("label", { "class":`np-chk`,});
     { svelteHTML.createElement("input", {       "type":`checkbox`,"checked":paramBool(node, 'persist'),"onchange":(e) => set('persist', e.currentTarget.checked),});}     
   }
}
};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ProductStep__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ProductStep__SvelteComponent_ = ReturnType<typeof ProductStep__SvelteComponent_>;
/*Ωignore_endΩ*/export default ProductStep__SvelteComponent_;