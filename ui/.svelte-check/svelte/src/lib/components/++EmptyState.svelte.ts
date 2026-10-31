///<reference types="svelte" />
;// The one empty-state pattern: icon tile + title + one-line body + ONE
// primary CTA.
//
// • variant="page"  — a whole page / main pane has nothing to show. Sits at a
//   fixed top offset (≈15vh), NOT vertically centred, so every module's empty
//   page reads the same regardless of pane height.
// • variant="panel" (default) — inside a pane/card/list; compact padding, the
//   parent decides placement (kept for the ~130 existing in-panel uses).
//
// `children` is an escape hatch for a secondary link/hint under the CTA —
// keep it quiet (a `.btn ghost` or dim text), never a second primary.

import type { Snippet } from 'svelte';
import Icon, { type IconName } from './Icon.svelte';
;

  interface Props {
    icon?: IconName;
    title: string;
    body?: string;
    actionLabel?: string;
    actionIcon?: IconName;
    onaction?: () => void;
    /** 'secondary' inside a card / widget / pane that already has (or sits
     *  beside) the view's one primary action. */
    actionKind?: 'primary' | 'secondary';
    variant?: 'page' | 'panel';
    /** 'error' marks a failed state: danger icon tile + role="alert". */
    tone?: 'neutral' | 'error';
    /** Heading level of the title, so it nests under the surrounding outline:
     *  2 for a page-level empty state under the page's h1, 4 inside a card
     *  under an h3. Default 3. Looks the same at every level. */
    headingLevel?: 2 | 3 | 4 | 5;
    children?: Snippet;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  

  let { icon = 'box', title, body, actionLabel, actionIcon, onaction, actionKind = 'primary', variant = 'panel', tone = 'neutral', headingLevel = 3, children }: Props = $props();

  // A label without a handler would render a dead button, so the CTA needs
  // both — and a caller that passes only the label is told, not silently
  // left without its action.
  $effect(() => {
    if (import.meta.env.DEV && actionLabel && !onaction) {
      console.warn(`EmptyState “${title}”: actionLabel “${actionLabel}” has no onaction — the CTA is not rendered.`);
    }
  });
;
async () => {

 { svelteHTML.createElement("div", {      "class":`empty`,"role":tone === 'error' ? 'alert' : undefined,...__sveltets_2_empty({"data-testid":variant === 'page' ? 'page-empty' : undefined}),});variant === 'page';
  
   { svelteHTML.createElement("div", {    "class":`empty-icon`,"aria-hidden":`true`,});tone === 'error'; { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":icon,"size":variant === 'page' ? 26 : 24,}});} }
   { svelteHTML.createElement(`h${headingLevel}`, {   "class":`empty-title`,});title; }
  if(body){ { svelteHTML.createElement("p", {});body; }}
  if(actionLabel && onaction){
     { svelteHTML.createElement("button", {    "class":`btn`,"onclick":onaction,});actionKind === 'primary';
      if(actionIcon){ { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":actionIcon,"size":13,}});}}
      actionLabel;
     }
  }
  if(children){;__sveltets_2_ensureSnippet(children());}
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const EmptyState__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type EmptyState__SvelteComponent_ = ReturnType<typeof EmptyState__SvelteComponent_>;
/*Ωignore_endΩ*/export default EmptyState__SvelteComponent_;