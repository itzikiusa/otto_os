///<reference types="svelte" />
;
import type { RoomSnapshot, RoomAction, RoomSubscriptionTier } from '../../lib/api/room-types';
import RoomAnnotations from './RoomAnnotations.svelte';
import { announceShown } from '../../lib/announce.svelte';
import { reconcilePin } from './room-state';

;type $$ComponentProps =  {room: RoomSnapshot; streams: ReadonlyMap<string, MediaStream>; send: (action: RoomAction) => boolean; onSubscriptions: (subscriptions: {sourceId: string; generation: number; tier: RoomSubscriptionTier}[]) => void};function $$render() {

  
  
  
  
  let { room, streams, send, onSubscriptions }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();
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
;
async () => {
if(sources.length){ { svelteHTML.createElement("section", { "aria-label":`Shared screens`,});
   { svelteHTML.createElement("header", {}); { svelteHTML.createElement("h2", {});  } { svelteHTML.createElement("span", {});    }if(pin){ { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => pin = null,});  }} }
   { svelteHTML.createElement("div", {  "class":`sources`,});!!pin;   for(let source of __sveltets_2_ensureArray(sources)){source.id;
    const presenter = room.members?.find(m => m.id === source.member_id);
     { svelteHTML.createElement("article", { });pin === source.id;
       { svelteHTML.createElement("div", { "class":`source-head`,}); { svelteHTML.createElement("strong", {});presenter?.name ?? 'Participant';  source.title; } { svelteHTML.createElement("button", {         "class":`btn small pin`,"aria-pressed":pin === source.id,"aria-label":`Pin ${presenter?.name ?? 'participant'}’s screen`,"title":pin === source.id ? 'Unpin this screen' : 'Pin this screen',"onclick":() => pin = pin === source.id ? null : source.id,});  }
        if(room.member_id === room.host_member_id || room.member_id === source.member_id){ { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => send({type: 'stop_present', source_id: source.id}),});  }}
       }
       { svelteHTML.createElement("div", { "class":`source-body`,});
        if(streams.has(source.id)){
          if(!pin || pin === source.id){ { const $$_snoitatonnAmooR4C = __sveltets_2_ensureComponent(RoomAnnotations); new $$_snoitatonnAmooR4C({ target: __sveltets_2_any(), props: {  children:() => { return __sveltets_2_any(0); },room,source,send,}});
             {const $$action_0 = __sveltets_2_ensureAction(streamVideo(svelteHTML.mapElementTag('video'),(streams.get(source.id))));{ svelteHTML.createElement("video", __sveltets_2_union($$action_0), {    "muted":true,"autoplay":true,"playsinline":true,"aria-label":`${presenter?.name ?? 'Participant'}’s shared screen`,}); }}
           RoomAnnotations}
          }else{ { svelteHTML.createElement("div", { "class":`video-stage`,}); {const $$action_0 = __sveltets_2_ensureAction(streamVideo(svelteHTML.mapElementTag('video'),(streams.get(source.id))));{ svelteHTML.createElement("video", __sveltets_2_union($$action_0), {    "muted":true,"autoplay":true,"playsinline":true,"aria-label":`${presenter?.name ?? 'Participant'}’s shared screen`,}); }} }}
        }else{ { svelteHTML.createElement("div", { "class":`video-stage`,}); {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('p'),(`Waiting for ${presenter?.name ?? 'the presenter'}’s screen…`)));{ svelteHTML.createElement("p", __sveltets_2_union($$action_0), {  });  presenter?.name ?? 'the presenter';  }} }}
       }
     }
  } }
 }}

};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const RoomScreens__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type RoomScreens__SvelteComponent_ = ReturnType<typeof RoomScreens__SvelteComponent_>;
/*Ωignore_endΩ*/export default RoomScreens__SvelteComponent_;