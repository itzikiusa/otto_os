///<reference types="svelte" />
;// product_publish: a dry-run preview (story, RFC or Jira story, destination),
// or — live — the publish of exactly what the approved preview showed.

import type { WorkflowNode } from '../../../lib/api/types';
import IdPicker from './IdPicker.svelte';
import ConfluenceSpaceSelect from '../../../lib/components/ConfluenceSpaceSelect.svelte';
import ConfluencePagePicker from '../../../lib/components/ConfluencePagePicker.svelte';
import type { InspectorOptions } from './options.svelte';
import { jiraAccountOptions, paramBool, paramStr, storyOptions, type SetParam } from './util';

;type $$ComponentProps =  { node: WorkflowNode; set: SetParam; wsId: string; pickers: InspectorOptions };function $$render() {

  
  
  
  
  
  
  
  

  let { node, set, wsId, pickers }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();

  const accountId = $derived(paramStr(node, 'account_id'));
  /** A parent page belongs to its space: a space change clears it. */
  function setSpace(key: string): void {
    if (key === paramStr(node, 'space_key')) return;
    set('space_key', key);
    set('parent_id', '');
    set('parent_title', '');
  }
;
async () => {

 { svelteHTML.createElement("label", { "class":`np-chk`,});
   { svelteHTML.createElement("input", {       "type":`checkbox`,"checked":paramBool(node, 'dry_run', true),"onchange":(e) => set('dry_run', e.currentTarget.checked),});}    
 }
if(paramBool(node, 'dry_run', true)){
 { svelteHTML.createElement("label", { "for":`np-story`,});  }
 { const $$_rekciPdI0C = __sveltets_2_ensureComponent(IdPicker); new $$_rekciPdI0C({ target: __sveltets_2_any(), props: {                    "id":`np-story`,"value":paramStr(node, 'story_id'),"cache":pickers.stories,"key":wsId,"toOptions":storyOptions,"noun":`story`,"placeholder":`product story id`,"what":`product stories`,"emptyHint":`No product stories in this workspace yet — import one on the Product page.`,"onchange":(v) => set('story_id', v),}});}
 { svelteHTML.createElement("label", { "for":`np-pubkind`,});  }
  { svelteHTML.createElement("select", {      "id":`np-pubkind`,"value":paramStr(node, 'kind') || 'rfc',"onchange":(e) => set('kind', e.currentTarget.value),});
   { svelteHTML.createElement("option", { "value":`rfc`,});  }
   { svelteHTML.createElement("option", { "value":`jira`,});  }
 }

   { svelteHTML.createElement("label", { "for":`np-account`,});  }
   { const $$_rekciPdI0C = __sveltets_2_ensureComponent(IdPicker); new $$_rekciPdI0C({ target: __sveltets_2_any(), props: {                      "id":`np-account`,"value":paramStr(node, 'account_id'),"cache":pickers.accounts,"key":`me`,"toOptions":jiraAccountOptions,"noun":`Jira account`,"emptyLabel":`Choose a Jira / Confluence account…`,"placeholder":`Jira/Confluence account id`,"what":`Jira accounts`,"emptyHint":`No Jira / Confluence account yet — connect one in Settings → Integrations.`,"onchange":(v) => set('account_id', v),}});}
  if((paramStr(node, 'kind') || 'rfc') === 'jira'){
     { svelteHTML.createElement("label", { "for":`np-project`,});  }
     { svelteHTML.createElement("input", {            "dir":`auto`,"id":`np-project`,"type":`text`,"placeholder":`e.g. PROJ`,"value":paramStr(node, 'project_key'),"oninput":(e) => set('project_key', e.currentTarget.value),});}
     { svelteHTML.createElement("label", { "for":`np-issuetype`,});  }
     { svelteHTML.createElement("input", {            "dir":`auto`,"id":`np-issuetype`,"type":`text`,"placeholder":`Story`,"value":paramStr(node, 'issue_type'),"oninput":(e) => set('issue_type', e.currentTarget.value),});}
  }else{
    if(accountId){
      
       { svelteHTML.createElement("label", { "for":`np-space`,});  }
       { const $$_tceleSecapSecneulfnoC0C = __sveltets_2_ensureComponent(ConfluenceSpaceSelect); new $$_tceleSecapSecneulfnoC0C({ target: __sveltets_2_any(), props: {       "id":`np-space`,accountId,"value":paramStr(node, 'space_key'),"onchange":setSpace,}});}
       { const $$_rekciPegaPecneulfnoC0C = __sveltets_2_ensureComponent(ConfluencePagePicker); new $$_rekciPegaPecneulfnoC0C({ target: __sveltets_2_any(), props: {              "id":`np-parent`,"label":`Parent page`,accountId,"spaceKey":paramStr(node, 'space_key'),"value":paramStr(node, 'parent_id') || null,"title":paramStr(node, 'parent_title') || null,"onchange":(id, title) => { set('parent_id', id ?? ''); set('parent_title', title ?? ''); },}});}
    }else{
       { svelteHTML.createElement("label", { "for":`np-space`,});  }
       { svelteHTML.createElement("input", {            "dir":`auto`,"id":`np-space`,"type":`text`,"placeholder":`Choose an account first, or type a space key`,"value":paramStr(node, 'space_key'),"oninput":(e) => set('space_key', e.currentTarget.value),});}
       { svelteHTML.createElement("label", { "for":`np-parent`,});    }
       { svelteHTML.createElement("input", {            "dir":`auto`,"id":`np-parent`,"type":`text`,"placeholder":`parent page id`,"value":paramStr(node, 'parent_id'),"oninput":(e) => { set('parent_id', e.currentTarget.value); set('parent_title', ''); },});}
    }
     { svelteHTML.createElement("label", { "for":`np-pubtitle`,});  }
     { svelteHTML.createElement("input", {            "dir":`auto`,"id":`np-pubtitle`,"type":`text`,"placeholder":`page title`,"value":paramStr(node, 'title'),"oninput":(e) => set('title', e.currentTarget.value),});}
  }
}else{
   { svelteHTML.createElement("p", { "class":`insp-note`,});                                     }
}
};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ProductPublish__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ProductPublish__SvelteComponent_ = ReturnType<typeof ProductPublish__SvelteComponent_>;
/*Ωignore_endΩ*/export default ProductPublish__SvelteComponent_;