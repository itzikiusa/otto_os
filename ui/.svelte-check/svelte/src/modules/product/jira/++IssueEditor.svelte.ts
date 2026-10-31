///<reference types="svelte" />
;// Inline editor for one editable Jira field, dispatched by its editmeta
// `schema_type`. The draft, the pickers' data and the save live on the
// shared `JiraIssueEditor` (issueEditorState.svelte.ts), so the host only
// decides WHERE a field's editor shows.

import Skeleton from '../../../lib/components/Skeleton.svelte';
import type { EditableField } from '../types';
import type { JiraIssueEditor } from './issueEditorState.svelte';
import { focusFirstField } from '../../../lib/inlineEdit';
import { focusOnMount } from '../../../lib/focusOnMount';

;type $$ComponentProps =  { editor: JiraIssueEditor; ef: EditableField; issueKey: string };function $$render() {

  
  
  
  
  
  
  
  
  

  let { editor, ef, issueKey }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();

  const isSelect = $derived(
    ['option', 'priority', 'version', 'component'].includes(ef.schema_type) && ef.allowed_values.length > 0,
  );
;
async () => {

 {const $$action_0 = __sveltets_2_ensureAction(focusFirstField(svelteHTML.mapElementTag('div')));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {  "class":`field-editor`,});
  if(ef.schema_type === 'number'){
     { svelteHTML.createElement("input", {          "aria-label":ef.name,"class":`field-input`,"type":`number`,"step":`any`,"bind:value":editor.fieldDraft,});/*Ωignore_startΩ*/() => editor.fieldDraft = __sveltets_2_any(null);/*Ωignore_endΩ*/}
  } else if (ef.schema_type === 'date'){
     { svelteHTML.createElement("input", {        "aria-label":ef.name,"class":`field-input`,"type":`date`,"bind:value":editor.fieldDraft,});/*Ωignore_startΩ*/() => editor.fieldDraft = __sveltets_2_any(null);/*Ωignore_endΩ*/}
  } else if (ef.schema_type === 'datetime'){
     { svelteHTML.createElement("input", {        "aria-label":ef.name,"class":`field-input`,"type":`datetime-local`,"bind:value":editor.fieldDraft,});/*Ωignore_startΩ*/() => editor.fieldDraft = __sveltets_2_any(null);/*Ωignore_endΩ*/}
  } else if (ef.schema_type === 'user'){
    if(editor.assignablesLoading && editor.assignables.length === 0){
       { const $$_notelekS1C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS1C({ target: __sveltets_2_any(), props: {      "rows":1,"height":27,"label":`people`,}});}
    }else{
      
       {const $$action_0 = __sveltets_2_ensureAction(focusOnMount(svelteHTML.mapElementTag('select')));{ svelteHTML.createElement("select", __sveltets_2_union($$action_0), {      "aria-label":ef.name,"class":`field-input`,"bind:value":editor.fieldDraft,});/*Ωignore_startΩ*/() => editor.fieldDraft = __sveltets_2_any(null);/*Ωignore_endΩ*/
         { svelteHTML.createElement("option", {"value":"",});  }
           for(let u of __sveltets_2_ensureArray(editor.assignables)){u.account_id;
           { svelteHTML.createElement("option", { "value":u.account_id,});u.display_name; }
        }
       }}
    }
  } else if (isSelect){
     { svelteHTML.createElement("select", {     "aria-label":ef.name,"class":`field-input`,"bind:value":editor.fieldDraft,});/*Ωignore_startΩ*/() => editor.fieldDraft = __sveltets_2_any(null);/*Ωignore_endΩ*/
      if(!ef.required){
         { svelteHTML.createElement("option", {"value":"",});   }
      }
         for(let opt of __sveltets_2_ensureArray(ef.allowed_values)){opt.id;
         { svelteHTML.createElement("option", { "value":opt.id,});opt.label; }
      }
     }
  } else if (ef.schema_type === 'array' && ef.items !== 'string' && ef.allowed_values.length > 0){
     { svelteHTML.createElement("div", {     "class":`field-multiselect`,"role":`group`,"aria-label":ef.name,});
         for(let opt of __sveltets_2_ensureArray(ef.allowed_values)){opt.id;
         { svelteHTML.createElement("label", { "class":`field-check`,});
           { svelteHTML.createElement("input", {       "type":`checkbox`,"checked":Array.isArray(editor.fieldDraft) && (editor.fieldDraft as string[]).includes(opt.id),"onchange":() => editor.toggleArrayOption(opt.id),});}
          opt.label;
         }
      }
     }
  } else if (ef.schema_type === 'array'){
    
     { svelteHTML.createElement("input", {            "dir":`auto`,"aria-label":ef.name,"class":`field-input`,"type":`text`,"placeholder":`e.g. backend, payments`,"bind:value":editor.fieldDraft,});/*Ωignore_startΩ*/() => editor.fieldDraft = __sveltets_2_any(null);/*Ωignore_endΩ*/}
  }else{
    
     { svelteHTML.createElement("input", {          "dir":`auto`,"aria-label":ef.name,"class":`field-input`,"type":`text`,"bind:value":editor.fieldDraft,});/*Ωignore_startΩ*/() => editor.fieldDraft = __sveltets_2_any(null);/*Ωignore_endΩ*/}
    if(ef.schema_type !== 'string'){
       { svelteHTML.createElement("span", { "class":`field-raw-note`,}); ef.schema_type;  }
    }
  }
   { svelteHTML.createElement("div", { "class":`field-editor-actions`,});
     { svelteHTML.createElement("button", {       "class":`btn small primary`,"onclick":() => editor.saveField(ef),"disabled":editor.fieldSaving,"title":`Writes to the live Jira issue — everyone with access sees it`,});
      editor.fieldSaving ? 'Saving…' : `Save to ${issueKey || 'Jira'}`;
     }
     { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":() => editor.cancelEdit(),"disabled":editor.fieldSaving,});  }
   }
 }}


};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const IssueEditor__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type IssueEditor__SvelteComponent_ = ReturnType<typeof IssueEditor__SvelteComponent_>;
/*Ωignore_endΩ*/export default IssueEditor__SvelteComponent_;