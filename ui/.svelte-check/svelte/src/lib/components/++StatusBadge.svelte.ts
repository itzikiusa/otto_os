///<reference types="svelte" />
;// The shared status badge: a tone dot + a sentence-case word (colour never
// carries meaning alone). Feed it a `StatusInfo` from lib/status.ts —
// `runStatus(raw)` for runs/steps/jobs, `sessionState(...)` for sessions —
// or a bare `tone` + `label`. `variant="pill"` (default) is the soft-tinted
// pill; `variant="text"` is the same dot + word without the tint, for dense
// list rows. Tones use the text-safe semantic tokens, so the label passes AA.
// The pill IS the shared Badge (one pill system); only the text variant is
// drawn here.

import { badgeTone, type StatusInfo, type Tone } from '../status';
import Badge from './Badge.svelte';
;

  interface Props {
    status?: StatusInfo;
    tone?: Tone;
    label?: string;
    variant?: 'pill' | 'text';
    /** Hide the dot (e.g. the row already draws one). */
    dot?: boolean;
    title?: string;
    testid?: string;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  

  let { status, tone, label, variant = 'pill', dot = true, title, testid }: Props = $props();

  const t = $derived<Tone>(tone ?? status?.tone ?? 'neutral');
  const text = $derived(label ?? status?.label ?? '');
  const live = $derived(status?.live === true);
;
async () => {

if(variant === 'pill'){
   { const $$_egdaB0C = __sveltets_2_ensureComponent(Badge); new $$_egdaB0C({ target: __sveltets_2_any(), props: {            "tone":badgeTone(t),"label":text,dot,"live":dot && live,"status":status?.key,testid,"title":title ?? status?.hint,}});}
}else{
   { svelteHTML.createElement("span", {         "class":`sbadge tone-${t}`,...__sveltets_2_empty({"data-status":status?.key}),...__sveltets_2_empty({"data-testid":testid}),"title":title ?? status?.hint,});
    if(dot){ { svelteHTML.createElement("span", {   "class":`sbadge-dot`,"aria-hidden":`true`,});live; }}text;
   }
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const StatusBadge__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type StatusBadge__SvelteComponent_ = ReturnType<typeof StatusBadge__SvelteComponent_>;
/*Ωignore_endΩ*/export default StatusBadge__SvelteComponent_;