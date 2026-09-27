<script lang="ts">
  import { onMount } from 'svelte';
  import type { RoomSnapshot, RoomAction, RoomEvent } from '../../lib/api/room-types';
  import { RoomMediaClient, type RoomMediaState } from './room-media';
  import RoomScreens from './RoomScreens.svelte';
  let {room, send, connected}: {room: RoomSnapshot; send: (action: RoomAction) => boolean; connected: boolean} = $props();
  let media = $state<RoomMediaClient | null>(null);
  let mediaState = $state<RoomMediaState>({audioJoined: false, muted: true, presenting: false, error: null, connection: 'idle', relayConfigured: false});
  let streams = $state<ReadonlyMap<string, MediaStream>>(new Map());
  let busy = $state(false), localError = $state(''), devices = $state<MediaDeviceInfo[]>([]), microphone = $state('');
  const self = $derived(room.members?.find(m => m.id === room.member_id));
  onMount(() => {
    media = new RoomMediaClient({memberId: room.member_id, send, onState: value => mediaState = value, onStreams: value => streams = value});
    return () => media?.dispose();
  });
  $effect(() => { if (!media) return; if (connected) media.update(room); else media.disconnect(); });
  export function handleEvent(event: RoomEvent) { if (event.type === 'snapshot') media?.update(event.room); media?.handleEvent(event); }
  export function getRecapAudioSources() { return media?.getRecapAudioSources() ?? new Map(); }
  export function getPresentationStreams() { return streams; }
  async function act(action: () => Promise<void>) {
    busy = true; localError = '';
    try { await action(); } catch (e) { localError = e instanceof Error ? e.message : 'Media could not start. Session and chat remain available.'; }
    finally { busy = false; }
  }
  async function unmute() {
    await media?.unmute(microphone || undefined);
    devices = (await navigator.mediaDevices?.enumerateDevices() ?? []).filter(d => d.kind === 'audioinput');
  }
</script>
<div class="media-controls" aria-label="Room voice and screens">
  <div class="controls">
    {#if mediaState.audioJoined}
      <span>{self?.room_muted ? 'Muted by host' : mediaState.muted ? 'Microphone off' : 'Microphone on'}</span>
      <button class="btn small" disabled={!connected || busy || self?.room_muted} onclick={() => mediaState.muted ? act(unmute) : media?.mute()}>{mediaState.muted ? 'Turn microphone on' : 'Mute microphone'}</button>
      <button class="btn small" onclick={() => media?.leaveAudio()}>Leave audio</button>
      {#if devices.length > 1}<label>Microphone <select bind:value={microphone} onchange={() => { media?.mute(); }}><option value="">System default</option>{#each devices as device}<option value={device.deviceId}>{device.label || 'Microphone'}</option>{/each}</select></label>{/if}
    {:else}<button class="btn small" disabled={!connected || busy} onclick={() => act(async () => { await media?.joinAudio(); })}>Join audio</button>{/if}
    {#if mediaState.presenting}<button class="btn small" onclick={() => media?.stopPresentation()}>Stop sharing screen</button>
    {:else if self?.role === 'host' || self?.presenter_allowed}<button class="btn small" disabled={!connected || busy} onclick={() => act(async () => { await media?.startPresentation(); })}>Share screen…</button>
    {:else}<button class="btn small" disabled={!connected || self?.presenter_requested} onclick={() => send({type: 'request_present'})}>{self?.presenter_requested ? 'Presentation requested' : 'Request to present'}</button>{/if}
    {#if room.member_id === room.host_member_id}<button class="btn small" aria-pressed={room.annotations_enabled !== false} onclick={() => send({type: 'annotations_enabled', enabled: room.annotations_enabled === false})}>{room.annotations_enabled === false ? 'Allow annotations' : 'Disable annotations'}</button>{/if}
  </div>
  {#if mediaState.connection === 'connecting'}<p role="status">Connecting voice and screens…</p>{/if}
  {#if !mediaState.relayConfigured}<p>A media relay is not configured. Voice and screen sharing may not connect across networks. Session and chat remain available.</p>{/if}
  {#if mediaState.error || localError}<p role="alert">{localError || mediaState.error}</p>{/if}
</div>
<RoomScreens {room} {streams} {send} onSubscriptions={subscriptions => media?.setSubscriptions(subscriptions)} />
<style>
  .media-controls { padding: 8px 12px; border-bottom: 1px solid var(--border); background: var(--surface); }
  .controls { display: flex; align-items: center; flex-wrap: wrap; gap: 8px; }
  p { margin-block: 8px 0; font-size: var(--fs-s); color: var(--text-dim); line-height: 1.4; } [role='alert'] { color: var(--danger); }
  label { display: flex; gap: 8px; align-items: center; } span, label { font-size: var(--fs-s); }
</style>
