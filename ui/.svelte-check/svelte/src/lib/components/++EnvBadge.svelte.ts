///<reference types="svelte" />
;// Environment badge — the one treatment for a connection / cluster / account
// environment everywhere (DB Explorer, Brokers, Kubernetes, AWS): prod in the
// danger tone, staging in warning, dev neutral (lib/status.ts `envTone`).
// `readOnly` on a non-prod target shows "RO" instead (prod stays "prod" —
// prod is the stronger guard). Text is ≥ --fs-xs, weight 600.

import { envTone } from '../status';
;

  interface Props {
    env: string | null | undefined;
    readOnly?: boolean;
    /** Override the label (e.g. "…" while opening). */
    label?: string;
  };function $$render() {

  
  
  
  
  
  

  let { env, readOnly = false, label }: Props = $props();

  const info = $derived(envTone(env));
  const ro = $derived(readOnly && info.key !== 'prod');
  const text = $derived(label ?? (ro ? 'RO' : info.label));
  const tip = $derived(ro ? `Read-only${info.hint ? ` · ${info.hint}` : ''}` : (info.hint ?? info.label));
;
async () => {

  { svelteHTML.createElement("span", {      "class":`env-badge tone-${ro ? 'neutral' : info.tone}`,...__sveltets_2_empty({"data-env":info.key}),"title":tip,});text; }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const EnvBadge__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type EnvBadge__SvelteComponent_ = ReturnType<typeof EnvBadge__SvelteComponent_>;
/*Ωignore_endΩ*/export default EnvBadge__SvelteComponent_;