///<reference types="svelte" />
;// loop: max iterations / until, and a structured sub-step editor — the loop
// body runs these in order each iteration. Agent sub-steps expose Provider +
// Model + Prompt directly (no more raw-JSON editing to set a provider).

import type { WorkflowNode } from '../../../lib/api/types';
import Icon from '../../../lib/components/Icon.svelte';
import { agentProviders, defaultAgentProvider } from '../../../lib/providers';
import JsonLabel from './JsonLabel.svelte';
import {
    LOOP_AGENT_KINDS, LOOP_STEP_KINDS, jsonValue, loopSteps as loopStepsOf, loopStepParam as stepParam, loopStepParamList as stepParamList,
    loopStepSummarizer as stepSummarizer, paramBool, paramJson, paramNum, paramStr, withLoopStepParam, withLoopStepSummarizer,
    type LoopStep, type SetParam,
  } from './util';

;type $$ComponentProps =  { node: WorkflowNode; set: SetParam; onzoom: (field: string, label: string) => void };function $$render() {

  
  
  
  
  
  
  
  

  let { node, set, onzoom }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();
  function loopSteps(): LoopStep[] {
    return loopStepsOf(node);
  }
  function setLoopSteps(steps: LoopStep[]): void {
    set('steps', steps.length ? steps : undefined);
  }
  function addLoopStep(): void {
    setLoopSteps([...loopSteps(), { kind: 'agent_prompt', name: '', params: {} }]);
  }
  function removeLoopStep(i: number): void {
    setLoopSteps(loopSteps().filter((_, idx) => idx !== i));
  }
  function updateLoopStep(i: number, patch: Partial<LoopStep>): void {
    setLoopSteps(loopSteps().map((s, idx) => (idx === i ? { ...s, ...patch } : s)));
  }
  const loopStepParam = (i: number, key: string): string => stepParam(loopSteps()[i], key);
  const loopStepParamList = (i: number, key: string): string => stepParamList(loopSteps()[i], key);
  const loopStepSummarizer = (i: number): string => stepSummarizer(loopSteps()[i]);
  function updateLoopStepParam(i: number, key: string, value: unknown): void {
    const step = loopSteps()[i];
    if (step) updateLoopStep(i, { params: withLoopStepParam(step, key, value) });
  }
  function updateLoopStepSummarizer(i: number, provider: string): void {
    const step = loopSteps()[i];
    if (step) updateLoopStep(i, { params: withLoopStepSummarizer(step, provider) });
  }
;
async () => {

 { svelteHTML.createElement("label", { "for":`np-maxiter`,});   }
 { svelteHTML.createElement("input", {             "id":`np-maxiter`,"type":`number`,"min":`1`,"max":`10`,"value":paramNum(node, 'max_iterations', 3),"oninput":(e) => set('max_iterations', Number(e.currentTarget.value)),});}
 { svelteHTML.createElement("label", { "for":`np-until`,});   }
 { svelteHTML.createElement("input", {            "dir":`auto`,"id":`np-until`,"type":`text`,"placeholder":`e.g. passed == true`,"value":paramStr(node, 'until'),"oninput":(e) => set('until', e.currentTarget.value),});}

 { svelteHTML.createElement("div", { "class":`rv-h np-sec`,});
   { svelteHTML.createElement("span", { "class":`np-label`,});       }
   { svelteHTML.createElement("button", {     "class":`btn small ghost`,"type":`button`,"onclick":addLoopStep,});
     { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}  
   }
 }
if(loopSteps().length === 0){
   { svelteHTML.createElement("p", { "class":`insp-note`,});         { svelteHTML.createElement("code", {});  }    { svelteHTML.createElement("code", {});  }  }
}
    for(let step of __sveltets_2_ensureArray(loopSteps())){let i = 1;i;
   { svelteHTML.createElement("div", { "class":`rv-row`,});
     { svelteHTML.createElement("div", { "class":`rv-top`,});
       { svelteHTML.createElement("select", {        "aria-label":`Step kind`,"class":`ls-kind`,"value":step.kind,"onchange":(e) => updateLoopStep(i, { kind: e.currentTarget.value }),});
           for(let k of __sveltets_2_ensureArray(LOOP_STEP_KINDS)){k; { svelteHTML.createElement("option", { "value":k,});k; }}
        if(!LOOP_STEP_KINDS.includes(step.kind)){ { svelteHTML.createElement("option", { "value":step.kind,});step.kind; }}
       }
       { svelteHTML.createElement("input", {              "dir":`auto`,"aria-label":`Step name`,"class":`rv-lens`,"type":`text`,"placeholder":`name (e.g. fix)`,"value":step.name ?? '',"oninput":(e) => updateLoopStep(i, { name: e.currentTarget.value || undefined }),});}
       { svelteHTML.createElement("button", {         "class":`rv-del`,"type":`button`,"title":`Remove step`,"aria-label":`Remove step`,"onclick":() => removeLoopStep(i),});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":12,}});}
       }
     }
    if(LOOP_AGENT_KINDS.includes(step.kind)){
       { svelteHTML.createElement("textarea", {              "dir":`auto`,"aria-label":`Step prompt`,"class":`rv-instr np-prompt`,"rows":6,"placeholder":`prompt / instructions for this agent step`,"value":loopStepParam(i, 'prompt'),"oninput":(e) => updateLoopStepParam(i, 'prompt', e.currentTarget.value),}); }
       { svelteHTML.createElement("div", { "class":`ls-pm`,});
         { svelteHTML.createElement("select", {        "aria-label":`Step provider`,"class":`ls-prov`,"value":loopStepParam(i, 'provider'),"onchange":(e) => updateLoopStepParam(i, 'provider', e.currentTarget.value || undefined),});
           { svelteHTML.createElement("option", {"value":"",}); defaultAgentProvider();  }
             for(let pv of __sveltets_2_ensureArray(agentProviders())){pv; { svelteHTML.createElement("option", { "value":pv,});pv; }}
         }
         { svelteHTML.createElement("input", {              "dir":`ltr`,"aria-label":`Step model`,"class":`ls-model`,"type":`text`,"placeholder":`model (optional)`,"value":loopStepParam(i, 'model'),"oninput":(e) => updateLoopStepParam(i, 'model', e.currentTarget.value || undefined),});}
       }
    } else if (step.kind === 'review_run'){
       { svelteHTML.createElement("input", {              "dir":`auto`,"aria-label":`Reviewer providers`,"class":`rv-lens`,"type":`text`,"placeholder":`reviewer providers (comma-separated, e.g. claude, codex)`,"value":loopStepParamList(i, 'providers'),"oninput":(e) => updateLoopStepParam(i, 'providers', e.currentTarget.value.split(',').map((s) => s.trim()).filter(Boolean)),});}
       { svelteHTML.createElement("input", {              "dir":`auto`,"aria-label":`Summarizer provider`,"class":`rv-lens`,"type":`text`,"placeholder":`summarizer provider (optional)`,"value":loopStepSummarizer(i),"oninput":(e) => updateLoopStepSummarizer(i, e.currentTarget.value),});}
    }else{
       { svelteHTML.createElement("p", { "class":`insp-note`,});
         { svelteHTML.createElement("code", {});step.kind; }           
       }
    }
   }
}
 { svelteHTML.createElement("label", { "class":`np-chk`,});
   { svelteHTML.createElement("input", {       "type":`checkbox`,"checked":paramBool(node, 'continue_on_error'),"onchange":(e) => set('continue_on_error', e.currentTarget.checked),});}    
 }
 { svelteHTML.createElement("details", { "class":`ls-advanced`,});
   { svelteHTML.createElement("summary", {});       }
   { const $$_lebaLnosJ1C = __sveltets_2_ensureComponent(JsonLabel); new $$_lebaLnosJ1C({ target: __sveltets_2_any(), props: {      "forId":`np-steps`,"label":`Steps (JSON array)`,"onzoom":() => onzoom('steps', 'Steps (JSON array)'),}});}
   { svelteHTML.createElement("textarea", {            "dir":`ltr`,"id":`np-steps`,"rows":5,"placeholder":'[ { "kind": "agent_prompt", "params": {} } ]',"value":paramJson(node, 'steps'),"oninput":(e) => set('steps', jsonValue(e.currentTarget.value)),}); }
 }
};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const Loop__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type Loop__SvelteComponent_ = ReturnType<typeof Loop__SvelteComponent_>;
/*Ωignore_endΩ*/export default Loop__SvelteComponent_;