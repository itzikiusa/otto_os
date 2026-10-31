///<reference types="svelte" />
;// A successful run's deliverable (`final-output.md`) in a sandboxed,
// token-themed iframe, or its failed read inline with Retry. Shown in the
// inspector's run view on phone/tablet and in the Context sidebar on desktop.

import Icon from '../../lib/components/Icon.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import type { WorkflowRun } from '../../lib/api/types';
import type { RunSync } from './runSync.svelte';

;type $$ComponentProps =  { mon: RunSync; run: WorkflowRun };function $$render() {

  
  
  
  
  
  
  

  let { mon, run }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();
;
async () => {

if(mon.hasFinalOutput(run)){
   { svelteHTML.createElement("details", { "open":true,"class":`final-output`,});
     { svelteHTML.createElement("summary", {});
       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":13,}});}
       { svelteHTML.createElement("span", {});  }
     }
     { svelteHTML.createElement("iframe", {       "class":`final-output-frame`,"title":`Final output`,"sandbox":`allow-same-origin`,"srcdoc":mon.finalOutputSrcdoc,}); }
   }
} else if (mon.finalOutputError?.runId === run.id){
  const runId = run.id;
   { const $$_etatSdaoL0C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL0C({ target: __sveltets_2_any(), props: {        "what":`the final output`,"variant":`compact`,"error":mon.finalOutputError.text,"empty":true,"onretry":() => void mon.loadFinalOutput(runId),}});}
}


};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const RunFinalOutput__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type RunFinalOutput__SvelteComponent_ = ReturnType<typeof RunFinalOutput__SvelteComponent_>;
/*Ωignore_endΩ*/export default RunFinalOutput__SvelteComponent_;