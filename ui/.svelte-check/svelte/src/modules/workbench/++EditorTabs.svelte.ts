///<reference types="svelte" />
;// Open-file tab strip: click to switch, × / middle-click to close, a dot for
// unsaved edits. Closing flushes the buffer first (nothing is dropped).

import Icon from '../../lib/components/Icon.svelte';
import { workbench } from './workbench.svelte';
function $$render() {

  
  
  
  

  function nameOf(id: string): string {
    return workbench.open[id]?.doc?.name ?? workbench.metaOf(id)?.name ?? 'Loading file…';
  }

  function onAux(e: MouseEvent, id: string): void {
    if (e.button === 1) {
      e.preventDefault();
      workbench.closeTab(id);
    }
  }

  function onKey(e: KeyboardEvent): void {
    if (e.key === 'ArrowRight' || e.key === 'ArrowLeft') {
      e.preventDefault();
      // Logical direction: in RTL the visual right is "previous".
      const rtl = getComputedStyle(e.currentTarget as Element).direction === 'rtl';
      const fwd = (e.key === 'ArrowRight') !== rtl;
      workbench.cycle(fwd ? 1 : -1);
      queueMicrotask(() => {
        (document.querySelector('.wb-tab.active .wb-tab-main') as HTMLElement | null)?.focus();
      });
    }
  }
;
async () => {

if(workbench.tabs.length > 0){
   { svelteHTML.createElement("div", {         "class":`wb-tabs scroll-thin`,"role":`tablist`,"aria-label":`Open files`,"tabindex":-1,"onkeydown":onKey,});
       for(let id of __sveltets_2_ensureArray(workbench.tabs)){id;
      const active = workbench.active === id;
       { svelteHTML.createElement("div", {       "class":`wb-tab`,...__sveltets_2_empty({"data-testid":`wb-tab`}),...__sveltets_2_empty({"data-doc-id":id}),"onauxclick":(e) => onAux(e, id),});active;
         { svelteHTML.createElement("button", {             "class":`wb-tab-main`,"role":`tab`,"aria-selected":active,"tabindex":active ? 0 : -1,"title":nameOf(id),"onclick":() => workbench.openDoc(id),});
           { svelteHTML.createElement("span", { "class":`wb-tab-name`,});nameOf(id); }
          if(workbench.isDirty(id)){ { svelteHTML.createElement("span", {     "class":`wb-tab-dot`,"role":`img`,"aria-label":`Unsaved changes`,}); }}
         }
         { svelteHTML.createElement("button", {         "class":`wb-tab-close`,"aria-label":`Close ${nameOf(id)}`,"title":`Close tab (⌘W)`,"onclick":() => workbench.closeTab(id),});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});}
         }
       }
    }
   }
}


};
return { props: {} as Record<string, never>, exports: {}, bindings: "", slots: {}, events: {} }}
const EditorTabs__SvelteComponent_ = __sveltets_2_isomorphic_component(__sveltets_2_with_any_event($$render()));
/*Ωignore_startΩ*/type EditorTabs__SvelteComponent_ = InstanceType<typeof EditorTabs__SvelteComponent_>;
/*Ωignore_endΩ*/export default EditorTabs__SvelteComponent_;