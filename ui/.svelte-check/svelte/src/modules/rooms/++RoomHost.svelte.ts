///<reference types="svelte" />
;
import { onMount } from 'svelte';
import type { RoomCredential } from '../../lib/api/room-types';
import { isTauri } from '../../lib/desktop';
import { rememberRoom, forgetRoom } from './room-access';
import RoomPage from './RoomPage.svelte';
import { requestRoomContext } from './room-window';
import LoadState from '../../lib/components/LoadState.svelte';
import ConfirmDialog from '../../lib/components/ConfirmDialog.svelte';
import ContextMenu from '../../lib/components/ContextMenu.svelte';
import { loadErrorText } from '../../lib/loadError';

;type $$ComponentProps =  { roomId: string };function $$render() {

  
  
  
  
  
  
  
  
  
  

  let { roomId }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();
  let ready = $state(false), error = $state(''), loading = $state(true);
  let disposed = false;
  async function load() {
    loading = true; error = '';
    try {
      const context = isTauri
        ? await (await import('@tauri-apps/api/core')).invoke<{ origin: string; credential: RoomCredential }>('get_host_room_context')
        : await requestRoomContext(roomId);
      if (disposed) return;
      if (context.credential.room_id !== roomId) throw new Error('This window belongs to another room. Reopen it from Rooms.');
      rememberRoom(context.origin, context.credential);
      ready = true;
    } catch (e) { if (!disposed) error = loadErrorText(e); }
    finally { if (!disposed) loading = false; }
  }
  onMount(() => { void load(); return () => { disposed = true; forgetRoom(roomId); }; });
;
async () => {

if(ready){
   { const $$_egaPmooR0C = __sveltets_2_ensureComponent(RoomPage); new $$_egaPmooR0C({ target: __sveltets_2_any(), props: { roomId,}});}
}else{
   { svelteHTML.createElement("div", { "class":`room-loading`,});
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {          "what":`this room`,"variant":`page`,loading,error,"empty":true,"onretry":load,}});}
   }
}
 { const $$_golaiDmrifnoC0C = __sveltets_2_ensureComponent(ConfirmDialog); new $$_golaiDmrifnoC0C({ target: __sveltets_2_any(), props: {}});}
 { const $$_uneMtxetnoC0C = __sveltets_2_ensureComponent(ContextMenu); new $$_uneMtxetnoC0C({ target: __sveltets_2_any(), props: {}});}


};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const RoomHost__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type RoomHost__SvelteComponent_ = ReturnType<typeof RoomHost__SvelteComponent_>;
/*Ωignore_endΩ*/export default RoomHost__SvelteComponent_;