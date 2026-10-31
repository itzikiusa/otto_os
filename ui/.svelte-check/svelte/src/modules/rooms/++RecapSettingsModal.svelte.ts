///<reference types="svelte" />
;
import {onMount} from 'svelte';
import {announceShown} from '../../lib/announce.svelte';
import Modal from '../../lib/components/Modal.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import {loadErrorText} from '../../lib/loadError';
import {recapRequest} from './recap-client';
import type {RecapCapabilities, RecapEngineSettings} from '../../lib/api/room-recap-types';

;type $$ComponentProps =  {onclose: () => void};function $$render() {

  
  
  
  
  
  
  
  let {onclose}:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();
  let config = $state<RecapEngineSettings>({whisper_executable: '', whisper_model: '', language: 'auto', threads: 2});
  let capability = $state<RecapCapabilities | null>(null), loading = $state(true), busy = $state(false), error = $state(''), loaded = $state(false), loadError = $state('');
  onMount(() => { void load(); });
  async function load() { loading = true; error = ''; loadError = ''; try { [config, capability] = await Promise.all([recapRequest<RecapEngineSettings>('/room-recap-settings'), recapRequest<RecapCapabilities>('/room-recap-capabilities')]); loaded = true; } catch (e) { loadError = loadErrorText(e); if (loaded) error = loadError; } finally { loading = false; } }
  async function save() { busy = true; error = ''; try { await recapRequest('/room-recap-settings', config, undefined, 'PUT'); await load(); } catch (e) { error = e instanceof TypeError ? loadErrorText(e) : e instanceof Error ? e.message : 'Couldn’t save settings.'; } finally { busy = false; } }
;
async () => {
 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {    children:() => { return __sveltets_2_any(0); },"title":`Local recap engines`,"width":600,onclose,footer:() => { async ()/*Ωignore_positionΩ*/ => { { svelteHTML.createElement("button", {   "class":`btn`,"onclick":onclose,});  } { svelteHTML.createElement("button", {     "class":`btn primary`,"disabled":!loaded || loading || busy,"onclick":save,});busy ? 'Checking…' : 'Save and check'; }};return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
   { svelteHTML.createElement("p", {});                      }
  if(!loaded){
    
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {           "what":`local engine settings`,"variant":`compact`,loading,"error":loadError,"empty":true,"onretry":load,}});}
  }else{ { svelteHTML.createElement("div", { "class":`fields`,});
     { svelteHTML.createElement("label", {});   { svelteHTML.createElement("input", {      "dir":`ltr`,"bind:value":config.whisper_executable,"placeholder":`/absolute/path/to/whisper-cli`,});/*Ωignore_startΩ*/() => config.whisper_executable = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
     { svelteHTML.createElement("label", {});    { svelteHTML.createElement("input", {      "dir":`ltr`,"bind:value":config.whisper_model,"placeholder":`/absolute/path/to/ggml-model.bin`,});/*Ωignore_startΩ*/() => config.whisper_model = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
     { svelteHTML.createElement("label", {});   { svelteHTML.createElement("select", { "bind:value":config.language,});/*Ωignore_startΩ*/() => config.language = __sveltets_2_any(null);/*Ωignore_endΩ*/ { svelteHTML.createElement("option", { "value":`auto`,});  } { svelteHTML.createElement("option", { "value":`en`,});  } { svelteHTML.createElement("option", { "value":`he`,});  } } }
     { svelteHTML.createElement("label", {});   { svelteHTML.createElement("select", { "bind:value":config.threads,});/*Ωignore_startΩ*/() => config.threads = __sveltets_2_any(null);/*Ωignore_endΩ*/  for(let threads of __sveltets_2_ensureArray(([1, 2, 3, 4]))){ { svelteHTML.createElement("option", { "value":threads,});threads; }} } }
   }
  if(capability){ { svelteHTML.createElement("dl", {}); { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", {});capability.speech_ready ? 'Ready' : capability.configuration_error ?? 'Setup needed'; } { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", {});capability.codex_subscription_ready ? 'Ready' : 'Sign in to the local Codex CLI'; } { svelteHTML.createElement("dt", {});   } { svelteHTML.createElement("dd", {});capability.screen_text_ready ? 'Ready' : 'Unavailable on this computer'; } } { svelteHTML.createElement("p", {});capability.setup; }}}
  if(error){ {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('p'),({text: error, assertive: true})));{ svelteHTML.createElement("p", __sveltets_2_union($$action_0), {   "class":`form-error`,});error; }}}
  
 Modal}

};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const RecapSettingsModal__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type RecapSettingsModal__SvelteComponent_ = ReturnType<typeof RecapSettingsModal__SvelteComponent_>;
/*Ωignore_endΩ*/export default RecapSettingsModal__SvelteComponent_;