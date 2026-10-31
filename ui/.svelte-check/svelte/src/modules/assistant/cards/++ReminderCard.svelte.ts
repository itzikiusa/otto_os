///<reference types="svelte" />
;// A one-shot reminder: when it fires and where it is delivered (back to the
// origin that asked, plus a notification). Cancel while it's still pending.

import ActionCard from './ActionCard.svelte';
import Badge from '../../../lib/components/Badge.svelte';
import { assistant, describeError } from '../../../lib/stores/assistant.svelte';
import { confirmer } from '../../../lib/confirm.svelte';
import { toasts } from '../../../lib/toast.svelte';
import { taskStateLabel, taskTone } from '../../../lib/assistant/model';
import { deliverLabel, whenLabel } from '../format';
import type { AssistantTask } from '../../../lib/api/types';
;

  interface Props {
    task: AssistantTask;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  

  let { task }: Props = $props();

  let busy = $state(false);
  async function cancel(): Promise<void> {
    const ok = await confirmer.ask(`Cancel “${task.title}”? It will not fire.`, {
      title: 'Cancel reminder',
      confirmLabel: 'Cancel reminder',
      cancelLabel: 'Keep reminder',
      danger: true,
    });
    if (!ok) return;
    busy = true;
    try {
      await assistant.act(task.id, 'cancel');
    } catch (e) {
      toasts.error('Couldn’t cancel the reminder', describeError(e));
    } finally {
      busy = false;
    }
  }
;
async () => {

 { const $$_draCnoitcA0C = __sveltets_2_ensureComponent(ActionCard); const $$_draCnoitcA0 = new $$_draCnoitcA0C({ target: __sveltets_2_any(), props: {       children:() => { return __sveltets_2_any(0); },"icon":`clock`,"heading":`Reminder`,"summary":task.title,"testid":`card-reminder`,pill:() => { async ()/*Ωignore_positionΩ*/ => { { const $$_egdaB0C = __sveltets_2_ensureComponent(Badge); new $$_egdaB0C({ target: __sveltets_2_any(), props: {    "tone":taskTone(task),"label":taskStateLabel(task),}});}};return __sveltets_2_any(0)},footer:() => { async ()/*Ωignore_positionΩ*/ => {
    if(task.state === 'queued'){
       { svelteHTML.createElement("button", {     "class":`btn small ghost`,"onclick":() => void cancel(),"disabled":busy,});busy ? 'Canceling…' : 'Cancel reminder'; }
    }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {pill, footer} = $$_draCnoitcA0.$$prop_def;/*Ωignore_endΩ*/
  
   { svelteHTML.createElement("p", { "class":`line`,});
    if(task.run_at){
       { svelteHTML.createElement("time", {   "datetime":task.run_at,"title":new Date(task.run_at).toLocaleString(),});whenLabel(task.run_at); }
       { svelteHTML.createElement("span", {   "class":`sep`,"aria-hidden":`true`,});  }
    }
     { svelteHTML.createElement("span", {});deliverLabel(task.origin); }
     { svelteHTML.createElement("span", {   "class":`sep`,"aria-hidden":`true`,});  }
     { svelteHTML.createElement("span", { "class":`dim`,});  }
   }
  if(task.detail){ { svelteHTML.createElement("p", { "class":`detail`,});task.detail; }}
  
 ActionCard}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ReminderCard__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ReminderCard__SvelteComponent_ = ReturnType<typeof ReminderCard__SvelteComponent_>;
/*Ωignore_endΩ*/export default ReminderCard__SvelteComponent_;