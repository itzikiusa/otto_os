///<reference types="svelte" />
;// Keep each destination independent: Games must not fetch the live terminal,
// room call and recap dependency graphs before it can render its chooser.

import LazyMount from '../lib/components/LazyMount.svelte';
import { lazyComponent } from '../lib/lazy-component.svelte';
import { router } from '../lib/router.svelte';
function $$render() {

  
  
  
  
  
  const games = lazyComponent(() => import('../modules/rooms/games/GamesHub.svelte'));
  const room = lazyComponent(() => import('../modules/rooms/RoomPage.svelte'));
  const recaps = lazyComponent(() => import('../modules/rooms/RoomRecapsPage.svelte'));
  const lobby = lazyComponent(() => import('../modules/rooms/RoomLobby.svelte'));
;
async () => {

if(router.parts[1] === 'games'){
   { const $$_tnuoMyzaL0C = __sveltets_2_ensureComponent(LazyMount); new $$_tnuoMyzaL0C({ target: __sveltets_2_any(), props: {    "lazy":games,"what":`games`,}});}
} else if (router.parts[1] === 'recaps'){
   { const $$_tnuoMyzaL0C = __sveltets_2_ensureComponent(LazyMount); new $$_tnuoMyzaL0C({ target: __sveltets_2_any(), props: {    "lazy":recaps,"what":`room recaps`,}});}
} else if (router.parts[1]){
  router.parts[1]; { { const $$_tnuoMyzaL0C = __sveltets_2_ensureComponent(LazyMount); new $$_tnuoMyzaL0C({ target: __sveltets_2_any(), props: {      "lazy":room,"what":`room`,"props":{roomId:router.parts[1]},}});}}
}else{
   { const $$_tnuoMyzaL0C = __sveltets_2_ensureComponent(LazyMount); new $$_tnuoMyzaL0C({ target: __sveltets_2_any(), props: {    "lazy":lobby,"what":`rooms`,}});}
}
};
return { props: {} as Record<string, never>, exports: {}, bindings: "", slots: {}, events: {} }}
const RoomsRoute__SvelteComponent_ = __sveltets_2_isomorphic_component(__sveltets_2_with_any_event($$render()));
/*Ωignore_startΩ*/type RoomsRoute__SvelteComponent_ = InstanceType<typeof RoomsRoute__SvelteComponent_>;
/*Ωignore_endΩ*/export default RoomsRoute__SvelteComponent_;