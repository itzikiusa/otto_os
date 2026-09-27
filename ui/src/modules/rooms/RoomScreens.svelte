<script lang="ts">
  import type { RoomSnapshot, RoomAction, RoomSubscriptionTier } from '../../lib/api/room-types';
  import RoomAnnotations from './RoomAnnotations.svelte';
  import { reconcilePin } from './room-state';
  let { room, streams, send, onSubscriptions }: {room: RoomSnapshot; streams: ReadonlyMap<string, MediaStream>; send: (action: RoomAction) => boolean; onSubscriptions: (subscriptions: {sourceId: string; generation: number; tier: RoomSubscriptionTier}[]) => void} = $props();
  let pin = $state<string | null>(null), hidden = $state(document.hidden);
  const sources = $derived(room.presentations ?? []);
  $effect(() => { pin = reconcilePin(pin, sources.map(s => s.id)); });
  $effect(() => { const listener = () => hidden = document.hidden; document.addEventListener('visibilitychange', listener); return () => document.removeEventListener('visibilitychange', listener); });
  $effect(() => {
    onSubscriptions(sources.map(source => ({sourceId: source.id, generation: source.generation, tier: hidden ? 'hidden' : pin ? source.id === pin ? 'full' : 'preview' : 'grid'})));
  });
  function streamVideo(node: HTMLVideoElement, stream: MediaStream | undefined) {
    function set(value: MediaStream | undefined) { node.srcObject = value ?? null; if (value) void node.play().catch(() => {}); }
    set(stream); return {update: set, destroy() { node.pause(); node.srcObject = null; }};
  }
</script>
{#if sources.length}<section aria-label="Shared screens">
  <header><h2>Shared screens</h2><span>Your layout is private</span>{#if pin}<button class="btn small" onclick={() => pin = null}>Show grid</button>{/if}</header>
  <div class="sources" class:pinned={!!pin}>{#each sources as source (source.id)}
    {@const presenter = room.members?.find(m => m.id === source.member_id)}
    <article class:focused={pin === source.id}>
      <div class="source-head"><strong>{presenter?.name ?? 'Participant'} · {source.title}</strong><button class="btn small" aria-pressed={pin === source.id} aria-label={`${pin === source.id ? 'Unpin' : 'Pin'} ${presenter?.name ?? 'participant'}’s screen`} onclick={() => pin = pin === source.id ? null : source.id}>{pin === source.id ? 'Unpin' : 'Pin'}</button>
        {#if room.member_id === room.host_member_id || room.member_id === source.member_id}<button class="btn small" onclick={() => send({type: 'stop_present', source_id: source.id})}>Stop</button>{/if}
      </div>
      <div class="source-body">
        {#if streams.has(source.id)}
          {#if !pin || pin === source.id}<RoomAnnotations {room} {source} {send}>
            <video use:streamVideo={streams.get(source.id)} muted autoplay playsinline aria-label={`${presenter?.name ?? 'Participant'}’s shared screen`}></video>
          </RoomAnnotations>
          {:else}<div class="video-stage"><video use:streamVideo={streams.get(source.id)} muted autoplay playsinline aria-label={`${presenter?.name ?? 'Participant'}’s shared screen`}></video></div>{/if}
        {:else}<div class="video-stage"><p role="status">Waiting for {presenter?.name ?? 'the presenter'}’s screen…</p></div>{/if}
      </div>
    </article>
  {/each}</div>
</section>{/if}
<style>
  section { padding: 12px; border-bottom: 1px solid var(--border); max-height: 60%; overflow: auto; }
  header, .source-head { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; } header { margin-block-end: 8px; }
  h2 { font-size: var(--fs-m); margin: 0; } header span { color: var(--text-dim); font-size: var(--fs-s); margin-inline-end: auto; }
  .sources { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 8px; } .sources.pinned { grid-template-columns: repeat(3, minmax(0, 1fr)); }
  article { min-width: 0; border: 1px solid var(--border); border-radius: var(--radius-m); overflow: hidden; background: var(--surface); }
  .focused { grid-column: 1 / -1; order: -1; } .source-head { padding: 8px; } strong { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: var(--fs-s); }
  .video-stage { position: relative; aspect-ratio: 16/9; min-height: 120px; } video { width: 100%; height: 100%; object-fit: contain; display: block; } p { padding: 16px; color: var(--text-dim); }
  @media (max-width: 640px) { .sources, .sources.pinned { grid-template-columns: minmax(0, 1fr); } .pinned article:not(.focused) .source-body { display: none; } }
</style>
