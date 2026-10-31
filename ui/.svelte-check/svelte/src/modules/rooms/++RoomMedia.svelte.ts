///<reference types="svelte" />
;
import { onMount, untrack } from 'svelte';
import type { RoomSnapshot, RoomAction, RoomEvent } from '../../lib/api/room-types';
import { RoomMediaClient, type RoomMediaState } from './room-media';
import RoomScreens from './RoomScreens.svelte';
import { announce } from '../../lib/announce.svelte';

;type $$ComponentProps =  {room: RoomSnapshot; send: (action: RoomAction) => boolean; connected: boolean};function $$render() {

  
  
  
  
  
  let {room, send, connected}:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();
  let media = $state<RoomMediaClient | null>(null);
  let mediaState = $state<RoomMediaState>({audioJoined: false, muted: true, presenting: false, error: null, connection: 'idle', relayConfigured: false});
  let streams = $state<ReadonlyMap<string, MediaStream>>(new Map());
  let busy = $state(false), localError = $state(''), devices = $state<MediaDeviceInfo[]>([]), microphone = $state('');
  const self = $derived(room.members?.find(m => m.id === room.member_id));
  // a11y-6-01: announced at the edge; the visible lines are inserted with their text.
  const mediaError = $derived(localError || mediaState.error || '');
  const mediaConnecting = $derived(mediaState.connection === 'connecting');
  $effect(() => { if (mediaError) announce(mediaError, { assertive: true }); });
  $effect(() => { if (mediaConnecting) announce('Connecting voice and screens…'); });
  onMount(() => {
    media = new RoomMediaClient({memberId: room.member_id, send, onState: value => mediaState = value, onStreams: value => streams = value});
    return () => media?.dispose();
  });
  // Tracks only the media client and `connected`: every snapshot already reaches
  // the client through handleEvent, and annotation/chat traffic re-creates `room`
  // without touching media state, so tracking it rebuilt the audio mix per event.
  $effect(() => { const client = media; if (!client) return; if (connected) untrack(() => client.update(room)); else client.disconnect(); });
   function handleEvent(event: RoomEvent) { if (event.type === 'snapshot') media?.update(event.room); media?.handleEvent(event); }
   function getRecapAudioSources() { return media?.getRecapAudioSources() ?? new Map(); }
   function getPresentationStreams() { return streams; }
  async function act(action: () => Promise<void>) {
    busy = true; localError = '';
    try { await action(); } catch (e) { localError = e instanceof Error ? e.message : 'Media couldn’t start. Session and chat remain available.'; }
    finally { busy = false; }
  }
  async function unmute() {
    await media?.unmute(microphone || undefined);
    devices = (await navigator.mediaDevices?.enumerateDevices() ?? []).filter(d => d.kind === 'audioinput');
  }
;
async () => {
 { svelteHTML.createElement("div", {   "class":`media-controls`,"aria-label":`Room voice and screens`,});
   { svelteHTML.createElement("div", { "class":`controls`,});
    if(mediaState.audioJoined){
       { svelteHTML.createElement("span", {});self?.room_muted ? 'Muted by host' : mediaState.muted ? 'Microphone off' : 'Microphone on'; }
       { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":!connected || busy || self?.room_muted,"onclick":() => mediaState.muted ? act(unmute) : media?.mute(),});mediaState.muted ? 'Turn microphone on' : 'Mute microphone'; }
       { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => media?.leaveAudio(),});  }
      if(devices.length > 1){ { svelteHTML.createElement("label", {});  { svelteHTML.createElement("select", {   "bind:value":microphone,"onchange":() => { media?.mute(); },});/*Ωignore_startΩ*/() => microphone = __sveltets_2_any(null);/*Ωignore_endΩ*/ { svelteHTML.createElement("option", {"value":"",});  }  for(let device of __sveltets_2_ensureArray(devices)){ { svelteHTML.createElement("option", { "value":device.deviceId,});device.label || 'Microphone'; }} } }}
    }else{ { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":!connected || busy,"onclick":() => act(async () => { await media?.joinAudio(); }),});  }}
    if(mediaState.presenting){ { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => media?.stopPresentation(),});   }
    } else if (self?.role === 'host' || self?.presenter_allowed){ { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":!connected || busy,"onclick":() => act(async () => { await media?.startPresentation(); }),});  }
    }else{ { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":!connected || self?.presenter_requested,"onclick":() => send({type: 'request_present'}),});self?.presenter_requested ? 'Presentation requested' : 'Request to present'; }}
    if(room.member_id === room.host_member_id){ { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => send({type: 'annotations_enabled', enabled: room.annotations_enabled === false}),});room.annotations_enabled === false ? 'Allow annotations' : 'Disable annotations'; }}
   }
  if(mediaState.connection === 'connecting'){ { svelteHTML.createElement("p", {});    }}
  if(!mediaState.relayConfigured){ { svelteHTML.createElement("p", {});                    }}
  if(mediaError){ { svelteHTML.createElement("p", { "class":`alert-text`,});mediaError; }}
 }
 { const $$_sneercSmooR0C = __sveltets_2_ensureComponent(RoomScreens); new $$_sneercSmooR0C({ target: __sveltets_2_any(), props: {     room,streams,send,"onSubscriptions":subscriptions => media?.setSubscriptions(subscriptions),}});}

};
return { props: {} as any as $$ComponentProps, exports: {} as any as { handleEvent: typeof handleEvent,getRecapAudioSources: typeof getRecapAudioSources,getPresentationStreams: typeof getPresentationStreams }, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const RoomMedia__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type RoomMedia__SvelteComponent_ = ReturnType<typeof RoomMedia__SvelteComponent_>;
/*Ωignore_endΩ*/export default RoomMedia__SvelteComponent_;