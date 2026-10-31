///<reference types="svelte" />
;
import { plural } from '../../lib/plural';
import Skeleton from '../../lib/components/Skeleton.svelte';
// The Review Findings workflow board. For a completed review it lists the
// persisted Finding rows (GET /reviews/{id}/findings) as expandable cards — a
// status chip + severity chip + category + path:Lstart–Lend + reviewer +
// artifact chips (commit/test/Jira), the 7 action buttons (FindingActions), and
// on expand the evidence/reasoning/suggested-fix + the event timeline
// (GET /findings/{id}). Filters by status + severity; a header with counts and a
// Proof Pack button. Subscribes to the finding WS bus and refetches on match —
// the same pattern ReviewPanel uses for review_changed.
import { sentenceCase, severityLabel } from '../../lib/labels';
import Badge from '../../lib/components/Badge.svelte';
import type { BadgeTone } from '../../lib/status';
import Icon from '../../lib/components/Icon.svelte';
import { listFindings, getFinding } from '../../lib/api/client';
import type {
    Finding,
    FindingDetail,
    FindingStatus,
    FindingSeverity,
  } from '../../lib/api/types';
import LoadState from '../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../lib/loadError';
import { findingBus } from '../../lib/events.svelte';
import FindingActions from './FindingActions.svelte';
import ProofPackView from './ProofPackView.svelte';
;

  interface Props {
    reviewId: string;
    workspaceId: string;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { reviewId, workspaceId }: Props = $props();

  let findings: Finding[] = $state([]);
  let loading = $state(true);
  /** Failed list load — inline with Retry, never "No tracked findings". */
  let loadError = $state<string | null>(null);
  /** Failed silent refetch (WS bus) — the findings stay, with a stale bar + Retry. */
  let reloadError = $state<string | null>(null);
  /** Per-finding failed detail/timeline load → "Couldn’t load the timeline · Retry". */
  let detailError: Record<string, boolean> = $state({});
  let expanded: Record<string, boolean> = $state({});
  let details: Record<string, FindingDetail> = $state({});
  let detailLoading: Record<string, boolean> = $state({});

  // Filters
  let statusFilter: FindingStatus | 'all' = $state('all');
  let sevFilter: FindingSeverity | 'all' = $state('all');

  let showProofPack = $state(false);

  const STATUSES: FindingStatus[] = ['open', 'accepted', 'fixed', 'verified', 'false_positive', 'waived'];
  const SEVERITIES: FindingSeverity[] = ['critical', 'high', 'medium', 'low', 'info'];

  // Initial + reviewId-change load.
  $effect(() => {
    void load(reviewId);
  });

  // WS bus: refetch when a finding under THIS review changed / an action started.
  // (Keep this read-only of findingBus + reviewId; reload mutates state in an
  // effect, never in a derived.)
  $effect(() => {
    const _tick = findingBus.tick;
    if (_tick > 0 && findingBus.reviewId && findingBus.reviewId === reviewId) {
      void reload();
    }
  });

  async function load(rid: string): Promise<void> {
    loading = true;
    try {
      findings = await listFindings(rid);
      loadError = null;
    } catch (e) {
      loadError = loadErrorText(e);
    } finally {
      loading = false;
    }
  }

  /** Silent refetch (no loading flicker) — used by the WS bus. */
  async function reload(): Promise<void> {
    try {
      const next = await listFindings(reviewId);
      findings = next;
      loadError = null;
      reloadError = null;
      // Refresh any open detail so its timeline reflects the new events.
      for (const id of Object.keys(expanded)) {
        if (expanded[id] && details[id]) void loadDetail(id, true);
      }
    } catch (e) {
      // Non-blocking: keep what is on screen and say the refresh failed.
      reloadError = loadErrorText(e);
    }
  }

  function patchFinding(f: Finding): void {
    findings = findings.map((x) => (x.id === f.id ? f : x));
  }

  function toggle(id: string): void {
    const open = !expanded[id];
    expanded = { ...expanded, [id]: open };
    if (open && !details[id] && !detailLoading[id]) void loadDetail(id);
  }

  async function loadDetail(id: string, silent = false): Promise<void> {
    if (!silent) detailLoading = { ...detailLoading, [id]: true };
    try {
      const d = await getFinding(id);
      details = { ...details, [id]: d };
      patchFinding(d.finding);
      if (detailError[id]) detailError = { ...detailError, [id]: false };
    } catch {
      // Non-blocking — the card still shows the summary fields it already has;
      // a silent (WS-driven) refresh failure keeps the last good timeline.
      if (!silent) detailError = { ...detailError, [id]: true };
    } finally {
      if (!silent) detailLoading = { ...detailLoading, [id]: false };
    }
  }

  // --- derived (pure) --------------------------------------------------------
  const statusCounts = $derived.by(() => {
    const m: Record<string, number> = {};
    for (const f of findings) m[f.status] = (m[f.status] ?? 0) + 1;
    return m;
  });
  const sevCounts = $derived.by(() => {
    const m: Record<string, number> = {};
    for (const f of findings) m[f.severity] = (m[f.severity] ?? 0) + 1;
    return m;
  });
  const filtered = $derived.by(() =>
    findings.filter(
      (f) =>
        (statusFilter === 'all' || f.status === statusFilter) &&
        (sevFilter === 'all' || f.severity === sevFilter),
    ),
  );

  // --- helpers ---------------------------------------------------------------
  function loc(f: Finding): string {
    if (!f.path) return '';
    if (f.line == null) return f.path;
    if (f.line_end != null && f.line_end !== f.line) return `${f.path}:L${f.line}–L${f.line_end}`;
    return `${f.path}:L${f.line}`;
  }
  // Wire values (`false_positive`) never go on screen — lib/labels words them.
  const statusLabel = sentenceCase;
  function transitionLabel(from: string | null, to: string | null): string {
    if (from && to) return ` · ${statusLabel(from)} → ${statusLabel(to)}`;
    return '';
  }

  /** Finding status / severity → the shared Badge tone. */
  const STATUS_TONE: Record<string, BadgeTone> = { accepted: 'accent', fixed: 'warn', verified: 'ok' };
  function statusTone(st: string): BadgeTone {
    return STATUS_TONE[st] ?? 'neutral';
  }
  const SEV_TONE: Record<string, BadgeTone> = { critical: 'bad', high: 'bad', medium: 'warn', low: 'info' };
  function sevTone(sev: string): BadgeTone {
    return SEV_TONE[sev] ?? 'neutral';
  }
;
async () => {

 { svelteHTML.createElement("div", {   "class":`fb`,...__sveltets_2_empty({"data-workspace-id":workspaceId}),});
  
   { svelteHTML.createElement("div", { "class":`fb-header`,});
     { svelteHTML.createElement("span", { "class":`fb-count`,});plural(findings.length, 'finding'); }
    if(statusCounts['verified']){
       { const $$_egdaB2C = __sveltets_2_ensureComponent(Badge); new $$_egdaB2C({ target: __sveltets_2_any(), props: {    "tone":`ok`,"label":`${statusCounts['verified']} verified`,}});}
    }
    if(statusCounts['open']){
       { const $$_egdaB2C = __sveltets_2_ensureComponent(Badge); new $$_egdaB2C({ target: __sveltets_2_any(), props: {  "label":`${statusCounts['open']} open`,}});}
    }
     { svelteHTML.createElement("span", { "class":`grow`,}); }
     { svelteHTML.createElement("button", {     "class":`btn small ghost`,"onclick":() => (showProofPack = true),"disabled":findings.length === 0,});
       
     }
   }

  
   { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL1 = new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {            children:() => { return __sveltets_2_any(0); },"what":`findings`,loading,"error":loadError ?? reloadError,"empty":findings.length === 0,"rows":3,"onretry":() => void load(reviewId),emptyView:() => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("p", { "class":`dim fb-empty`,});       }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL1.$$prop_def;/*Ωignore_endΩ*/
    
    
     { svelteHTML.createElement("div", { "class":`fb-filters`,});
       { svelteHTML.createElement("div", {     "class":`fb-filter-row`,"role":`group`,"aria-label":`Filter by status`,});
         { svelteHTML.createElement("span", {   "class":`fb-filter-label`,"aria-hidden":`true`,});  }
         { svelteHTML.createElement("button", {      "class":`fb-pill`,"aria-pressed":statusFilter === 'all',"onclick":() => (statusFilter = 'all'),});statusFilter === 'all';
          
         }
          for(let s of __sveltets_2_ensureArray(STATUSES)){
          if(statusCounts[s]){
             { svelteHTML.createElement("button", {        "class":`fb-pill status-${s}`,"aria-pressed":statusFilter === s,"onclick":() => (statusFilter = statusFilter === s ? 'all' : s),});statusFilter === s;
              statusLabel(s); statusCounts[s];
             }
          }
        }
       }
       { svelteHTML.createElement("div", {     "class":`fb-filter-row`,"role":`group`,"aria-label":`Filter by severity`,});
         { svelteHTML.createElement("span", {   "class":`fb-filter-label`,"aria-hidden":`true`,});  }
         { svelteHTML.createElement("button", {      "class":`fb-pill`,"aria-pressed":sevFilter === 'all',"onclick":() => (sevFilter = 'all'),});sevFilter === 'all';
          
         }
          for(let s of __sveltets_2_ensureArray(SEVERITIES)){
          if(sevCounts[s]){
             { svelteHTML.createElement("button", {        "class":`fb-pill sev2-${s}`,"aria-pressed":sevFilter === s,"onclick":() => (sevFilter = sevFilter === s ? 'all' : s),});sevFilter === s;
              s; sevCounts[s];
             }
          }
        }
       }
     }

    
     { svelteHTML.createElement("div", { "class":`fb-list`,});
         for(let f of __sveltets_2_ensureArray(filtered)){f.id;
        const isOpen = !!expanded[f.id];
        const detail = details[f.id];
         { svelteHTML.createElement("div", {  "class":`fb-card card`,});f.regressed;
           { svelteHTML.createElement("button", {       "class":`fb-card-head`,"onclick":() => toggle(f.id),"aria-expanded":isOpen,});
             { const $$_egdaB5C = __sveltets_2_ensureComponent(Badge); new $$_egdaB5C({ target: __sveltets_2_any(), props: {    "tone":sevTone(f.severity),"label":severityLabel(f.severity),}});}
             { const $$_egdaB5C = __sveltets_2_ensureComponent(Badge); new $$_egdaB5C({ target: __sveltets_2_any(), props: {    "tone":statusTone(f.status),"label":statusLabel(f.status),}});}
            if(f.regressed){ { const $$_egdaB5C = __sveltets_2_ensureComponent(Badge); new $$_egdaB5C({ target: __sveltets_2_any(), props: {    "tone":`warn`,"label":`Regressed`,}});}}
            if(f.requires_human_approval && !f.approved_at){
               { const $$_egdaB5C = __sveltets_2_ensureComponent(Badge); new $$_egdaB5C({ target: __sveltets_2_any(), props: {    "tone":`warn`,"label":`Needs approval`,}});}
            }
             { svelteHTML.createElement("span", {   "class":`fb-title`,"title":f.title || f.body.split('\n')[0],});f.title || f.body.split('\n')[0]; }
             { svelteHTML.createElement("span", { "class":`grow`,}); }
             { svelteHTML.createElement("span", {   "class":`dim fb-caret`,"aria-hidden":`true`,}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":isOpen ? 'chevronDown' : 'chevronRight',"size":12,}});} }
           }

           { svelteHTML.createElement("div", { "class":`fb-meta`,});
            if(f.category){ { svelteHTML.createElement("span", { "class":`fb-cat`,});f.category; }}
            if(loc(f)){ { svelteHTML.createElement("span", { "class":`mono fb-loc`,});loc(f); }}
            if(f.reviewer){ { svelteHTML.createElement("span", { "class":`dim fb-reviewer`,}); f.reviewer; }}
            if(f.occurrence_count > 1){ { svelteHTML.createElement("span", { "class":`dim`,});  f.occurrence_count;  }}
             { svelteHTML.createElement("span", { "class":`grow`,}); }
            if(f.linked_commit){ { const $$_egdaB5C = __sveltets_2_ensureComponent(Badge); new $$_egdaB5C({ target: __sveltets_2_any(), props: {    "tone":`ok`,"label":`commit ${f.linked_commit.slice(0, 9)}`,}});}}
            if(f.linked_test){ { const $$_egdaB5C = __sveltets_2_ensureComponent(Badge); new $$_egdaB5C({ target: __sveltets_2_any(), props: {      "tone":`ok`,"label":`Test`,"title":f.linked_test,}});}}
            if(f.jira_key){
              if(f.jira_url){
                 { svelteHTML.createElement("a", {       "class":`chip fb-artifact fb-jira`,"href":f.jira_url,"target":`_blank`,"rel":`noreferrer`,});f.jira_key; }
              }else{
                 { const $$_egdaB5C = __sveltets_2_ensureComponent(Badge); new $$_egdaB5C({ target: __sveltets_2_any(), props: {    "tone":`ok`,"label":f.jira_key,}});}
              }
            }
           }

          if(isOpen){
             { svelteHTML.createElement("div", { "class":`fb-detail`,});
              if(f.evidence){
                 { svelteHTML.createElement("div", { "class":`fb-field`,});
                   { svelteHTML.createElement("span", { "class":`fb-field-label`,});  }
                   { svelteHTML.createElement("pre", { "class":`fb-pre`,});f.evidence; }
                 }
              }
              if(f.agent_reasoning_summary){
                 { svelteHTML.createElement("div", { "class":`fb-field`,});
                   { svelteHTML.createElement("span", { "class":`fb-field-label`,});  }
                   { svelteHTML.createElement("p", { "class":`fb-field-text`,});f.agent_reasoning_summary; }
                 }
              }
              if(f.suggested_fix){
                 { svelteHTML.createElement("div", { "class":`fb-field`,});
                   { svelteHTML.createElement("span", { "class":`fb-field-label`,});  }
                   { svelteHTML.createElement("pre", { "class":`fb-pre`,});f.suggested_fix; }
                 }
              }
              if(!f.evidence && !f.agent_reasoning_summary && !f.suggested_fix){
                 { svelteHTML.createElement("p", { "class":`fb-field-text`,});f.body; }
              }

              
               { svelteHTML.createElement("div", { "class":`fb-field`,});
                 { svelteHTML.createElement("span", { "class":`fb-field-label`,});  }
                if(detailLoading[f.id] && !detail){
                   { const $$_notelekS6C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS6C({ target: __sveltets_2_any(), props: {      "rows":2,"height":14,"label":`the timeline`,}});}
                } else if (detail && detail.events.length > 0){
                   { svelteHTML.createElement("ul", { "class":`fb-timeline`,});
                       for(let ev of __sveltets_2_ensureArray(detail.events)){ev.id;
                       { svelteHTML.createElement("li", { "class":`fb-event`,});
                         { svelteHTML.createElement("span", { "class":`fb-event-kind`,});statusLabel(ev.kind); }
                         { svelteHTML.createElement("span", { "class":`dim fb-event-meta`,});ev.actor;transitionLabel(ev.from_status, ev.to_status); }
                       }
                    }
                   }
                } else if (detail){
                   { svelteHTML.createElement("p", {   "class":`dim`,"style":`font-size: var(--fs-xs)`,});   }
                } else if (detailError[f.id]){
                   { svelteHTML.createElement("p", {   "class":`fb-tl-error`,"role":`alert`,});
                        
                     { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":() => void loadDetail(f.id),});  }
                   }
                }
               }

              
               { const $$_snoitcAgnidniF5C = __sveltets_2_ensureComponent(FindingActions); new $$_snoitcAgnidniF5C({ target: __sveltets_2_any(), props: {    "finding":f,"onupdated":patchFinding,}});}
             }
          }
         }
      }
      if(filtered.length === 0){
         { svelteHTML.createElement("p", { "class":`dim fb-empty`,});      }
      }
     }
   LoadState}
 }

if(showProofPack){
   { const $$_weiVkcaPfoorP0C = __sveltets_2_ensureComponent(ProofPackView); new $$_weiVkcaPfoorP0C({ target: __sveltets_2_any(), props: {   reviewId,"onclose":() => (showProofPack = false),}});}
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const FindingsBoard__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type FindingsBoard__SvelteComponent_ = ReturnType<typeof FindingsBoard__SvelteComponent_>;
/*Ωignore_endΩ*/export default FindingsBoard__SvelteComponent_;