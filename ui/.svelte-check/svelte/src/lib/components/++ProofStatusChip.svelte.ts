///<reference types="svelte" />
;// A colored pill for a proof pack's overall status (missing/partial/passed/
// failed/waived), with an optional risk score. `compact` shrinks it for the
// inline sidebar session-row use.

import { sentenceCase } from '../labels';
;

  interface Props {
    status: string;
    risk?: number;
    compact?: boolean;
  };;

  type Tone = 'ok' | 'bad' | 'warn' | 'dim' | 'neutral';;function $$render() {

  
  
  
  

  let { status, risk, compact = false }: Props = $props();

  const TONE: Record<string, Tone> = {
    passed: 'ok',
    failed: 'bad',
    partial: 'warn',
    missing: 'dim',
    waived: 'neutral',
  };
  const tone = $derived(TONE[status] ?? 'dim');
  const showRisk = $derived(typeof risk === 'number' && risk > 0);
;
async () => {

  { svelteHTML.createElement("span", {     "class":`proof-status ${tone}`,"title":showRisk ? `proof: ${status} · risk ${risk}` : `proof: ${status}`,});compact;
   { svelteHTML.createElement("span", { "class":`lbl`,});sentenceCase(status); }
  if(showRisk){ { svelteHTML.createElement("span", { "class":`risk`,}); risk; }}
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ProofStatusChip__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ProofStatusChip__SvelteComponent_ = ReturnType<typeof ProofStatusChip__SvelteComponent_>;
/*Ωignore_endΩ*/export default ProofStatusChip__SvelteComponent_;