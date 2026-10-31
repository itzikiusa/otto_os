///<reference types="svelte" />
;// A longer job the assistant took on, with its LIVE state (queued → running
// → needs you → done / failed), what it is doing now, and Stop while it runs.

import ActionCard from './ActionCard.svelte';
import Badge from '../../../lib/components/Badge.svelte';
import Icon from '../../../lib/components/Icon.svelte';
import { assistant, describeError } from '../../../lib/stores/assistant.svelte';
import { rel } from '../../../lib/stores/now.svelte';
import { toasts } from '../../../lib/toast.svelte';
import { confirmer } from '../../../lib/confirm.svelte';
import { TASK_KIND, taskStateLabel, taskTone } from '../../../lib/assistant/model';
import type { AssistantTask } from '../../../lib/api/types';
;

  interface Props {
    task: AssistantTask;
    onopen?: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  

  let { task, onopen }: Props = $props();

  const summary = $derived(typeof task.result?.summary === 'string' ? (task.result.summary as string) : null);
  let busy = $state(false);
  async function stop(): Promise<void> {
    const ok = await confirmer.ask(`Stop “${task.title}”? Otto drops what it has not finished.`, { title: 'Stop task', confirmLabel: 'Stop' });
    if (!ok) return;
    busy = true;
    try {
      await assistant.act(task.id, 'cancel');
    } catch (e) {
      toasts.error('Couldn’t stop the task', describeError(e));
    } finally {
      busy = false;
    }
  }
;
async () => {

 { const $$_draCnoitcA0C = __sveltets_2_ensureComponent(ActionCard); const $$_draCnoitcA0 = new $$_draCnoitcA0C({ target: __sveltets_2_any(), props: {         children:() => { return __sveltets_2_any(0); },"icon":`check`,"heading":TASK_KIND[task.kind] ?? 'Task',"summary":task.title,"attention":task.state === 'needs_you',"testid":`card-task`,pill:() => { async ()/*Ωignore_positionΩ*/ => {
     { const $$_egdaB0C = __sveltets_2_ensureComponent(Badge); new $$_egdaB0C({ target: __sveltets_2_any(), props: {      "tone":taskTone(task),"label":taskStateLabel(task),"live":task.state === 'running',}});}
  };return __sveltets_2_any(0)},footer:() => { async ()/*Ωignore_positionΩ*/ => {
    if(task.state === 'running' || task.state === 'queued'){
       { svelteHTML.createElement("button", {     "class":`btn small ghost`,"onclick":() => void stop(),"disabled":busy,}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`stop`,"size":12,}});} busy ? 'Stopping…' : 'Stop'; }
    }
    if(onopen){ { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":onopen,});   }}
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {pill, footer} = $$_draCnoitcA0.$$prop_def;/*Ωignore_endΩ*/
  
  if(task.detail){ { svelteHTML.createElement("p", { "class":`detail`,});task.detail; }}
  if(summary && task.state !== 'running'){ { svelteHTML.createElement("p", { "class":`detail`,});summary; }}
   { svelteHTML.createElement("p", { "class":`meta`,});
      { svelteHTML.createElement("time", {   "datetime":task.updated_at,"title":new Date(task.updated_at).toLocaleString(),});rel(task.updated_at); }
   }
  
 ActionCard}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const TaskCard__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type TaskCard__SvelteComponent_ = ReturnType<typeof TaskCard__SvelteComponent_>;
/*Ωignore_endΩ*/export default TaskCard__SvelteComponent_;