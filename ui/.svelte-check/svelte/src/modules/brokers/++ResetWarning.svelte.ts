///<reference types="svelte" />
;
  import { resetWarning } from './resetWarning';
  export { resetWarning };
;;

import { announce } from '../../lib/announce.svelte';
import Icon from '../../lib/components/Icon.svelte';
import type { DryRunResp } from './types';

;type $$ComponentProps =  { preview: DryRunResp | null; group: string | null };function $$render() {

  
  
  

  let { preview, group }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();
  const text = $derived(resetWarning(preview, group));
  // Spoken at the state edge through the shell's always-mounted announcer: a
  // live region inserted together with its text is not read (WebKit/VoiceOver).
  $effect(() => { if (text) announce(text); });
;
async () => {




if(text){
   { svelteHTML.createElement("p", { "class":`reset-warning`,});
     { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});}
     { svelteHTML.createElement("span", {});text; }
   }
}


};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ResetWarning__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ResetWarning__SvelteComponent_ = ReturnType<typeof ResetWarning__SvelteComponent_>;
/*Ωignore_endΩ*/export default ResetWarning__SvelteComponent_;