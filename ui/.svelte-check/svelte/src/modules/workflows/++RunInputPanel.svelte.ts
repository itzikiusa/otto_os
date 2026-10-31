///<reference types="svelte" />
;// The Run… panel (W2): WHERE you provide the run input the trigger emits
// (repo_id, story_id, goals, msg, jira_ticket, …), the run's prompt and the
// review-mode override. Its state is RunInputForm (runInputForm.svelte.ts);
// the page starts the run.

import { announceShown } from '../../lib/announce.svelte';
import Icon from '../../lib/components/Icon.svelte';
import type { RunInputForm } from './runInputForm.svelte';
;

  interface Props {
    form: RunInputForm;
    /** The run POST is in flight. */
    running: boolean;
    onrun: () => void;
  };function $$render() {

  
  
  
  
  
  
  

  let { form, running, onrun }: Props = $props();
;
async () => {

 { svelteHTML.createElement("div", { "class":`run-input`,});
   { svelteHTML.createElement("div", { "class":`ri-head`,});
     { svelteHTML.createElement("strong", {});  }
     { svelteHTML.createElement("span", { "class":`ri-hint`,});               }
   }
   { svelteHTML.createElement("textarea", {            "dir":`auto`,"aria-label":`Run prompt`,"class":`ri-prompt`,"rows":3,"bind:value":form.prompt,"placeholder":`What you want done — instructions for the agents.`,});/*Ωignore_startΩ*/() => form.prompt = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
   { svelteHTML.createElement("div", { "class":`ri-head`,});
     { svelteHTML.createElement("strong", {});  }
     { svelteHTML.createElement("span", { "class":`ri-hint`,});                         }
     { svelteHTML.createElement("button", {     "class":`btn small ghost`,"onclick":() => form.suggest(),"title":`Insert the keys this workflow needs, as placeholders to replace`,});  }
   }
   { svelteHTML.createElement("textarea", {                  "dir":`ltr`,"class":`ri-text mono`,"rows":8,"bind:value":form.text,"spellcheck":`false`,"aria-label":`Run input (JSON)`,"aria-invalid":form.error ? 'true' : undefined,"aria-describedby":form.error ? 'wf-run-input-err' : undefined,"placeholder":form.placeholder,});/*Ωignore_startΩ*/() => form.text = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
  if(form.error){ {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('p'),({ text: form.error, assertive: true })));{ svelteHTML.createElement("p", __sveltets_2_union($$action_0), {     "class":`ri-err`,"id":`wf-run-input-err`,});form.error; }}}
   { svelteHTML.createElement("div", { "class":`ri-head`,});
     { svelteHTML.createElement("strong", {});  }
     { svelteHTML.createElement("span", { "class":`ri-hint`,});                 }
   }
   { svelteHTML.createElement("select", {         "class":`ri-select`,...__sveltets_2_empty({"data-testid":`run-review-mode`}),"bind:value":form.reviewMode,"title":`Overrides the execution mode of every review step in this run, including steps that set their own.`,});/*Ωignore_startΩ*/() => form.reviewMode = __sveltets_2_any(null);/*Ωignore_endΩ*/
     { svelteHTML.createElement("option", {"value":"",});  }
     { svelteHTML.createElement("option", { "value":`fan_out`,});  }
     { svelteHTML.createElement("option", { "value":`orchestrator`,});  }
   }
  if(form.destinations.length){
     { svelteHTML.createElement("p", {   "class":`ri-dest`,...__sveltets_2_empty({"data-testid":`run-result-destination`}),});
       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`send`,"size":12,}});}      form.destinations.join(' and ');
     }
  }
   { svelteHTML.createElement("div", { "class":`ri-actions`,});
     { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => (form.open = false),});  }
     { svelteHTML.createElement("button", {     "class":`btn primary small`,"disabled":running,"onclick":onrun,});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`play`,"size":12,}});} running ? 'Starting…' : 'Run';
     }
   }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const RunInputPanel__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type RunInputPanel__SvelteComponent_ = ReturnType<typeof RunInputPanel__SvelteComponent_>;
/*Ωignore_endΩ*/export default RunInputPanel__SvelteComponent_;