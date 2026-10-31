///<reference types="svelte" />
;// The frame every in-thread action card shares (reminder, task, browser,
// approval): an icon + bold heading + dim one-line summary header with a state
// pill at the trailing edge, a body, and an optional footer well for its
// actions. Hairline border, no shadow (foundations §5).

import type { Snippet } from 'svelte';
import Icon, { type IconName } from '../../../lib/components/Icon.svelte';
;

  interface Props {
    icon: IconName;
    /** The card's bold heading — already human words ("Reminder", "Approval
     *  needed"; map wire kinds through `TASK_KIND`), never a raw enum. */
    heading: string;
    summary?: string;
    /** Attention ring for a card that is waiting on you. */
    attention?: boolean;
    pill?: Snippet;
    children?: Snippet;
    footer?: Snippet;
    testid?: string;
  };function $$render() {

  
  
  
  
  
  

  let { icon, heading, summary, attention = false, pill, children, footer, testid }: Props = $props();
;
async () => {

 { svelteHTML.createElement("section", {     "class":`card`,...__sveltets_2_empty({"data-testid":testid}),"aria-label":summary ? `${heading}: ${summary}` : heading,});attention;
   { svelteHTML.createElement("header", { "class":`head`,});
     { svelteHTML.createElement("span", {   "class":`ic`,"aria-hidden":`true`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":icon,"size":14,}});} }
     { svelteHTML.createElement("strong", { "class":`heading`,});heading; }
    if(summary){ { svelteHTML.createElement("span", {   "class":`summary`,"title":summary,});summary; }}
    if(pill){ { svelteHTML.createElement("span", { "class":`pill-slot`,});;__sveltets_2_ensureSnippet(pill()); }}
   }
  if(children){ { svelteHTML.createElement("div", { "class":`body`,});;__sveltets_2_ensureSnippet(children()); }}
  if(footer){ { svelteHTML.createElement("footer", { "class":`foot`,});;__sveltets_2_ensureSnippet(footer()); }}
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ActionCard__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ActionCard__SvelteComponent_ = ReturnType<typeof ActionCard__SvelteComponent_>;
/*Ωignore_endΩ*/export default ActionCard__SvelteComponent_;