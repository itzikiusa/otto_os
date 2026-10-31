///<reference types="svelte" />
;// Quiet one-line entries in the thread: a delegation to a Personal Agent
// ("Asked Daily Recap to …"), a provider switch, a limit notice, or any
// other system line the daemon posted. Attributed, never louder than chat.

import Icon from '../../../lib/components/Icon.svelte';
import { announceShown } from '../../../lib/announce.svelte';
import ProviderIcon from '../../../lib/components/ProviderIcon.svelte';
import Badge from '../../../lib/components/Badge.svelte';
import { router } from '../../../lib/router.svelte';
import { taskStateLabel, taskTone } from '../../../lib/assistant/model';
import type { AssistantTask, AssistantTurn } from '../../../lib/api/types';
;

  interface Props {
    turn: AssistantTurn | null;
    /** The delegation task, when this line is one. */
    task?: AssistantTask | null;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  

  let { turn, task = null }: Props = $props();

  const to = $derived(typeof turn?.data?.to === 'string' ? (turn.data.to as string) : turn?.provider ?? null);
  const agentName = $derived(typeof task?.result?.agent_name === 'string' ? (task.result.agent_name as string) : null);
;
async () => {

if(task?.kind === 'delegation' || turn?.kind === 'delegation'){
   { svelteHTML.createElement("p", {   "class":`line`,...__sveltets_2_empty({"data-testid":`line-delegation`}),});
     { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`user`,"size":12,}});}
     { svelteHTML.createElement("span", { "class":`what`,});turn?.text || `Asked ${agentName ?? 'a Personal Agent'}: ${task?.title ?? ''}`; }
    if(task?.agent_id){
       { svelteHTML.createElement("button", {   "class":`link`,"onclick":() => router.go(`personal-agents/${task.agent_id}`),});  }
    }
    if(task){ { const $$_egdaB1C = __sveltets_2_ensureComponent(Badge); new $$_egdaB1C({ target: __sveltets_2_any(), props: {      "tone":taskTone(task),"label":taskStateLabel(task),"live":task.state === 'running',}});}}
   }
} else if (turn?.kind === 'route'){
   { svelteHTML.createElement("p", {   "class":`line center`,...__sveltets_2_empty({"data-testid":`line-route`}),});
    if(to){ { const $$_nocIredivorP1C = __sveltets_2_ensureComponent(ProviderIcon); new $$_nocIredivorP1C({ target: __sveltets_2_any(), props: {    "provider":to,"size":12,}});}}
     { svelteHTML.createElement("span", { "class":`what`,});turn.text; }
   }
} else if (turn?.kind === 'limit'){
  
   {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('p'),(turn.text)));{ svelteHTML.createElement("p", __sveltets_2_union($$action_0), {     "class":`line limit`,...__sveltets_2_empty({"data-testid":`line-limit`}),});
     { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});}
     { svelteHTML.createElement("span", { "class":`what`,});turn.text; }
   }}
} else if (turn){
   { svelteHTML.createElement("p", { "class":`line center`,}); { svelteHTML.createElement("span", { "class":`what`,});turn.text; } }
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const SystemLine__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type SystemLine__SvelteComponent_ = ReturnType<typeof SystemLine__SvelteComponent_>;
/*Ωignore_endΩ*/export default SystemLine__SvelteComponent_;