///<reference types="svelte" />
;// One card for every "needs you" item (patterns §5), in the thread and on
// the Tasks tab:
//  • approval — who asked, WHERE it goes, WHO sees it, WHY, and a preview of
//    exactly WHAT is sent; the primary names the action, Deny asks for an
//    optional reason, and "Always allow" is opt-in, scoped to this
//    destination and never offered for purchases or prod.
//  • question — the agent's question with its options (or a free answer).
//  • limit    — "Claude limit reached until 14:00 — continue on Codex?"
//  • takeover — you have control; Hand back resumes the agent.
//  • memory   — a suggested memory to accept or reject.
// A decided card stays in the thread with its outcome.

import ActionCard from './ActionCard.svelte';
import Badge from '../../../lib/components/Badge.svelte';
import ApprovalActions from '../../../lib/components/ApprovalActions.svelte';
import DenySheet from '../../../lib/components/DenySheet.svelte';
import Icon, { type IconName } from '../../../lib/components/Icon.svelte';
import { assistant, describeError } from '../../../lib/stores/assistant.svelte';
import { rel } from '../../../lib/stores/now.svelte';
import { toasts } from '../../../lib/toast.svelte';
import { limitNotice, providerLabel, providerName, TASK_KIND } from '../../../lib/assistant/model';
import { clock } from '../format';
import type { AssistantApprovalCard, AssistantDecisionReq, AssistantNeedsYou, AssistantTask, AssistantTaskAction } from '../../../lib/api/types';
;

  interface Props {
    task: AssistantTask;
    /** Tighter card (disclosed preview) — the Tasks tab lists many. */
    compact?: boolean;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { task, compact = false }: Props = $props();

  // The payload can be cleared once decided; keep the last one we saw so the
  // card still shows what was approved.
  let lastNeeds = $state<AssistantNeedsYou | null>(null);
  $effect(() => {
    if (task.needs_you) lastNeeds = task.needs_you;
  });
  const ny = $derived(task.needs_you ?? lastNeeds);
  const pending = $derived(task.state === 'needs_you');
  const kind = $derived(ny?.kind ?? (task.kind === 'memory_review' ? 'memory' : task.kind));
  const a = $derived<AssistantApprovalCard | null>(ny?.approval ?? null);

  const VERB: Record<AssistantApprovalCard['category'], string> = {
    send: 'Send',
    post: 'Post',
    publish: 'Publish',
    purchase: 'Buy',
    delete: 'Delete',
    submit: 'Submit',
    prod: 'Run on prod',
    other: 'Approve',
  };
  // Busy label for each verb: name the action while it is in flight.
  const BUSY: Record<string, string> = {
    Send: 'Sending…',
    Post: 'Posting…',
    Publish: 'Publishing…',
    Buy: 'Buying…',
    Delete: 'Deleting…',
    Submit: 'Submitting…',
    'Run on prod': 'Running…',
  };
  const verb = $derived(a ? VERB[a.category] ?? 'Approve' : 'Approve');
  // Never for purchases or prod, whatever the payload says.
  const canAlwaysAllow = $derived(!!a?.always_allow_allowed && a.category !== 'purchase' && a.category !== 'prod');

  const ICON: Record<string, IconName> = { approval: 'hand', question: 'comment', limit: 'warning', takeover: 'cursor', memory: 'book' };
  const TITLE: Record<string, string> = { approval: 'Approval needed', question: 'Otto asks', limit: 'Usage limit', takeover: 'You have control', memory: 'Suggested memory' };
  const headKind = $derived(pending ? (TITLE[kind] ?? 'Needs you') : (TASK_KIND[task.kind] ?? 'Request'));

  const decision = $derived(typeof task.result?.decision === 'string' ? (task.result.decision as string) : null);
  const outcome = $derived.by(() => {
    if (pending) return null;
    const by = typeof task.result?.decided_by === 'string' ? ` by ${task.result.decided_by}` : ' by you';
    // Outcome verbs match the buttons that decided them (Skip… / Reject / Keep).
    if (decision === 'denied' || task.state === 'cancelled')
      return { tone: 'neutral' as const, label: kind === 'question' ? `Skipped${by}` : kind === 'memory' ? `Rejected${by}` : `Denied${by}` };
    if (decision === 'approved' || task.state === 'done' || task.state === 'running')
      return { tone: 'ok' as const, label: kind === 'approval' ? `Approved${by}` : kind === 'memory' ? `Kept${by}` : 'Answered' };
    if (task.state === 'failed') return { tone: 'bad' as const, label: 'Failed' };
    return { tone: 'neutral' as const, label: 'Closed' };
  });

  let always = $state(false);
  let answer = $state('');
  let busy = $state<string | null>(null);
  let denying = $state(false);

  async function run(action: AssistantTaskAction, body: AssistantDecisionReq, label: string): Promise<void> {
    busy = label;
    try {
      await assistant.act(task.id, action, body);
    } catch (e) {
      // A memory proposal speaks Keep/Reject, not approve/deny.
      const title = label === 'accept' ? 'Couldn’t keep that memory'
        : label === 'reject' ? 'Couldn’t reject that memory'
        : action === 'deny' ? 'Couldn’t deny the request' : `Couldn’t ${label.toLowerCase()}`;
      toasts.error(title, describeError(e));
    } finally {
      busy = null;
    }
  }
  function deny(reason: string | null): void {
    denying = false;
    void run('deny', reason ? { reason } : {}, 'deny');
  }
  // Keyboard users can scroll long payloads; this is a named scroll region.
  function focusablePreview(node: HTMLElement): void { node.tabIndex = 0; }
  const suggestion = $derived(ny?.suggestion?.provider ?? null);
;
async () => {

 { const $$_draCnoitcA0C = __sveltets_2_ensureComponent(ActionCard); const $$_draCnoitcA0 = new $$_draCnoitcA0C({ target: __sveltets_2_any(), props: {         children:() => { return __sveltets_2_any(0); },"icon":ICON[kind] ?? 'hand',"heading":headKind,"summary":pending ? (kind === 'approval' ? ny?.prompt || task.title : task.title) : task.title,"attention":pending,"testid":`card-needs-${kind}`,pill:() => { async ()/*Ωignore_positionΩ*/ => {
    if(pending){
       { const $$_egdaB0C = __sveltets_2_ensureComponent(Badge); new $$_egdaB0C({ target: __sveltets_2_any(), props: {    "tone":`warn`,"label":`Needs you`,}});}
    } else if (outcome){
       { const $$_egdaB0C = __sveltets_2_ensureComponent(Badge); new $$_egdaB0C({ target: __sveltets_2_any(), props: {    "tone":outcome.tone,"label":outcome.label,}});}
    }
  };return __sveltets_2_any(0)},footer:() => { async ()/*Ωignore_positionΩ*/ => {
    if(pending){
      if(kind === 'approval'){
         { const $$_snoitcAlavorppA0C = __sveltets_2_ensureComponent(ApprovalActions); const $$_snoitcAlavorppA0 = new $$_snoitcAlavorppA0C({ target: __sveltets_2_any(), props: {             "approveLabel":verb,"approveBusyLabel":BUSY[verb] ?? 'Approving…',"busy":busy === 'deny' ? 'deny' : busy !== null ? 'approve' : null,"denyTarget":`this ${verb.toLowerCase()}`,"onapprove":() => run('approve', canAlwaysAllow && always ? { always_allow: true } : {}, verb),"ondeny":(reason) => run('deny', reason ? { reason } : {}, 'deny'),extra:() => { async ()/*Ωignore_positionΩ*/ => {
            if(canAlwaysAllow){
               { svelteHTML.createElement("label", { "class":`always checkbox-row`,});
                 { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":always,});/*Ωignore_startΩ*/() => always = __sveltets_2_any(null);/*Ωignore_endΩ*/}
                   a?.destination ?? a?.where;
               }
            }
          };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {extra} = $$_snoitcAlavorppA0.$$prop_def;/*Ωignore_endΩ*/
          
         ApprovalActions}
      } else if (kind === 'limit'){
         { svelteHTML.createElement("button", {       "class":`btn small ghost`,"onclick":() => void run('deny', {}, 'wait'),"disabled":busy !== null,"title":`Stay on this provider and wait for the reset`,});  }
        if(suggestion){
           { svelteHTML.createElement("button", {     "class":`btn small primary`,"onclick":() => void run('approve', { provider: suggestion }, 'switch'),"disabled":busy !== null,});
            busy === 'switch' ? 'Switching…' : `Continue on ${providerLabel(suggestion, ny?.suggestion?.model ?? null)}`;
           }
        }
      } else if (kind === 'takeover'){
         { svelteHTML.createElement("button", {     "class":`btn small primary`,"onclick":() => void run('handback', {}, 'hand back'),"disabled":busy !== null,});busy ? 'Handing back…' : 'Hand back'; }
      } else if (kind === 'memory'){
         { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":() => void run('deny', {}, 'reject'),"disabled":busy !== null,});busy === 'reject' ? 'Rejecting…' : 'Reject'; }
         { svelteHTML.createElement("button", {     "class":`btn small primary`,"onclick":() => void run('approve', {}, 'accept'),"disabled":busy !== null,});busy === 'accept' ? 'Keeping…' : 'Keep'; }
      } else if (kind === 'question'){
         { svelteHTML.createElement("button", {     "class":`btn small ghost`,"onclick":() => (denying = true),"disabled":busy !== null,});  }
      }
    }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {pill, footer} = $$_draCnoitcA0.$$prop_def;/*Ωignore_endΩ*/
  

   { svelteHTML.createElement("p", { "class":`asker`,});
     { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`assistant`,"size":12,}});}
     { svelteHTML.createElement("span", {});   { svelteHTML.createElement("strong", {});  } }
     { svelteHTML.createElement("span", { "class":`dim`,});  { svelteHTML.createElement("time", {   "datetime":task.created_at,"title":new Date(task.created_at).toLocaleString(),});rel(task.created_at); } }
   }

  if(kind === 'approval' && a){
     { svelteHTML.createElement("dl", { "class":`kv`,});
       { svelteHTML.createElement("dt", {});  }
       { svelteHTML.createElement("dd", {});a.where; }
       { svelteHTML.createElement("dt", {});  }
       { svelteHTML.createElement("dd", {});a.who_sees; }
       { svelteHTML.createElement("dt", {});  }
       { svelteHTML.createElement("dd", {});a.reason; }
     }
    if(a.what){
      if(compact){
         { svelteHTML.createElement("details", { "class":`payload`,});
           { svelteHTML.createElement("summary", {});     }
           {const $$action_0 = __sveltets_2_ensureAction(focusablePreview(svelteHTML.mapElementTag('div')));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {     "class":`preview`,"role":`region`,"aria-label":`Exactly what is sent`,});a.what; }}
         }
      }else{
         {const $$action_0 = __sveltets_2_ensureAction(focusablePreview(svelteHTML.mapElementTag('div')));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {     "class":`preview`,"role":`region`,"aria-label":`Exactly what is sent`,});a.what; }}
      }
    }
  } else if (kind === 'limit'){
     { svelteHTML.createElement("p", { "class":`q`,});limitNotice(ny?.limit?.provider ?? 'claude', ny?.limit?.until ?? null, pending ? suggestion : null, clock); }
    if(ny?.limit?.message){ { svelteHTML.createElement("p", { "class":`dim small`,});ny.limit.message; }}
  } else if (kind === 'takeover'){
     { svelteHTML.createElement("p", { "class":`q`,});pending ? 'Otto is paused while you use it. Hand it back when you’re done.' : ny?.prompt || task.title; }
  }else{
     { svelteHTML.createElement("p", { "class":`q`,});ny?.prompt || task.detail || task.title; }
    if(kind === 'question' && pending){
      if(ny?.options?.length){
         { svelteHTML.createElement("div", {     "class":`opts`,"role":`group`,"aria-label":`Answers`,});
              for(let o of __sveltets_2_ensureArray(ny.options)){let i = 1;i;
             { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":() => void run('approve', { answer: o }, 'answer'),"disabled":busy !== null,});o; }
          }
         }
      }else{
         { svelteHTML.createElement("form", {   "class":`answer`,"onsubmit":(e) => { e.preventDefault(); if (answer.trim()) void run('approve', { answer: answer.trim() }, 'answer'); },});
           { svelteHTML.createElement("label", {   "class":`sr-only`,"for":`answer-${task.id}`,});  }
           { svelteHTML.createElement("input", {          "dir":`auto`,"id":`answer-${task.id}`,"class":`input`,"bind:value":answer,"placeholder":`Your answer`,});/*Ωignore_startΩ*/() => answer = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           { svelteHTML.createElement("button", {     "class":`btn small`,"type":`submit`,"disabled":!answer.trim() || busy !== null,});  }
         }
      }
    }
  }
  if(!pending && typeof task.result?.reason === 'string'){
     { svelteHTML.createElement("p", { "class":`dim small`,}); task.result.reason as string; }
  }

  
 ActionCard}
if(denying && kind === 'question'){
   { const $$_teehSyneD0C = __sveltets_2_ensureComponent(DenySheet); new $$_teehSyneD0C({ target: __sveltets_2_any(), props: {             "action":`answer`,"title":`Skip question`,"confirmLabel":`Skip`,"hint":`Otto gets no answer to this question. It sees your reason and can carry on another way.`,"onclose":() => (denying = false),"ondeny":deny,}});}
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const NeedsYouCard__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type NeedsYouCard__SvelteComponent_ = ReturnType<typeof NeedsYouCard__SvelteComponent_>;
/*Ωignore_endΩ*/export default NeedsYouCard__SvelteComponent_;