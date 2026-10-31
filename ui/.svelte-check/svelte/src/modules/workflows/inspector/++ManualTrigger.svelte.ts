///<reference types="svelte" />
;// manual_trigger: the run input the workflow starts with, plus the Slack
// message that triggers THIS workflow by name.

import type { WorkflowNode } from '../../../lib/api/types';
import PathField from '../../../lib/components/PathField.svelte';
import { copyTextOrThrow } from '../../../lib/clipboard';
import { toasts } from '../../../lib/toast.svelte';
import IdPicker from './IdPicker.svelte';
import RepoPicker from './RepoPicker.svelte';
import type { InspectorOptions } from './options.svelte';
import { linesValue, paramLines, paramStr, storyOptions, type SetParam } from './util';

;type $$ComponentProps =  {
    node: WorkflowNode; set: SetParam; wsId: string; pickers: InspectorOptions; workflowName: string | undefined;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  

  let { node, set, wsId, pickers, workflowName }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();

  // Copy-paste Slack message that triggers THIS workflow by name.
  let mtCopied = $state(false);
  const mtSlackSnippet = $derived(
    `@otto\n` +
      `Action: Workflow\n` +
      `Name: ${workflowName ?? '<workflow name>'}\n` +
      `Msg: what you want done — instructions for the agents\n` +
      `Jira ticket: PROJ-1234\n` +
      `Working Directory: ~/path/to/repo\n` +
      `Relevant Info: ~/path/a, ~/path/b\n` +
      `Goals:\n  - 100% test coverage (services)\n  - under 2 minutes runtime`,
  );
  async function copyMtSlack(): Promise<void> {
    try {
      await copyTextOrThrow(mtSlackSnippet);
      mtCopied = true;
      setTimeout(() => (mtCopied = false), 1500);
    } catch {
      toasts.error('Couldn’t copy to the clipboard', 'Select the text and copy it manually.');
    }
  }
;
async () => {

 { svelteHTML.createElement("p", { "class":`insp-note`,});
       { svelteHTML.createElement("strong", {});  }       
       { svelteHTML.createElement("strong", {});  }      
   { svelteHTML.createElement("strong", {});  }     
 }
 { svelteHTML.createElement("label", { "for":`mt-msg`,});   }
  { svelteHTML.createElement("textarea", {             "dir":`auto`,"id":`mt-msg`,"class":`np-prompt`,"rows":6,"placeholder":`What you want done — instructions for the agents`,"value":paramStr(node, 'msg'),"oninput":(e) => set('msg', e.currentTarget.value),}); }
 { svelteHTML.createElement("label", { "for":`mt-wd`,});     }
 { const $$_dleiFhtaP0C = __sveltets_2_ensureComponent(PathField); new $$_dleiFhtaP0C({ target: __sveltets_2_any(), props: {   children:() => { return __sveltets_2_any(0); },"value":paramStr(node, 'working_directory'),"onpick":(path) => set('working_directory', path),}}); { svelteHTML.createElement("input", {            "dir":`ltr`,"id":`mt-wd`,"type":`text`,"placeholder":`~/path/to/repo (default: workspace root)`,"value":paramStr(node, 'working_directory'),"oninput":(e) => set('working_directory', e.currentTarget.value),});} PathField}
 { svelteHTML.createElement("label", { "for":`mt-repo`,});      }
 { const $$_rekciPopeR0C = __sveltets_2_ensureComponent(RepoPicker); new $$_rekciPopeR0C({ target: __sveltets_2_any(), props: {        "id":`mt-repo`,"emptyLabel":`None`,node,set,wsId,pickers,}});}
 { svelteHTML.createElement("label", { "for":`mt-base`,});  }
 { svelteHTML.createElement("input", {            "dir":`auto`,"id":`mt-base`,"type":`text`,"placeholder":`main`,"value":paramStr(node, 'base'),"oninput":(e) => set('base', e.currentTarget.value),});}
 { svelteHTML.createElement("label", { "for":`mt-story`,});     }
 { const $$_rekciPdI0C = __sveltets_2_ensureComponent(IdPicker); new $$_rekciPdI0C({ target: __sveltets_2_any(), props: {                    "id":`mt-story`,"value":paramStr(node, 'story_id'),"cache":pickers.stories,"key":wsId,"toOptions":storyOptions,"noun":`story`,"placeholder":`product story id`,"what":`product stories`,"emptyHint":`No product stories in this workspace yet — import one on the Product page.`,"onchange":(v) => set('story_id', v),}});}
 { svelteHTML.createElement("label", { "for":`mt-jira`,});  }
 { svelteHTML.createElement("input", {            "dir":`auto`,"id":`mt-jira`,"type":`text`,"placeholder":`PROJ-1234`,"value":paramStr(node, 'jira_ticket'),"oninput":(e) => set('jira_ticket', e.currentTarget.value),});}
 { svelteHTML.createElement("label", { "for":`mt-goals`,});    }
  { svelteHTML.createElement("textarea", {             "dir":`auto`,"id":`mt-goals`,"class":`np-prompt`,"rows":6,"placeholder":'100% test coverage (services)\nunder 2 minutes runtime',"value":paramLines(node, 'goals'),"oninput":(e) => set('goals', linesValue(e.currentTarget.value)),}); }
 { svelteHTML.createElement("div", { "class":`insp-slack`,});
   { svelteHTML.createElement("div", { "class":`is-head`,});
     { svelteHTML.createElement("strong", {});    }
     { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":copyMtSlack,});mtCopied ? 'Copied' : 'Copy'; }
   }
   { svelteHTML.createElement("p", { "class":`insp-note`,});              }
   { svelteHTML.createElement("pre", { "class":`is-snip`,});mtSlackSnippet; }
 }
};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ManualTrigger__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ManualTrigger__SvelteComponent_ = ReturnType<typeof ManualTrigger__SvelteComponent_>;
/*Ωignore_endΩ*/export default ManualTrigger__SvelteComponent_;