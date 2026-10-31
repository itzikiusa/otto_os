///<reference types="svelte" />
;
import { onMount } from 'svelte';
import { announceShown } from '../../lib/announce.svelte';
import Modal from '../../lib/components/Modal.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../lib/loadError';
import { api } from '../../lib/api/client';
import type { RoomSettings, RoomSettingsUpdate } from '../../lib/api/room-types';

;type $$ComponentProps =  {onclose: () => void};function $$render() {

  
  
  
  
  
  
  
  let {onclose}:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();
  let loading = $state(true), busy = $state(false), error = $state(''), loaded = $state(false), loadError = $state('');
  let origin = $state(''), stun = $state(''), turn = $state(''), secret = $state(''), configured = $state(false), relayOnly = $state(false), clearSecret = $state(false);
  onMount(() => { void load(); });
  async function load() {
    loading = true; error = ''; loadError = '';
    try { const settings = await api.get<RoomSettings>('/room-settings'); origin = settings.public_origin; stun = settings.stun_urls.join('\n'); turn = settings.turn_urls.join('\n'); configured = settings.turn_secret_configured; relayOnly = settings.relay_only; loaded = true; }
    catch (e) { loadError = loadErrorText(e); }
    finally { loading = false; }
  }
  async function save() {
    busy = true; error = '';
    const lines = (value: string) => value.split(/\r?\n/).map(v => v.trim()).filter(Boolean);
    const settings: RoomSettingsUpdate = {public_origin: origin.trim(), stun_urls: lines(stun), turn_urls: lines(turn), relay_only: relayOnly};
    if (secret || clearSecret) settings.turn_secret = clearSecret ? '' : secret;
    try { await api.put('/room-settings', settings); secret = ''; onclose(); }
    catch (e) { error = e instanceof TypeError ? loadErrorText(e) : e instanceof Error ? e.message : 'Couldn’t save room settings.'; }
    finally { busy = false; }
  }
;
async () => {
 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {    children:() => { return __sveltets_2_any(0); },"title":`Room connection settings`,"width":580,onclose,footer:() => { async ()/*Ωignore_positionΩ*/ => { { svelteHTML.createElement("button", {     "class":`btn`,"onclick":onclose,"disabled":busy,});  } { svelteHTML.createElement("button", {     "class":`btn primary`,"disabled":!loaded || loading || busy || !origin.trim(),"onclick":save,});busy ? 'Saving…' : 'Save settings'; }};return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
  if(!loaded){
    
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {           "what":`room settings`,"variant":`compact`,loading,"error":loadError,"empty":true,"onretry":load,}});}
  }else{
     { svelteHTML.createElement("p", {});                           }
     { svelteHTML.createElement("div", { "class":`fields`,});
       { svelteHTML.createElement("label", {});    { svelteHTML.createElement("input", {        "dir":`ltr`,"type":`url`,"bind:value":origin,"placeholder":`https://otto.example.com`,});/*Ωignore_startΩ*/() => origin = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
       { svelteHTML.createElement("label", {});      { svelteHTML.createElement("textarea", {       "dir":`ltr`,"rows":2,"bind:value":stun,"placeholder":`stun:stun.example.com:3478`,});/*Ωignore_startΩ*/() => stun = __sveltets_2_any(null);/*Ωignore_endΩ*/ } }
       { svelteHTML.createElement("label", {});      { svelteHTML.createElement("textarea", {       "dir":`ltr`,"rows":2,"bind:value":turn,"placeholder":`turns:turn.example.com:5349`,});/*Ωignore_startΩ*/() => turn = __sveltets_2_any(null);/*Ωignore_endΩ*/ } }
       { svelteHTML.createElement("label", {});    { svelteHTML.createElement("input", {          "type":`password`,"bind:value":secret,"autocomplete":`new-password`,"placeholder":configured ? 'Saved in Keychain — leave empty to keep' : 'Not configured',"disabled":clearSecret,});/*Ωignore_startΩ*/() => secret = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
      if(configured){ { svelteHTML.createElement("label", { "class":`check`,}); { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":clearSecret,});/*Ωignore_startΩ*/() => clearSecret = __sveltets_2_any(null);/*Ωignore_endΩ*/}      }}
       { svelteHTML.createElement("label", { "class":`check`,}); { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":relayOnly,});/*Ωignore_startΩ*/() => relayOnly = __sveltets_2_any(null);/*Ωignore_endΩ*/}         }
     }
     { svelteHTML.createElement("p", {});                     }
  }
  if(error){ {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('p'),({ text: error, assertive: true })));{ svelteHTML.createElement("p", __sveltets_2_union($$action_0), {   "class":`form-error`,});error; }}}
  
 Modal}

};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const RoomSettingsModal__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type RoomSettingsModal__SvelteComponent_ = ReturnType<typeof RoomSettingsModal__SvelteComponent_>;
/*Ωignore_endΩ*/export default RoomSettingsModal__SvelteComponent_;