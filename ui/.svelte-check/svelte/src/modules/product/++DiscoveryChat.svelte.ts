///<reference types="svelte" />
;
import { announceEdge } from '../../lib/announce.svelte';
import { onDestroy, tick, untrack } from 'svelte';
// DiscoveryChat — the chat pane for one Discovery Chat. A conversational agent
// that works from an EMPTY/Untitled draft to help with early discovery &
// research, and proposes Apply-able action cards.
//
// Forks RefineChat's optimistic-bubble + thinking-indicator pattern, and adds:
//   • an EmptyState + starter-prompt chip row (chips PREFILL the composer,
//     never auto-send) when the chat has zero messages;
//   • ActionCards parsed from each agent message's `actions_json`.
// Props: { cid } (the chat id). Parent (ChatTab) controls which chat is active.
import { product } from '../../lib/stores/product.svelte';
import { toastError } from '../../lib/toastError';
import LoadState from '../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../lib/loadError';
import { viewport } from '../../lib/stores/viewport.svelte';
import { renderMarkdown } from '../../lib/md';
import AgentByline from '../../lib/components/AgentByline.svelte';
import LiveWorkingDot from '../../lib/components/LiveWorkingDot.svelte';
import RelTime from '../../lib/components/RelTime.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import ActionCard from './ActionCard.svelte';
import type { DiscoveryChatMessage, DiscoveryAction } from './types';
import { agentProviders, defaultAgentProvider } from '../../lib/providers';

;type $$ComponentProps =  { cid: string };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { cid }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();

  // Provider for this chat's persistent agent session — chosen BEFORE the first
  // message (the session is created on it and then retained). Sourced from the
  // live registry so custom providers (e.g. grok) work.
  let provider = $state(defaultAgentProvider());

  // ── Local state ────────────────────────────────────────────────────────────
  let messages = $state<DiscoveryChatMessage[]>([]);
  /** The chat's model alias, when it has one — shown in the agent byline. */
  let chatModel = $state<string | null>(null);
  let messagesEl = $state<HTMLDivElement | null>(null);

  // Keep the newest turn in view (the optimistic user bubble bumps the count on
  // send; the agent reply bumps it again on completion).
  $effect(() => {
    void messages.length;
    void sending;
    if (messagesEl) messagesEl.scrollTop = messagesEl.scrollHeight;
  });
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

  // Starter prompts — clicking a chip PREFILLS the composer (does not auto-send).
  const STARTERS = [
    'Help me scope a story for: ',
    'What questions should I answer before building this?',
    'Research how other products solve this problem',
    'What edge cases and failure modes am I missing?',
    'Draft acceptance criteria from what we’ve discussed',
    'Summarize this into a story I can publish',
  ];

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

  // ── Load / reload when cid changes ────────────────────────────────────────
  $effect(() => {
    // Reactive on cid — re-runs whenever the active chat switches.
    const currentCid = cid;
    backgroundTurn = false;
    stopNotice = null;
    // Untracked: the loader's synchronous prefix reads/writes the composer
    // (`inputText`, `sending`, `stopping`); tracked, the first keystroke after a
    // switch re-ran this effect and reloaded the thread mid-typing.
    untrack(() => void loadChat(currentCid));
  });

  async function loadChat(chatId: string, opts?: { preserveMessages?: DiscoveryChatMessage[] }): Promise<void> {
    const my = ++loadSeq;
    const current = () => alive && my === loadSeq && cid === chatId;
    if (activeId !== chatId) {
      // An empty draft is an intentional decision too. While sending or stopping,
      // the cleared composer belongs to that in-flight request, not a new draft.
      if (activeId && !sending && !stopping) drafts.set(activeId, inputText);
      activeId = chatId;
      inputText = drafts.get(chatId) ?? '';
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
      const detail = await product.getDiscoveryChat(chatId);
      if (!current()) return;
      messages = detail.messages.length === 0 && opts?.preserveMessages?.length
        ? opts.preserveMessages
        : detail.messages;
      chatModel = detail.chat.model;
    } catch (e) {
      if (current()) loadError = loadErrorText(e);
    } finally {
      if (current()) loading = false;
    }
  }

  // ── Send & Stop ───────────────────────────────────────────────────────────

  async function stopTurn(): Promise<void> {
    if ((!sending && !stopping) || stopping || !activeTurnId) return;
    const target = cid;
    const turnId = activeTurnId;
    const generation = loadSeq;
    const ownsTurn = () => alive && cid === target && loadSeq === generation && activeTurnId === turnId;
    stopRequestedTurns.add(turnId);
    stopping = true;
    stopNotice = null;
    try {
      const res = await product.stopDiscoveryTurn(target, turnId);
      if (!ownsTurn()) return;
      if (res.stopped) {
        const snapshot = messages;
        sending = false;
        stopping = false;
        activeTurnId = null;
        inFlightTurnId = null;
        backgroundTurn = false;
        stopNotice = 'Turn stopped.';
        await loadChat(target, { preserveMessages: snapshot });
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
    await loadChat(cid);
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
    const target = cid;
    const turnId = newTurnId();
    const generation = loadSeq;
    const current = () => alive && cid === target && loadSeq === generation && activeTurnId === turnId;
    drafts.delete(target);

    // Optimistic user bubble (temporary — reconciled with the server message).
    const optimisticMsg: DiscoveryChatMessage = {
      id: `optimistic-${Date.now()}`,
      chat_id: cid,
      role: 'user',
      body,
      actions_json: null,
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
      // Provider only matters on the FIRST message (session create); pass it
      // while the chat is still empty so a custom provider is honored.
      const resp = await product.sendDiscoveryMessage(
        cid,
        body,
        messages.length <= 1 ? provider : undefined,
        undefined,
        turnId,
      );
      if (inFlightTurnId === turnId) inFlightTurnId = null;
      if (!current()) return;
      // Reconcile: drop the optimistic bubble, append the real user + agent msgs.
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
      // Roll back the optimistic bubble, restore the typed text, show an error.
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

  function useStarter(text: string): void {
    inputText = text;
    // Focus the composer so the PO can keep typing right where the chip left off.
    composerEl?.focus();
    // Move the caret to the end (important for the "Help me scope a story for: "
    // prefix, which expects the PO to continue typing).
    queueMicrotask(() => {
      if (composerEl) {
        const end = composerEl.value.length;
        composerEl.setSelectionRange(end, end);
      }
    });
  }

  // ── Helpers ────────────────────────────────────────────────────────────────

  function parseActions(actionsJson: string | null): DiscoveryAction[] {
    if (!actionsJson) return [];
    try {
      const parsed = JSON.parse(actionsJson);
      return Array.isArray(parsed) ? (parsed as DiscoveryAction[]) : [];
    } catch {
      return [];
    }
  }

;
async () => {

 { svelteHTML.createElement("div", { "class":`discovery-chat`,});
  
   { const $$_div1 = svelteHTML.createElement("div", {  "class":`messages-area`,});messagesEl = $$_div1;
    if((loading || loadError) && messages.length === 0){
       { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {       "what":`this chat`,loading,"error":loadError,"empty":true,"onretry":() => void loadChat(cid),}});}
    } else if (messages.length === 0){
      
       { svelteHTML.createElement("div", { "class":`empty-wrap`,});
         { const $$_etatSytpmE3C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE3C({ target: __sveltets_2_any(), props: {       "icon":`zap`,"title":`Let’s figure out what to build`,"body":`Tell me the rough idea or a problem you’re chasing. I’ll research, ask the right questions, and turn it into a story — no need to write anything first.`,}});}
         { svelteHTML.createElement("div", {   "class":`starters`,});viewport.isPhone;viewport.isPhone;
             for(let s of __sveltets_2_ensureArray(STARTERS)){s;
             { svelteHTML.createElement("button", {     "class":`starter-chip`,"onclick":() => useStarter(s),"title":`Prefill the composer`,});
              s.trim();
             }
          }
         }
         { svelteHTML.createElement("p", { "class":`see-hint`,});         }
       }
    }else{
      if(loadError){
         { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {       "what":`this chat`,loading,"error":loadError,"onretry":() => void loadChat(cid),}});}
      }
         for(let m of __sveltets_2_ensureArray(messages)){m.id;
        const actions = m.role === 'agent' ? parseActions(m.actions_json) : [];
         { svelteHTML.createElement("div", {   "class":`bubble-row`,});m.role === 'user';m.role === 'agent';
           { svelteHTML.createElement("div", {   "class":`bubble`,});m.role === 'user';m.role === 'agent';
             { svelteHTML.createElement("div", { "class":`bubble-header`,});
              if(m.role === 'agent'){
                
                 { const $$_enilyBtnegA5C = __sveltets_2_ensureComponent(AgentByline); new $$_enilyBtnegA5C({ target: __sveltets_2_any(), props: {    "model":chatModel,"at":m.created_at,}});}
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
            if(actions.length > 0){
               { svelteHTML.createElement("div", { "class":`action-cards`,});
                    for(let a of __sveltets_2_ensureArray(actions)){let i = 1;i;
                   { const $$_draCnoitcA5C = __sveltets_2_ensureComponent(ActionCard); new $$_draCnoitcA5C({ target: __sveltets_2_any(), props: {   cid,"action":a,}});}
                }
               }
            }
           }
         }
      }

      
      if(sending || stopping){
         { svelteHTML.createElement("div", { "class":`bubble-row row-agent`,});
           { svelteHTML.createElement("div", { "class":`bubble bubble-agent thinking`,});
             { svelteHTML.createElement("div", { "class":`bubble-header`,}); { const $$_enilyBtnegA5C = __sveltets_2_ensureComponent(AgentByline); new $$_enilyBtnegA5C({ target: __sveltets_2_any(), props: {   provider,"model":chatModel,}});} }
             { svelteHTML.createElement("div", { "class":`thinking-row`,});
               { svelteHTML.createElement("span", { "class":`thinking-dots`,}); { const $$_toDgnikroWeviL6C = __sveltets_2_ensureComponent(LiveWorkingDot); new $$_toDgnikroWeviL6C({ target: __sveltets_2_any(), props: {  "label":stopping ? 'Stopping…' : 'Thinking…',}});} }
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
    if(messages.length === 0){
      
       { svelteHTML.createElement("label", { "class":`dc-provider`,});
         { svelteHTML.createElement("span", { "class":`dim`,});  }
         { svelteHTML.createElement("select", {     "bind:value":provider,"aria-label":`Chat provider`,"disabled":sending || stopping || loading || Boolean(loadError),});/*Ωignore_startΩ*/() => provider = __sveltets_2_any(null);/*Ωignore_endΩ*/
             for(let p of __sveltets_2_ensureArray(agentProviders())){p; { svelteHTML.createElement("option", { "value":p,});p; }}
         }
       }
    }
     { const $$_textarea2 = svelteHTML.createElement("textarea", {                 "dir":`auto`,"aria-label":`Message`,"class":`msg-input`,"placeholder":`Describe the idea or problem… (Enter to send, Shift+Enter for newline)`,"bind:value":inputText,"onkeydown":handleKeydown,"disabled":sending || stopping || loading || Boolean(loadError),"rows":3,});composerEl = $$_textarea2;/*Ωignore_startΩ*/() => inputText = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
     { svelteHTML.createElement("button", {       "class":`send-btn`,"onclick":() => void send(),"disabled":sending || stopping || loading || Boolean(loadError) || !inputText.trim(),});
      stopping ? 'Stopping…' : sending ? 'Sending…' : 'Send';
     }
   }
 }


};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const DiscoveryChat__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type DiscoveryChat__SvelteComponent_ = ReturnType<typeof DiscoveryChat__SvelteComponent_>;
/*Ωignore_endΩ*/export default DiscoveryChat__SvelteComponent_;