///<reference types="svelte" />
;
  import { lazyComponent } from '../lazy-component.svelte';

  // ONE shared loader for every LazyTerminal: the xterm + WebGL chunk (~0.5 MB)
  // is fetched the first time any embed renders, then every later mount draws
  // in the same frame (lazy-component.svelte.ts).
  const TerminalLazy = lazyComponent(() => import('./Terminal.svelte'));
;;// Terminal.svelte, code-split. Pages that only show a terminal INSIDE a
// running-agent section (Docs agents, Refine drawer, Product analysis, mockup
// assist…) use this so xterm stays out of the page's static import set — a
// first visit pays for it only when an agent pane actually opens. Terminal-
// first hosts (SessionView, the share page, k8s exec) keep the static import.
//
// Same props as Terminal (forwarded verbatim) and the same exported methods
// (no-ops until the chunk has loaded). Every state is designed: a compact
// skeleton while loading, an inline error + Retry when the chunk fails.


import type { ComponentProps } from 'svelte';
import type Terminal from './Terminal.svelte';
import LoadState from './LoadState.svelte';
;

  type Props = ComponentProps<typeof Terminal>;;function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  

  let props: Props = $props();

  let inner = $state<ReturnType<typeof Terminal> | undefined>();

   function focus(): void {
    inner?.focus();
  }
   function redraw(): void {
    inner?.redraw();
  }
   function openFind(): void {
    inner?.openFind();
  }
;
async () => {



if(TerminalLazy.component){
  const T = TerminalLazy.component;
   { const $$_T0C = __sveltets_2_ensureComponent(T); const $$_T0 = new $$_T0C({ target: __sveltets_2_any(), props: {   ...props,}});inner = $$_T0;}
}else{
   { const $$_etatSdaoL0C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL0C({ target: __sveltets_2_any(), props: {            "what":`the terminal`,"variant":`panel`,"empty":true,"loading":!TerminalLazy.error,"error":TerminalLazy.error,"onretry":() => TerminalLazy.retry(),}});}
}
};
return { props: {} as any as Props, exports: {} as any as { focus: typeof focus,redraw: typeof redraw,openFind: typeof openFind }, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const LazyTerminal__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type LazyTerminal__SvelteComponent_ = ReturnType<typeof LazyTerminal__SvelteComponent_>;
/*Ωignore_endΩ*/export default LazyTerminal__SvelteComponent_;