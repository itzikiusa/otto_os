///<reference types="svelte" />
;// Guest-only chrome stays in the room chunk: the desktop's hidden bar and
// tray windows must not load room media, xterm, or the owner shell at boot.

import RoomPage from './RoomPage.svelte';
import ConfirmDialog from '../../lib/components/ConfirmDialog.svelte';
import ContextMenu from '../../lib/components/ContextMenu.svelte';

;type $$ComponentProps =  { roomId: string };function $$render() {

  
  
  
  
  
  let { roomId }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();
;
async () => {

  { const $$_egaPmooR0C = __sveltets_2_ensureComponent(RoomPage); new $$_egaPmooR0C({ target: __sveltets_2_any(), props: { roomId,"guest":true,}});}
 { const $$_golaiDmrifnoC0C = __sveltets_2_ensureComponent(ConfirmDialog); new $$_golaiDmrifnoC0C({ target: __sveltets_2_any(), props: {}});}
 { const $$_uneMtxetnoC0C = __sveltets_2_ensureComponent(ContextMenu); new $$_uneMtxetnoC0C({ target: __sveltets_2_any(), props: {}});}
};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const RoomGuest__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type RoomGuest__SvelteComponent_ = ReturnType<typeof RoomGuest__SvelteComponent_>;
/*Ωignore_endΩ*/export default RoomGuest__SvelteComponent_;