///<reference types="svelte" />
;
import { announceShown } from '../../lib/announce.svelte';
// The Tasks tab: everything the assistant is doing or waiting on — needs
// you → running → queued → done. What needs you comes first, in the full
// approval / question / limit shape, so it can be decided right here.
import Icon from '../../lib/components/Icon.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
import { assistant, describeError } from '../../lib/stores/assistant.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { toasts } from '../../lib/toast.svelte';
import { rel } from '../../lib/stores/now.svelte';
import NeedsYouCard from './cards/NeedsYouCard.svelte';
import Badge from '../../lib/components/Badge.svelte';
import { TASK_KIND, groupTasks, taskStateLabel, taskTone } from '../../lib/assistant/model';
import { whenLabel } from './format';
import type { AssistantTask } from '../../lib/api/types';
import LoadState from '../../lib/components/LoadState.svelte';
;

  interface Props {
    onopenthread: (id: string) => void;
  };function $$render() {
 const row/*Ωignore_positionΩ*/ = (t: AssistantTask)/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
   { svelteHTML.createElement("li", { "class":`task`,});
     { svelteHTML.createElement("div", { "class":`line1`,});
       { svelteHTML.createElement("span", {   "class":`title`,"title":t.title,});t.title; }
       { const $$_egdaB2C = __sveltets_2_ensureComponent(Badge); new $$_egdaB2C({ target: __sveltets_2_any(), props: {      "tone":taskTone(t),"label":taskStateLabel(t),"live":t.state === 'running',}});}
     }
    if(t.detail){ { svelteHTML.createElement("div", { "class":`detail`,});t.detail; }}
     { svelteHTML.createElement("div", { "class":`meta`,});
       { svelteHTML.createElement("span", {});TASK_KIND[t.kind] ?? t.kind; }
      if(t.run_at && t.state === 'queued'){ { svelteHTML.createElement("span", {}); whenLabel(t.run_at); }}
       { svelteHTML.createElement("span", {});  { svelteHTML.createElement("time", {   "datetime":t.updated_at,"title":new Date(t.updated_at).toLocaleString(),});rel(t.updated_at); } }
      if(t.thread_id && threadTitle(t.thread_id)){
         { svelteHTML.createElement("span", {});  }
         { svelteHTML.createElement("button", {   "class":`link`,"onclick":() => onopenthread(t.thread_id!),});threadTitle(t.thread_id); }
      }
     }
    if(t.state === 'running' || t.state === 'queued'){
       { svelteHTML.createElement("div", { "class":`acts`,});
         { svelteHTML.createElement("button", {       "class":`btn small ghost`,"onclick":() => void stopTask(t),"disabled":stopping === t.id,"aria-label":`Stop ${t.title}`,});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`stop`,"size":12,}});} stopping === t.id ? 'Stopping…' : 'Stop';
         }
       }
    }
   }
};return __sveltets_2_any(0)};
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { onopenthread }: Props = $props();

  $effect(() => {
    void assistant.loadTasks();
    void assistant.loadNeedsYou();
  });

  const tasks = $derived(assistant.tasks);
  // The board lists everything except the queue; the queue comes from its own (live) source.
  const needs = $derived(assistant.needs.items);
  const needIds = $derived(new Set(needs.map((n) => n.id)));
  const groups = $derived(groupTasks(tasks.data.filter((t) => !needIds.has(t.id) && t.state !== 'needs_you')));
  const total = $derived(tasks.data.length + needs.length);
  const threadTitle = (id: string | null): string | null => (id ? (assistant.thread(id)?.title ?? null) : null);
  let stopping = $state<string | null>(null);
  async function stopTask(t: AssistantTask): Promise<void> {
    const ok = await confirmer.ask(`Stop “${t.title}”? Otto drops what it has not finished.`, { title: 'Stop task', confirmLabel: 'Stop', danger: true });
    if (!ok) return;
    stopping = t.id;
    try {
      await assistant.act(t.id, 'cancel');
    } catch (e) {
      toasts.error('Couldn’t stop the task', describeError(e));
    } finally {
      stopping = null;
    }
  }
  const COLUMNS = $derived([
    { key: 'running', label: 'Running', list: groups.running },
    { key: 'queued', label: 'Queued', list: groups.queued },
    { key: 'finished', label: 'Done', list: groups.finished },
  ]);
;
async () => {



 { svelteHTML.createElement("div", {   "class":`tasks`,...__sveltets_2_empty({"data-testid":`assistant-tasks`}),});
  if(assistant.needsState === 'error'){
     {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('div'),({ text: 'Couldn’t load requests waiting on you.', assertive: true })));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {   "class":`error`,});
       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":14,}});}
       { svelteHTML.createElement("div", { "class":`error-t`,}); { svelteHTML.createElement("strong", {});      } { svelteHTML.createElement("span", { "class":`dim`,});        } }
       { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => void assistant.loadNeedsYou(),});  }
     }}
  }
  if(tasks.state === 'loading' && !tasks.data.length && !needs.length){
     { const $$_notelekS1C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS1C({ target: __sveltets_2_any(), props: {      "rows":4,"height":52,"label":`tasks`,}});}
  } else if (tasks.state === 'unsupported'){
     { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {      "icon":`check`,"title":`Tasks aren’t available yet`,"body":`This daemon doesn’t have the assistant’s task board. Update Otto to track reminders and long jobs here.`,}});}
  } else if (tasks.state === 'error' && !tasks.data.length){
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {        "variant":`compact`,"what":`tasks`,"error":tasks.error || 'No details were reported.',"empty":true,"onretry":() => void assistant.loadTasks(),}});}
  } else if (total === 0){
     { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {      "icon":`check`,"title":`No tasks yet`,"body":`Ask Otto for something that takes a while — “find me a cheaper flight”, “remind me at 5” — and it shows up here while it runs.`,}});}
  }else{
    if(needs.length){
       { svelteHTML.createElement("section", {});
         { svelteHTML.createElement("h2", { "class":`section-title`,});   { svelteHTML.createElement("span", { "class":`count warn`,});needs.length; } }
         { svelteHTML.createElement("div", { "class":`stack`,});
             for(let n of __sveltets_2_ensureArray(needs)){n.id;
             { svelteHTML.createElement("div", {});
                { const $$_draCuoYsdeeN4C = __sveltets_2_ensureComponent(NeedsYouCard); new $$_draCuoYsdeeN4C({ target: __sveltets_2_any(), props: {  "task":n,"compact":true,}});}
              if(n.thread_id && threadTitle(n.thread_id)){
                 { svelteHTML.createElement("button", {   "class":`link from`,"onclick":() => onopenthread(n.thread_id!),});  threadTitle(n.thread_id); }
              }
             }
          }
         }
       }
    }
       for(let col of __sveltets_2_ensureArray(COLUMNS)){col.key;
      if(col.list.length){
         { svelteHTML.createElement("section", {});
           { svelteHTML.createElement("h2", { "class":`section-title`,});col.label;  { svelteHTML.createElement("span", { "class":`count`,});col.list.length; } }
           { svelteHTML.createElement("ul", { "class":`list`,});
               for(let t of __sveltets_2_ensureArray(col.list)){t.id;;__sveltets_2_ensureSnippet(row(t));}
           }
         }
      }
    }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const TasksTab__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type TasksTab__SvelteComponent_ = ReturnType<typeof TasksTab__SvelteComponent_>;
/*Ωignore_endΩ*/export default TasksTab__SvelteComponent_;