///<reference types="svelte" />
;// Docked detail column for list/detail pages (AWS EC2/RDS, Kubernetes
// resources). Desktop: an inline column of `width` beside the content — not
// modal, Esc closes it unless an input or a stacked dialog owns the key.
// Phone (or `modal`): the shell Drawer as a full-width right sheet, which
// owns the modal plumbing (pushModal, focus trap, Esc, ✕). Both modes share
// one header row — the `title` heading, or the richer `head` snippet — so a
// caller never re-implements the drawer chrome.

import type { Snippet } from 'svelte';
import Drawer from '../../shell/Drawer.svelte';
import Icon from './Icon.svelte';
import { viewport } from '../stores/viewport.svelte';
import { dialogReturnTarget } from '../dialogFocus';
;

  interface Props {
    open: boolean;
    /** Heading and accessible name ("Instance details"). */
    title: string;
    onclose: () => void;
    /** Desktop column width (CSS length); capped at 55% of the row. */
    width?: string;
    /** Present as the modal sheet; defaults to phone width. */
    modal?: boolean;
    /** Richer header content (kind, name, status badge) in place of the title. */
    head?: Snippet;
    testid?: string;
    children: Snippet;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  

  let { open, title, onclose, width = '460px', modal, head, testid, children }: Props = $props();

  const sheet = $derived(modal ?? viewport.isPhone);

  // Desktop column focus return (the sheet's Drawer owns its own). Closing
  // from the ✕ or Esc unmounts the column with the focused control, which
  // dropped keyboard focus to <body> and lost the user's place in the list
  // (WCAG 2.4.3). Remember the control that opened the column — then the last
  // one focused outside it, so picking another row updates the target — and
  // hand focus back on close, only when focus was lost with the column.
  let column = $state<HTMLElement | null>(null);
  $effect(() => {
    if (!open || sheet) return;
    let opener = dialogReturnTarget();
    const onFocusIn = (e: FocusEvent): void => {
      const t = e.target;
      if (t instanceof HTMLElement && t !== document.body && !column?.contains(t)) opener = t;
    };
    document.addEventListener('focusin', onFocusIn);
    return () => {
      document.removeEventListener('focusin', onFocusIn);
      const target = opener;
      queueMicrotask(() => {
        const now = document.activeElement;
        if (target?.isConnected && (!now || now === document.body)) target.focus();
      });
    };
  });

  function onKey(e: KeyboardEvent): void {
    if (!open || sheet || e.key !== 'Escape' || e.defaultPrevented) return;
    const t = e.target as HTMLElement | null;
    if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.tagName === 'SELECT' || t.isContentEditable)) return;
    // A dialog stacked over the column (confirm, picker) owns its own Esc.
    if (document.querySelector('[aria-modal="true"]')) return;
    e.stopPropagation();
    onclose();
  }
;
async () => {

 { svelteHTML.createElement("svelte:window", {  "onkeydown":onKey,});}

if(sheet){
   { const $$_rewarD0C = __sveltets_2_ensureComponent(Drawer); new $$_rewarD0C({ target: __sveltets_2_any(), props: {       children:() => { return __sveltets_2_any(0); },open,"side":`right`,title,head,"width":`100vw`,onclose,}});
     { svelteHTML.createElement("div", {   "class":`dd-body`,...__sveltets_2_empty({"data-testid":testid}),});;__sveltets_2_ensureSnippet(children()); }
   Drawer}
} else if (open){
   { const $$_aside0 = svelteHTML.createElement("aside", {        "class":`docked`,"aria-label":title,...__sveltets_2_empty({"data-testid":testid}),});column = $$_aside0;__sveltets_2_ensureType(String, Number, width);
     { svelteHTML.createElement("header", { "class":`dd-head`,});
      if(head){ { svelteHTML.createElement("div", { "class":`dd-head-main`,});;__sveltets_2_ensureSnippet(head()); }}else{ { svelteHTML.createElement("h2", {});title; }}
       { svelteHTML.createElement("button", {         "class":`icon-btn`,"onclick":onclose,"aria-label":`Close ${title}`,"title":`Close ${title}`,"aria-keyshortcuts":`Escape`,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":14,}});}
       }
     }
     { svelteHTML.createElement("div", { "class":`dd-body`,});;__sveltets_2_ensureSnippet(children()); }
   }
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const DockedDrawer__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type DockedDrawer__SvelteComponent_ = ReturnType<typeof DockedDrawer__SvelteComponent_>;
/*Ωignore_endΩ*/export default DockedDrawer__SvelteComponent_;