///<reference types="svelte" />
;// The open run's detail panel (right-side drawer): the goal + source link, the
// stage timeline, proof + findings, the approval gate, and the PR draft. Reads
// the open run + its events straight from the store.

import { runWithOtto } from '../../lib/stores/runWithOtto.svelte';
import { announceShown } from '../../lib/announce.svelte';
import { loadErrorText } from '../../lib/loadError';
import { confirmer } from '../../lib/confirm.svelte';
import ProofStatusChip from '../../lib/components/ProofStatusChip.svelte';
import RunStageRail from './RunStageRail.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import RelTime from '../../lib/components/RelTime.svelte';
import Icon from '../../lib/components/Icon.svelte';
import ApprovalActions from '../../lib/components/ApprovalActions.svelte';
import { plural } from '../../lib/plural';
import AgentByline from '../../lib/components/AgentByline.svelte';
import type { OttoRun } from '../../lib/api/types';
import StatusBadge from '../../lib/components/StatusBadge.svelte';
import { humanize, isTerminal, prNumberFromUrl, runStatusInfo, sourceLabel } from './runStatus';
import { router } from '../../lib/router.svelte';
import { runEvidenceLinks } from './runEvidence';
;

  interface Props {
    run: OttoRun;
    onClose: () => void;
  };function $$render() {
 const prLinks/*Ωignore_positionΩ*/ = (url: string)/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
  const n = prNumberFromUrl(url);
  if(n !== null && run.repo_id){
     { svelteHTML.createElement("button", {   "class":`btn primary`,"onclick":() => router.go(`git/${run.repo_id}/pr/${n}`),});   }
     { svelteHTML.createElement("a", {       "class":`btn`,"href":url,"target":`_blank`,"rel":`noreferrer`,});    { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`external`,"size":12,}});} }
  }else{
     { svelteHTML.createElement("a", {       "class":`btn primary`,"href":url,"target":`_blank`,"rel":`noreferrer`,});   { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`external`,"size":12,}});} }
  }
};return __sveltets_2_any(0)};
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { run, onClose }: Props = $props();

  let busy = $state(false);
  let error = $state('');
  /** Which gate decision is in flight (drives the Approving… / Denying… label). */
  let deciding = $state<'approve' | 'deny' | null>(null);

  const events = $derived(runWithOtto.eventsByRun[run.id] ?? []);
  /** In-app evidence behind the gate counts (proof pack, findings, goal loop,
   *  branch) — so Approve is never blind (S20-02). */
  const evidence = $derived(runEvidenceLinks(run));

  // Best-effort parse of the stored PR draft into title/description.
  const prDraft = $derived.by(() => {
    if (!run.pr_draft_json) return null;
    try {
      const j = JSON.parse(run.pr_draft_json) as Record<string, unknown>;
      const title = (j.title ?? j.pr_title ?? '') as string;
      const description = (j.description ?? j.body ?? j.description_md ?? '') as string;
      const source = (j.source_branch ?? '') as string;
      const target = (j.target_branch ?? '') as string;
      return { title, description, source, target };
    } catch {
      return null;
    }
  });

  // The wire still says `reject`; the UI says Deny (one queue vocabulary).
  async function approve(decision: 'approve' | 'reject', note?: string | null): Promise<void> {
    error = '';
    busy = true;
    deciding = decision === 'approve' ? 'approve' : 'deny';
    try {
      await runWithOtto.approve(run.id, {
        decision,
        note: decision === 'reject' && note?.trim() ? note.trim() : undefined,
      });
    } catch (e) {
      error = loadErrorText(e);
    } finally {
      busy = false;
      deciding = null;
    }
  }

  async function cancel(): Promise<void> {
    const runId = run.id;
    if (!(await confirmer.ask('Stop this run? The agent stops and the run can’t be resumed — you would launch a new one.', { title: 'Stop this run?', danger: true, confirmLabel: 'Stop run', cancelLabel: 'Keep running' }))) return;
    error = '';
    busy = true;
    try {
      await runWithOtto.cancel(runId);
    } catch (e) {
      error = loadErrorText(e);
    } finally {
      busy = false;
    }
  }

  // Proof here is the execution snapshot. The server checks current evidence
  // and the working revision when publishing; a later waiver must stay actionable.
  const prBlock = $derived.by(() => {
    if (run.approval_decision !== 'approved') return 'Approve the run first';
    if (!run.repo_id) return 'This run has no repository to open a PR in';
    return '';
  });

  /** The one outward-facing action: say where it goes and who sees it first. */
  async function openPr(): Promise<void> {
    const runId = run.id;
    const into = prDraft?.target || run.base_branch || 'the default branch';
    const from = prDraft?.source || run.branch || 'the run branch';
    const where = run.repo_path ? ` in ${run.repo_path}` : '';
    const ok = await confirmer.ask(
      `Push ${from} and open “${prDraft?.title || run.title}” as a pull request into ${into}${where}? It is created on the remote, where everyone with access to the repository can see it.`,
      { title: 'Open pull request', confirmLabel: 'Open PR', danger: false },
    );
    if (!ok) return;
    error = '';
    busy = true;
    try {
      await runWithOtto.openPr(runId);
    } catch (e) {
      error = loadErrorText(e);
    } finally {
      busy = false;
    }
  }
;
async () => {




 { svelteHTML.createElement("aside", { "class":`detail`,});
   { svelteHTML.createElement("header", { "class":`d-head`,});
     { svelteHTML.createElement("div", { "class":`d-title`,});
       { svelteHTML.createElement("span", { "class":`chip`,});sourceLabel(run.source_kind); }
       { const $$_egdaBsutatS3C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS3C({ target: __sveltets_2_any(), props: {  "status":runStatusInfo(run.status),}});}
     }
     { svelteHTML.createElement("div", { "class":`d-actions`,});
      
      if(!isTerminal(run.status)){
         { svelteHTML.createElement("button", {     "class":`btn small danger`,"disabled":busy,"onclick":cancel,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`stop`,"size":12,}});}   }
      }
       { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":onClose,"aria-label":`Close run detail`,"title":`Close run detail`,});
         { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":14,}});}
       }
     }
   }
   { svelteHTML.createElement("h2", { "class":`d-name`,});run.title || run.source_ref; }

  if(error){ { svelteHTML.createElement("div", {   "class":`err`,"role":`alert`,});error; }}
  if(run.status === 'failed' && run.error){
     {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('div'),({ text: `Run failed: ${run.error}`, assertive: true })));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {   "class":`err`,}); { svelteHTML.createElement("strong", {});  } run.error; }}
  }

  
   { svelteHTML.createElement("section", { "class":`block`,});
     { const $$_liaRegatSnuR2C = __sveltets_2_ensureComponent(RunStageRail); new $$_liaRegatSnuR2C({ target: __sveltets_2_any(), props: {  "status":run.status,}});}
   }

   { svelteHTML.createElement("section", { "class":`block`,});
    if(run.goal){
       { svelteHTML.createElement("div", { "class":`goal`,});run.goal; }
    }else{
       { svelteHTML.createElement("div", { "class":`goal muted`,});   }
    }
     { svelteHTML.createElement("div", { "class":`src-row`,});
      if(run.source_url){
         { svelteHTML.createElement("a", {         "class":`link`,"href":run.source_url,"target":`_blank`,"rel":`noreferrer`,"title":run.source_url,});run.source_ref;  { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`external`,"size":12,}});} }
      }else{
         { svelteHTML.createElement("span", { "class":`muted`,});run.source_ref; }
      }
       { svelteHTML.createElement("span", { "class":`dot`,});  }
       { svelteHTML.createElement("span", { "class":`muted`,});humanize(run.mode); }
      if(run.provider){
         { svelteHTML.createElement("span", { "class":`dot`,});  }
         { svelteHTML.createElement("span", { "class":`muted mono`,});run.provider;run.model ? ` · ${run.model}` : ''; }
      }
     }
   }

  
   { svelteHTML.createElement("section", { "class":`block stats`,});
    if(run.proof_pack_id && run.proof_status){
       { svelteHTML.createElement("span", { "class":`muted`,});   }
       { svelteHTML.createElement("button", {           "type":`button`,"class":`chip-btn`,"onclick":() => router.go(`proof/${encodeURIComponent(run.proof_pack_id ?? '')}`),"aria-label":`Open the proof pack`,"title":`Open the current proof pack; this badge records proof at execution`,});
         { const $$_pihCsutatSfoorP3C = __sveltets_2_ensureComponent(ProofStatusChip); new $$_pihCsutatSfoorP3C({ target: __sveltets_2_any(), props: {    "status":run.proof_status,"risk":run.risk_score,}});}
       }
    }
     { svelteHTML.createElement("span", { "class":`findings`,});
       { svelteHTML.createElement("span", { "class":`fnum`,});run.findings_total; } run.findings_total === 1 ? 'finding' : 'findings';
      if(run.findings_blocking > 0){
         { svelteHTML.createElement("span", {   "class":`blocking`,"title":`Findings that block the PR`,});run.findings_blocking;  }
      }
     }
    if(run.branch){ { svelteHTML.createElement("span", {   "class":`muted mono branch`,"title":run.branch,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`branch`,"size":12,}});}  { svelteHTML.createElement("span", { "class":`branch-name`,});run.branch; } }}
   }

  
   { svelteHTML.createElement("section", { "class":`block`,});
     { svelteHTML.createElement("h3", { "class":`h`,});  }
     { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL2 = new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {             children:() => { return __sveltets_2_any(0); },"what":`the stage timeline`,"variant":`compact`,"loading":!runWithOtto.eventsByRun[run.id] && !runWithOtto.eventsError[run.id],"error":runWithOtto.eventsError[run.id],"empty":events.length === 0,"onretry":() => void runWithOtto.loadEvents(run.id),emptyView:() => { async ()/*Ωignore_positionΩ*/ => { { svelteHTML.createElement("div", { "class":`muted`,});    }};return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL2.$$prop_def;/*Ωignore_endΩ*/
      
       { svelteHTML.createElement("ol", { "class":`timeline`,});
           for(let ev of __sveltets_2_ensureArray(events)){ev.id;
           { svelteHTML.createElement("li", { "class":`tl-item`,});
             { svelteHTML.createElement("span", {   "class":`tl-dot tone-${ev.status ? runStatusInfo(ev.status).tone : 'neutral'}`,"aria-hidden":`true`,}); }
             { svelteHTML.createElement("div", { "class":`tl-body`,});
               { svelteHTML.createElement("div", { "class":`tl-top`,});
                 { svelteHTML.createElement("span", { "class":`tl-kind`,});humanize(ev.kind); }
                if(ev.status){ { const $$_egdaBsutatS7C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS7C({ target: __sveltets_2_any(), props: {      "status":{ ...runStatusInfo(ev.status), live: false },"variant":`text`,"dot":false,}});}}
                 { svelteHTML.createElement("span", { "class":`tl-when`,}); { const $$_emiTleR8C = __sveltets_2_ensureComponent(RelTime); new $$_emiTleR8C({ target: __sveltets_2_any(), props: {  "iso":ev.created_at,}});} }
               }
              if(ev.message){ { svelteHTML.createElement("div", { "class":`tl-msg`,});ev.message; }}
             }
           }
        }
       }
     LoadState}
   }

  
  if(run.status === 'awaiting_approval'){
     { svelteHTML.createElement("section", { "class":`block gate`,});
       { svelteHTML.createElement("h3", { "class":`h`,});   }
      
       { svelteHTML.createElement("p", { "class":`gate-who`,}); { const $$_enilyBtnegA3C = __sveltets_2_ensureComponent(AgentByline); new $$_enilyBtnegA3C({ target: __sveltets_2_any(), props: {        "provider":run.provider,"model":run.model,"at":run.updated_at,"label":`Run output`,}});} }
       { svelteHTML.createElement("p", { "class":`gate-what`,});
         { svelteHTML.createElement("span", { "class":`mono`,});run.branch || 'the run branch'; }
         plural(run.findings_total, 'finding');run.findings_blocking > 0 ? ` (${run.findings_blocking} blocking)` : '';
         run.proof_status ? `proof at execution: ${run.proof_status}` : 'no proof pack yet';run.result_summary ? ` · ${run.result_summary}` : '';
       }
      if(evidence.length > 0){
         { svelteHTML.createElement("div", {   "class":`gate-review`,...__sveltets_2_empty({"data-testid":`run-gate-evidence`}),});
           { svelteHTML.createElement("span", { "class":`gate-review-h`,});   }
           { svelteHTML.createElement("div", { "class":`actions`,});
               for(let ev of __sveltets_2_ensureArray(evidence)){ev.key;
               { svelteHTML.createElement("button", {       "type":`button`,"class":`btn small`,"title":ev.title,"onclick":() => router.go(ev.route),});
                 { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":ev.icon,"size":12,}});} ev.label;
               }
            }
           }
         }
      }else{
         { svelteHTML.createElement("p", { "class":`gate-what`,});         }
      }
       { svelteHTML.createElement("p", { "class":`gate-note`,});
               { svelteHTML.createElement("span", { "class":`mono`,});run.branch || 'the run branch'; }   
                    
       }
       { const $$_snoitcAlavorppA2C = __sveltets_2_ensureComponent(ApprovalActions); new $$_snoitcAlavorppA2C({ target: __sveltets_2_any(), props: {               "busy":deciding,"disabled":busy,"denyTarget":`this run`,"denyTitle":`Deny run`,"denyHint":`Denying ends the run and removes its worktree — commits that weren’t pushed are lost. Your reason is recorded with the decision.`,"onapprove":() => approve('approve'),"ondeny":(reason) => approve('reject', reason),}});}
     }
  }

  
  if(prDraft){
     { svelteHTML.createElement("section", { "class":`block pr`,});
       { svelteHTML.createElement("h3", { "class":`h`,});  }
       { const $$_enilyBtnegA2C = __sveltets_2_ensureComponent(AgentByline); new $$_enilyBtnegA2C({ target: __sveltets_2_any(), props: {    "provider":run.provider,"model":run.model,}});}
       { svelteHTML.createElement("div", { "class":`pr-title`,});prDraft.title || 'Untitled PR'; }
      if(prDraft.source || prDraft.target){
         { svelteHTML.createElement("div", { "class":`muted mono`,});prDraft.source || run.branch;prDraft.target ? ` → ${prDraft.target}` : ''; }
      }
      if(prDraft.description){ { svelteHTML.createElement("pre", { "class":`pr-desc`,});prDraft.description; }}
       { svelteHTML.createElement("div", { "class":`actions`,});
        if(run.pr_url){
          ;__sveltets_2_ensureSnippet(prLinks(run.pr_url));
        }else{
           { svelteHTML.createElement("button", {       "class":`btn primary`,"disabled":busy || !!prBlock,"title":prBlock || 'Recheck current proof, push the branch and open this pull request',"onclick":openPr,});  }
          if(prBlock){ { svelteHTML.createElement("span", { "class":`muted hint`,});prBlock;  }}
        }
       }
     }
  } else if (run.pr_url){
     { svelteHTML.createElement("section", { "class":`block pr`,});
       { svelteHTML.createElement("h3", { "class":`h`,});  }
      ;__sveltets_2_ensureSnippet(prLinks(run.pr_url));
     }
  }

  if(run.result_summary){
     { svelteHTML.createElement("section", { "class":`block`,});
       { svelteHTML.createElement("h3", { "class":`h`,});  }
       { svelteHTML.createElement("div", { "class":`muted summary`,});run.result_summary; }
     }
  }

 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const RunDetail__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type RunDetail__SvelteComponent_ = ReturnType<typeof RunDetail__SvelteComponent_>;
/*Ωignore_endΩ*/export default RunDetail__SvelteComponent_;