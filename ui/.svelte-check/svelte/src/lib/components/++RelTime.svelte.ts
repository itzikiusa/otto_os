///<reference types="svelte" />
;// A timestamp as people read it: relative ("3m ago", "in 14h") with the exact
// local time on hover and the machine value in `datetime`. Ticks with the
// shared clock (stores/now.svelte.ts). Never render a raw ISO string.

import { rel } from '../stores/now.svelte';
;

  interface Props {
    iso: string | number | null | undefined;
    /** Shown when `iso` is missing/unparseable. */
    fallback?: string;
    class?: string;
  };function $$render() {

  
  
  
  

  let { iso, fallback = '—', class: klass = '' }: Props = $props();

  const ms = $derived(iso == null || iso === '' ? NaN : typeof iso === 'number' ? iso : Date.parse(iso));
  const valid = $derived(Number.isFinite(ms) && ms > 0);
;
async () => {

if(valid){
   { svelteHTML.createElement("time", {     "class":klass,"datetime":new Date(ms).toISOString(),"title":new Date(ms).toLocaleString(),});rel(ms); }
}else{
   { svelteHTML.createElement("span", { "class":klass,});fallback; }
}
};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const RelTime__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type RelTime__SvelteComponent_ = ReturnType<typeof RelTime__SvelteComponent_>;
/*Ωignore_endΩ*/export default RelTime__SvelteComponent_;