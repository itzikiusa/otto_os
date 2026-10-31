///<reference types="svelte" />
;
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

;type $$ComponentProps =  {room: RoomSnapshot; connected: boolean; send: (action: RoomAction) => boolean; audioSources: () => ReadonlyMap<string, RoomRecapAudioSource>; screenStreams: () => ReadonlyMap<string, MediaStream>};function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  let {room, connected, send, audioSources, screenStreams}:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();
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
   async function finishCapture() { await capture?.finish(); }
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
;
async () => {
 { svelteHTML.createElement("section", {   "class":`recap-banner`,"aria-label":`Room recap`,});
  
   { svelteHTML.createElement("p", {   "class":`sr-only`,"role":`status`,});recapAnnouncement; }
  if(room.recap){
     { svelteHTML.createElement("div", { "class":`status`,}); { svelteHTML.createElement("strong", {}); recapStateLabel(room.recap.state).toLowerCase(); } { svelteHTML.createElement("span", {});            } }
    if(room.recap.state === 'finalizing'){ { svelteHTML.createElement("p", {}); plural(room.recap.pending_jobs, 'accepted media job');       } { svelteHTML.createElement("p", { "class":`hint`,});             }}
    if(room.recap.reason){ { svelteHTML.createElement("p", {});room.recap.reason; }}
    if(room.recap.state !== 'stopped'){
       { svelteHTML.createElement("p", {});                          }
       { svelteHTML.createElement("div", { "class":`actions`,});
        if(consented){ { svelteHTML.createElement("span", {});  }}
         { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":!connected || busy,"onclick":consentButton,});consented ? 'Withdraw consent' : 'I consent to capture'; }
        if(!consented){ { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":!connected || busy,"onclick":() => void withdraw(),});   }}
        if(host && room.recap.state !== 'finalizing'){
           { svelteHTML.createElement("button", {       "class":`btn small`,"disabled":busy || (!capturing && (!connected || room.recap.state !== 'ready')),"title":capturing ? 'Pause immediately; unfinished speech is discarded' : undefined,"onclick":captureToggle,});capturing ? 'Pause capture' : 'Start capture'; }
           { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":busy || (!capturing && !connected),"onclick":() => stop(true),});  }
        }
       }
       { svelteHTML.createElement("p", { "class":`hint`,}); room.members?.filter(m => room.recap?.consented_member_ids.includes(m.id)).map(m => m.name).join(', ') || 'Waiting for everyone'; }
    }
    if(host){ { svelteHTML.createElement("div", { "class":`actions`,}); { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => panel = true,});  }if(room.recap.state === 'stopped'){ { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => openPrepare(),});   }} }
      if(room.recap.state === 'capturing'){ { svelteHTML.createElement("p", { "class":`hint`,}); captureStatus.audioSources;  captureStatus.audioSources === 1 ? 'source' : 'sources';   captureStatus.screenSources;  captureStatus.screenSources === 1 ? 'screen' : 'screens';        }
        if(captureStatus.speechIssue){ { svelteHTML.createElement("p", { "class":`speech-issue`,});    captureStatus.speechIssue; } { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => capture?.resumeAudio(),});   }}}
      if(captureStatus.gaps.length){ { svelteHTML.createElement("details", {}); { svelteHTML.createElement("summary", {});   captureStatus.gaps.length;  } { svelteHTML.createElement("ul", {});  for(let gap of __sveltets_2_ensureArray(captureStatus.gaps)){ { svelteHTML.createElement("li", {});gap; }} } }}
    }
  } else if (host){ { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => openPrepare(),});    } { svelteHTML.createElement("span", { "class":`hint`,});      }}
  if(error && !prepare){ { svelteHTML.createElement("p", { "class":`error`,}); { svelteHTML.createElement("strong", {});error.what; }  { svelteHTML.createElement("span", {});error.detail; } }}
 }
if(prepare){ { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"title":`Prepare a room recap`,"width":600,"onclose":() => prepare = false,footer:() => { async ()/*Ωignore_positionΩ*/ => { { svelteHTML.createElement("button", {   "class":`btn`,"onclick":() => prepare = false,});  } { svelteHTML.createElement("button", {     "class":`btn primary`,"disabled":busy || !capability || (!capability.speech_ready && !partial),"onclick":create,});    }};return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
   { svelteHTML.createElement("p", {});                             }
   { svelteHTML.createElement("p", {});                           }
   { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {             children:() => { return __sveltets_2_any(0); },"what":`the local recap engines`,"variant":`compact`,"rows":1,"loading":busy && !capability,"error":error?.check ? error.detail : null,"empty":!capability,"onretry":() => openPrepare(true),}});
    if(capability && !capability.speech_ready){ { svelteHTML.createElement("p", { "class":`error`,});     capability.configuration_error ?? 'Configure a local whisper.cpp executable and multilingual model.'; } { svelteHTML.createElement("label", { "class":`check`,}); { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":partial,});/*Ωignore_startΩ*/() => partial = __sveltets_2_any(null);/*Ωignore_endΩ*/}            }}
   LoadState}
  if(error && !error.check){ { svelteHTML.createElement("p", { "class":`error`,}); { svelteHTML.createElement("strong", {});error.what; }  { svelteHTML.createElement("span", {});error.detail; } }}
  if(auth.isRoot){ { svelteHTML.createElement("button", {   "class":`btn`,"onclick":() => settings = true,});   }}
  
 Modal}}
if(settings){ { const $$_ladoMsgnitteSpaceR0C = __sveltets_2_ensureComponent(RecapSettingsModal); new $$_ladoMsgnitteSpaceR0C({ target: __sveltets_2_any(), props: {  "onclose":() => { settings = false; void openPrepare(); },}});}}
if(panel && room.recap){ { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); new $$_ladoM0C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"title":`Room recap`,"width":960,"onclose":() => panel = false,}});room.recap.id; { { const $$_lenaPpaceR1C = __sveltets_2_ensureComponent(RecapPanel); new $$_lenaPpaceR1C({ target: __sveltets_2_any(), props: {  "recapId":room.recap.id,}});}} Modal}}

};
return { props: {} as any as $$ComponentProps, exports: {} as any as { finishCapture: typeof finishCapture }, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const RoomRecap__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type RoomRecap__SvelteComponent_ = ReturnType<typeof RoomRecap__SvelteComponent_>;
/*Ωignore_endΩ*/export default RoomRecap__SvelteComponent_;