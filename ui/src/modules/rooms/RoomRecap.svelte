<script lang="ts">
  import {onDestroy} from 'svelte';
  import {recapStateLabel} from '../../lib/labels';
  import {plural} from '../../lib/plural';
  import type {RoomSnapshot, RoomAction} from '../../lib/api/room-types';
  import type {RecapCapabilities, RecapDetail} from '../../lib/api/room-recap-types';
  import type {RoomRecapAudioSource} from './room-media';
  import {RoomRecapCapture, type RecapCaptureStatus} from './recap-capture';
  import {recapRequest} from './recap-client';
  import {auth} from '../../lib/stores/auth.svelte';
  import {confirmer} from '../../lib/confirm.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import RecapSettingsModal from './RecapSettingsModal.svelte';
  import RecapPanel from './RecapPanel.svelte';
  import {loadErrorText} from '../../lib/loadError';
  import {announce} from '../../lib/announce.svelte';
  let {room, connected, send, audioSources, screenStreams}: {room: RoomSnapshot; connected: boolean; send: (action: RoomAction) => boolean; audioSources: () => ReadonlyMap<string, RoomRecapAudioSource>; screenStreams: () => ReadonlyMap<string, MediaStream>} = $props();
  let prepare = $state(false), panel = $state(false), settings = $state(false), busy = $state(false), partial = $state(false);
  /** What failed (a sentence) + why (dim), and whether it was the engine check (the only one "Retry checks" re-runs). */
  let error = $state<{what: string; detail: string; check: boolean} | null>(null);
  // A failed engine check renders as LoadState's error view (an alert that speaks itself);
  // other failures are plain lines announced here, at the edge.
  function fail(what: string, e: unknown, check: boolean) { error = {what, detail: loadErrorText(e), check}; if (!check) announce(`${what} ${error.detail}`, {assertive: true}); }
  let speechAvailable = $state(false);
  let capability = $state<RecapCapabilities | null>(null);
  let captureStatus = $state<RecapCaptureStatus>({audioSources: 0, screenSources: 0, gaps: [], speechIssue: null});
  let capture: RoomRecapCapture | null = null;
  const host = $derived(room.member_id === room.host_member_id);
  // Known from the snapshot (no loading state): without a recap nobody has consented.
  const consented = $derived(room.recap ? room.recap.consented_member_ids.includes(room.member_id) : false);
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
  /** `retry` keeps the failed check (and its focused Retry) on screen until the new answer. */
  async function openPrepare(retry = false) {
    partial = false; prepare = true; busy = true; capability = null;
    if (!retry) error = null;
    try {
      capability = await recapRequest<RecapCapabilities>('/room-recap-capabilities');
      error = null;
      if (!capability.speech_ready) announce('Speech recognition is not ready. You can continue without a spoken transcript.');
    } catch (e) { fail('Couldn’t check the local recap engines.', e, true); } finally { busy = false; }
  }
  async function create() { busy = true; error = null; try { await recapRequest(`/rooms/${room.room_id}/recaps`, {allow_unavailable_speech: partial}); prepare = false; } catch (e) { fail('Couldn’t prepare the recap.', e, false); } finally { busy = false; } }
  // Speech-capture trouble is announced at its edge; the visible line is not a live region.
  $effect(() => { const issue = captureStatus.speechIssue; if (issue) announce(`Speech capture is paused: ${issue}`); });
  async function withdraw(): Promise<void> {
    const ok = await confirmer.ask('The host stops capturing speech, chat and screens until everyone consents again.', {
      title: 'Pause capture for everyone?',
      confirmLabel: 'Pause capture',
      danger: true,
    });
    if (!ok) return;
    capture?.cancelCapture();
    send({type: 'recap_consent', epoch: room.recap!.epoch, allow: false});
  }
  /** One always-mounted consent button: the server echo swaps its label and action
   *  instead of unmounting the focused control. */
  function consentButton() { if (consented) void withdraw(); else send({type: 'recap_consent', epoch: room.recap!.epoch, allow: true}); }
  /** One always-mounted capture toggle (Start ↔ Pause), for the same reason. */
  const capturing = $derived(room.recap?.state === 'capturing');
  function captureToggle() { if (capturing) void stop(false); else start(); }
  /** The one status for capture edges (the visible lines are inserted with their text). */
  const recapAnnouncement = $derived(!room.recap ? '' : room.recap.state === 'finalizing' ? 'Recap finishing. No new content is being captured.' : room.recap.state === 'paused' ? 'Capture paused.' : `Recap ${recapStateLabel(room.recap.state).toLowerCase()}.`);
  async function stop(final: boolean) { busy = true; try { if (final) await capture?.finish(); else capture?.cancelCapture(); send({type: final ? 'recap_stop' : 'recap_pause'}); } finally { busy = false; } }
</script>
<section class="recap-banner" aria-label="Room recap">
  <!-- Always mounted (empty until a recap exists), so the first state and every
       later edge are changes to an existing region, which a screen reader speaks. -->
  <p class="sr-only" role="status">{recapAnnouncement}</p>
  {#if room.recap}
    <div class="status"><strong>Recap {recapStateLabel(room.recap.state).toLowerCase()}</strong><span>Saved locally by the host · speech, room activity and sampled screens</span></div>
    {#if room.recap.state === 'finalizing'}<p>Finishing {plural(room.recap.pending_jobs, 'accepted media job')}. No new content is being captured.</p><p class="hint">This can take up to three minutes. Withdrawing consent cancels unfinished work immediately.</p>{/if}
    {#if room.recap.reason}<p>{room.recap.reason}</p>{/if}
    {#if room.recap.state !== 'stopped'}
      <p>The host keeps a local transcript and images of shared screens. Sampling may miss changes between frames. You can withdraw consent to pause capture for everyone.</p>
      <div class="actions">
        {#if consented}<span>You consented</span>{/if}
        <button class="btn small" disabled={!connected || busy} onclick={consentButton}>{consented ? 'Withdraw consent' : 'I consent to capture'}</button>
        {#if !consented}<button class="btn small" disabled={!connected || busy} onclick={() => void withdraw()}>Do not consent</button>{/if}
        {#if host && room.recap.state !== 'finalizing'}
          <button class="btn small" disabled={busy || (!capturing && (!connected || room.recap.state !== 'ready'))} title={capturing ? 'Pause immediately; unfinished speech is discarded' : undefined} onclick={captureToggle}>{capturing ? 'Pause capture' : 'Start capture'}</button>
          <button class="btn small" disabled={busy || (!capturing && !connected)} onclick={() => stop(true)}>Finish recap</button>
        {/if}
      </div>
      <p class="hint">Consented: {room.members?.filter(m => room.recap?.consented_member_ids.includes(m.id)).map(m => m.name).join(', ') || 'Waiting for everyone'}</p>
    {/if}
    {#if host}<div class="actions"><button class="btn small" onclick={() => panel = true}>Open recap</button>{#if room.recap.state === 'stopped'}<button class="btn small" onclick={() => openPrepare()}>Prepare another recap…</button>{/if}</div>
      {#if room.recap.state === 'capturing'}<p class="hint">Capturing {captureStatus.audioSources} microphone {captureStatus.audioSources === 1 ? 'source' : 'sources'} and sampling {captureStatus.screenSources} shared {captureStatus.screenSources === 1 ? 'screen' : 'screens'}. Hidden or unavailable screens have reduced coverage.</p>
        {#if captureStatus.speechIssue}<p class="speech-issue">Speech capture is paused: {captureStatus.speechIssue}</p><button class="btn small" onclick={() => capture?.resumeAudio()}>Resume speech capture</button>{/if}{/if}
      {#if captureStatus.gaps.length}<details><summary>Capture coverage notes ({captureStatus.gaps.length})</summary><ul>{#each captureStatus.gaps as gap}<li>{gap}</li>{/each}</ul></details>{/if}
    {/if}
  {:else if host}<button class="btn small" onclick={() => openPrepare()}>Prepare a full recap…</button><span class="hint">Capture starts only after everyone consents.</span>{/if}
  {#if error && !prepare}<p class="error"><strong>{error.what}</strong> <span>{error.detail}</span></p>{/if}
</section>
{#if prepare}<Modal title="Prepare a room recap" width={600} onclose={() => prepare = false}>
  <p>Ask everyone to consent to a local archive of speech, chat, terminal output, shared-source changes, sampled screen images and annotations. The host keeps the archive after the room ends.</p>
  <p>Only content shared after capture starts is included. Images are sampled up to every 15 seconds, not recorded as video. Consent withdrawal or connection loss pauses capture.</p>
  <LoadState what="the local recap engines" variant="compact" rows={1} loading={busy && !capability} error={error?.check ? error.detail : null} empty={!capability} onretry={() => openPrepare(true)}>
    {#if capability && !capability.speech_ready}<p class="error">Speech recognition is not ready: {capability.configuration_error ?? 'Configure a local whisper.cpp executable and multilingual model.'}</p><label class="check"><input type="checkbox" bind:checked={partial} /> Continue with room activity and screen samples, without a spoken transcript</label>{/if}
  </LoadState>
  {#if error && !error.check}<p class="error"><strong>{error.what}</strong> <span>{error.detail}</span></p>{/if}
  {#if auth.isRoot}<button class="btn" onclick={() => settings = true}>Configure local engines…</button>{/if}
  {#snippet footer()}<button class="btn" onclick={() => prepare = false}>Cancel</button><button class="btn primary" disabled={busy || !capability || (!capability.speech_ready && !partial)} onclick={create}>Ask everyone to consent</button>{/snippet}
</Modal>{/if}
{#if settings}<RecapSettingsModal onclose={() => { settings = false; void openPrepare(); }} />{/if}
{#if panel && room.recap}<Modal title="Room recap" width={960} onclose={() => panel = false}>{#key room.recap.id}<RecapPanel recapId={room.recap.id} />{/key}</Modal>{/if}
<style>
  .recap-banner { padding: 8px 16px; border-bottom: 1px solid var(--border); background: var(--surface); max-height: 35%; overflow: auto; }
  .recap-banner:empty { display: none; } .status, .actions { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; } .status span, .hint { font-size: var(--fs-s); color: var(--text-dim); } p, li { line-height: 1.5; overflow-wrap: anywhere; } .recap-banner p { margin-block: 8px; font-size: var(--fs-s); } .error { color: var(--danger); } .error span { color: var(--text-dim); } .speech-issue { color: var(--warning); } .check { display: flex; gap: 8px; align-items: start; margin-block: 16px; } details { margin-block-start: 8px; font-size: var(--fs-s); }
</style>
