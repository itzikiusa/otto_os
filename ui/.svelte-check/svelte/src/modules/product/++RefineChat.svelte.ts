///<reference types="svelte" />
;
import { announceEdge } from '../../lib/announce.svelte';
import { onDestroy, tick, untrack } from 'svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
// RefineChat — displays a single refinement thread transcript and handles
// sending new messages to the agent.  Props: { tid } (the thread id).
// Parent (RefineTab) controls which thread is active.
import Icon from '../../lib/components/Icon.svelte';
import { toastError } from '../../lib/toastError';
import { product } from '../../lib/stores/product.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../lib/loadError';
import { renderMarkdown } from '../../lib/md';
import AgentByline from '../../lib/components/AgentByline.svelte';
import LiveWorkingDot from '../../lib/components/LiveWorkingDot.svelte';
import RelTime from '../../lib/components/RelTime.svelte';
import type { RefinementMessage } from './types';
import { agentProviders, defaultAgentProvider } from '../../lib/providers';

;type $$ComponentProps =  { tid: string };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { tid }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();

  // Provider for this turn's agent. Each refinement turn replays the full thread
  // history in the prompt, so a per-turn provider is self-contained — sourced
  // from the live registry so custom providers (e.g. grok) work.
  let provider = $state(defaultAgentProvider());

  // ── Local state ────────────────────────────────────────────────────────────
  let messages = $state<RefinementMessage[]>([]);
  /** The thread's model alias, when it has one — shown in the agent byline. */
  let threadModel = $state<string | null>(null);
  let messagesEl = $state<HTMLDivElement | null>(null);
  let loadError = $state<string | null>(null);
  let loading = $state(false);

  let inputText = $state('');
  let sending = $state(false);
  let stopping = $state(false);
  let activeTurnId = $state<string | null>(null);
  let stopNotice = $state<string | null>(null);
  let backgroundTurn = $state(false);
  /** "Reload …" in the after-stop bubble: where focus goes once a stop control removes itself. */
  let reloadBtn = $state<HTMLButtonElement | null>(null);
  let composerEl = $state<HTMLTextAreaElement | null>(null);

  let loadSeq = 0;
  let activeId = '';
  let alive = true;
  let inFlightTurnId: string | null = null;
  const drafts = new Map<string, string>();
  const stopRequestedTurns = new Set<string>();
  const BACKGROUND_NOTICE = 'Stopped waiting — the agent turn may still finish in the background.';
  onDestroy(() => { alive = false; ++loadSeq; });

  function newTurnId(): string {
    return typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function'
      ? crypto.randomUUID()
      : `turn-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`;
  }

  // Keep the newest turn in view on initial load, user send, and agent reply.
  $effect(() => {
    void messages.length;
    void sending;
    if (messagesEl) messagesEl.scrollTop = messagesEl.scrollHeight;
  });

  // ── Load / reload when tid changes ────────────────────────────────────────
  $effect(() => {
    // Reactive on tid — re-runs whenever the active thread switches.
    const currentTid = tid;
    backgroundTurn = false;
    stopNotice = null;
    // Untracked: the loader's synchronous prefix reads/writes the composer
    // (`inputText`, `sending`, `stopping`); tracked, the first keystroke after a
    // switch re-ran this effect and reloaded the thread mid-typing.
    untrack(() => void loadThread(currentTid));
  });

  async function loadThread(threadId: string, opts?: { preserveMessages?: RefinementMessage[] }): Promise<void> {
    const my = ++loadSeq;
    const current = () => alive && my === loadSeq && tid === threadId;
    if (activeId !== threadId) {
      // An empty draft is an intentional decision too. While sending or stopping,
      // the cleared composer belongs to that in-flight request, not a new draft.
      if (activeId && !sending && !stopping) drafts.set(activeId, inputText);
      activeId = threadId;
      inputText = drafts.get(threadId) ?? '';
      messages = [];
      sending = false;
      stopping = false;
      activeTurnId = null;
      inFlightTurnId = null;
      stopNotice = null;
      backgroundTurn = false;
    }
    loading = true;
    loadError = null;
    try {
      const detail = await product.getRefinementThread(threadId);
      if (!current()) return;
      messages = detail.messages.length === 0 && opts?.preserveMessages?.length
        ? opts.preserveMessages
        : detail.messages;
      threadModel = detail.thread.model;
    } catch (e) {
      if (current()) loadError = loadErrorText(e);
    } finally {
      if (current()) loading = false;
    }
  }

  // ── Send & Stop ───────────────────────────────────────────────────────────

  async function stopTurn(): Promise<void> {
    if ((!sending && !stopping) || stopping || !activeTurnId) return;
    const target = tid;
    const turnId = activeTurnId;
    const generation = loadSeq;
    const ownsTurn = () => alive && tid === target && loadSeq === generation && activeTurnId === turnId;
    stopRequestedTurns.add(turnId);
    stopping = true;
    stopNotice = null;
    try {
      const res = await product.stopRefinementTurn(target, turnId);
      if (!ownsTurn()) return;
      if (res.stopped) {
        const snapshot = messages;
        sending = false;
        stopping = false;
        activeTurnId = null;
        inFlightTurnId = null;
        backgroundTurn = false;
        stopNotice = 'Turn stopped.';
        await loadThread(target, { preserveMessages: snapshot });
      } else {
        // stopped: false means the turn was not active on the server (not yet admitted or already finished),
        // NOT proof of cancellation. Keep the accepted user bubble and report truthfully.
        stopping = false;
        stopNotice = 'Couldn’t confirm stop — the turn was not active on the server yet (or already finished).';
        if (inFlightTurnId !== turnId) {
          sending = false;
          activeTurnId = null;
        }
      }
    } catch (e) {
      if (!ownsTurn()) return;
      stopping = false;
      if (inFlightTurnId !== turnId) {
        sending = false;
        activeTurnId = null;
      }
      toastError('Couldn’t stop the turn', e);
    }
  }

  function stopWaiting(): void {
    if (!sending && !stopping) return;
    if (activeTurnId) stopRequestedTurns.add(activeTurnId);
    ++loadSeq;
    sending = false;
    stopping = false;
    activeTurnId = null;
    inFlightTurnId = null;
    stopNotice = null;
    backgroundTurn = true;
  }

  /** "Reload …" removes itself: once the reload lands, focus the composer. */
  async function reloadAfterStop(): Promise<void> {
    backgroundTurn = false;
    stopNotice = null;
    await loadThread(tid);
    await tick();
    composerEl?.focus();
  }

  // a11y-6w2-02: the stop notice (in the thinking bubble, or the after-stop
  // bubble once the turn is let go) is spoken ONCE, here at its edge. Neither
  // bubble is a live region — a status bubble around a self-announcing line
  // spoke it twice.
  const sayStop = announceEdge();
  function speakStopNotice(): void {
    sayStop(backgroundTurn ? BACKGROUND_NOTICE : stopNotice);
  }
  $effect(speakStopNotice);

  /** "Stop" / "Stop waiting" remove themselves with the thinking bubble: when
   *  that drops focus to <body>, move it to "Reload …" in the after-stop
   *  bubble. Focus the person put elsewhere is left alone. */
  function rescueStopFocus(): void {
    const lost = !document.activeElement || document.activeElement === document.body;
    if (reloadBtn && lost) reloadBtn.focus();
  }
  $effect(rescueStopFocus);

  async function send(): Promise<void> {
    const body = inputText.trim();
    if (!body || sending || stopping || loading || Boolean(loadError)) return;
    const target = tid;
    const turnId = newTurnId();
    const generation = loadSeq;
    const current = () => alive && tid === target && loadSeq === generation && activeTurnId === turnId;
    drafts.delete(target);

    // Optimistic user bubble (temporary — will be reconciled with server msg).
    const optimisticMsg: RefinementMessage = {
      id: `optimistic-${Date.now()}`,
      thread_id: tid,
      role: 'user',
      body,
      meta_json: null,
      created_at: new Date().toISOString(),
    };
    messages = [...messages, optimisticMsg];
    inputText = '';
    activeTurnId = turnId;
    inFlightTurnId = turnId;
    sending = true;
    stopping = false;
    stopNotice = null;
    backgroundTurn = false;

    try {
      const resp = await product.sendRefinementMessage(tid, body, provider, undefined, turnId);
      if (inFlightTurnId === turnId) inFlightTurnId = null;

      if (!current()) return;
      // Reconcile: replace the optimistic bubble with the real one and append agent reply.
      messages = [
        ...messages.filter((m) => m.id !== optimisticMsg.id),
        resp.user_message,
        resp.agent_message,
      ];
      stopNotice = null;
      sending = false;
      stopping = false;
      activeTurnId = null;
    } catch (e) {
      if (inFlightTurnId === turnId) inFlightTurnId = null;
      if (stopRequestedTurns.has(turnId)) {
        // User requested Stop or Stop waiting on this turn: the user message was
        // already accepted, so never restore it to the composer or drafts map.
        if (current() && !stopping) {
          sending = false;
          activeTurnId = null;
        }
        return;
      }
      // Keep a failed send with its original conversation; never overwrite
      // the draft the person is composing in the newly selected one.
      // A newer draft (including an explicit clear) wins over an old failure.
      if (!drafts.has(target)) drafts.set(target, body);
      if (!current()) return;
      // Roll back the optimistic bubble, restore the typed text, and show an error.
      messages = messages.filter((m) => m.id !== optimisticMsg.id);
      inputText = body;
      sending = false;
      stopping = false;
      activeTurnId = null;
      toastError('Couldn’t send the message', e);
    } finally {
      if (inFlightTurnId === turnId) inFlightTurnId = null;
      if (current() && !stopping) {
        sending = false;
        activeTurnId = null;
      }
    }
  }

  function handleKeydown(e: KeyboardEvent): void {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      void send();
    }
  }

  // ── Helpers ────────────────────────────────────────────────────────────────

  function parseMeta(metaJson: string | null): Record<string, unknown> {
    if (!metaJson) return {};
    try {
      return JSON.parse(metaJson) as Record<string, unknown>;
    } catch {
      return {};
    }
  }

;
async () => {

 { svelteHTML.createElement("div", { "class":`refine-chat`,});
  
   { const $$_div1 = svelteHTML.createElement("div", {  "class":`messages-area`,});messagesEl = $$_div1;
    if((loading || loadError) && messages.length === 0){
       { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {       "what":`this thread`,loading,"error":loadError,"empty":true,"onretry":() => void loadThread(tid),}});}
    } else if (messages.length === 0){
       { const $$_etatSytpmE2C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE2C({ target: __sveltets_2_any(), props: {      "icon":`comment`,"title":`No messages yet`,"body":`Type a message below to start the conversation with the agent.`,}});}
    }else{
      if(loadError){
         { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {       "what":`this thread`,loading,"error":loadError,"onretry":() => void loadThread(tid),}});}
      }
         for(let m of __sveltets_2_ensureArray(messages)){m.id;
        const meta = m.role === 'agent' ? parseMeta(m.meta_json) : {};
        const storyUpdated = Boolean(meta.story_updated);
        const versionNo = meta.version_no != null ? Number(meta.version_no) : null;
         { svelteHTML.createElement("div", {   "class":`bubble-row`,});m.role === 'user';m.role === 'agent';
           { svelteHTML.createElement("div", {   "class":`bubble`,});m.role === 'user';m.role === 'agent';
             { svelteHTML.createElement("div", { "class":`bubble-header`,});
              if(m.role === 'agent'){
                
                 { const $$_enilyBtnegA5C = __sveltets_2_ensureComponent(AgentByline); new $$_enilyBtnegA5C({ target: __sveltets_2_any(), props: {    "model":threadModel,"at":m.created_at,}});}
              }else{
                 { svelteHTML.createElement("span", { "class":`bubble-role`,});  }
                 { const $$_emiTleR5C = __sveltets_2_ensureComponent(RelTime); new $$_emiTleR5C({ target: __sveltets_2_any(), props: {    "iso":m.created_at,"class":`bubble-time`,}});}
              }
             }
            if(m.role === 'agent'){
               { svelteHTML.createElement("div", { "class":`bubble-body md-body`,}); renderMarkdown(m.body); }
            }else{
               { svelteHTML.createElement("div", { "class":`bubble-body plain`,});m.body; }
            }
            if(storyUpdated && versionNo !== null){
               { svelteHTML.createElement("button", {       "class":`story-updated-chip`,"onclick":() => (product.tab = 'rewrite'),"title":`Switch to Rewrite tab to review and apply this suggested draft version`,});
                 { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`edit`,"size":12,}});}   versionNo;  
               }
            }
           }
         }
      }

      
      if(sending || stopping){
         { svelteHTML.createElement("div", { "class":`bubble-row row-agent`,});
           { svelteHTML.createElement("div", { "class":`bubble bubble-agent thinking`,});
             { svelteHTML.createElement("div", { "class":`bubble-header`,}); { const $$_enilyBtnegA5C = __sveltets_2_ensureComponent(AgentByline); new $$_enilyBtnegA5C({ target: __sveltets_2_any(), props: {   provider,"model":threadModel,}});} }
             { svelteHTML.createElement("div", { "class":`thinking-row`,});
               { svelteHTML.createElement("span", { "class":`thinking-dots`,});
                 { const $$_toDgnikroWeviL6C = __sveltets_2_ensureComponent(LiveWorkingDot); new $$_toDgnikroWeviL6C({ target: __sveltets_2_any(), props: {  "label":stopping ? 'Stopping…' : 'Thinking…',}});}
               }
               { svelteHTML.createElement("div", { "class":`thinking-actions`,});
                 { svelteHTML.createElement("button", {           "class":`btn small`,"type":`button`,"onclick":() => void stopTurn(),"disabled":stopping || !activeTurnId,"title":`Cancel this in-flight agent turn on the server`,});
                  stopping ? 'Stopping…' : 'Stop';
                 }
                 { svelteHTML.createElement("button", {         "class":`btn small`,"type":`button`,"onclick":stopWaiting,"title":`Stop waiting in the UI (the agent turn may still finish in the background)`,});
                   
                 }
               }
             }
            if(stopNotice){
              
               { svelteHTML.createElement("p", { "class":`stop-notice`,});stopNotice; }
            }
           }
         }
      } else if (backgroundTurn || stopNotice){
         { svelteHTML.createElement("div", { "class":`bubble-row row-agent`,});
          
           { svelteHTML.createElement("div", { "class":`bubble bubble-agent thinking`,});
             { svelteHTML.createElement("div", { "class":`thinking-row`,});
               { svelteHTML.createElement("span", { "class":`thinking-dots`,});backgroundTurn ? BACKGROUND_NOTICE : stopNotice; }
               { const $$_button5 = svelteHTML.createElement("button", {      "class":`btn small`,"type":`button`,"onclick":() => void reloadAfterStop(),});reloadBtn = $$_button5;
                 
               }
             }
           }
         }
      }
    }
   }

  
   { svelteHTML.createElement("div", { "class":`input-area`,});
     { svelteHTML.createElement("label", { "class":`rc-provider`,});
       { svelteHTML.createElement("span", { "class":`dim`,});  }
       { svelteHTML.createElement("select", {     "bind:value":provider,"aria-label":`Refinement provider`,"disabled":sending || stopping || loading || Boolean(loadError),});/*Ωignore_startΩ*/() => provider = __sveltets_2_any(null);/*Ωignore_endΩ*/
           for(let p of __sveltets_2_ensureArray(agentProviders())){p; { svelteHTML.createElement("option", { "value":p,});p; }}
       }
     }
     { const $$_textarea2 = svelteHTML.createElement("textarea", {                 "dir":`auto`,"aria-label":`Message`,"class":`msg-input`,"placeholder":`Type a message… (Enter to send, Shift+Enter for newline)`,"bind:value":inputText,"onkeydown":handleKeydown,"disabled":sending || stopping || loading || Boolean(loadError),"rows":3,});composerEl = $$_textarea2;/*Ωignore_startΩ*/() => inputText = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
     { svelteHTML.createElement("button", {       "class":`send-btn`,"onclick":() => void send(),"disabled":sending || stopping || loading || Boolean(loadError) || !inputText.trim(),});
      stopping ? 'Stopping…' : sending ? 'Sending…' : 'Send';
     }
   }
 }


};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const RefineChat__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type RefineChat__SvelteComponent_ = ReturnType<typeof RefineChat__SvelteComponent_>;
/*Ωignore_endΩ*/export default RefineChat__SvelteComponent_;