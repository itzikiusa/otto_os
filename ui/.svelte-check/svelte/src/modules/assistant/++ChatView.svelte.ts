///<reference types="svelte" />
;// The Chat tab: the thread as a conversation, NOT a terminal. Messages are
// rendered by the Conversation view's own pieces (TurnItem + the transcript
// store under the same CONV_CTX): the thread's turn index is the backbone
// (it spans provider hand-offs), and replies of the current CLI session come
// from its transcript with their tool steps and images (mergeWithTranscript).
// Every action the assistant takes sits between the messages as a card
// (threadCards → buildTimeline); each reply carries who wrote it.

import { setContext, tick, untrack } from 'svelte';
import Badge from '../../lib/components/Badge.svelte';
import Icon from '../../lib/components/Icon.svelte';
import AgentChip from '../../lib/components/AgentChip.svelte';
import ProviderIcon from '../../lib/components/ProviderIcon.svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import LiveWorkingDot from '../../lib/components/LiveWorkingDot.svelte';
import TurnItem from '../agents/conversation/TurnItem.svelte';
import { stableGroupTurns, type RenderItem } from '../agents/conversation/format';
import { CONV_CTX, type ConvContext } from '../agents/conversation/context';
import { transcript } from '../../lib/stores/transcript.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { assistant } from '../../lib/stores/assistant.svelte';
import { buildTimeline, providerLabel, threadCards, type ChatCard } from '../../lib/assistant/model';
import { mergeWithTranscript, messageAuthor, type ChatMessage } from './chat';
import { clock } from './format';
import AssistantComposer from './AssistantComposer.svelte';
import MemoryChips from './cards/MemoryChips.svelte';
import TaskEntry from './cards/TaskEntry.svelte';
import SystemLine from './cards/SystemLine.svelte';
import type { AssistantThread } from '../../lib/api/types';
import LoadState from '../../lib/components/LoadState.svelte';
;

  interface Props {
    thread: AssistantThread;
    onopentasks: () => void;
    onopenmemory: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { thread, onopentasks, onopenmemory }: Props = $props();

  // ── data ───────────────────────────────────────────────────────────────────
  // Metadata updates keep the same acquisition; only a new thread releases it.
  const threadId = $derived(thread.id);
  $effect(() => {
    const id = threadId;
    return untrack(() => assistant.acquireTurns(id));
  });
  $effect(() => {
    if (untrack(() => assistant.tasks.state) === 'idle') void assistant.loadTasks();
  });
  const index = $derived(assistant.turns[thread.id]);
  const turns = $derived(index?.data ?? []);

  // The transcript of the thread's current CLI session (when there is one).
  const src = $derived(thread.session_id ? { sessionId: thread.session_id } : null);
  const conv = $derived(src ? transcript.conversation(src) : null);
  $effect(() => {
    const s = src;
    if (!s) return;
    void conv;
    return untrack(() => transcript.acquireView(s));
  });

  // Context for the Conversation-view pieces (images, tool steps, subagents).
  const ctx: ConvContext = $state(
    untrack(() => ({
      conv: transcript.conversation(src ?? { sessionId: thread.id }),
      sessionId: thread.session_id,
      readonly: true,
      provider: thread.provider === 'codex' ? ('codex' as const) : ('claude' as const),
      queuedLive: [],
    })),
  );
  setContext(CONV_CTX, ctx);
  $effect(() => {
    if (conv) ctx.conv = conv;
    ctx.sessionId = thread.session_id;
    ctx.provider = conv?.transcript?.provider ?? (thread.provider === 'codex' ? 'codex' : 'claude');
  });

  // Stable items (format.ts `stableGroupTurns`): only the reply that changed
  // gets a new object, so a live delta doesn't re-render every message.
  const liveCache = new Map<string, RenderItem>();
  const live = $derived(conv ? stableGroupTurns(conv.turns, liveCache) : []);
  const messages = $derived(mergeWithTranscript(turns, live, thread.session_id));
  const cards = $derived(threadCards(turns, assistant.tasks.data, thread.id));
  const timeline = $derived(
    buildTimeline(
      messages.map((m) => ({ id: m.item.id, role: m.item.role, ts: m.item.ts, m })),
      cards,
    ),
  );
  const pending = $derived(assistant.pending[thread.id] ?? []);

  const sessionStatus = $derived(thread.session_id ? (ws.statusMap[thread.session_id] ?? null) : null);
  const working = $derived(thread.status === 'working' || sessionStatus === 'working' || pending.length > 0);
  // Screen-reader announcements on the edges only ("working" → "replied"),
  // through the always-mounted region at the end of the view. A thread switch
  // resets the edge so opening an idle thread doesn't announce a reply.
  let announce = $state('');
  let wasWorking = false;
  let announcedFor = '';
  $effect(() => {
    const w = working;
    const id = thread.id;
    if (id !== announcedFor) {
      announcedFor = id;
      wasWorking = w;
      announce = '';
      return;
    }
    if (wasWorking && !w) announce = 'Otto replied';
    else if (!wasWorking && w) announce = 'Otto is working';
    wasWorking = w;
  });

  const loading = $derived(!index || (index.state === 'loading' && !index.data.length));
  const failed = $derived(index?.state === 'error' && !index.data.length);
  const empty = $derived(!loading && !failed && timeline.length === 0 && pending.length === 0);

  function author(m: ChatMessage): { provider: string; label: string } {
    const a = messageAuthor(m, thread);
    return { provider: a.provider, label: providerLabel(a.provider, a.model) };
  }
  function cardKey(c: ChatCard): string {
    return `c:${c.id}`;
  }

  // ── scroll: follow the tail unless the reader scrolled up ──────────────────
  let listEl = $state<HTMLDivElement | null>(null);
  let atBottom = $state(true);
  function onScroll(): void {
    const el = listEl;
    if (el) atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 48;
  }
  function toBottom(): void {
    const el = listEl;
    if (!el) return;
    el.scrollTop = el.scrollHeight;
    atBottom = true;
  }
  let paintedFor = '';
  $effect(() => {
    const n = timeline.length + pending.length + (working ? 1 : 0);
    if (loading) return;
    const first = paintedFor !== thread.id;
    paintedFor = thread.id;
    if (first || untrack(() => atBottom)) void tick().then(toBottom);
    void n;
  });
;
async () => {

 { svelteHTML.createElement("div", { "class":`chat`,});
   { const $$_div1 = svelteHTML.createElement("div", {      "class":`scroll`,"onscroll":onScroll,...__sveltets_2_empty({"data-testid":`assistant-thread`}),});listEl = $$_div1;
     { svelteHTML.createElement("div", { "class":`col`,});
      if(loading){
         { svelteHTML.createElement("div", { "class":`state`,});
           { const $$_notelekS4C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS4C({ target: __sveltets_2_any(), props: {      "rows":4,"height":44,"label":`the conversation`,}});}
         }
      } else if (failed){
         { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {        "variant":`compact`,"what":`this conversation`,"error":index?.error || 'No details were reported.',"empty":true,"onretry":() => void assistant.loadTurns(thread.id),}});}
      } else if (empty){
         { const $$_etatSytpmE3C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE3C({ target: __sveltets_2_any(), props: {      "icon":`assistant`,"title":`Nothing here yet`,"body":`Ask Otto anything — plan, research, remember, remind. Every action shows up here, and it asks before anything leaves your Mac.`,}});}
      }else{
           for(let entry of __sveltets_2_ensureArray(timeline)){entry.kind === 'turn' ? `t:${entry.turn.id}` : cardKey(entry.card);
          if(entry.kind === 'turn'){
            const m = entry.turn.m;
            const item = m.item;
            
            if(m.item.role === 'assistant'){
              const w = author(m);
               { svelteHTML.createElement("div", {   "class":`msg agent`,...__sveltets_2_empty({"data-testid":`assistant-message`}),});
                 { svelteHTML.createElement("div", { "class":`who`,});
                   { svelteHTML.createElement("span", {   "class":`avatar`,"aria-hidden":`true`,}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`assistant`,"size":12,}});} }
                   { svelteHTML.createElement("strong", {});  }
                   { const $$_pihCtnegA5C = __sveltets_2_ensureComponent(AgentChip); new $$_pihCtnegA5C({ target: __sveltets_2_any(), props: {}});}
                   { const $$_egdaB5C = __sveltets_2_ensureComponent(Badge); new $$_egdaB5C({ target: __sveltets_2_any(), props: {   children:() => { return __sveltets_2_any(0); },"testid":`provider-badge`,"title":`Written by ${w.label}`,}}); { const $$_nocIredivorP6C = __sveltets_2_ensureComponent(ProviderIcon); new $$_nocIredivorP6C({ target: __sveltets_2_any(), props: {    "provider":w.provider,"size":12,}});}w.label; Badge}
                  if(m.item.ts){ { svelteHTML.createElement("span", { "class":`dim`,});  { svelteHTML.createElement("time", {   "datetime":m.item.ts,"title":new Date(m.item.ts).toLocaleString(),});clock(m.item.ts); } }}
                 }
                 { const $$_spihCyromeM4C = __sveltets_2_ensureComponent(MemoryChips); new $$_spihCyromeM4C({ target: __sveltets_2_any(), props: {    "cards":entry.memory,"onreview":onopenmemory,}});}
                 { const $$_metInruT4C = __sveltets_2_ensureComponent(TurnItem); new $$_metInruT4C({ target: __sveltets_2_any(), props: { item,}});}
               }
            }else{
               { svelteHTML.createElement("div", { "class":`msg user`,});
                 { const $$_metInruT4C = __sveltets_2_ensureComponent(TurnItem); new $$_metInruT4C({ target: __sveltets_2_any(), props: { item,}});}
                if(m.turn?.attachments.length){
                   { svelteHTML.createElement("ul", {   "class":`atts`,"aria-label":`Attachments`,});
                       for(let a of __sveltets_2_ensureArray(m.turn.attachments)){a.id;
                       { svelteHTML.createElement("li", {   "class":`att`,"title":a.path,}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`file`,"size":12,}});}a.name; }
                    }
                   }
                }
               }
            }
          }else{
            const c = entry.card;
             { svelteHTML.createElement("div", { "class":`entry`,});
              if(c.kind === 'memory'){
                 { const $$_spihCyromeM4C = __sveltets_2_ensureComponent(MemoryChips); new $$_spihCyromeM4C({ target: __sveltets_2_any(), props: {    "cards":[c],"onreview":onopenmemory,}});}
              } else if (c.kind === 'task'){
                 { const $$_yrtnEksaT4C = __sveltets_2_ensureComponent(TaskEntry); new $$_yrtnEksaT4C({ target: __sveltets_2_any(), props: {     "taskId":c.task_id,"turn":c.turn,onopentasks,}});}
              }else{
                 { const $$_eniLmetsyS4C = __sveltets_2_ensureComponent(SystemLine); new $$_eniLmetsyS4C({ target: __sveltets_2_any(), props: {  "turn":c.turn,}});}
              }
             }
          }
        }
           for(let p of __sveltets_2_ensureArray(pending)){p.id;
           { svelteHTML.createElement("div", {   "class":`pending-bubble`,...__sveltets_2_empty({"data-testid":`pending-turn`}),});
             { svelteHTML.createElement("div", { "class":`bubble-text chat-bubble-end`,});p.text; }
             { svelteHTML.createElement("span", { "class":`dim`,});  }
           }
        }
        if(working){
           { svelteHTML.createElement("p", { "class":`working`,});
             { const $$_toDgnikroWeviL4C = __sveltets_2_ensureComponent(LiveWorkingDot); new $$_toDgnikroWeviL4C({ target: __sveltets_2_any(), props: {    "size":7,"label":`Otto is working…`,}});}
           }
        }
      }
     }
   }
   { svelteHTML.createElement("div", {     "class":`sr-only`,"aria-live":`polite`,"aria-atomic":`true`,});announce; }
  if(!atBottom && !loading){
     { svelteHTML.createElement("button", {   "class":`jump btn small`,"onclick":toBottom,}); { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`arrowDown`,"size":12,}});}    }
  }
   { const $$_resopmoCtnatsissA1C = __sveltets_2_ensureComponent(AssistantComposer); new $$_resopmoCtnatsissA1C({ target: __sveltets_2_any(), props: {   thread,"busy":thread.status === 'working',}});}
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ChatView__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ChatView__SvelteComponent_ = ReturnType<typeof ChatView__SvelteComponent_>;
/*Ωignore_endΩ*/export default ChatView__SvelteComponent_;