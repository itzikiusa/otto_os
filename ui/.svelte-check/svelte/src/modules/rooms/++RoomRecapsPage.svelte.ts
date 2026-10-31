///<reference types="svelte" />
;
import {onMount} from 'svelte';
import PageHeader from '../../lib/components/PageHeader.svelte';
import PageBody from '../../lib/components/PageBody.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import {loadErrorText} from '../../lib/loadError';
import RecapPanel from './RecapPanel.svelte';
import RecapSettingsModal from './RecapSettingsModal.svelte';
import {recapRequest} from './recap-client';
import {auth} from '../../lib/stores/auth.svelte';
import {router} from '../../lib/router.svelte';
import type {RecapMetadata} from '../../lib/api/room-recap-types';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  let archives = $state<RecapMetadata[]>([]), selected = $state(''), loading = $state(true), error = $state(''), settings = $state(false);
  onMount(() => { void load(); });
  async function load() { loading = true; error = ''; try { archives = await recapRequest<RecapMetadata[]>('/room-recaps'); if (!archives.some(a => a.id === selected)) selected = archives[0]?.id ?? ''; } catch (e) { error = loadErrorText(e); } finally { loading = false; } }
;
async () => {
 { svelteHTML.createElement("div", { "class":`recaps-page`,}); { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); const $$_redaeHegaP1 = new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {     "title":`Room recaps`,"subtitle":`Your local archives`,"crumbs":[{label: 'Rooms', onclick: () => router.go('rooms')}],actions:() => { async ()/*Ωignore_positionΩ*/ => {if(auth.isRoot){ { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => settings = true,});  }} { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":loading,"onclick":load,});loading ? 'Refreshing…' : 'Refresh'; }};return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {actions} = $$_redaeHegaP1.$$prop_def;/*Ωignore_endΩ*/
  
 PageHeader} { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: {children:() => { return __sveltets_2_any(0); },}});
  
   { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL2 = new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {         children:() => { return __sveltets_2_any(0); },"what":`room recaps`,"variant":`page`,loading,error,"empty":!archives.length,"onretry":load,emptyView:() => { async ()/*Ωignore_positionΩ*/ => { { const $$_etatSytpmE0C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE0C({ target: __sveltets_2_any(), props: {        "variant":`page`,"icon":`people`,"title":`No room recaps yet`,"body":`Prepare a recap from a room, ask everyone for consent, then start capture. Archives remain here after a room ends.`,}});}};return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL2.$$prop_def;/*Ωignore_endΩ*/
  
   { svelteHTML.createElement("label", {});  { svelteHTML.createElement("select", { "bind:value":selected,});/*Ωignore_startΩ*/() => selected = __sveltets_2_any(null);/*Ωignore_endΩ*/  for(let archive of __sveltets_2_ensureArray(archives)){ { svelteHTML.createElement("option", { "value":archive.id,});archive.session_title;  new Date(archive.created_at).toLocaleString(); }} } }selected; { { const $$_lenaPpaceR3C = __sveltets_2_ensureComponent(RecapPanel); new $$_lenaPpaceR3C({ target: __sveltets_2_any(), props: {  "recapId":selected,}});}}
   LoadState}
 PageBody} }
if(settings){ { const $$_ladoMsgnitteSpaceR0C = __sveltets_2_ensureComponent(RecapSettingsModal); new $$_ladoMsgnitteSpaceR0C({ target: __sveltets_2_any(), props: {  "onclose":() => settings = false,}});}}

};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const RoomRecapsPage__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type RoomRecapsPage__SvelteComponent_ = ReturnType<typeof RoomRecapsPage__SvelteComponent_>;
/*Ωignore_endΩ*/export default RoomRecapsPage__SvelteComponent_;