///<reference types="svelte" />
;
import RoomSettingsModal from './RoomSettingsModal.svelte';
import { announceShown } from '../../lib/announce.svelte';
import { onMount } from 'svelte';
import PageHeader from '../../lib/components/PageHeader.svelte';
import PageBody from '../../lib/components/PageBody.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import {loadErrorText} from '../../lib/loadError';
import Modal from '../../lib/components/Modal.svelte';
import { auth } from '../../lib/stores/auth.svelte';
import { api } from '../../lib/api/client';
import { confirmer } from '../../lib/confirm.svelte';
import { toasts } from '../../lib/toast.svelte';
import { router } from '../../lib/router.svelte';
import { registry } from '../../lib/commands.svelte';
import { parseRoomInvite } from './room-state';
import { recallRoom } from './room-access';
import { openHostedRoom } from './room-window';
import type { RoomSnapshot } from '../../lib/api/room-types';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  let settingsOpen = $state(false);
  let rooms = $state<RoomSnapshot[]>([]);
  let loading = $state(true), error = $state(''), joinOpen = $state(false), link = $state(''), joinError = $state('');
  let opening = $state(''), joining = $state(false);
  /** Opening the room's window is an action: its failure is a toast whose
   *  action tries again (the room list itself stays as loaded). */
  async function openRoom(id: string) {
    opening = id;
    try { await openHostedRoom(id); }
    catch (e) { toasts.push('error', 'Couldn’t open the room', loadErrorText(e), 12000, {action: {label: 'Open again', run: () => openRoom(id)}}); }
    finally { opening = ''; }
  }
  onMount(() => { void load(); });
  // ⌘K verbs while the lobby is open.
  $effect(() =>
    registry.register('rooms', [
      { id: 'rooms.games', title: 'Play a game', group: 'Rooms', keywords: 'games shooter kart racing computer multiplayer', run: () => router.go('rooms/games') },
      { id: 'rooms.join', title: 'Join room…', group: 'Rooms', keywords: 'invite link connect session room', run: () => (joinOpen = true) },
      { id: 'rooms.recaps', title: 'Open recap archives', group: 'Rooms', keywords: 'room recap summary history', run: () => router.go('rooms/recaps') },
      ...(auth.isRoot ? [{ id: 'rooms.settings', title: 'Room connection settings…', group: 'Rooms', keywords: 'relay host server', run: () => (settingsOpen = true) }] : []),
      ...rooms.filter((r) => recallRoom(r.room_id)).map((r) => ({
        id: `rooms.open.${r.room_id}`, title: `Open room ${r.session_title ?? 'Session room'}`, group: 'Rooms', keywords: 'room session', run: () => void openRoom(r.room_id),
      })),
    ]),
  );
  async function load() {
    loading = true; error = '';
    try { rooms = await api.get<RoomSnapshot[]>('/rooms'); }
    catch (e) { error = loadErrorText(e); }
    finally { loading = false; }
  }
  async function join() {
    joinError = ''; joining = true;
    try {
      const invitation = parseRoomInvite(link);
      if (!await confirmer.ask(`Connect to ${invitation.origin}? Your display name and room activity will be shared with this host and admitted participants.`, {title: 'Join room', confirmLabel: 'Join'})) return;
      if ('__TAURI_INTERNALS__' in window) {
        const { invoke } = await import('@tauri-apps/api/core');
        const opened = await invoke<string>('open_room_window', {url: invitation.url});
        joinOpen = false; link = '';
        // A host on this network serves a self-signed certificate the native
        // window cannot accept, so the room opens in the default browser.
        if (opened === 'browser') toasts.push('info', 'Room opened in your browser', 'This host is on your local network. If the browser warns about its certificate, confirm the address matches and continue.', 12000);
      } else { window.location.assign(invitation.url); }
    } catch (e) { joinError = loadErrorText(e) || 'Couldn’t open this room. Try again.'; }
    finally { joining = false; }
  }
;
async () => {
 { svelteHTML.createElement("div", { "class":`rooms-page`,});
   { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); const $$_redaeHegaP1 = new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {   "title":`Rooms`,"subtitle":`Work together in a session`,actions:() => { async ()/*Ωignore_positionΩ*/ => { { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => router.go('rooms/games'),});  } { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => router.go('rooms/recaps'),});  }if(auth.isRoot){ { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => settingsOpen = true,});  }} { svelteHTML.createElement("button", {   "class":`btn small primary`,"onclick":() => joinOpen = true,});  }};return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {actions} = $$_redaeHegaP1.$$prop_def;/*Ωignore_endΩ*/
    
   PageHeader}
   { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: {children:() => { return __sveltets_2_any(0); },}});
     { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL2 = new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {         children:() => { return __sveltets_2_any(0); },"what":`rooms`,"variant":`page`,loading,error,"empty":!rooms.length,"onretry":load,emptyView:() => { async ()/*Ωignore_positionΩ*/ => { { const $$_etatSytpmE0C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE0C({ target: __sveltets_2_any(), props: {        "variant":`page`,"icon":`people`,"title":`Bring someone into your session`,"body":`Open a running session’s More menu and choose Start room. You control admission and terminal access.`,}});}};return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL2.$$prop_def;/*Ωignore_endΩ*/
    
     { svelteHTML.createElement("div", { "class":`room-list`,});   for(let room of __sveltets_2_ensureArray(rooms)){room.room_id;
       { svelteHTML.createElement("article", {}); { svelteHTML.createElement("h2", {});room.session_title ?? 'Session room'; } { svelteHTML.createElement("p", {});room.members?.filter(m => m.admission === 'admitted').length ?? 1;    }
        if(recallRoom(room.room_id)){ { svelteHTML.createElement("button", {     "class":`btn`,"disabled":!!opening,"onclick":() => openRoom(room.room_id),});opening === room.room_id ? 'Opening…' : 'Open room'; }
        }else{ { svelteHTML.createElement("p", {});         }}
       }
    } }
     LoadState}
   PageBody}
 }
if(settingsOpen){ { const $$_ladoMsgnitteSmooR0C = __sveltets_2_ensureComponent(RoomSettingsModal); new $$_ladoMsgnitteSmooR0C({ target: __sveltets_2_any(), props: {  "onclose":() => settingsOpen = false,}});}}
if(joinOpen){ { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {   children:() => { return __sveltets_2_any(0); },"title":`Join a room`,"onclose":() => joinOpen = false,footer:() => { async ()/*Ωignore_positionΩ*/ => { { svelteHTML.createElement("button", {   "class":`btn`,"onclick":() => joinOpen = false,});  } { svelteHTML.createElement("button", {     "class":`btn primary`,"disabled":!link.trim() || joining,"onclick":join,});joining ? 'Joining…' : 'Continue'; }};return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
   { svelteHTML.createElement("label", {});   { svelteHTML.createElement("input", {            "dir":`ltr`,"type":`url`,"bind:value":link,"placeholder":`https://host/#/room/…`,"autocomplete":`off`,"spellcheck":`false`,});/*Ωignore_startΩ*/() => link = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
   { svelteHTML.createElement("p", {});       }
  if(joinError){ {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('p'),({ text: joinError, assertive: true })));{ svelteHTML.createElement("p", __sveltets_2_union($$action_0), {   "class":`form-error`,});joinError; }}}
  
 Modal}}

};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const RoomLobby__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type RoomLobby__SvelteComponent_ = ReturnType<typeof RoomLobby__SvelteComponent_>;
/*Ωignore_endΩ*/export default RoomLobby__SvelteComponent_;