///<reference types="svelte" />
;// The shared session's Chat view inside a room: the same turn rendering as
// the owner's ConversationView, read through the room capability
// (`/room-transcript/…`, never owner auth). Everyone admitted reads it; the
// member driving the terminal also gets a composer (`/room-prompt/…`).
// The transcript is polled — rooms have no owner event stream.

import { onMount, setContext, tick, untrack } from 'svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
import { TurnItem, CONV_CTX, groupTurns, type ConvContext, type RenderItem } from '../agents/conversation';
import { Conversation } from '../../lib/stores/transcript.svelte';
import { openExternal } from '../../lib/external';
import { loadErrorText } from '../../lib/loadError';
import { announceShown } from '../../lib/announce.svelte';
import { pollWhileVisible } from '../../lib/poll';
import type { RoomCredential } from '../../lib/api/room-types';
import type { Transcript, TranscriptProvider } from '../../lib/api/types';
import { fetchRoomTranscript, roomImageUrl, roomPrompt } from './room-client';
;

  interface Props {
    origin: string;
    credential: RoomCredential;
    sessionId: string;
    /** Set while this member drives the terminal (enables the composer). */
    grantEpoch: number | null;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { origin, credential, sessionId, grantEpoch }: Props = $props();

  const POLL_MS = 2500;
  // The room's session never changes under one mount (RoomPage remounts on a
  // new room), so the initial id is the id.
  const sid = untrack(() => sessionId);
  const conv = new Conversation({ sessionId: sid }, () => true, () => {});
  let items = $state<RenderItem[]>([]);
  let error = $state('');
  let loaded = $state(false);
  let unavailable = $state('');
  let draft = $state('');
  let sending = $state(false);
  let sendError = $state('');
  let scroller: HTMLElement | undefined = $state();
  let lastKey = '';

  const ctx: ConvContext = {
    conv,
    sessionId: sid,
    readonly: true,
    provider: 'claude' as TranscriptProvider,
    queuedLive: [],
    openUrl: (url) => void openExternal(url),
    imageUrl: (id) => roomImageUrl(origin, credential, id),
    reusePrompt: null,
  };
  setContext(CONV_CTX, ctx);

  function nearBottom(): boolean {
    if (!scroller) return true;
    return scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 80;
  }
  async function refresh(): Promise<boolean> {
    try {
      const t: Transcript = await fetchRoomTranscript(origin, credential);
      error = '';
      loaded = true;
      unavailable = t.unavailable_reason ? 'The agent’s conversation isn’t available for this session yet.' : '';
      const turns = t.turns ?? [];
      const last = turns[turns.length - 1];
      const key = `${turns.length}:${last?.id ?? ''}:${JSON.stringify(last ?? null).length}`;
      if (key === lastKey) return true;
      lastKey = key;
      ctx.provider = (t.provider ?? 'claude') as TranscriptProvider;
      const follow = nearBottom();
      conv.transcript = t;
      conv.turns = turns;
      items = groupTurns(turns);
      if (follow) { await tick(); scroller?.scrollTo({ top: scroller.scrollHeight }); }
      return true;
    } catch (e) {
      error = loadErrorText(e);
      return false;
    }
  }
  onMount(() => {
    const poller = pollWhileVisible(() => refresh(), { ms: POLL_MS, immediate: true });
    return () => poller.stop();
  });

  async function submit(): Promise<void> {
    const text = draft.trim();
    if (!text || grantEpoch === null) return;
    sending = true; sendError = '';
    try { await roomPrompt(origin, credential, grantEpoch, text, true); draft = ''; setTimeout(() => void refresh(), 800); }
    catch (e) { sendError = loadErrorText(e); }
    finally { sending = false; }
  }
  async function interrupt(): Promise<void> {
    if (grantEpoch === null) return;
    try { await roomPrompt(origin, credential, grantEpoch, '\u001b', false); } catch (e) { sendError = loadErrorText(e); }
  }
  function onKey(e: KeyboardEvent) {
    if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) { e.preventDefault(); void submit(); }
  }
;
async () => {

 { svelteHTML.createElement("section", {   "class":`room-conv`,"aria-label":`Agent conversation`,});
   { const $$_div1 = svelteHTML.createElement("div", {  "class":`turns`,});scroller = $$_div1;
    if(error && !loaded){
       { const $$_etatSytpmE2C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE2C({ target: __sveltets_2_any(), props: {              "variant":`page`,"icon":`comment`,"tone":`error`,"title":`Couldn’t load the conversation`,"body":error,"actionLabel":`Retry`,"onaction":() => void refresh(),}});}
    } else if (!loaded){
       { const $$_notelekS2C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS2C({ target: __sveltets_2_any(), props: {    "rows":4,"label":`the conversation`,}});}
    } else if (unavailable){
       { const $$_etatSytpmE2C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE2C({ target: __sveltets_2_any(), props: {        "variant":`page`,"icon":`comment`,"title":`No conversation yet`,"body":unavailable,}});}
    } else if (!items.length){
       { const $$_etatSytpmE2C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE2C({ target: __sveltets_2_any(), props: {        "variant":`page`,"icon":`comment`,"title":`No messages yet`,"body":`The agent’s turns appear here as the session runs.`,}});}
    }else{
         for(let item of __sveltets_2_ensureArray(items)){item.id; { const $$_metInruT2C = __sveltets_2_ensureComponent(TurnItem); new $$_metInruT2C({ target: __sveltets_2_any(), props: { item,}});}}
    }
    if(error && loaded){ {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('p'),({ text: `Updates paused: ${error}` })));{ svelteHTML.createElement("p", __sveltets_2_union($$action_0), {   "class":`error`,});  error; }}}
   }
  if(grantEpoch !== null){
     { svelteHTML.createElement("form", {   "class":`composer`,"onsubmit":(e) => { e.preventDefault(); void submit(); },});
       { svelteHTML.createElement("textarea", {             "dir":`auto`,"rows":2,"bind:value":draft,"onkeydown":onKey,"placeholder":`Message the agent (Enter to send, Shift+Enter for a new line)`,"aria-label":`Message the agent`,"disabled":sending,});/*Ωignore_startΩ*/() => draft = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
       { svelteHTML.createElement("div", { "class":`actions`,});
         { svelteHTML.createElement("button", {       "type":`button`,"class":`btn small`,"onclick":interrupt,"title":`Interrupt the agent (Esc)`,});  }
         { svelteHTML.createElement("button", {   "class":`btn small primary`,"disabled":sending || !draft.trim(),});sending ? 'Sending…' : 'Send'; }
       }
      if(sendError){ {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('p'),({ text: sendError, assertive: true })));{ svelteHTML.createElement("p", __sveltets_2_union($$action_0), {   "class":`error`,});sendError; }}}
     }
  }else{
     { svelteHTML.createElement("p", { "class":`muted hint`,});         }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const RoomConversation__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type RoomConversation__SvelteComponent_ = ReturnType<typeof RoomConversation__SvelteComponent_>;
/*Ωignore_endΩ*/export default RoomConversation__SvelteComponent_;