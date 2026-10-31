///<reference types="svelte" />
;// A response's footer facts: how long it worked, on which model, and where
// its tokens went — input (your side: blue), thinking (violet), output (the
// agent's side: green), cache read / write (dim). Every figure carries a
// tooltip that says what it counts. Renders nothing it does not know: no
// usage recorded → no token pieces.

import { fmtDuration, fmtTokens, usageParts } from './format';
import type { TurnUsage } from '../../../lib/api/types';
;

  interface Props {
    usage: TurnUsage | null;
    durationMs?: number | null;
    model?: string | null;
  };function $$render() {

  
  
  
  
  
  
  

  let { usage, durationMs = null, model = null }: Props = $props();
  const parts = $derived(usageParts(usage));
  const dur = $derived(fmtDuration(durationMs));
  const total = $derived(usage ? usage.input_tokens + usage.output_tokens + usage.cache_read_tokens + usage.cache_creation_tokens : 0);
;
async () => {

if(parts.length || dur || model){
   { svelteHTML.createElement("span", {   "class":`usage`,...__sveltets_2_empty({"data-usage":usage ? total : undefined}),});
    if(dur){ { svelteHTML.createElement("span", {   "class":`u-dur`,"title":`How long the agent worked on this response`,});dur; }}
    if(model){ { svelteHTML.createElement("span", {   "class":`u-model mono`,"title":`Model`,});model; }}
       for(let p of __sveltets_2_ensureArray(parts)){p.key;
       { svelteHTML.createElement("span", {     "class":`u-part u-${p.key}`,"title":`${p.title}: ${p.value.toLocaleString()}`,...__sveltets_2_empty({"data-part":p.key}),});
         { svelteHTML.createElement("span", {   "class":`u-dot`,"aria-hidden":`true`,}); }fmtTokens(p.value) || '0';  { svelteHTML.createElement("span", { "class":`u-lbl`,});p.label; }
       }
    }
   }
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const UsageLine__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type UsageLine__SvelteComponent_ = ReturnType<typeof UsageLine__SvelteComponent_>;
/*Ωignore_endΩ*/export default UsageLine__SvelteComponent_;