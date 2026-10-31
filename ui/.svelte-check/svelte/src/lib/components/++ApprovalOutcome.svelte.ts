///<reference types="svelte" />
;// The decided half of the approval shape (patterns §5): one line, "Approved
// by Dana · 3m ago · “reason”". ApprovalActions swaps its buttons for this
// once a request is decided; use it directly wherever only the outcome is
// shown (a closed queue row, a decided card).

import RelTime from './RelTime.svelte';
;

  interface Props {
    /** `approved` / `denied`, or `expired` for a request nobody decided. */
    outcome: 'approved' | 'denied' | 'expired';
    by?: string | null;
    at?: string | number | null;
    /** The reason / note recorded with the decision. */
    note?: string | null;
    testid?: string;
  };function $$render() {

  
  
  
  
  

  let { outcome, by, at, note, testid }: Props = $props();

  const word = $derived(outcome === 'approved' ? 'Approved' : outcome === 'expired' ? 'Expired' : 'Denied');
;
async () => {

 { svelteHTML.createElement("div", {   "class":`decided`,...__sveltets_2_empty({"data-testid":testid}),});
   { svelteHTML.createElement("span", {});word;by ? ` by ${by}` : ''; }
  if(at != null && at !== ''){ { svelteHTML.createElement("span", { "aria-hidden":`true`,});  } { const $$_emiTleR1C = __sveltets_2_ensureComponent(RelTime); new $$_emiTleR1C({ target: __sveltets_2_any(), props: {  "iso":at,}});}}
  if(note){ { svelteHTML.createElement("span", { "aria-hidden":`true`,});  } { svelteHTML.createElement("span", { "class":`note`,}); note;  }}
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ApprovalOutcome__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ApprovalOutcome__SvelteComponent_ = ReturnType<typeof ApprovalOutcome__SvelteComponent_>;
/*Ωignore_endΩ*/export default ApprovalOutcome__SvelteComponent_;