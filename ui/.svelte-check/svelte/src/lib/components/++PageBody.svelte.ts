///<reference types="svelte" />
;// The scrolling body under a PageHeader. One content-width rule app-wide:
//
// • width="full"     — tool / dashboard / list pages use the whole pane.
// • width="readable" — text-heavy forms & settings-like pages cap their
//   column at --page-readable (1200px) and stay LEFT-aligned with the header
//   title (no centred island with dead margins on both sides).
//
// `padded={false}` for pages that manage their own inner layout (split panes).
// `fill` pins the content to the pane height (no page scroll) for bodies
// whose children scroll internally (split views, boards).

import type { Snippet } from 'svelte';
;

  interface Props {
    width?: 'full' | 'readable';
    padded?: boolean;
    fill?: boolean;
    class?: string;
    children: Snippet;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  

  let { width = 'full', padded = true, fill = false, class: klass = '', children }: Props = $props();
;
async () => {

 { svelteHTML.createElement("div", {   "class":`page-body ${klass}`,});padded;fill;
   { svelteHTML.createElement("div", {  "class":`page-body-inner`,});width === 'readable';
    ;__sveltets_2_ensureSnippet(children());
   }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const PageBody__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type PageBody__SvelteComponent_ = ReturnType<typeof PageBody__SvelteComponent_>;
/*Ωignore_endΩ*/export default PageBody__SvelteComponent_;