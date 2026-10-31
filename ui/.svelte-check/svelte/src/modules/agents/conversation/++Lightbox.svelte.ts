///<reference types="svelte" />
;// Full-window image viewer for transcript images. Esc / click-outside closes;
// dialogFocus traps Tab inside and returns focus to the thumbnail on close.

import { untrack } from 'svelte';
import Icon from '../../../lib/components/Icon.svelte';
import { ui } from '../../../lib/stores/ui.svelte';
import { dialogFocus } from '../../../lib/dialogFocus';
;
  interface Props {
    src: string;
    alt: string;
    onclose: () => void;
  };function $$render() {

  
  
  
  
  
  

  let { src, alt, onclose }: Props = $props();

  // A full-window overlay on the Modal layer: register it so the native
  // browser webview hides under it (untracked: pushModal reads the counter it
  // bumps — see Modal.svelte).
  $effect(() => {
    untrack(() => ui.pushModal());
    return () => untrack(() => ui.popModal());
  });
;
async () => {

 {const $$action_0 = __sveltets_2_ensureAction(dialogFocus(svelteHTML.mapElementTag('div'),(onclose)));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {         "class":`lb`,"role":`dialog`,"aria-modal":`true`,"aria-label":alt || 'Image',});
   { svelteHTML.createElement("div", {     "class":`lb-backdrop`,"role":`presentation`,"onclick":onclose,}); }
   { svelteHTML.createElement("button", {         "class":`lb-close icon-btn`,"aria-label":`Close`,"title":`Close`,"aria-keyshortcuts":`Escape`,"onclick":onclose,}); { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":16,}});} }
   { svelteHTML.createElement("img", {  src,alt,});}
 }}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const Lightbox__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type Lightbox__SvelteComponent_ = ReturnType<typeof Lightbox__SvelteComponent_>;
/*Ωignore_endΩ*/export default Lightbox__SvelteComponent_;