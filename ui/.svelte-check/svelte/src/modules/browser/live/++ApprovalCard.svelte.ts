///<reference types="svelte" />
;
import { announce } from '../../../lib/announce.svelte';
// An agent's outward action held for a person, drawn over the live frame
// (plan §4.6, mockup panel 1). While an agent drives, the daemon holds any
// state-changing document request (a form submit / non-GET navigation),
// captures a screenshot and files an MCP approval (`kind:"browser_action"`);
// the live socket announces it with an `approval` frame.
//
// The one approval shape (docs/design/guidelines/patterns.md §5): WHO asked,
// WHAT (where / what / who sees it), WHY, WHEN — then the shared
// ApprovalActions: Take over · Deny… (DenySheet, optional reason that goes
// back to the agent) · Approve (primary). Every button is an explicit
// decision recorded in the MCP approvals queue; nothing is decided silently.
//
// Not a Modal: it parks only the agent, not the rest of Otto, so it sits in
// the live pane and the user can still switch tabs or read the page.
import AgentChip from '../../../lib/components/AgentChip.svelte';
import ApprovalActions from '../../../lib/components/ApprovalActions.svelte';
import Icon from '../../../lib/components/Icon.svelte';
import { rel } from '../../../lib/stores/now.svelte';
import type { McpApproval } from '../../../lib/api/types';
import { approvalFacts, type ApprovalChoice } from './approvalFacts';
;

  interface Props {
    /** The approval id + title from the `approval` frame. */
    id: string;
    title: string;
    /** Full queue row, when this viewer can read the queue (mcp:view). */
    detail: McpApproval | null;
    /** The live session's profile (the "As" line). */
    profile: string | null;
    /** `reason` is only set for a deny that came through the DenySheet. */
    ondecide: (choice: ApprovalChoice, reason?: string | null) => void;
    /** A decision is in flight (buttons disabled). */
    busy?: boolean;
    /** Why the last decision failed, inline. */
    error?: string;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { id, title, detail, profile, ondecide, busy = false, error = '' }: Props = $props();

  let cardEl = $state<HTMLElement | null>(null);

  // Move focus to the card (not onto Approve — an Enter meant for the page
  // must never approve by accident) so screen readers announce it and the
  // keyboard lands next to the decisions.
  $effect(() => {
    const _id = id;
    cardEl?.focus({ preventScroll: true });
  });

  // Which decision is in flight, so the busy label names it ("Denying…").
  let inflight = $state<'approve' | 'deny'>('approve');
  const facts = $derived(approvalFacts(detail, profile));
  const titleId = $derived(`approval-title-${id}`);
  // Spoken at the state edge through the shell's always-mounted announcer: a
  // live region inserted together with its text is not read (WebKit/VoiceOver).
  $effect(() => { if (error) announce(error, { assertive: true }); });
;
async () => {

  { const $$_div0 = svelteHTML.createElement("div", {             "class":`approval`,"role":`alertdialog`,"aria-modal":`false`,"aria-labelledby":titleId,"tabindex":-1,...__sveltets_2_empty({"data-testid":`live-approval`}),});cardEl = $$_div0;
   { svelteHTML.createElement("div", { "class":`head`,});
     { svelteHTML.createElement("span", {   "class":`tile`,"aria-hidden":`true`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":16,}});} }
     { svelteHTML.createElement("div", { "class":`head-text`,});
       { svelteHTML.createElement("h2", { "id":titleId,});title; }
       { svelteHTML.createElement("p", { "class":`summary`,});
                         
       }
     }
   }

  if(facts.rows.length){
     { svelteHTML.createElement("dl", { "class":`facts`,});
         for(let row of __sveltets_2_ensureArray(facts.rows)){row.label;
         { svelteHTML.createElement("dt", {});row.label; }
         { svelteHTML.createElement("dd", {  "dir":row.ltr ? 'ltr' : undefined,});row.ltr;row.value; }
      }
     }
  }

  if(facts.why){
     { svelteHTML.createElement("p", { "class":`why`,}); { svelteHTML.createElement("span", { "class":`dim`,});  } facts.why; }
  }

  if(facts.screenshot){
     { svelteHTML.createElement("p", { "class":`shot`,});
       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`image`,"size":14,}});}
       { svelteHTML.createElement("span", {});            }
     }
  }

   { svelteHTML.createElement("p", { "class":`who`,});
     { const $$_pihCtnegA2C = __sveltets_2_ensureComponent(AgentChip); new $$_pihCtnegA2C({ target: __sveltets_2_any(), props: {}});}
     { svelteHTML.createElement("span", { "title":detail?.requested_by ?? undefined,});facts.requester; }
    if(detail?.created_at){
       { svelteHTML.createElement("span", { "aria-hidden":`true`,});  }
       { svelteHTML.createElement("time", {   "datetime":detail.created_at,"title":new Date(detail.created_at).toLocaleString(),});rel(detail.created_at); }
    }
   }

  if(error){
     { svelteHTML.createElement("p", { "class":`error`,});error; }
  }

   { svelteHTML.createElement("div", { "class":`actions`,});
     { const $$_snoitcAlavorppA2C = __sveltets_2_ensureComponent(ApprovalActions); const $$_snoitcAlavorppA2 = new $$_snoitcAlavorppA2C({ target: __sveltets_2_any(), props: {             "size":`normal`,"busy":busy ? inflight : null,"onapprove":() => {
        inflight = 'approve';
        ondecide('approve');
      },"ondeny":(reason) => {
        inflight = 'deny';
        ondecide('deny', reason);
      },"denyTarget":`the agent’s request`,"denyTitle":`Deny the request`,extra:() => { async ()/*Ωignore_positionΩ*/ => {
         { svelteHTML.createElement("button", {         "class":`btn`,"disabled":busy,"onclick":() => ondecide('take_over'),"title":`Deny the agent’s request and drive the page yourself`,});  }
      };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {extra} = $$_snoitcAlavorppA2.$$prop_def;/*Ωignore_endΩ*/
      
     ApprovalActions}
   }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ApprovalCard__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ApprovalCard__SvelteComponent_ = ReturnType<typeof ApprovalCard__SvelteComponent_>;
/*Ωignore_endΩ*/export default ApprovalCard__SvelteComponent_;