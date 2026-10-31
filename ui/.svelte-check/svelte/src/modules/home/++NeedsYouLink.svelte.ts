///<reference types="svelte" />
;// The ONE "Needs you" inbox (S20-07). Home → Today → Needs you is canonical:
// it is the only surface that joins every source — assistant tasks, MCP
// approvals, sessions waiting on input, work items awaiting approval and
// unread warnings. The narrower surfaces (Assistant rail, Agents → Work
// Queue, Mission Control, MCP approvals) keep their own lists but show the
// SAME total from the same store (`today.needs`) and link here, so a user
// learns there is one place that is never missing anything.

import { onDestroy, onMount } from 'svelte';
import Icon from '../../lib/components/Icon.svelte';
import { router } from '../../lib/router.svelte';
import { today } from './today.svelte';
import { needsYouLinkLabel } from './needsYou';
;

  interface Props {
    /** Extra class for placement tweaks by the host surface. */
    class?: string;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  

  let { class: cls = '' }: Props = $props();

  // Ref-counted: the Today store polls (event-fed) only while something reads it.
  onMount(() => today.start());
  onDestroy(() => today.stop());

  const count = $derived(today.needs.length);
  const label = $derived(needsYouLinkLabel(count));
;
async () => {

  { svelteHTML.createElement("button", {           "type":`button`,"class":`nyl ${cls}`,...__sveltets_2_empty({"data-testid":`needs-you-inbox-link`}),"title":`Everything waiting on you, from every part of Otto, is on Home → Needs you`,"onclick":() => router.go('home'),});count > 0;
   { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`bell`,"size":12,}});}
   { svelteHTML.createElement("span", {});label; }
   { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`arrow`,"size":12,}});}
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const NeedsYouLink__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type NeedsYouLink__SvelteComponent_ = ReturnType<typeof NeedsYouLink__SvelteComponent_>;
/*Ωignore_endΩ*/export default NeedsYouLink__SvelteComponent_;