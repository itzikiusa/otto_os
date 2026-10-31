///<reference types="svelte" />
;// The one tinted word-in-a-pill: a `.chip` (app.css) with a tone. Status,
// kind and count pills render through it instead of hand-rolling a local
// `.pill`. The tone comes from a domain mapper in lib/status.ts
// (`badgeTone(runStatus(x).tone)`, `designStatus(x)`, …) or a module's model,
// never picked ad hoc at the call site. Colour never carries meaning alone —
// the label is always there; `dot` adds a leading dot and `live` pulses it
// (live state only, stops under reduced motion).
//
//   <Badge tone="warn" label="In review" dot />
//   <Badge tone="info" variant="outline">3 open</Badge>

import type { Snippet } from 'svelte';
import type { BadgeTone } from '../status';
;

  interface Props {
    tone?: BadgeTone;
    /** `soft` (default) = tinted fill; `outline` = border + coloured text only,
     *  for dense rows where several badges sit side by side. */
    variant?: 'soft' | 'outline';
    label?: string;
    /** Leading dot in the tone colour. */
    dot?: boolean;
    /** The dot pulses — only for state that is live right now. Implies `dot`. */
    live?: boolean;
    title?: string;
    testid?: string;
    /** The domain state key, exposed as `data-status` for tests and styling hooks. */
    status?: string;
    children?: Snippet;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  

  let { tone = 'neutral', variant = 'soft', label, dot = false, live = false, title, testid, status, children }: Props = $props();
;
async () => {

 { svelteHTML.createElement("span", {         "class":`chip badge ${tone}`,title,...__sveltets_2_empty({"data-tone":tone}),...__sveltets_2_empty({"data-status":status}),...__sveltets_2_empty({"data-testid":testid}),});variant === 'outline';
  if(dot || live){ { svelteHTML.createElement("span", {   "class":`badge-dot`,"aria-hidden":`true`,});live; }}if(children){;__sveltets_2_ensureSnippet(children());}else{ { svelteHTML.createElement("span", { "class":`badge-label`,});label; }}
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const Badge__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type Badge__SvelteComponent_ = ReturnType<typeof Badge__SvelteComponent_>;
/*Ωignore_endΩ*/export default Badge__SvelteComponent_;