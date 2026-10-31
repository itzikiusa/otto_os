///<reference types="svelte" />
;// The side-by-side pane's controls, drawn at the trailing end of the pane's
// own top row (PageHeader / the Agents TabBar — lib/stores/embedChrome).
// Browser controls ask the iframe host; native controls are also available
// on the primary pane and operate on the authenticated native pair.

import Icon from './Icon.svelte';
import { postToHost } from '../embedGuest';
import { isTauri } from '../desktop';
import { ctxMenu } from '../contextmenu.svelte';
import { sidePane } from '../stores/sidePane.svelte';
import { observeNativePane, type PaneState } from '../nativePane';
import { paneWindowMenuItems } from '../nativePaneMenu';

;type $$ComponentProps =  { pane?: 'side' | 'primary' };function $$render() {

  
  
  
  
  
  
  
  
  
  
  

  let { pane = 'side' }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();
  let nativeState: PaneState | null = $state(null);
  $effect(() => isTauri ? observeNativePane((state) => { nativeState = state; }) : undefined);
  function showWindowMenu(event: MouseEvent): void {
    const items = paneWindowMenuItems();
    if (nativeState?.mode === 'attached') {
      items.push({ separator: true },
        { label: 'Swap panes', icon: 'swap', action: () => pane === 'side' ? postToHost({ type: 'swap' }) : sidePane.swap() },
        { label: 'Open in main pane', icon: 'maximize', action: () => pane === 'side' ? postToHost({ type: 'promote' }) : sidePane.promote() },
        { label: 'Close side pane', icon: 'x', action: () => pane === 'side' ? postToHost({ type: 'close' }) : sidePane.close() },
      );
    }
    ctxMenu.show(event, items);
  }
;
async () => {

 { svelteHTML.createElement("div", {       "class":`pane-controls`,"role":`group`,"aria-label":pane === 'side' ? 'Side pane' : 'Main pane',...__sveltets_2_empty({"data-testid":`pane-controls`}),});
   { svelteHTML.createElement("span", {   "class":`pc-sep`,"aria-hidden":`true`,}); }
  if(isTauri){
     { svelteHTML.createElement("button", {             "class":`icon-btn`,"onclick":showWindowMenu,"aria-label":`Pane window`,"title":`Pane window`,...__sveltets_2_empty({"data-testid":`pane-window-menu`}),"aria-haspopup":`menu`,"disabled":!nativeState,});
       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":14,}});}
     }
  }else{
   { svelteHTML.createElement("button", {           "class":`icon-btn`,"onclick":() => postToHost({ type: 'swap' }),"aria-label":`Swap panes`,"title":`Swap panes`,...__sveltets_2_empty({"data-testid":`side-pane-swap`}),});
     { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`swap`,"size":14,}});}
   }
   { svelteHTML.createElement("button", {           "class":`icon-btn`,"onclick":() => postToHost({ type: 'promote' }),"aria-label":`Open in main pane`,"title":`Open in main pane`,...__sveltets_2_empty({"data-testid":`side-pane-promote`}),});
     { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`maximize`,"size":14,}});}
   }
   { svelteHTML.createElement("button", {             "class":`icon-btn`,"onclick":() => postToHost({ type: 'close' }),"aria-label":`Close side pane`,"title":`Close side pane`,"aria-keyshortcuts":`Meta+\\`,...__sveltets_2_empty({"data-testid":`side-pane-close`}),});
     { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":14,}});}
   }
  }
 }


};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const PaneControls__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type PaneControls__SvelteComponent_ = ReturnType<typeof PaneControls__SvelteComponent_>;
/*Ωignore_endΩ*/export default PaneControls__SvelteComponent_;