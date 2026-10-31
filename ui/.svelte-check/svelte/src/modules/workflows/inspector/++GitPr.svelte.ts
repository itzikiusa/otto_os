///<reference types="svelte" />
;// git_pr: repository / base inherit the reference the implementer or reviewer
// used; open on pass and detect changed repos.

import type { WorkflowNode } from '../../../lib/api/types';
import RepoPicker from './RepoPicker.svelte';
import type { InspectorOptions } from './options.svelte';
import { paramBool, paramStr, type SetParam } from './util';

;type $$ComponentProps =  { node: WorkflowNode; set: SetParam; wsId: string; pickers: InspectorOptions };function $$render() {

  
  
  
  
  
  

  let { node, set, wsId, pickers }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();
;
async () => {

 { svelteHTML.createElement("p", { "class":`insp-note`,});
         { svelteHTML.createElement("strong", {});   } 
            
              
   { svelteHTML.createElement("strong", {});    }       
 }
 { svelteHTML.createElement("label", { "for":`np-repo`,});      }
 { const $$_rekciPopeR0C = __sveltets_2_ensureComponent(RepoPicker); new $$_rekciPopeR0C({ target: __sveltets_2_any(), props: {        "id":`np-repo`,"emptyLabel":`Inherit from the upstream review / working folder`,node,set,wsId,pickers,}});}
 { svelteHTML.createElement("label", { "for":`np-base`,});       }
 { svelteHTML.createElement("input", {            "dir":`auto`,"id":`np-base`,"type":`text`,"placeholder":`inherits (per-repo base), else main`,"value":paramStr(node, 'base'),"oninput":(e) => set('base', e.currentTarget.value),});}
 { svelteHTML.createElement("label", { "class":`np-chk`,});
   { svelteHTML.createElement("input", {       "type":`checkbox`,"checked":paramBool(node, 'open'),"onchange":(e) => set('open', e.currentTarget.checked),});}             
 }
 { svelteHTML.createElement("label", { "class":`np-chk`,});
   { svelteHTML.createElement("input", {       "type":`checkbox`,"checked":paramBool(node, 'detect_changed'),"onchange":(e) => set('detect_changed', e.currentTarget.checked),});}              
 }
};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const GitPr__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type GitPr__SvelteComponent_ = ReturnType<typeof GitPr__SvelteComponent_>;
/*Ωignore_endΩ*/export default GitPr__SvelteComponent_;