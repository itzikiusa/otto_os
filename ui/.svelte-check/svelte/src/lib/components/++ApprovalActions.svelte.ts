///<reference types="svelte" />
;// The one approval shape (patterns §5) for every queue — MCP approvals, work
// item gates, Run-with-Otto, self-improvement edits: "Deny…" (opens the
// DenySheet for an optional reason) beside a primary "Approve", busy labels
// "Approving…" / "Denying…", and — once decided — the outcome line
// "Approved by X · 3m ago". Vocabulary is Approve / Deny everywhere; the
// caller wires the actual API call.

import type { Snippet } from 'svelte';
import DenySheet from './DenySheet.svelte';
import ApprovalOutcome from './ApprovalOutcome.svelte';
;

  interface Decided {
    /** `approved`, `expired`, or anything else (rendered as Denied). */
    outcome: 'approved' | 'denied' | 'expired';
    by?: string | null;
    at?: string | number | null;
    /** The reason / note recorded with the decision. */
    note?: string | null;
  };;
  interface Props {
    /** Fired on Approve. */
    onapprove: () => void | Promise<void>;
    /** Fired when the DenySheet is confirmed; `reason` is null when left empty. */
    ondeny: (reason: string | null) => void | Promise<void>;
    /** Which call is in flight — drives the busy label and disables both. */
    busy?: 'approve' | 'deny' | null;
    /** Disable both buttons (e.g. no permission); explain it in `disabledReason`. */
    disabled?: boolean;
    disabledReason?: string;
    /** Primary verb; defaults to "Approve". */
    approveLabel?: string;
    /** Busy label for the primary; defaults to "Approving…". Name the verb
     *  when approveLabel is not "Approve" ("Sending…"). */
    approveBusyLabel?: string;
    /** What a denial stops, for the sheet hint ("the PR draft", "this edit"). */
    denyTarget?: string;
    /** The sheet's hint in full, when a denial has a consequence to state
     *  ("Deny removes the run's worktree."); overrides the denyTarget default. */
    denyHint?: string;
    denyTitle?: string;
    /** Ask for an optional reason via the DenySheet (default). Pass false when
     *  the endpoint has nowhere to record one — Deny then fires immediately
     *  rather than collecting text that would be dropped. */
    askReason?: boolean;
    /** Set once decided: replaces the buttons with the outcome line. */
    decided?: Decided | null;
    /** Extra actions rendered before Deny (e.g. "Approve & always allow…"). */
    extra?: Snippet;
    size?: 'small' | 'normal';
    testid?: string;
  };function $$render() {

  
  
  
  
  
  
  
  
  


  let {
    onapprove,
    ondeny,
    busy = null,
    disabled = false,
    disabledReason,
    approveLabel = 'Approve',
    approveBusyLabel = 'Approving…',
    denyTarget,
    denyHint,
    denyTitle = 'Deny request',
    askReason = true,
    decided = null,
    extra,
    size = 'small',
    testid,
  }: Props = $props();

  let denying = $state(false);
  const off = $derived(disabled || busy !== null);

  function confirmDeny(reason: string | null): void {
    denying = false;
    void ondeny(reason);
  }
;
async () => {

if(decided){
   { const $$_emoctuOlavorppA0C = __sveltets_2_ensureComponent(ApprovalOutcome); new $$_emoctuOlavorppA0C({ target: __sveltets_2_any(), props: {         "outcome":decided.outcome,"by":decided.by,"at":decided.at,"note":decided.note,testid,}});}
}else{
   { svelteHTML.createElement("div", {   "class":`actions`,...__sveltets_2_empty({"data-testid":testid}),});
    ;__sveltets_2_ensureSnippet(extra?.());
     { svelteHTML.createElement("button", {          "class":`btn`,"disabled":off,"title":disabled ? disabledReason : undefined,"onclick":() => (askReason ? (denying = true) : void ondeny(null)),});size === 'small';busy === 'deny' ? 'Denying…' : askReason ? 'Deny…' : 'Deny'; }
     { svelteHTML.createElement("button", {          "class":`btn primary`,"disabled":off,"title":disabled ? disabledReason : undefined,"onclick":() => void onapprove(),});size === 'small';busy === 'approve' ? approveBusyLabel : approveLabel; }
   }
}

if(denying){
   { const $$_teehSyneD0C = __sveltets_2_ensureComponent(DenySheet); new $$_teehSyneD0C({ target: __sveltets_2_any(), props: {           "action":denyTarget ?? 'proceed',"title":denyTitle,"hint":denyHint ?? (denyTarget ? `${denyTarget[0].toUpperCase()}${denyTarget.slice(1)} is not approved. Your reason is recorded with the decision.` : undefined),"onclose":() => (denying = false),"ondeny":confirmDeny,}});}
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ApprovalActions__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ApprovalActions__SvelteComponent_ = ReturnType<typeof ApprovalActions__SvelteComponent_>;
/*Ωignore_endΩ*/export default ApprovalActions__SvelteComponent_;