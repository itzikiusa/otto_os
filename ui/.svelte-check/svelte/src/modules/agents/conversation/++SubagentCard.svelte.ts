///<reference types="svelte" />
;// Nested subagent card: header from the parent's `subagent` block (or a
// `subagents[]` meta row for grandchildren), body fetched lazily via `?sub=`
// on first expand and rendered with the same turn renderer. Children come
// from `Transcript.subagents[]` (parent_agent_id), never from the body.

import { getContext, untrack } from 'svelte';
import Skeleton from '../../../lib/components/Skeleton.svelte';
import Icon from '../../../lib/components/Icon.svelte';
import TurnItem from './TurnItem.svelte';
import SubagentCard from './SubagentCard.svelte';
import { groupTurns } from './format';
import type { SubagentMeta, SubagentStatus } from '../../../lib/api/types';
import { CONV_CTX, type ConvContext } from './context';
;

  interface Props {
    agentId: string;
    description: string;
    agentType: string;
    status: SubagentStatus | null;
    depth?: number;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  

  let { agentId, description, agentType, status, depth = 0 }: Props = $props();
  const ctx = getContext<ConvContext>(CONV_CTX);

  let open = $state(false);
  const STATUS_LABEL = { running: 'Running…', done: 'Done', error: 'Failed', unknown: 'Status not recorded' } as const;
  const meta = $derived<SubagentMeta | null>(
    ctx.conv.transcript?.subagents.find((s) => s.agent_id === agentId) ?? null,
  );
  const children = $derived(
    (ctx.conv.transcript?.subagents ?? []).filter((s) => s.parent_agent_id === agentId),
  );
  const body = $derived(ctx.conv.subagents[agentId] ?? null);
  const items = $derived(groupTurns(body?.turns ?? []));

  $effect(() => {
    const conv = ctx.conv, id = agentId;
    if (!open) return;
    return untrack(() => {
      const release = conv.acquireSubagent(id);
      void conv.loadSubagent(id);
      return release;
    });
  });

  function toggle(): void {
    open = !open;
  }
;
async () => {

 { svelteHTML.createElement("div", {       "class":`sub`,"style":`--depth:${depth}`,...__sveltets_2_empty({"data-agent":agentId}),...__sveltets_2_empty({"data-status":status ?? ''}),});open;
   { svelteHTML.createElement("button", {     "class":`sub-head`,"onclick":toggle,"aria-expanded":open,});
     { svelteHTML.createElement("span", { "class":`sub-icon`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`radar`,"size":13,}});} }
     { svelteHTML.createElement("span", { "class":`sub-title`,});
       { svelteHTML.createElement("span", { "class":`sub-type`,});agentType || meta?.agent_type || 'agent'; }
       { svelteHTML.createElement("span", { "class":`sub-desc`,});description || meta?.description || agentId; }
     }
    if(meta?.model){ { svelteHTML.createElement("span", { "class":`chip sub-model`,});meta.model; }}
    if(children.length){ { svelteHTML.createElement("span", {   "class":`chip sub-kids`,"title":`${children.length} nested agents`,});children.length;  }}
     { svelteHTML.createElement("span", {       "class":`sub-status ${status ?? 'unknown'}`,"role":`img`,"aria-label":STATUS_LABEL[status ?? 'unknown'],"title":STATUS_LABEL[status ?? 'unknown'],});
      if(status === 'running'){ { svelteHTML.createElement("span", {    "class":`spinner`,"aria-hidden":`true`,});__sveltets_2_ensureType(String, Number, " "); }
      } else if (status === 'done'){ { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}
      } else if (status === 'error'){ { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});}
      }else{ { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`minus`,"size":12,}});}}
     }
     { svelteHTML.createElement("span", {   "class":`sub-caret`,"aria-hidden":`true`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":open ? 'chevronDown' : 'chevronRight',"size":12,}});} }
   }
  if(open){
     { svelteHTML.createElement("div", { "class":`sub-body`,});
      if(body?.loading && !body.turns.length){
         { svelteHTML.createElement("div", { "class":`sub-note`,}); { const $$_notelekS3C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS3C({ target: __sveltets_2_any(), props: {      "rows":2,"height":18,"label":`the subagent transcript`,}});} }
      }else{
        if(body?.error){
           { svelteHTML.createElement("div", { "class":`sub-note err`,});  body.error; }
          if(body.retryable){ { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":() => ctx.conv.loadSubagent(agentId),});  }}
        }
        if(!items.length && !body?.error){
           { svelteHTML.createElement("div", { "class":`dim sub-note`,});   }
        }else{
          if(body?.has_earlier){
             { svelteHTML.createElement("button", {     "class":`btn small ghost`,"disabled":body.loading,"onclick":() => ctx.conv.loadSubagentEarlier(agentId),});
              body.loading ? 'Loading earlier turns…' : 'Load earlier';
             }
          }
             for(let item of __sveltets_2_ensureArray(items)){item.id;
             { const $$_metInruT2C = __sveltets_2_ensureComponent(TurnItem); new $$_metInruT2C({ target: __sveltets_2_any(), props: {   item,"nested":true,"active":status === 'running',}});}
          }
          if(body?.has_later){
             { svelteHTML.createElement("button", {     "class":`btn small ghost`,"disabled":body.loading,"onclick":() => ctx.conv.loadSubagentLater(agentId),});  }
          }
        }
      }
      if(children.length){
         { svelteHTML.createElement("div", { "class":`sub-children`,});
             for(let c of __sveltets_2_ensureArray(children)){c.agent_id;
             { const $$_draCtnegabuS3C = __sveltets_2_ensureComponent(SubagentCard); new $$_draCtnegabuS3C({ target: __sveltets_2_any(), props: {          "agentId":c.agent_id,"description":c.description,"agentType":c.agent_type,"status":null,"depth":depth + 1,}});}
          }
         }
      }
     }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const SubagentCard__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type SubagentCard__SvelteComponent_ = ReturnType<typeof SubagentCard__SvelteComponent_>;
/*Ωignore_endΩ*/export default SubagentCard__SvelteComponent_;