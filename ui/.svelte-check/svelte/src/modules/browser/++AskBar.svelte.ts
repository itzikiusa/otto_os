///<reference types="svelte" />
;
import { plural } from '../../lib/plural';
// Ask bar: one line of input that submits a `/browser/ask` turn into an
// agent session — the page the user is on, the marks on it (fenced
// server-side, same as send-to-session), and the question. The Browser
// page hosts it under its embedded agent dock (AgentDock); the agent-mode
// right panel's v2 Browser tab hosts it standalone, targeting the active
// session. Marks are included by default so "what does the element I
// marked do?" resolves without the agent guessing which element is meant.
import { browser } from '../../lib/stores/browser.svelte';
import { toastError } from '../../lib/toastError';
import { ws } from '../../lib/stores/workspace.svelte';
import { toasts } from '../../lib/toast.svelte';
import Icon from '../../lib/components/Icon.svelte';
;

  interface Props {
    /** The session the question goes to; null disables the bar with a hint. */
    sessionId: string | null;
    /** Hint shown when no session is bound (differs per host). */
    unboundHint?: string;
    /** The bound session's terminal is on screen right above the bar (no
     *  "sent" toast needed — the answer is visibly arriving). */
    terminalVisible?: boolean;
  };function $$render() {

  
  
  
  
  
  
  
  

  
  
  
  
  

  let { sessionId, unboundHint = 'Attach an agent session to ask about this page.', terminalVisible = false }: Props = $props();

  let text = $state('');
  let includeMarks = $state(true);
  let sending = $state(false);
  let inputEl = $state<HTMLTextAreaElement | null>(null);
  // Set right after a mark lands so the placeholder nudges toward it.
  let justMarked = $state(false);

  const url = $derived(browser.activeTab?.url ?? '');
  const marks = $derived(browser.annotations.filter((a) => a.url === url));
  const session = $derived(sessionId ? (ws.getSession(sessionId) ?? null) : null);
  const status = $derived(sessionId ? (ws.statusMap[sessionId] ?? session?.status ?? null) : null);
  const live = $derived(status === 'running' || status === 'working' || status === 'idle');
  const canSend = $derived(!!sessionId && live && !!url && text.trim() !== '' && !sending);

  // A mark just landed from this device — focus the bar and hint at it, so
  // the natural next step ("what is this?") is one keystroke away.
  let lastTick = browser.markTick;
  $effect(() => {
    const tick = browser.markTick;
    if (tick === lastTick) return;
    lastTick = tick;
    if (!sessionId) return;
    justMarked = true;
    inputEl?.focus();
  });
  // Clear the hint once the user starts typing or moves to another page.
  $effect(() => {
    void url;
    if (text !== '') justMarked = false;
  });

  const placeholder = $derived(
    !sessionId
      ? unboundHint
      : !url
        ? 'Open a page first.'
        : !live
          ? 'The attached session is not live — resume it to ask.'
          : justMarked
            ? 'Ask about the element you just marked… (⏎ to send)'
            : 'Ask the agent about this page… (⏎ to send, ⇧⏎ for a new line)',
  );

  async function send(): Promise<void> {
    if (!canSend || !sessionId) return;
    const q = text.trim();
    const ids = includeMarks ? marks.slice(-20).map((a) => a.id) : [];
    sending = true;
    try {
      await browser.ask({ session_id: sessionId, url, text: q, annotation_ids: ids });
      text = '';
      justMarked = false;
      // The answer streams into the agent's terminal; when that isn't on
      // screen (collapsed dock, or the embedded bar) say where it went.
      if (!terminalVisible) toasts.success('Sent to the agent', session?.title ?? undefined);
    } catch (e) {
      toastError('Couldn’t ask the question', e);
    } finally {
      sending = false;
    }
  }

  function onkeydown(e: KeyboardEvent): void {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      void send();
    }
  }

  // Grow with content up to a few lines; the terminal above keeps the rest.
  function autosize(): void {
    if (!inputEl) return;
    inputEl.style.height = 'auto';
    inputEl.style.height = `${Math.min(inputEl.scrollHeight, 120)}px`;
  }
;
async () => {

 { svelteHTML.createElement("div", {  "class":`askbar`,});!sessionId;
   { const $$_textarea1 = svelteHTML.createElement("textarea", {                  "dir":`auto`,"bind:value":text,"rows":1,placeholder,"disabled":!sessionId || !url,"onkeydown":onkeydown,"oninput":autosize,"spellcheck":`false`,"aria-label":`Ask the agent about this page`,});inputEl = $$_textarea1;/*Ωignore_startΩ*/() => text = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
  if(url){
   { svelteHTML.createElement("label", {    "class":`marks-chip`,"title":marks.length === 0 ? 'No marks on this page yet — use Mark passage to add one' : "Include this page's marks in the question so the agent knows which elements you mean",});!includeMarks || marks.length === 0;
     { svelteHTML.createElement("input", {      "type":`checkbox`,"bind:checked":includeMarks,"disabled":marks.length === 0,});/*Ωignore_startΩ*/() => includeMarks = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`target`,"size":12,}});}
     { svelteHTML.createElement("span", {});plural(marks.length, 'mark'); }
   }
  }
   { svelteHTML.createElement("button", {         "class":`send`,"onclick":send,"disabled":!canSend,"title":`Send to the agent (⏎)`,"aria-label":`Send`,});
     { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`send`,"size":13,}});}
   }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const AskBar__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type AskBar__SvelteComponent_ = ReturnType<typeof AskBar__SvelteComponent_>;
/*Ωignore_endΩ*/export default AskBar__SvelteComponent_;