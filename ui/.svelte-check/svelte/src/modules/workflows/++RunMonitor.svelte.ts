///<reference types="svelte" />
;// The inspector's run view (architecture-1-08): the run bar (live status,
// Stop, maximize), the run-level error, the step timeline, the step list,
// and on phone/tablet the final output and the context files (on desktop
// those live in RunContextSidebar). Live state comes from the page's
// RunSync (runSync.svelte.ts).

import { announceShown } from '../../lib/announce.svelte';
import Icon from '../../lib/components/Icon.svelte';
import StatusBadge from '../../lib/components/StatusBadge.svelte';
import { runStatus } from '../../lib/status';
import { viewport } from '../../lib/stores/viewport.svelte';
import FileTree from '../panels/FileTree.svelte';
import RunFinalOutput from './RunFinalOutput.svelte';
import RunSteps from './RunSteps.svelte';
import { fmtStepMs } from './runProgress';
import { contextFs, isLive, type RunSync } from './runSync.svelte';
;

  interface Props {
    mon: RunSync;
    nodeName: (id: string) => string;
    /** The step selected in the timeline / on the canvas. */
    selectedId: string | null;
    /** "Zoom" the run detail to ~85vh (R6). */
    runDetailMax: boolean;
    onstop: () => void;
    onOpenSession: (sid: string) => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { mon, nodeName, selectedId = $bindable(), runDetailMax = $bindable(), onstop, onOpenSession }: Props = $props()/*Ωignore_startΩ*/;selectedId;runDetailMax;/*Ωignore_endΩ*/;

  const run = $derived(mon.run);
  // Whether the run is still active (so Stop makes sense even when this
  // editor didn't start it — e.g. opened from the Running list).
  const runActive = $derived(!!run && isLive(run.status));
  const fmtMs = fmtStepMs;
  function runStatusLabel(status: string): string {
    return runStatus(status).label;
  }
  /** Normalised run/step key for the status dot (`succeeded`, `failed`, …). */
  function dotKey(status: string): string {
    return runStatus(status).key;
  }
;
async () => {

if(run){
  
   { svelteHTML.createElement("div", { "class":`insp-bar`,});
     { svelteHTML.createElement("span", { "class":`tl-label`,}); { const $$_egdaBsutatS2C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS2C({ target: __sveltets_2_any(), props: {    "status":runStatus(run.status),"testid":`run-status`,}});} }
     { svelteHTML.createElement("span", { "class":`grow`,}); }
    if(runActive){
       { svelteHTML.createElement("button", {         "class":`btn small danger`,...__sveltets_2_empty({"data-testid":`run-cancel`}),"onclick":onstop,"title":`Stop this run (finishes the current step, then halts)`,});
         { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`stop`,"size":12,}});}  
       }
    }
     { svelteHTML.createElement("button", {             "class":`icon-btn`,...__sveltets_2_empty({"data-testid":`run-detail-max`}),"onclick":() => (runDetailMax = !runDetailMax),"aria-pressed":runDetailMax,"aria-label":`Maximize run detail`,"title":runDetailMax ? 'Restore run-detail height' : 'Maximize run detail',});
       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":runDetailMax ? 'minimize' : 'maximize',"size":13,}});}
     }
   }
  
  if(run.status === 'error' && run.error){
     {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('p'),({ text: `Run failed: ${run.error}`, assertive: true })));{ svelteHTML.createElement("p", __sveltets_2_union($$action_0), {     "class":`run-error`,...__sveltets_2_empty({"data-testid":`run-error`}),}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});} run.error; }}
  }
   { svelteHTML.createElement("div", { "class":`timeline`,});
       for(let ns of __sveltets_2_ensureArray(run.nodes)){ns.node_id;
       { svelteHTML.createElement("button", {        "class":`tl-step`,...__sveltets_2_empty({"data-status":ns.status}),"onclick":() => (selectedId = ns.node_id),});selectedId === ns.node_id;
         { svelteHTML.createElement("span", {     "class":`dot ${dotKey(ns.status)}`,"role":`img`,"aria-label":runStatusLabel(ns.status),}); }
         { svelteHTML.createElement("span", { "class":`tl-name`,});nodeName(ns.node_id); }
        if(ns.duration_ms != null){ { svelteHTML.createElement("span", { "class":`tl-ms`,});fmtMs(ns.duration_ms); }}
       }
    }
   }
   { svelteHTML.createElement("div", { "class":`run-detail`,}); { const $$_spetSnuR1C = __sveltets_2_ensureComponent(RunSteps); new $$_spetSnuR1C({ target: __sveltets_2_any(), props: {         run,"nodeName":(id) => nodeName(id),"onOpenSession":onOpenSession,"onRunUpdated":mon.applyRunSnapshot,"onRefresh":() => void mon.refetchRun(run.id),}});} }
  if(run.context_dir && !viewport.isDesktop){
    
     { const $$_tuptuOlaniFnuR0C = __sveltets_2_ensureComponent(RunFinalOutput); new $$_tuptuOlaniFnuR0C({ target: __sveltets_2_any(), props: {  mon,run,}});}
    
     { svelteHTML.createElement("details", { "class":`ctx-files`,});
       { svelteHTML.createElement("summary", {});
         { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`folder`,"size":13,}});}
         { svelteHTML.createElement("span", {});  }
         { svelteHTML.createElement("code", { "class":`dim`,});run.context_dir; }
       }
       { svelteHTML.createElement("div", { "class":`ctx-files-tree`,}); { const $$_eerTeliF2C = __sveltets_2_ensureComponent(FileTree); new $$_eerTeliF2C({ target: __sveltets_2_any(), props: {   "root":run.context_dir,...contextFs(run.id, run.context_dir),}});} }
     }
  }
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings('selectedId', 'runDetailMax'), slots: {}, events: {} }}
const RunMonitor__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type RunMonitor__SvelteComponent_ = ReturnType<typeof RunMonitor__SvelteComponent_>;
/*Ωignore_endΩ*/export default RunMonitor__SvelteComponent_;