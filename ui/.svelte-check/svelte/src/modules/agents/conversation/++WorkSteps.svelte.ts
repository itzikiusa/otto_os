///<reference types="svelte" />
;// A run of tool activity inside one response, collapsed to ONE quiet line
// that says what happened — "Ran 2 commands, edited retry.rs, read 3 files ·
// 2m 7s" — with a status mark (spinner while the agent is still on it, ✓, or
// "N failed"). Expanding lists the steps as a timeline of one-line rows
// (ToolStep), subagent cards and plan snapshots. A group of exactly one call
// renders that row directly. Thinking markers carry no text (the transcript
// keeps only that it happened): in the timeline they are their own violet,
// italic "Thought" rows, never confusable with a step or the answer.

import { untrack } from 'svelte';
import Icon from '../../../lib/components/Icon.svelte';
import ToolStep from './ToolStep.svelte';
import SubagentCard from './SubagentCard.svelte';
import TasksBlock from './TasksBlock.svelte';
import { fmtDuration, stepSummary, type StepBlock } from './format';
import type { Block } from '../../../lib/api/types';
;

  interface Props {
    steps: StepBlock[];
    /** Response duration (only shown on the response's first group). */
    durationMs?: number | null;
    /** The agent is working on this response right now: starts expanded. */
    live?: boolean;
    /** The session is alive and this is its newest response: a call without a
     *  result is in progress (or waiting on you), not abandoned. */
    active?: boolean;
    /** The agent is stopped waiting for you on this response's open call. */
    waiting?: boolean;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { steps, durationMs = null, live = false, active = live, waiting = false }: Props = $props();

  const visible = $derived(steps.filter((s) => s.kind !== 'thinking'));
  const calls = $derived(steps.filter((s) => s.kind === 'tool_call' || s.kind === 'subagent').length);
  const thinking = $derived(steps.reduce((n, s) => (s.kind === 'thinking' ? n + s.count : n), 0));
  const failed = $derived(steps.filter((s) => s.kind === 'tool_call' && s.result != null && !s.result.ok).length);
  const running = $derived(active && steps.some((s) => (s.kind === 'tool_call' && s.result == null) || (s.kind === 'subagent' && s.status === 'running')));
  const summary = $derived(stepSummary(steps));
  // One-liners (a single tool call) don't need the group header.
  const single = $derived(visible.length === 1 && visible[0].kind === 'tool_call');
  // Initial-open only: a live response starts expanded, the reader owns it after.
  let open = $state(untrack(() => live));
  const dur = $derived(fmtDuration(durationMs));
  const tip = $derived(
    [`${calls} ${calls === 1 ? 'step' : 'steps'}`, dur && `took ${dur}`, thinking && `thought ${thinking}×`].filter(Boolean).join(' · '),
  );
;
async () => {

if(single){
   { svelteHTML.createElement("div", {   "class":`steps single`,...__sveltets_2_empty({"data-steps":calls}),});
     { const $$_petSlooT1C = __sveltets_2_ensureComponent(ToolStep); new $$_petSlooT1C({ target: __sveltets_2_any(), props: {     "block":visible[0] as Extract<Block, { kind: 'tool_call' }>,"live":active,waiting,}});}
   }
} else if (visible.length){
   { svelteHTML.createElement("div", {     "class":`steps`,...__sveltets_2_empty({"data-steps":calls}),...__sveltets_2_empty({"data-failed":failed || undefined}),});open;
     { svelteHTML.createElement("button", {       "class":`steps-head`,"onclick":() => (open = !open),"aria-expanded":open,"title":tip,});
       { svelteHTML.createElement("span", {      "class":`steps-mark`,"aria-hidden":`true`,});running;running && waiting;!running && failed > 0;
        if(running && waiting){ { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});}
        } else if (running){ { svelteHTML.createElement("span", {    "class":`spinner`,"aria-hidden":`true`,});__sveltets_2_ensureType(String, Number, " "); }
        } else if (failed){ { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});}
        }else{ { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}}
       }
       { svelteHTML.createElement("span", { "class":`steps-title`,});summary; }
      if(failed){ { svelteHTML.createElement("span", { "class":`steps-fail`,});failed;  }}
       { svelteHTML.createElement("span", { "class":`steps-meta`,});dur ? `${dur} · ` : '';calls; calls === 1 ? 'step' : 'steps'; }
       { svelteHTML.createElement("span", {   "class":`steps-caret`,"aria-hidden":`true`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":open ? 'chevronDown' : 'chevronRight',"size":12,}});} }
     }
    if(open){
       { svelteHTML.createElement("div", { "class":`steps-list`,});
            for(let s of __sveltets_2_ensureArray(steps)){let i = 1;s.kind === 'tool_call' ? s.id : s.kind === 'subagent' ? s.agent_id : `${s.kind}-${i}`;
          if(s.kind === 'thinking'){
             { svelteHTML.createElement("div", {   "class":`think-step`,"title":`Reasoning — the text is not saved to the transcript`,});
               { svelteHTML.createElement("span", {   "class":`think-icon`,"aria-hidden":`true`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`sparkle`,"size":12,}});} }
               { svelteHTML.createElement("span", {}); s.count > 1 ? ` ×${s.count}` : ''; }
             }
          } else if (s.kind === 'tool_call'){
             { const $$_petSlooT2C = __sveltets_2_ensureComponent(ToolStep); new $$_petSlooT2C({ target: __sveltets_2_any(), props: {     "block":s,"live":active,waiting,}});}
          } else if (s.kind === 'subagent'){
             { const $$_draCtnegabuS2C = __sveltets_2_ensureComponent(SubagentCard); new $$_draCtnegabuS2C({ target: __sveltets_2_any(), props: {        "agentId":s.agent_id,"description":s.description,"agentType":s.agent_type,"status":s.status,}});}
          } else if (s.kind === 'tasks'){
             { const $$_kcolBsksaT2C = __sveltets_2_ensureComponent(TasksBlock); new $$_kcolBsksaT2C({ target: __sveltets_2_any(), props: {  "tasks":s.tasks,}});}
          }
        }
       }
    }
   }
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const WorkSteps__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type WorkSteps__SvelteComponent_ = ReturnType<typeof WorkSteps__SvelteComponent_>;
/*Ωignore_endΩ*/export default WorkSteps__SvelteComponent_;