///<reference types="svelte" />
;// Attribution line for agent-authored content (patterns.md §2): who · what
// model · when, plus the small AGENT chip that marks it as not-human.
//
//   [provider mark] claude  AGENT · sonnet · 3m ago
//
// Shows only what is known: a provider mark + name when the record carries a
// provider (otherwise a neutral sparkle + `name`, default "Otto"), the model
// when known, and the time (RelTime — relative, exact on hover). `label` is
// an optional lead-in such as "Plan v3". Agent identity is never the accent
// colour — the chip is the shared AgentChip (a neutral `.chip`).

import AgentChip from './AgentChip.svelte';
import Icon from './Icon.svelte';
import ProviderIcon from './ProviderIcon.svelte';
import RelTime from './RelTime.svelte';
;

  interface Props {
    /** Provider slug (claude / codex / agy / custom). Omit when unknown. */
    provider?: string | null;
    /** Model alias or id, when known. */
    model?: string | null;
    /** When the agent produced it (ISO or epoch ms). */
    at?: string | number | null;
    /** Name shown when the provider is unknown. */
    name?: string;
    /** Optional lead-in, e.g. "Plan v3". */
    label?: string;
    testid?: string;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { provider, model, at, name = 'Otto', label, testid }: Props = $props();

  const p = $derived((provider ?? '').trim());
  const m = $derived((model ?? '').trim());
;
async () => {

 { svelteHTML.createElement("span", {   "class":`agent-byline`,...__sveltets_2_empty({"data-testid":testid}),});
  if(label){ { svelteHTML.createElement("span", { "class":`ab-label`,});label; } { svelteHTML.createElement("span", {   "class":`ab-sep`,"aria-hidden":`true`,});  }}
   { svelteHTML.createElement("span", { "class":`ab-who`,});
    if(p){
       { const $$_nocIredivorP2C = __sveltets_2_ensureComponent(ProviderIcon); new $$_nocIredivorP2C({ target: __sveltets_2_any(), props: {    "provider":p,"size":13,}});}
       { svelteHTML.createElement("span", { "class":`ab-name`,});p; }
    }else{
       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`sparkle`,"size":12,}});}
       { svelteHTML.createElement("span", { "class":`ab-name`,});name; }
    }
   }
   { const $$_pihCtnegA1C = __sveltets_2_ensureComponent(AgentChip); new $$_pihCtnegA1C({ target: __sveltets_2_any(), props: {}});}
  if(m){ { svelteHTML.createElement("span", {   "class":`ab-sep`,"aria-hidden":`true`,});  } { svelteHTML.createElement("span", { "class":`ab-model mono`,});m; }}
  if(at != null && at !== ''){
     { svelteHTML.createElement("span", {   "class":`ab-sep`,"aria-hidden":`true`,});  } { const $$_emiTleR1C = __sveltets_2_ensureComponent(RelTime); new $$_emiTleR1C({ target: __sveltets_2_any(), props: {    "iso":at,"class":`ab-time`,}});}
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const AgentByline__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type AgentByline__SvelteComponent_ = ReturnType<typeof AgentByline__SvelteComponent_>;
/*Ωignore_endΩ*/export default AgentByline__SvelteComponent_;