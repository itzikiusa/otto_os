///<reference types="svelte" />
;
import { tick, untrack } from 'svelte';
import type { RoomMessage, RoomAction } from '../../lib/api/room-types';
import Icon from '../../lib/components/Icon.svelte';

;type $$ComponentProps =  {messages: RoomMessage[]; send: (action: RoomAction) => boolean; connected: boolean};function $$render() {

  
  
  
  let { messages, send, connected }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();
  let draft = $state('');
  let pending = $state<{text: string; nonce: string} | null>(null);
  const delivered = $derived(pending && messages.some(m => m.nonce === pending?.nonce));
  const bytes = $derived(new TextEncoder().encode(draft).length);
  let formEl = $state<HTMLFormElement | null>(null);
  let inputEl = $state<HTMLTextAreaElement | null>(null);
  // Sent from the composer (button or ⌘↵) → focus returns to the textarea once
  // the echo lands, so the next message is typed without Tabbing back in.
  let refocus = false;
  function submit() {
    if (!connected) return;
    refocus = !!formEl?.contains(document.activeElement);
    // ONE submit button: while the last message is unconfirmed it re-sends that
    // message (same nonce, so the room dedupes), otherwise it sends the draft.
    if (pending) { send({type: 'chat', ...pending}); return; }
    if (!draft.trim() || bytes > 4096) return;
    pending = {text: draft, nonce: crypto.randomUUID()};
    send({type: 'chat', ...pending});
  }
  function onDraftKey(event: KeyboardEvent): void {
    if (event.key === 'Enter' && (event.metaKey || event.ctrlKey) && !event.isComposing) {
      event.preventDefault();
      submit();
    }
  }
  $effect(() => {
    if (delivered && pending) {
      if (draft === pending.text) draft = '';
      pending = null;
      if (refocus) { refocus = false; void tick().then(() => inputEl?.focus()); }
    }
  });

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
  let paintedFor = -1;
  $effect(() => {
    const last = messages.at(-1)?.seq ?? -1;
    const first = paintedFor < 0;
    paintedFor = last;
    if (first || untrack(() => atBottom)) void tick().then(toBottom);
  });
;
async () => {
 { svelteHTML.createElement("section", { "aria-label":`Room chat`,});
   { svelteHTML.createElement("h2", {});  } { svelteHTML.createElement("p", { "class":`hint`,});         }
   { svelteHTML.createElement("div", { "class":`log`,});
     { const $$_div2 = svelteHTML.createElement("div", {          "class":`messages`,"onscroll":onScroll,"role":`log`,"aria-label":`Room messages`,"aria-relevant":`additions`,});listEl = $$_div2;
      if(!messages.length){ { svelteHTML.createElement("p", { "class":`hint`,});     }}
         for(let message of __sveltets_2_ensureArray(messages)){message.seq; { svelteHTML.createElement("article", {}); { svelteHTML.createElement("header", {}); { svelteHTML.createElement("strong", {});message.name; } { svelteHTML.createElement("time", { "datetime":message.created_at,});new Date(message.created_at).toLocaleTimeString([], {hour: '2-digit', minute: '2-digit'}); } } { svelteHTML.createElement("p", {});message.text; } }}
     }
    if(!atBottom && messages.length > 0){
       { svelteHTML.createElement("button", {     "class":`btn small jump`,"type":`button`,"onclick":toBottom,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`arrowDown`,"size":12,}});}    }
    }
   }
   { const $$_form1 = svelteHTML.createElement("form", {   "onsubmit":(event) => { event.preventDefault(); submit(); },});formEl = $$_form1;
     { svelteHTML.createElement("label", { "for":`room-message`,});  }
     { const $$_textarea2 = svelteHTML.createElement("textarea", {              "dir":`auto`,"id":`room-message`,"bind:value":draft,"onkeydown":onDraftKey,"aria-keyshortcuts":`Meta+Enter Control+Enter`,"rows":3,"placeholder":connected ? 'Write a message…' : 'Reconnecting — your draft is saved here',});inputEl = $$_textarea2;/*Ωignore_startΩ*/() => draft = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
     { svelteHTML.createElement("div", { "class":`send-row`,}); { svelteHTML.createElement("span", { });bytes > 4096;bytes > 4096 ? 'Message exceeds 4 KiB' : ''; }
      
       { svelteHTML.createElement("button", {       "class":`btn`,"type":`submit`,"title":`Send (⌘↵)`,"disabled":pending ? !connected : !connected || !draft.trim() || bytes > 4096,});pending ? 'Resend message' : 'Send'; }
     }
   }
 }

};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const RoomChat__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type RoomChat__SvelteComponent_ = ReturnType<typeof RoomChat__SvelteComponent_>;
/*Ωignore_endΩ*/export default RoomChat__SvelteComponent_;