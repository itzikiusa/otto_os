<script lang="ts">
  import { tick, untrack } from 'svelte';
  import type { RoomMessage, RoomAction } from '../../lib/api/room-types';
  import Icon from '../../lib/components/Icon.svelte';
  let { messages, send, connected }: {messages: RoomMessage[]; send: (action: RoomAction) => boolean; connected: boolean} = $props();
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
</script>
<section aria-label="Room chat">
  <h2>Chat</h2><p class="hint">Shared with this room. Never sent to the agent.</p>
  <div class="log">
    <div class="messages" bind:this={listEl} onscroll={onScroll} role="log" aria-label="Room messages" aria-relevant="additions">
      {#if !messages.length}<p class="hint">Say hello to the room.</p>{/if}
      {#each messages as message (message.seq)}<article><header><strong>{message.name}</strong><time datetime={message.created_at}>{new Date(message.created_at).toLocaleTimeString([], {hour: '2-digit', minute: '2-digit'})}</time></header><p>{message.text}</p></article>{/each}
    </div>
    {#if !atBottom && messages.length > 0}
      <button class="btn small jump" type="button" onclick={toBottom}><Icon name="arrowDown" size={12} /> Jump to latest</button>
    {/if}
  </div>
  <form bind:this={formEl} onsubmit={(event) => { event.preventDefault(); submit(); }}>
    <label for="room-message">Message everyone</label>
    <textarea dir="auto" id="room-message" bind:this={inputEl} bind:value={draft} onkeydown={onDraftKey} aria-keyshortcuts="Meta+Enter Control+Enter" rows="3" placeholder={connected ? 'Write a message…' : 'Reconnecting — your draft is saved here'}></textarea>
    <div class="send-row"><span class:too-long={bytes > 4096}>{bytes > 4096 ? 'Message exceeds 4 KiB' : ''}</span>
      <!-- Always mounted: swapping Send ↔ Resend for two buttons destroyed the
           focused one, dropping keyboard focus to <body> on every message. -->
      <button class="btn" type="submit" title="Send (⌘↵)" disabled={pending ? !connected : !connected || !draft.trim() || bytes > 4096}>{pending ? 'Resend message' : 'Send'}</button>
    </div>
  </form>
</section>
<style>
  section { position: relative; padding: 16px; display: flex; flex: 1; min-height: 0; flex-direction: column; gap: 8px; }
  h2 { font-size: var(--fs-m); margin: 0; } .hint { color: var(--text-dim); font-size: var(--fs-s); margin: 0; }
  .log { position: relative; flex: 1; min-height: 0; display: flex; flex-direction: column; }
  .messages { flex: 1; overflow-y: auto; min-height: 0; }
  .jump { position: absolute; inset-inline-start: 50%; bottom: 8px; transform: translateX(-50%); box-shadow: var(--glass-shadow); }
  :global([dir='rtl']) .jump { transform: translateX(50%); }
  form { flex-shrink: 0; }
  article { padding-block: 12px; } header { display: flex; justify-content: space-between; gap: 8px; }
  time { color: var(--text-dim); font-size: var(--fs-s); } article p { white-space: pre-wrap; overflow-wrap: anywhere; margin-block: 4px 0; line-height: 1.5; }
  form { display: grid; gap: 8px; } textarea { resize: vertical; width: 100%; min-height: 64px; max-height: 180px; }
  label { font-size: var(--fs-s); } .send-row { display: flex; justify-content: space-between; align-items: center; gap: 8px; } .too-long { color: var(--danger); }
</style>
