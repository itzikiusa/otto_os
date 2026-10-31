///<reference types="svelte" />
;// Scale sheet: replicas stepper (current value from `extra.desired`). Scaling
// to 0 goes through the typed-name confirm in `runAction`.

import { untrack } from 'svelte';
import Modal from '../../lib/components/Modal.svelte';
import Icon from '../../lib/components/Icon.svelte';
import type { K8sRow } from '../../lib/api/types';
;

  interface Props {
    row: K8sRow;
    kindLabel: string;
    onsubmit: (replicas: number) => void;
    onclose: () => void;
  };function $$render() {

  
  
  
  
  
  

  let { row, kindLabel, onsubmit, onclose }: Props = $props();

  const current = $derived(Number.parseInt(row.extra?.desired ?? '', 10));
  // Seeded once from the row the sheet opened with (the sheet is remounted per row).
  const seed = untrack(() => Number.parseInt(row.extra?.desired ?? '', 10));
  let replicas = $state(Number.isFinite(seed) ? seed : 1);
  const valid = $derived(Number.isInteger(replicas) && replicas >= 0 && replicas <= 1000);

  function submit(): void {
    if (!valid) return;
    onsubmit(replicas);
  }
;
async () => {

 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {    children:() => { return __sveltets_2_any(0); },"title":`Scale ${kindLabel}`,"width":420,onclose,footer:() => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("button", {   "class":`btn`,"onclick":onclose,});  }
     { svelteHTML.createElement("button", {     "class":`btn primary`,"disabled":!valid,"onclick":submit,});  }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
   { svelteHTML.createElement("div", { "class":`scale`,});
     { svelteHTML.createElement("div", { "class":`target mono`,});row.namespace ? `${row.namespace}/` : '';row.name; }
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`k8s-scale-n`,}); if(Number.isFinite(current)){ { svelteHTML.createElement("span", { "class":`dim`,}); current;  }} }
       { svelteHTML.createElement("div", { "class":`stepper`,});
         { svelteHTML.createElement("button", {       "class":`btn`,"onclick":() => (replicas = Math.max(0, replicas - 1)),"aria-label":`Fewer replicas`,"title":`Fewer replicas`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`minus`,"size":13,}});} }
         { svelteHTML.createElement("input", {              "id":`k8s-scale-n`,"class":`input mono`,"type":`number`,"min":`0`,"max":`1000`,"bind:value":replicas,"onkeydown":(e) => { if (e.key === 'Enter') submit(); },});/*Ωignore_startΩ*/() => replicas = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         { svelteHTML.createElement("button", {       "class":`btn`,"onclick":() => (replicas = Math.min(1000, replicas + 1)),"aria-label":`More replicas`,"title":`More replicas`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":13,}});} }
       }
      if(replicas === 0){ { svelteHTML.createElement("span", { "class":`hint danger`,});               }}
     }
   }
  
 Modal}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ScaleDialog__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ScaleDialog__SvelteComponent_ = ReturnType<typeof ScaleDialog__SvelteComponent_>;
/*Ωignore_endΩ*/export default ScaleDialog__SvelteComponent_;