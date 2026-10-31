///<reference types="svelte" />
;// The agent's plan (task-list state AFTER a TodoWrite / TaskCreate /
// TaskUpdate call) as a checklist in the flow: "Plan · 1 of 4 done" + items
// with a status mark. Earlier snapshots of the same response start folded to
// their header line (the latest one is the plan that matters).

import { untrack } from 'svelte';
import Icon from '../../../lib/components/Icon.svelte';
import type { TaskItem } from '../../../lib/api/types';
;

  interface Props {
    tasks: TaskItem[];
    /** Start folded to the header line. */
    folded?: boolean;
  };function $$render() {

  
  
  
  
  
  
  

  let { tasks, folded = false }: Props = $props();
  const done = $derived(tasks.filter((t) => t.status === 'completed').length);
  const current = $derived(tasks.find((t) => t.status === 'in_progress') ?? null);
  let open = $state(untrack(() => !folded));
;
async () => {

 { svelteHTML.createElement("div", {   "class":`tasks`,...__sveltets_2_empty({"data-total":tasks.length}),});open;
   { svelteHTML.createElement("button", {     "class":`tasks-head`,"onclick":() => (open = !open),"aria-expanded":open,});
     { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}
     { svelteHTML.createElement("span", { "class":`tasks-title`,});  }
     { svelteHTML.createElement("span", { "class":`tasks-count`,});done;  tasks.length;  }
    if(!open && current){ { svelteHTML.createElement("span", { "class":`tasks-now`,}); current.active_form || current.title; }}
     { svelteHTML.createElement("span", {   "class":`tasks-caret`,"aria-hidden":`true`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":open ? 'chevronDown' : 'chevronRight',"size":12,}});} }
   }
  if(open){
     { svelteHTML.createElement("ul", {});
          for(let t of __sveltets_2_ensureArray(tasks)){let i = 1;t.ext_id ?? `${i}:${t.title}`;
         { svelteHTML.createElement("li", { "class":t.status,});
           { svelteHTML.createElement("span", {   "class":`mark`,"aria-hidden":`true`,});
            if(t.status === 'completed'){ { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}}else{ { svelteHTML.createElement("span", { "class":`ring`,}); }}
           }
           { svelteHTML.createElement("span", { "class":`ttl`,});t.status === 'in_progress' && t.active_form ? t.active_form : t.title; }
           { svelteHTML.createElement("span", { "class":`sr-only`,}); t.status === 'completed' ? 'done' : t.status === 'in_progress' ? 'in progress' : 'to do'; }
         }
      }
     }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const TasksBlock__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type TasksBlock__SvelteComponent_ = ReturnType<typeof TasksBlock__SvelteComponent_>;
/*Ωignore_endΩ*/export default TasksBlock__SvelteComponent_;