///<reference types="svelte" />
;// Tab strip for the Browser module: one row per open BrowserTab. The store
// (`browser.svelte.ts`) owns the list — this is a thin, stateless renderer.
// Live updates (a tab created/navigated from another device) arrive via the
// `browser_tab_updated` WS event, which the store applies in place (see
// `browser.applyEvent` wired from `lib/events.svelte.ts`); no polling here.

import Icon from '../../lib/components/Icon.svelte';
import { browser } from '../../lib/stores/browser.svelte';
import { toastError } from '../../lib/toastError';

;type $$ComponentProps =  { onnew: () => void; inbar?: boolean };function $$render() {

  
  
  
  
  

  
  
  

  // inbar: rendered inside the PageHeader bar (no own border row / padding).
  let { onnew, inbar = false }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();

  function close(e: MouseEvent, id: string): void {
    e.stopPropagation();
    browser.closeTab(id).catch((err: unknown) => toastError('Couldn’t close the tab', err));
  }
;
async () => {


 { svelteHTML.createElement("div", {  "class":`strip`,});inbar;
   { svelteHTML.createElement("div", { "class":`tabs-scroll scroll-thin`,});
     for(let tab of __sveltets_2_ensureArray(browser.tabs)){tab.id;
    
     { svelteHTML.createElement("div", {  "class":`tab`,});tab.id === browser.activeId;
       { svelteHTML.createElement("button", {           "class":`tab-main`,"onclick":() => browser.select(tab.id),"onauxclick":(e) => { if (e.button === 1) close(e, tab.id); },"title":tab.title ? `${tab.title}\n${tab.url}` : tab.url,"aria-current":tab.id === browser.activeId ? 'page' : undefined,});
         { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`globe`,"size":12,}});}
         { svelteHTML.createElement("span", { "class":`title`,});tab.title || tab.url; }
       }
       { svelteHTML.createElement("button", {         "class":`close`,"onclick":(e) => close(e, tab.id),"aria-label":`Close tab ${tab.title || tab.url}`,"title":`Close tab ${tab.title || tab.url}`,});
         { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});}
       }
     }
  }
   }
   { svelteHTML.createElement("button", {       "class":`new`,"onclick":onnew,"title":`New tab`,"aria-label":`New tab`,});
     { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":13,}});}
   }
 }


};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const TabStrip__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type TabStrip__SvelteComponent_ = ReturnType<typeof TabStrip__SvelteComponent_>;
/*Ωignore_endΩ*/export default TabStrip__SvelteComponent_;