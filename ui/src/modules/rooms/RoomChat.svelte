<script lang="ts">
  import type { RoomMessage, RoomAction } from '../../lib/api/room-types';
  let { messages, send, connected }: {messages: RoomMessage[]; send: (action: RoomAction) => boolean; connected: boolean} = $props();
  let draft = $state('');
  let pending = $state<{text: string; nonce: string} | null>(null);
  const delivered = $derived(pending && messages.some(m => m.nonce === pending?.nonce));
  const bytes = $derived(new TextEncoder().encode(draft).length);
  function submit() {
    if (!draft.trim() || bytes > 4096 || !connected) return;
    pending = {text: draft, nonce: crypto.randomUUID()};
    send({type: 'chat', ...pending});
  }
  $effect(() => { if (delivered && pending) { if (draft === pending.text) draft = ''; pending = null; } });
</script>
<section aria-label="Room chat">
  <h2>Chat</h2><p class="hint">Shared with this room. Never sent to the agent.</p>
  <div class="messages" role="log" aria-label="Room messages" aria-relevant="additions">
    {#if !messages.length}<p class="hint">Say hello to the room.</p>{/if}
    {#each messages as message (message.seq)}<article><header><strong>{message.name}</strong><time datetime={message.created_at}>{new Date(message.created_at).toLocaleTimeString([], {hour: '2-digit', minute: '2-digit'})}</time></header><p>{message.text}</p></article>{/each}
  </div>
  <form onsubmit={(event) => { event.preventDefault(); submit(); }}>
    <label for="room-message">Message everyone</label>
    <textarea dir="auto" id="room-message" bind:value={draft} rows="3" placeholder={connected ? 'Write a message…' : 'Reconnecting — your draft is saved here'}></textarea>
    <div class="send-row"><span class:too-long={bytes > 4096}>{bytes > 4096 ? 'Message exceeds 4 KiB' : ''}</span>
      {#if pending}<button class="btn" type="button" disabled={!connected} onclick={() => pending && send({type: 'chat', ...pending})}>Retry message</button>
      {:else}<button class="btn" disabled={!connected || !draft.trim() || bytes > 4096}>Send</button>{/if}
    </div>
  </form>
</section>
<style>
  section { padding: 16px; display: flex; flex: 1; min-height: 260px; flex-direction: column; gap: 8px; }
  h2 { font-size: var(--fs-m); margin: 0; } .hint { color: var(--text-dim); font-size: var(--fs-s); margin: 0; }
  .messages { flex: 1; overflow-y: auto; min-height: 80px; max-height: 45vh; }
  article { padding-block: 12px; } header { display: flex; justify-content: space-between; gap: 8px; }
  time { color: var(--text-dim); font-size: var(--fs-s); } article p { white-space: pre-wrap; overflow-wrap: anywhere; margin-block: 4px 0; line-height: 1.5; }
  form { display: grid; gap: 8px; } textarea { resize: vertical; width: 100%; min-height: 64px; max-height: 180px; }
  label { font-size: var(--fs-s); } .send-row { display: flex; justify-content: space-between; align-items: center; gap: 8px; } .too-long { color: var(--danger); }
</style>
