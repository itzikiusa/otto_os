///<reference types="svelte" />
;// Reusable off-canvas slide-over used by the mobile shell to host the
// Navigator (left) and the RightPanel (right). Open-state is a bound prop so
// callers can wire it to an existing store flag (e.g. ui.navDrawerOpen /
// ui.rightOpen). A backdrop fades in behind the panel; tapping it — or
// pressing Esc — dismisses. Body content is provided via a snippet.
//
// `inline` presents the same mounted children in the desktop layout. This
// lets draft-bearing panels cross a breakpoint without destroying state.

import { tick, untrack, type Snippet } from 'svelte';
import { dialogFocus } from '../lib/dialogFocus';
import { ui } from '../lib/stores/ui.svelte';
import Icon from '../lib/components/Icon.svelte';
;

  interface Props {
    /** Bound: whether the drawer is shown. */
    open: boolean;
    /** Present children in normal flow, without modal behavior. */
    inline?: boolean;
    /** Which edge the panel slides in from. */
    side?: 'left' | 'right';
    /** Accessible label for the dialog. */
    label?: string;
    /** A visible header (title + ✕) above a scrolling body, instead of the
     *  floating ✕ over the content — for drawers that host a long form/list. */
    title?: string;
    /** Panel width (CSS length). Defaults to a touch-friendly, viewport-capped value. */
    width?: string;
    /** Called after the drawer dismisses itself (✕, backdrop, Esc). */
    onclose?: () => void;
    /** Richer header content in place of the plain `title` heading (titled drawers only). */
    head?: Snippet;
    children: Snippet;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  


  let {
    open = $bindable(),
    inline = false,
    side = 'left',
    label = 'Panel',
    title,
    width = 'min(86vw, 320px)',
    onclose,
    head,
    children,
  }: Props = $props()/*Ωignore_startΩ*/;open;/*Ωignore_endΩ*/;

  function close(): void {
    open = false;
    onclose?.();
  }

  let panel: HTMLDivElement | undefined = $state();
  // Once shown inline, keep its state when the mobile drawer is closed too.
  let kept = $state(false);
  $effect(() => { if (inline) kept = true; });
  $effect.pre(() => {
    void inline;
    // Changing display:contents into the overlay box can blur a descendant.
    // Restore only the control already focused here before the layout changed.
    const focused = document.activeElement;
    if (focused instanceof HTMLElement && panel?.contains(focused)) {
      void tick().then(() => { if (focused.isConnected && (inline || open)) focused.focus(); });
    }
  });
  // Register as an open overlay (like Modal) so the native browser webview and
  // global shortcuts treat an open drawer as modal. untrack: pushModal reads
  // modalCount — the effect must depend only on `inline`/`open`.
  $effect(() => {
    if (!inline && open) {
      untrack(() => ui.pushModal());
      return () => untrack(() => ui.popModal());
    }
  });
  $effect(() => {
    if (!inline && open && panel) {
      const focus = dialogFocus(panel, close);
      return () => focus.destroy();
    }
  });
;
async () => {

if(open && !inline){
  
   { svelteHTML.createElement("div", {     "class":`drawer-backdrop`,"role":`presentation`,"onclick":close,}); }
}
if(open || inline || kept){
   { const $$_div0 = svelteHTML.createElement("div", {              "class":`drawer ${side}`,"hidden":!inline && !open,"role":inline ? undefined : 'dialog',"aria-modal":inline ? undefined : true,"aria-label":inline ? undefined : (title ?? label),});inline;panel = $$_div0;__sveltets_2_ensureType(String, Number, inline ? undefined : width);
    
    if(!inline && title){
       { svelteHTML.createElement("header", { "class":`drawer-head`,});
        if(head){ { svelteHTML.createElement("div", { "class":`drawer-head-main`,});;__sveltets_2_ensureSnippet(head()); }}else{ { svelteHTML.createElement("h2", {});title; }}
         { svelteHTML.createElement("button", {         "class":`icon-btn`,"onclick":close,"aria-label":`Close ${title}`,"title":`Close ${title}`,"aria-keyshortcuts":`Escape`,});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":14,}});}
         }
       }
       { svelteHTML.createElement("div", { "class":`drawer-body`,});;__sveltets_2_ensureSnippet(children()); }
    }else{
      if(!inline){
         { svelteHTML.createElement("button", {         "class":`icon-btn drawer-close`,"onclick":close,"aria-label":`Close ${label}`,"title":`Close ${label}`,"aria-keyshortcuts":`Escape`,});
           { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":14,}});}
         }
      }
      ;__sveltets_2_ensureSnippet(children());
    }
   }
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings('open'), slots: {}, events: {} }}
const Drawer__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type Drawer__SvelteComponent_ = ReturnType<typeof Drawer__SvelteComponent_>;
/*Ωignore_endΩ*/export default Drawer__SvelteComponent_;