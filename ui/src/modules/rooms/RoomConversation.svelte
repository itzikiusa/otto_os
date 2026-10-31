<script lang="ts">
  // The shared session's Chat view inside a room: the same turn rendering as
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
  import { detectBlockDirection } from '../../lib/textDirection';
  import type { RoomCredential } from '../../lib/api/room-types';
  import type { Transcript, TranscriptProvider } from '../../lib/api/types';
  import { fetchRoomTranscript, roomImageUrl, roomPrompt } from './room-client';

  interface Props {
    origin: string;
    credential: RoomCredential;
    sessionId: string;
    /** Set while this member drives the terminal (enables the composer). */
    grantEpoch: number | null;
  }
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
</script>

<section class="room-conv" aria-label="Agent conversation">
  <div class="turns" bind:this={scroller}>
    {#if error && !loaded}
      <EmptyState variant="page" icon="comment" tone="error" title="Couldn’t load the conversation" body={error} actionLabel="Retry" onaction={() => void refresh()} />
    {:else if !loaded}
      <Skeleton rows={4} label="the conversation" />
    {:else if unavailable}
      <EmptyState variant="page" icon="comment" title="No conversation yet" body={unavailable} />
    {:else if !items.length}
      <EmptyState variant="page" icon="comment" title="No messages yet" body="The agent’s turns appear here as the session runs." />
    {:else}
      {#each items as item (item.id)}<TurnItem {item} />{/each}
    {/if}
    {#if error && loaded}<p class="error" use:announceShown={{ text: `Updates paused: ${error}` }}>Updates paused: {error}</p>{/if}
  </div>
  {#if grantEpoch !== null}
    <form class="composer" onsubmit={(e) => { e.preventDefault(); void submit(); }}>
      <textarea dir={detectBlockDirection(draft)} rows="2" bind:value={draft} onkeydown={onKey} placeholder="Message the agent (Enter to send, Shift+Enter for a new line)" aria-label="Message the agent" disabled={sending}></textarea>
      <div class="actions">
        <button type="button" class="btn small" onclick={interrupt} title="Interrupt the agent (Esc)">Stop</button>
        <button class="btn small primary" disabled={sending || !draft.trim()}>{sending ? 'Sending…' : 'Send'}</button>
      </div>
      {#if sendError}<p class="error" use:announceShown={{ text: sendError, assertive: true }}>{sendError}</p>{/if}
    </form>
  {:else}
    <p class="muted hint">Take control of the terminal to message the agent.</p>
  {/if}
</section>

<style>
  .room-conv { display: flex; flex-direction: column; min-height: 0; height: 100%; }
  .turns { flex: 1; min-height: 0; overflow-y: auto; padding: 12px 16px; display: flex; flex-direction: column; gap: 8px; }
  .composer { border-top: 1px solid var(--border); padding: 8px 12px; display: grid; gap: 6px; }
  textarea { width: 100%; resize: vertical; min-height: 44px; font: inherit; unicode-bidi: plaintext; text-align: start; }
  .actions { display: flex; justify-content: flex-end; gap: 8px; }
  .muted { color: var(--text-dim); font-size: var(--fs-s); }
  .hint { padding: 8px 12px; border-top: 1px solid var(--border); margin: 0; }
  .error { color: var(--danger); font-size: var(--fs-s); margin: 0; }
</style>
