///<reference types="svelte" />
;// On-screen Back / Forward buttons wired to the hash router's history stack.
// Rendered in the mobile top bar (App.svelte) so touch users can navigate
// without the browser chrome. On desktop the router already exposes ⌘⇧←/→
// keyboard shortcuts (keys.ts → App.svelte navBack/navForward actions), so
// these buttons are optional there; they are included but can be placed
// anywhere in the shell layout.

import Icon from '../lib/components/Icon.svelte';
import { router } from '../lib/router.svelte';
function $$render() {

  
  
  
  
  
  
  
  
;
async () => {

 { svelteHTML.createElement("div", {     "class":`nav-btns`,"role":`group`,"aria-label":`Navigation history`,});
   { svelteHTML.createElement("button", {             "class":`nav-btn`,"onclick":() => router.back(),"disabled":!router.canBack,"aria-label":`Go back`,"title":`Go back`,"aria-keyshortcuts":`Meta+Shift+ArrowLeft`,});
     { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`chevronLeft`,"size":16,}});}
   }
   { svelteHTML.createElement("button", {             "class":`nav-btn`,"onclick":() => router.forward(),"disabled":!router.canForward,"aria-label":`Go forward`,"title":`Go forward`,"aria-keyshortcuts":`Meta+Shift+ArrowRight`,});
     { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`chevronRight`,"size":16,}});}
   }
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: "", slots: {}, events: {} }}
const NavButtons__SvelteComponent_ = __sveltets_2_isomorphic_component(__sveltets_2_with_any_event($$render()));
/*Ωignore_startΩ*/type NavButtons__SvelteComponent_ = InstanceType<typeof NavButtons__SvelteComponent_>;
/*Ωignore_endΩ*/export default NavButtons__SvelteComponent_;