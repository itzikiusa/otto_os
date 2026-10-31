///<reference types="svelte" />
;// The pipeline rail — the fixed stage machine drawn as a strip of steps.
// Three renderings of ONE truth (runStatus.STAGES):
//   • static  (no status): the launcher's "how a run executes" explainer
//   • full    (status, labels): RunDetail header — live, current step pulses
//   • mini    (status, no labels): run-list rows — 8 slim segments

import Icon from '../../lib/components/Icon.svelte';
import { STAGES, stageIndex, isTerminal } from './runStatus';
;

  interface Props {
    /** Current run status; omit for the static explainer rendering. */
    status?: string;
    /** Slim segment bar (list rows). */
    mini?: boolean;
  };function $$render() {

  
  
  
  
  
  
  

  let { status, mini = false }: Props = $props();

  const cur = $derived(status ? stageIndex(status) : -1);
  const failed = $derived(status === 'failed');
  const cancelled = $derived(status === 'cancelled'); // ui-guards: allow — wire status value
  const rejected = $derived(status === 'rejected');
  const done = $derived(status === 'completed');
  const live = $derived(!!status && !isTerminal(status));

  /** Per-step visual state class. */
  function stepState(i: number): string {
    if (!status) return 'idle';
    if (failed || cancelled) return 'off';
    if (done) return 'done';
    if (i < cur) return 'done';
    if (i === cur) return rejected ? 'bad' : live ? 'now' : 'done';
    return 'todo';
  }
;
async () => {

if(mini){
   { svelteHTML.createElement("div", {      "class":`rail mini`,"role":`img`,"aria-label":status ? (cur >= 0 ? `Pipeline stage: ${STAGES[cur].label}` : `Pipeline ${status}`) : 'Pipeline',});failed || cancelled;
        for(let s of __sveltets_2_ensureArray(STAGES)){let i = 1;s.key;
       { svelteHTML.createElement("span", {   "class":`seg ${stepState(i)}`,"title":s.label,}); }
    }
   }
}else{
   { svelteHTML.createElement("ol", {  "class":`rail full`,});failed || cancelled;
        for(let s of __sveltets_2_ensureArray(STAGES)){let i = 1;s.key;
       { svelteHTML.createElement("li", {    "class":`step ${stepState(i)}`,"title":`${s.label} — ${s.hint}`,});!status && s.key === 'approve';
         { svelteHTML.createElement("span", { "class":`bubble`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":s.icon,"size":12,}});} }
         { svelteHTML.createElement("span", { "class":`lbl`,});s.label; }
        if(i < STAGES.length - 1){ { svelteHTML.createElement("span", {   "class":`arrow`,"aria-hidden":`true`,}); }}
       }
    }
   }
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const RunStageRail__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type RunStageRail__SvelteComponent_ = ReturnType<typeof RunStageRail__SvelteComponent_>;
/*Ωignore_endΩ*/export default RunStageRail__SvelteComponent_;