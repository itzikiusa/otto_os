///<reference types="svelte" />
;// The run's human-approval gate above the canvas: the "resumed after a
// restart" note, the publication preview a product gate must show before it
// can be approved, the Approve / Deny banner, and the recorded outcome. The
// state and the write live in runSync.svelte.ts.

import ApprovalActions from '../../lib/components/ApprovalActions.svelte';
import ApprovalOutcome from '../../lib/components/ApprovalOutcome.svelte';
import Icon from '../../lib/components/Icon.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import RelTime from '../../lib/components/RelTime.svelte';
import { auth } from '../../lib/stores/auth.svelte';
import type { WorkflowGraph } from '../../lib/api/types';
import type { RunSync } from './runSync.svelte';
;

  interface Props {
    mon: RunSync;
    graph: WorkflowGraph;
    nodeName: (id: string) => string;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  

  let { mon, graph, nodeName }: Props = $props();
  const run = $derived(mon.run);

  /** "“Title”" for a parent page picked by name in the publish node (PC-9 keeps
   *  its title beside the id), else "page {id}". */
  function parentPageLabel(parentId: string): string {
    const n = graph.nodes.find((x) => x.kind === 'product_publish' && (x.params as { parent_id?: unknown } | null)?.parent_id === parentId);
    const t = (n?.params as { parent_title?: unknown } | null)?.parent_title;
    return typeof t === 'string' && t ? `“${t}”` : `page ${parentId}`;
  }
  /** What a paused run's approval gates: the step(s) that fed it (whose work
   *  is being approved), the node's own prompt, and the step(s) an approval
   *  lets run next. */
  function approvalGate(nodeId: string): { after: string[]; ask: string; next: string[] } {
    const params = graph.nodes.find((n) => n.id === nodeId)?.params as { prompt?: unknown } | null | undefined;
    return {
      after: graph.edges.filter((e) => e.target === nodeId).map((e) => nodeName(e.source)),
      ask: typeof params?.prompt === 'string' ? params.prompt.trim() : '',
      next: graph.edges.filter((e) => e.source === nodeId).map((e) => nodeName(e.target)),
    };
  }
  /** "you" for the signed-in approver, a name as given, nothing for a bare id. */
  function approverLabel(by: string | null | undefined): string | null {
    if (!by) return null;
    if (by === auth.me?.id) return 'you';
    return /^[0-9A-HJKMNP-TV-Z]{26}$/i.test(by) ? null : by;
  }
;
async () => {


if(run && (run.resume_attempts ?? 0) > 0){
   { svelteHTML.createElement("div", {   "class":`resumed-banner`,"title":`A daemon restart interrupted this run; it picked up from the step it was on`,});
     { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":13,}});}
     { svelteHTML.createElement("span", {});      run.resume_attempts;  }
   }
}

if(run?.waiting_approval && run.approval_node_id){
  const gate = approvalGate(run.approval_node_id);
  if(mon.approvalNeedsPreview){
     { svelteHTML.createElement("section", { "aria-label":`Publication review`,});
      if(mon.approvalPreviewLoading){  { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {    "what":`the content and destination for review`,"variant":`compact`,"loading":true,"empty":true,}});}
      } else if (mon.approvalPreviewError){
         { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {        "what":`the publication preview`,"variant":`compact`,"error":mon.approvalPreviewError,"empty":true,"onretry":() => mon.retryApprovalPreview(),}});}
      } else if (mon.approvalPreview){
        const preview = mon.approvalPreview;
         { svelteHTML.createElement("h3", {});preview.kind === 'rfc' ? (preview.request.title ?? preview.request.reviewed_content.title) : preview.request.reviewed_content.title; }
         { svelteHTML.createElement("p", {});  preview.account_label; preview.account_url;
          if(preview.kind === 'jira'){ preview.request.issue_type;   preview.request.project_key;        }else{     preview.request.space_key;preview.request.parent_id ? ` under ${parentPageLabel(preview.request.parent_id)}` : '';        }
         }
         { svelteHTML.createElement("textarea", {         "dir":`auto`,"class":`np-prompt`,"aria-label":`Full publication content`,"rows":12,"readonly":true,"value":mon.approvalBody,}); }
      }
     }
  }
   { svelteHTML.createElement("div", {     "class":`approval-banner`,"role":`group`,"aria-label":`Approval needed`,});
     { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`userCheck`,"size":14,}});}
     { svelteHTML.createElement("span", {});
          { svelteHTML.createElement("strong", { "title":run.approval_node_id,});nodeName(run.approval_node_id); }if(gate.after.length > 0){   { svelteHTML.createElement("strong", {});gate.after.join(', '); }}     
      if(gate.ask){ { svelteHTML.createElement("span", { "class":`approval-ask`,}); gate.ask;  }}
       { svelteHTML.createElement("span", { "class":`approval-sub`,});   { const $$_emiTleR3C = __sveltets_2_ensureComponent(RelTime); new $$_emiTleR3C({ target: __sveltets_2_any(), props: {  "iso":run.started_at,}});}    gate.next.length > 0 ? gate.next.join(', ') : 'the rest of the run';        }
     }
     { const $$_snoitcAlavorppA1C = __sveltets_2_ensureComponent(ApprovalActions); new $$_snoitcAlavorppA1C({ target: __sveltets_2_any(), props: {               "disabled":mon.approvalNeedsPreview && (!mon.approvalPreviewReady || mon.approvalPreviewLoading || !!mon.approvalPreviewError),"disabledReason":`Load and review the publication preview before approving.`,"busy":mon.approving ? mon.approvingKind : null,"denyTarget":`the run at ${nodeName(run.approval_node_id)}`,"denyTitle":`Deny approval`,"onapprove":() => mon.approveRun(true),"ondeny":(reason) => mon.approveRun(false, reason),}});}
   }
} else if (run?.approved_at && !run.waiting_approval){
   { svelteHTML.createElement("div", { "class":`approval-outcome`,});
     { const $$_emoctuOlavorppA1C = __sveltets_2_ensureComponent(ApprovalOutcome); new $$_emoctuOlavorppA1C({ target: __sveltets_2_any(), props: {         "outcome":run.approved_by ? 'approved' : 'denied',"by":approverLabel(run.approved_by),"at":run.approved_at,"note":run.approval_note && run.approval_note !== 'rejected' ? run.approval_note : null,}});}
   }
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const RunApprovals__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type RunApprovals__SvelteComponent_ = ReturnType<typeof RunApprovals__SvelteComponent_>;
/*Ωignore_endΩ*/export default RunApprovals__SvelteComponent_;