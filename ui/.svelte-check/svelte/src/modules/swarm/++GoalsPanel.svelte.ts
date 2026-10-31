///<reference types="svelte" />
;
import { announce } from '../../lib/announce.svelte';
import { toastError } from '../../lib/toastError';
// Per-task Goals view: the goals the Coordinator verifies for a task, each with
// a live status, the measured value + verdict summary, and retry budget. Lets
// you add / edit / delete goals and run (or stop) verification on demand.
import Modal from '../../lib/components/Modal.svelte';
import Icon from '../../lib/components/Icon.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import GoalEditor from './GoalEditor.svelte';
import { untrack } from 'svelte';
import { swarm } from '../../lib/stores/swarm.svelte';
import { toasts } from '../../lib/toast.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import StatusBadge from '../../lib/components/StatusBadge.svelte';
import { sentenceCase, type StatusInfo } from '../../lib/status';
import type { GoalStatus, SwarmGoal, SwarmTask } from './types';
;

  interface Props {
    task: SwarmTask;
    onclose: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { task, onclose }: Props = $props();

  const goals = $derived(swarm.goalsForTask(task.id));
  const goalsKnown = $derived(task.id in swarm.goalsByTask);
  const goalsLoading = $derived(!!swarm.taskGoalsLoading[task.id]);
  const goalsError = $derived(swarm.taskGoalsError[task.id] ?? null);
  const verify = $derived(swarm.verifyByTask[task.id] ?? null);
  const running = $derived(verify?.running ?? false);

  let editorOpen = $state(false);
  let editGoal = $state<SwarmGoal | null>(null);
  let verifying = $state(false);

  // loadVerification writes taskGoalsLoading before its first await. Tracking
  // that read would reschedule this effect on every load and freeze the page.
  $effect(() => {
    const id = task.id;
    untrack(() => void swarm.loadVerification(id));
  });

  function goalStatus(status: GoalStatus): StatusInfo {
    switch (status) {
      case 'verifying':
        return { key: status, label: 'Verifying', tone: 'info', live: true };
      case 'passed':
        return { key: status, label: 'Passed', tone: 'success' };
      case 'warned':
        return { key: status, label: 'Warned', tone: 'warning' };
      case 'unmet':
        return { key: status, label: 'Unmet', tone: 'danger' };
      case 'error':
        return { key: status, label: 'Error', tone: 'danger' };
      case 'skipped':
        return { key: status, label: 'Skipped', tone: 'neutral' };
      default:
        return { key: status, label: 'Pending', tone: 'neutral' };
    }
  }

  function measured(g: SwarmGoal): string | null {
    const m = g.verdict?.measured;
    if (m === null || m === undefined || m === '') return null;
    return String(m);
  }

  async function runVerify() {
    if (verifying) return;
    verifying = true;
    try {
      const res = await swarm.verifyTask(task.id);
      if (res.started) toasts.success('Verification started', 'Watch goal statuses update live.');
      else toasts.info("Verification didn’t start", res.reason ?? 'Verification couldn’t start.');
    } catch (e) {
      toastError("Couldn’t start verification", e);
    } finally {
      verifying = false;
    }
  }

  async function stop() {
    const ok = await confirmer.ask(
      'Stop verifying this task? Goals already checked keep their result. Checks still running do not.',
      { title: 'Stop verification', confirmLabel: 'Stop', danger: true },
    );
    if (!ok) return;
    try {
      await swarm.stopVerify(task.id);
      toasts.info('Verification stopped');
    } catch (e) {
      toastError("Couldn’t stop verification", e);
    }
  }

  function add() {
    editGoal = null;
    editorOpen = true;
  }
  function edit(g: SwarmGoal) {
    editGoal = g;
    editorOpen = true;
  }
  async function del(g: SwarmGoal) {
    if (
      await confirmer.ask(`Delete goal “${g.title}”? Its verification history goes with it.`, {
        title: 'Delete goal',
        confirmLabel: 'Delete',
        danger: true,
      })
    ) {
      try {
        await swarm.deleteGoal(g.id);
      } catch (e) {
        toastError("Couldn’t delete the goal", e);
      }
    }
  }

  // a11y-6-01: announce at the state edge — a status element inserted
  // together with its text is not read by VoiceOver/WebKit.
  $effect(() => { if (running) announce('Verifying…'); });
;
async () => {

 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); new $$_ladoM0C({ target: __sveltets_2_any(), props: {    children:() => { return __sveltets_2_any(0); },"title":`Goals — ${task.title}`,"width":620,onclose,}});
   { svelteHTML.createElement("div", { "class":`bar`,});
    if(running){
       { svelteHTML.createElement("span", { "class":`running`,}); { svelteHTML.createElement("span", {     "class":`spinner`,"style":`--spinner-size: 10px`,"aria-hidden":`true`,}); }  }
       { svelteHTML.createElement("button", {     "class":`btn small danger`,"onclick":() => void stop(),"title":`Stop verifying — goals already checked keep their result`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`stop`,"size":12,}});}  }
    }else{
       { svelteHTML.createElement("button", {       "class":`btn small primary`,"onclick":runVerify,"disabled":verifying || !goals.length,"title":goals.length ? 'Run every goal check for this task now' : 'Add a goal first',});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}  
       }
    }
     { svelteHTML.createElement("span", { "class":`grow`,}); }
     { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":add,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}   }
   }

  if(goalsLoading && !goalsKnown){
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {      "what":`goals`,"variant":`compact`,"loading":true,"empty":true,"rows":3,}});}
  } else if (goalsError && !goalsKnown){
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {        "what":`goals`,"variant":`compact`,"error":goalsError,"empty":true,"onretry":() => void swarm.loadVerification(task.id),}});}
  } else if (goals.length === 0){
     { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {      "icon":`check`,"title":`No goals yet`,"body":`Add a goal so the Coordinator can verify this task is actually done.`,}});}
  }else{
    if(goalsError){
       { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {          "what":`goals`,"variant":`compact`,"error":goalsError,"empty":false,"onretry":() => void swarm.loadVerification(task.id),}});}
    }
     { svelteHTML.createElement("div", { "class":`goals`,});
         for(let g of __sveltets_2_ensureArray(goals)){g.id;
        const m = measured(g);
         { svelteHTML.createElement("div", { "class":`goal`,});
           { svelteHTML.createElement("div", { "class":`g-head`,});
             { const $$_egdaBsutatS4C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS4C({ target: __sveltets_2_any(), props: {  "status":goalStatus(g.status),}});}
             { svelteHTML.createElement("span", { "class":`kind`,});sentenceCase(g.kind); }
            if(g.blocking){ { svelteHTML.createElement("span", {   "class":`blocking`,"title":`Blocks task completion until it passes`,});  }}
             { svelteHTML.createElement("span", { "class":`g-title`,});g.title; }
             { svelteHTML.createElement("span", { "class":`grow`,}); }
             { svelteHTML.createElement("button", {       "class":`icon-btn small`,"onclick":() => edit(g),"aria-label":`Edit goal “${g.title}”`,"title":`Edit goal “${g.title}”`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`edit`,"size":13,}});} }
             { svelteHTML.createElement("button", {       "class":`icon-btn small`,"onclick":() => del(g),"aria-label":`Delete goal “${g.title}”`,"title":`Delete goal “${g.title}”`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":13,}});} }
           }
          if(g.description){ { svelteHTML.createElement("div", { "class":`g-desc`,});g.description; }}
           { svelteHTML.createElement("div", { "class":`g-meta`,});
            if(g.metric){
               { svelteHTML.createElement("span", { "class":`pair`,}); { svelteHTML.createElement("span", { "class":`k`,});  } g.metric;g.comparator ? ` ${g.comparator}` : '';g.target_value != null ? ` ${g.target_value}` : ''; }
            }
            if(m !== null){
               { svelteHTML.createElement("span", { "class":`pair measured`,}); { svelteHTML.createElement("span", { "class":`k`,});  } m; }
            }
             { svelteHTML.createElement("span", { "class":`pair`,}); { svelteHTML.createElement("span", { "class":`k`,});  } g.iterations; g.max_retries; }
            if(g.verify_cmd){ { svelteHTML.createElement("span", {   "class":`pair cmd`,"title":g.verify_cmd,}); { svelteHTML.createElement("span", { "class":`k`,});  } g.verify_cmd; }}
           }
          if(g.verdict?.summary){
             { svelteHTML.createElement("div", {  "class":`verdict`,});g.verdict.blocker;g.verdict.summary; }
          }
         }
      }
     }
  }
 Modal}

if(editorOpen){
   { const $$_rotidElaoG0C = __sveltets_2_ensureComponent(GoalEditor); new $$_rotidElaoG0C({ target: __sveltets_2_any(), props: {       "goal":editGoal,"scope":editGoal ? null : { task: task.id },"onclose":() => { editorOpen = false; editGoal = null; },}});}
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const GoalsPanel__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type GoalsPanel__SvelteComponent_ = ReturnType<typeof GoalsPanel__SvelteComponent_>;
/*Ωignore_endΩ*/export default GoalsPanel__SvelteComponent_;