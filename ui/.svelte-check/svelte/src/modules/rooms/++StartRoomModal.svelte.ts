///<reference types="svelte" />
;
import Modal from '../../lib/components/Modal.svelte';
import { announceShown } from '../../lib/announce.svelte';
import { api, baseUrl } from '../../lib/api/client';
import { loadErrorText } from '../../lib/loadError';
import type { RoomCredential } from '../../lib/api/room-types';
import { rememberRoom } from './room-access';
import { openHostedRoom } from './room-window';

;type $$ComponentProps =  {sessionId: string; onclose: () => void};function $$render() {

  
  
  
  
  
  
  
  let { sessionId, onclose }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();
  let name = $state('');
  let busy = $state(false);
  let error = $state('');
  let created: RoomCredential | null = $state(null);
  async function start() {
    busy = true; error = '';
    try {
      // Retain a successful creation when native window opening fails. Retry
      // opens that same room, rather than creating an inaccessible duplicate.
      if (!created) {
        created = await api.post<RoomCredential>(`/sessions/${encodeURIComponent(sessionId)}/room`, {name: name.trim()});
        rememberRoom(baseUrl(), created);
      }
      await openHostedRoom(created.room_id);
      onclose();
    } catch (e) {
      // Lead with what failed (content.md §4): the room may exist already and
      // only its native window failed, which Retry reopens.
      error = `${created ? 'Couldn’t open the room window.' : 'Couldn’t start this room.'} ${loadErrorText(e)}`;
    }
    finally { busy = false; }
  }
;
async () => {
 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {  children:() => { return __sveltets_2_any(0); },"title":`Start a session room`,onclose,footer:() => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("button", {     "class":`btn`,"onclick":onclose,"disabled":busy,});  }
     { svelteHTML.createElement("button", {     "class":`btn primary`,"disabled":busy || !name.trim(),"onclick":start,});busy ? 'Opening…' : created ? 'Open room' : 'Start room'; }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
   { svelteHTML.createElement("p", {});                    }
   { svelteHTML.createElement("p", {});                 }
   { svelteHTML.createElement("label", {});    { svelteHTML.createElement("input", {          "dir":`auto`,"maxlength":80,"bind:value":name,"autocomplete":`nickname`,"disabled":!!created,});/*Ωignore_startΩ*/() => name = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
  if(error){ {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('p'),({ text: error, assertive: true })));{ svelteHTML.createElement("p", __sveltets_2_union($$action_0), {   "class":`form-error`,});error; }}}
  
 Modal}

};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const StartRoomModal__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type StartRoomModal__SvelteComponent_ = ReturnType<typeof StartRoomModal__SvelteComponent_>;
/*Ωignore_endΩ*/export default StartRoomModal__SvelteComponent_;