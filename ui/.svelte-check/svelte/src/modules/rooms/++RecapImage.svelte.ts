///<reference types="svelte" />
;
import {onDestroy} from 'svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import {loadErrorText} from '../../lib/loadError';
import {recapBlob} from './recap-client';

;type $$ComponentProps =  {recapId: string; imageId: string; title: string};function $$render() {

  
  
  
  
  let {recapId, imageId, title}:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();
  let url = $state(''), busy = $state(false), error = $state('');
  let live = true;
  onDestroy(() => { live = false; if (url) URL.revokeObjectURL(url); });
  // A failure stays on screen (LoadState's alert + Retry) until a retry answers,
  // so the focused Retry is not unmounted while the sample loads again.
  async function load() { busy = true; try { const blob = await recapBlob(`/room-recaps/${encodeURIComponent(recapId)}/images/${encodeURIComponent(imageId)}`); if (live) { url = URL.createObjectURL(blob); error = ''; } } catch (e) { if (live) error = loadErrorText(e); } finally { busy = false; } }
;
async () => {
if(url){ { svelteHTML.createElement("img", {    "src":url,"alt":`Recorded sample: ${title}`,});}
} else if (error){ { const $$_etatSdaoL0C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL0C({ target: __sveltets_2_any(), props: {         "what":`this screen sample`,"variant":`compact`,"loading":busy,error,"empty":true,"onretry":load,}});}
}else{ { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":busy,"onclick":load,});busy ? 'Loading sample…' : 'Show screen sample'; }}

};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const RecapImage__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type RecapImage__SvelteComponent_ = ReturnType<typeof RecapImage__SvelteComponent_>;
/*Ωignore_endΩ*/export default RecapImage__SvelteComponent_;