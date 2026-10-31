///<reference types="svelte" />
;// Picks the card for a task in the thread: needs-you items (approval,
// question, limit, takeover, memory) → NeedsYouCard; a browser chore →
// BrowserCard; a reminder → ReminderCard; a delegation → a quiet line;
// anything else → TaskCard. The task row is LIVE (store), so the card
// follows it through queued → running → needs you → done.

import { untrack } from 'svelte';
import { assistant } from '../../../lib/stores/assistant.svelte';
import { browserOf } from '../../../lib/assistant/model';
import NeedsYouCard from './NeedsYouCard.svelte';
import BrowserCard from './BrowserCard.svelte';
import ReminderCard from './ReminderCard.svelte';
import TaskCard from './TaskCard.svelte';
import SystemLine from './SystemLine.svelte';
import type { AssistantTurn } from '../../../lib/api/types';
;

  interface Props {
    taskId: string;
    turn: AssistantTurn | null;
    onopentasks?: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { taskId, turn, onopentasks }: Props = $props();

  const task = $derived(assistant.task(taskId));
  $effect(() => {
    const id = taskId;
    if (!task) untrack(() => void assistant.ensureTask(id));
  });
  const progress = $derived(browserOf(task));
  const NEEDS = new Set<string>(['approval', 'question', 'limit', 'takeover', 'memory_review']);
;
async () => {

if(!task){
  if(turn){ { const $$_eniLmetsyS0C = __sveltets_2_ensureComponent(SystemLine); new $$_eniLmetsyS0C({ target: __sveltets_2_any(), props: { turn,}});}}
} else if (progress){
   { const $$_draCresworB0C = __sveltets_2_ensureComponent(BrowserCard); new $$_draCresworB0C({ target: __sveltets_2_any(), props: {  task,progress,}});}
} else if (NEEDS.has(task.kind) || task.needs_you){
   { const $$_draCuoYsdeeN0C = __sveltets_2_ensureComponent(NeedsYouCard); new $$_draCuoYsdeeN0C({ target: __sveltets_2_any(), props: { task,}});}
} else if (task.kind === 'reminder'){
   { const $$_draCrednimeR0C = __sveltets_2_ensureComponent(ReminderCard); new $$_draCrednimeR0C({ target: __sveltets_2_any(), props: { task,}});}
} else if (task.kind === 'delegation'){
   { const $$_eniLmetsyS0C = __sveltets_2_ensureComponent(SystemLine); new $$_eniLmetsyS0C({ target: __sveltets_2_any(), props: {  turn,task,}});}
}else{
   { const $$_draCksaT0C = __sveltets_2_ensureComponent(TaskCard); new $$_draCksaT0C({ target: __sveltets_2_any(), props: {   task,"onopen":onopentasks,}});}
}
};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const TaskEntry__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type TaskEntry__SvelteComponent_ = ReturnType<typeof TaskEntry__SvelteComponent_>;
/*Ωignore_endΩ*/export default TaskEntry__SvelteComponent_;