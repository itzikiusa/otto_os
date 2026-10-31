///<reference types="svelte" />
;// Memory inspector: what the agent has learned, item by item, with where it
// learned it (chat / Slack / Telegram / vault / run). Edit or forget one
// item; the raw notes file stays editable below (AgentDocuments).

import { untrack } from 'svelte';
import { personalAgentsApi } from '../../lib/api/personalAgents';
import { confirmer } from '../../lib/confirm.svelte';
import { toasts } from '../../lib/toast.svelte';
import { loadErrorText } from '../../lib/loadError';
import Icon from '../../lib/components/Icon.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import type {
    PersonalAgentMemories,
    PersonalAgentMemoryItem,
    PersonalAgentMemorySource,
  } from '../../lib/api/types';
;

  interface Props {
    agentId: string;
    editable: boolean;
    /** Bumped by the parent after the raw notes are saved / the agent is reset. */
    reloadKey?: number;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  

  let { agentId, editable, reloadKey = 0 }: Props = $props();

  const SOURCE_LABEL: Record<PersonalAgentMemorySource, string> = {
    chat: 'Chat',
    slack: 'Slack',
    telegram: 'Telegram',
    vault: 'Vault',
    run: 'Run',
    user: 'You',
    notes: 'Notes',
  };

  let data = $state<PersonalAgentMemories | null>(null);
  let error = $state<string | null>(null);
  let loading = $state(true);
  let filter = $state<PersonalAgentMemorySource | 'all'>('all');
  let editingLine = $state<number | null>(null);
  let draft = $state('');
  let busy = $state(false);

  async function load(): Promise<void> {
    loading = true;
    try {
      data = await personalAgentsApi.memories(agentId);
      error = null;
    } catch (e) {
      error = loadErrorText(e);
    } finally {
      loading = false;
    }
  }
  $effect(() => {
    void agentId;
    void reloadKey;
    untrack(() => void load());
  });

  const sources = $derived(
    [...new Set((data?.items ?? []).map((i) => i.source))] as PersonalAgentMemorySource[],
  );
  const shown = $derived((data?.items ?? []).filter((i) => filter === 'all' || i.source === filter));

  async function submit(item: PersonalAgentMemoryItem, text: string | null): Promise<void> {
    if (!data) return;
    busy = true;
    try {
      data = await personalAgentsApi.editMemory(agentId, {
        version: data.version,
        line: item.line,
        raw: item.raw,
        text,
      });
      editingLine = null;
    } catch (e) {
      toasts.error(text === null ? 'Couldn’t forget that memory' : 'Couldn’t save that memory', loadErrorText(e));
      void load();
    } finally {
      busy = false;
    }
  }

  async function forget(item: PersonalAgentMemoryItem): Promise<void> {
    const ok = await confirmer.ask(`Forget “${item.text}”? The agent won’t see it in its next run or chat.`, {
      title: 'Forget memory',
      confirmLabel: 'Forget',
      danger: true,
    });
    if (ok) await submit(item, null);
  }
;
async () => {

 { svelteHTML.createElement("section", {   "class":`pa-panel`,"aria-labelledby":`mem-h`,});
   { svelteHTML.createElement("div", { "class":`card-head`,});
     { svelteHTML.createElement("h2", { "id":`mem-h`,});    }
     { svelteHTML.createElement("button", {       "class":`icon-btn`,"aria-label":`Refresh memories`,"title":`Refresh memories`,"onclick":() => void load(),}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":14,}});} }
   }
   { svelteHTML.createElement("p", { "class":`hint`,});                    }
   { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL1 = new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {           children:() => { return __sveltets_2_any(0); },"what":`this agent’s memories`,"loading":loading && !data,"error":data ? null : error,"empty":(data?.items.length ?? 0) === 0,"rows":3,"onretry":() => void load(),emptyView:() => { async ()/*Ωignore_positionΩ*/ => {
       { const $$_etatSytpmE0C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE0C({ target: __sveltets_2_any(), props: {      "icon":`bulb`,"title":`Nothing learned yet`,"body":`Items appear here as the agent writes bullets to its memory during runs and chats.`,}});}
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL1.$$prop_def;/*Ωignore_endΩ*/
    
    if(sources.length > 1){
       { svelteHTML.createElement("div", {     "class":`filters`,"role":`group`,"aria-label":`Filter by source`,});
         { svelteHTML.createElement("button", {      "class":`chip pa-filter`,"aria-pressed":filter === 'all',"onclick":() => (filter = 'all'),});filter === 'all';  }
           for(let s of __sveltets_2_ensureArray(sources)){s;
           { svelteHTML.createElement("button", {      "class":`chip pa-filter`,"aria-pressed":filter === s,"onclick":() => (filter = s),});filter === s;SOURCE_LABEL[s]; }
        }
       }
    }
     { svelteHTML.createElement("ul", { "class":`list`,});
         for(let item of __sveltets_2_ensureArray(shown)){item.line + ':' + item.raw;
         { svelteHTML.createElement("li", { "class":`item`,});
           { svelteHTML.createElement("span", {   "class":`chip src`,"title":`Learned from`,});SOURCE_LABEL[item.source]; }
          if(editingLine === item.line){
             { svelteHTML.createElement("input", {            "dir":`auto`,"class":`grow`,"bind:value":draft,"aria-label":`Memory text`,"disabled":busy,"onkeydown":(e) => { if (e.key === 'Enter') void submit(item, draft); if (e.key === 'Escape') editingLine = null; },});/*Ωignore_startΩ*/() => draft = __sveltets_2_any(null);/*Ωignore_endΩ*/}
             { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":busy,"onclick":() => (editingLine = null),});  }
             { svelteHTML.createElement("button", {     "class":`btn small primary`,"disabled":busy || !draft.trim(),"onclick":() => void submit(item, draft),});  }
          }else{
             { svelteHTML.createElement("span", { "class":`grow text`,});item.text; }
            if(item.section){ { svelteHTML.createElement("span", { "class":`meta`,});item.section; }}
            if(editable){
               { svelteHTML.createElement("button", {       "class":`icon-btn`,"aria-label":`Edit memory`,"title":`Edit`,"onclick":() => { editingLine = item.line; draft = item.text; },}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`edit`,"size":13,}});} }
               { svelteHTML.createElement("button", {       "class":`icon-btn`,"aria-label":`Forget memory`,"title":`Forget`,"onclick":() => void forget(item),}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":13,}});} }
            }
          }
         }
      }
     }
   LoadState}
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const AgentMemoryInspector__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type AgentMemoryInspector__SvelteComponent_ = ReturnType<typeof AgentMemoryInspector__SvelteComponent_>;
/*Ωignore_endΩ*/export default AgentMemoryInspector__SvelteComponent_;