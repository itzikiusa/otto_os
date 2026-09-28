<script lang="ts">
  import {onDestroy} from 'svelte';
  import type {RoomSnapshot, RoomAction} from '../../lib/api/room-types';
  import type {RecapCapabilities, RecapDetail} from '../../lib/api/room-recap-types';
  import type {RoomRecapAudioSource} from './room-media';
  import {RoomRecapCapture, type RecapCaptureStatus} from './recap-capture';
  import {recapRequest} from './recap-client';
  import {auth} from '../../lib/stores/auth.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import RecapSettingsModal from './RecapSettingsModal.svelte';
  import RecapPanel from './RecapPanel.svelte';
  let {room, connected, send, audioSources, screenStreams}: {room: RoomSnapshot; connected: boolean; send: (action: RoomAction) => boolean; audioSources: () => ReadonlyMap<string, RoomRecapAudioSource>; screenStreams: () => ReadonlyMap<string, MediaStream>} = $props();
  let prepare = $state(false), panel = $state(false), settings = $state(false), busy = $state(false), error = $state(''), partial = $state(false);
  let speechAvailable = $state(false);
  let capability = $state<RecapCapabilities | null>(null);
  let captureStatus = $state<RecapCaptureStatus>({audioSources: 0, screenSources: 0, gaps: []});
  let capture: RoomRecapCapture | null = null;
  const host = $derived(room.member_id === room.host_member_id);
  const consented = $derived(room.recap?.consented_member_ids.includes(room.member_id) ?? false);
  $effect(() => {
    if (host) { capture ??= new RoomRecapCapture(audioSources, screenStreams, status => captureStatus = status); capture.update(room, connected, speechAvailable); }
    else { capture?.dispose(); capture = null; }
  });
  $effect(() => {
    const id = host ? room.recap?.id : null;
    if (!id) { speechAvailable = false; return; }
    let live = true;
    void recapRequest<RecapDetail>(`/room-recaps/${id}?limit=1`).then(detail => { if (live) speechAvailable = detail.metadata.speech_available; }).catch(() => { if (live) speechAvailable = false; });
    return () => { live = false; };
  });
  onDestroy(() => capture?.dispose());
  export async function finishCapture() { await capture?.finish(); }
  function start() { capture?.prepareAudio(); send({type: 'recap_start', epoch: room.recap!.epoch}); }
  async function openPrepare() { partial = false; prepare = true; busy = true; error = ''; capability = null; try { capability = await recapRequest<RecapCapabilities>('/room-recap-capabilities'); } catch (e) { error = e instanceof Error ? e.message : 'Could not check local recap engines.'; } finally { busy = false; } }
  async function create() { busy = true; error = ''; try { await recapRequest(`/rooms/${room.room_id}/recaps`, {allow_unavailable_speech: partial}); prepare = false; } catch (e) { error = e instanceof Error ? e.message : 'Could not prepare the recap.'; } finally { busy = false; } }
  function withdraw() { capture?.cancelCapture(); send({type: 'recap_consent', epoch: room.recap!.epoch, allow: false}); }
  async function stop(final: boolean) { busy = true; try { if (final) await capture?.finish(); else capture?.cancelCapture(); send({type: final ? 'recap_stop' : 'recap_pause'}); } finally { busy = false; } }
</script>
<section class="recap-banner" aria-label="Room recap">
  {#if room.recap}
    <div class="status"><strong role="status">Recap {room.recap.state.replaceAll('_', ' ')}</strong><span>Saved locally by the host · speech, room activity and sampled screens</span></div>
    {#if room.recap.state === 'finalizing'}<p role="status">Finishing {room.recap.pending_jobs} accepted media jobs. No new content is being captured.</p><p class="hint">This can take up to three minutes. Withdrawing consent cancels unfinished work immediately.</p>{/if}
    {#if room.recap.reason}<p>{room.recap.reason}</p>{/if}
    {#if room.recap.state !== 'stopped'}
      <p>The host keeps a local transcript and images of shared screens. Sampling may miss changes between frames. You can withdraw consent to pause capture for everyone.</p>
      <div class="actions">
        {#if consented}<span>You consented</span><button class="btn small" disabled={!connected || busy} onclick={withdraw}>Withdraw consent</button>
        {:else}<button class="btn small" disabled={!connected || busy} onclick={() => send({type: 'recap_consent', epoch: room.recap!.epoch, allow: true})}>I consent to capture</button><button class="btn small" disabled={!connected || busy} onclick={withdraw}>Do not consent</button>{/if}
        {#if host && room.recap.state !== 'finalizing'}
          {#if room.recap.state === 'capturing'}<button class="btn small" disabled={busy} title="Pause immediately; unfinished speech is discarded" onclick={() => stop(false)}>Pause capture</button><button class="btn small" disabled={busy} onclick={() => stop(true)}>Finish recap</button>
          {:else}<button class="btn small" disabled={!connected || busy || room.recap.state !== 'ready'} onclick={start}>Start capture</button><button class="btn small" disabled={!connected || busy} onclick={() => stop(true)}>Finish recap</button>{/if}
        {/if}
      </div>
      <p class="hint">Consented: {room.members?.filter(m => room.recap?.consented_member_ids.includes(m.id)).map(m => m.name).join(', ') || 'Waiting for everyone'}</p>
    {/if}
    {#if host}<div class="actions"><button class="btn small" onclick={() => panel = true}>Open recap</button>{#if room.recap.state === 'stopped'}<button class="btn small" onclick={openPrepare}>Prepare another recap…</button>{/if}</div>
      {#if room.recap.state === 'capturing'}<p class="hint">Capturing {captureStatus.audioSources} microphone source(s) and sampling {captureStatus.screenSources} shared screen(s). Hidden or unavailable screens have reduced coverage.</p><button class="btn small" onclick={() => capture?.resumeAudio()}>Retry speech capture</button>{/if}
      {#if captureStatus.gaps.length}<details><summary>Capture coverage notes ({captureStatus.gaps.length})</summary><ul>{#each captureStatus.gaps as gap}<li>{gap}</li>{/each}</ul></details>{/if}
    {/if}
  {:else if host}<button class="btn small" onclick={openPrepare}>Prepare a full recap…</button><span class="hint">Capture starts only after everyone consents.</span>{/if}
  {#if error}<p role="alert">{error}</p>{/if}
</section>
{#if prepare}<Modal title="Prepare a room recap" width={600} onclose={() => prepare = false}>
  <p>Ask everyone to consent to a local archive of speech, chat, terminal output, shared-source changes, sampled screen images and annotations. The host keeps the archive after the room ends.</p>
  <p>Only content shared after capture starts is included. Images are sampled up to every 15 seconds, not recorded as video. Consent withdrawal or connection loss pauses capture.</p>
  {#if busy && !capability}<p role="status">Checking local engines…</p>{/if}
  {#if capability && !capability.speech_ready}<p role="alert">Speech recognition is not ready: {capability.configuration_error ?? 'Configure a local whisper.cpp executable and multilingual model.'}</p><label class="check"><input type="checkbox" bind:checked={partial} /> Continue with room activity and screen samples, without a spoken transcript</label>{/if}
  {#if error}<p role="alert">{error}</p><button class="btn" onclick={openPrepare}>Retry checks</button>{/if}
  {#if auth.isRoot}<button class="btn" onclick={() => settings = true}>Configure local engines…</button>{/if}
  {#snippet footer()}<button class="btn" onclick={() => prepare = false}>Cancel</button><button class="btn primary" disabled={busy || !capability || (!capability.speech_ready && !partial)} onclick={create}>Ask everyone to consent</button>{/snippet}
</Modal>{/if}
{#if settings}<RecapSettingsModal onclose={() => { settings = false; void openPrepare(); }} />{/if}
{#if panel && room.recap}<Modal title="Room recap" width={960} onclose={() => panel = false}><RecapPanel recapId={room.recap.id} /></Modal>{/if}
<style>
  .recap-banner { padding: 8px 16px; border-bottom: 1px solid var(--border); background: var(--surface); max-height: 35%; overflow: auto; }
  .recap-banner:empty { display: none; } .status, .actions { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; } .status span, .hint { font-size: var(--fs-s); color: var(--text-dim); } p, li { line-height: 1.5; overflow-wrap: anywhere; } .recap-banner p { margin-block: 8px; font-size: var(--fs-s); } [role='alert'] { color: var(--danger); } .check { display: flex; gap: 8px; align-items: start; margin-block: 16px; } details { margin-block-start: 8px; font-size: var(--fs-s); }
</style>
