///<reference types="svelte" />
;// The dim intro paragraph at the top of a Settings section body. It holds
// the rules that don't fit a PageHeader subtitle: subtitles are one short
// line, ellipsized when space runs out and hidden on phone, so anything a
// user must know (where secrets live, what applies when, who can do what)
// belongs here instead.

import type { Snippet } from 'svelte';

;type $$ComponentProps =  { children: Snippet };function $$render() {

  
  
  
  
  
  

  let { children }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();
;
async () => {

 { svelteHTML.createElement("p", { "class":`settings-intro`,});;__sveltets_2_ensureSnippet(children()); }


};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const SectionIntro__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type SectionIntro__SvelteComponent_ = ReturnType<typeof SectionIntro__SvelteComponent_>;
/*Ωignore_endΩ*/export default SectionIntro__SvelteComponent_;