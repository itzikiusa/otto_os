///<reference types="svelte" />
;
import { announceShown } from '../../lib/announce.svelte';
import { onDestroy, untrack } from 'svelte';
import { toastError } from '../../lib/toastError';
import { router } from '../../lib/router.svelte';
import Icon from '../../lib/components/Icon.svelte';
import { missionControlBus } from '../../lib/events.svelte';
import { missionControlApi } from '../../lib/api/missionControl';
import { ApiError } from '../../lib/api/client';
import { ws } from '../../lib/stores/workspace.svelte';
import type { WorkItemDetail, RiskLevel } from '../../lib/api/types';
import { KIND_ICON, KIND_LABEL, RISK_LABEL, RISK_LEVELS, ACTOR_LABEL, ARTIFACT_LABEL } from './lib';
import { riskColor, fmtCost, workStatus, ownerLabel, shortId, isUlid } from './lib';
import StatusBadge from '../../lib/components/StatusBadge.svelte';
import ApprovalActions from '../../lib/components/ApprovalActions.svelte';
import ApprovalOutcome from '../../lib/components/ApprovalOutcome.svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
import { rel } from '../../lib/stores/now.svelte';
import { auth } from '../../lib/stores/auth.svelte';
import { toasts } from '../../lib/toast.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { openExternal } from '../../lib/external';
import { copyText } from '../../lib/clipboard';
import { focusFirstField, refocusEditTrigger } from '../../lib/inlineEdit';
import { sentenceCase, type Tone } from '../../lib/status';
import { SOURCE_MODULE, directRoute, reviewRoute, stopPath, isActive } from './sourceLinks';
;

  interface Props {
    wsId: string;
    id: string;
    onClose: () => void;
    onOpen: (id: string) => void;
    /** Fired after a mutation so the parent can refresh its summary/list. */
    onChange?: () => void;
  };;

  type EditSnapshot = { goal: string; result_summary: string; risk_level: RiskLevel };;function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { wsId, id, onClose, onOpen, onChange }: Props = $props();

  let detail = $state<WorkItemDetail | null>(null);
  let loading = $state(false);
  let err = $state('');
  let busy = $state(false);

  // editing
  const EDIT_KEY = 'work-item-goal';
  let editing = $state(false);
  let editGoal = $state('');
  let editResult = $state('');
  let editRisk = $state<RiskLevel>('low');
  let approveReason = $state('');

  let editBaseline = $state<EditSnapshot | null>(null);
  let detailOwner = '';
  let viewGeneration = 0;
  let readGeneration = 0;
  let alive = true;
  onDestroy(() => { alive = false; ++viewGeneration; ++readGeneration; });

  function snapshot(item: Pick<WorkItemDetail, 'goal' | 'result_summary' | 'risk_level'>): EditSnapshot {
    return { goal: item.goal ?? '', result_summary: item.result_summary ?? '', risk_level: item.risk_level };
  }
  function draftSnapshot(): EditSnapshot {
    return { goal: editGoal, result_summary: editResult, risk_level: editRisk };
  }
  function sameDraft(a: EditSnapshot, b: EditSnapshot): boolean {
    return a.goal === b.goal && a.result_summary === b.result_summary && a.risk_level === b.risk_level;
  }
  function resetDraft(): void {
    if (!detail) return;
    editBaseline = snapshot(detail);
    editGoal = editBaseline.goal;
    editResult = editBaseline.result_summary;
    editRisk = editBaseline.risk_level;
  }
  function beginEdit(): void {
    resetDraft();
    editing = true;
  }
  /** Close the editor (Cancel or a saved draft); focus returns to Edit. */
  function cancelEdits(): void {
    editing = false;
    resetDraft();
    void refocusEditTrigger(EDIT_KEY);
  }
  function isDirty(): boolean {
    return !!detail && editing && !!editBaseline && !sameDraft(draftSnapshot(), editBaseline);
  }
  function ownsView(workspaceId: string, itemId: string, generation: number): boolean {
    return alive && wsId === workspaceId && id === itemId && viewGeneration === generation;
  }
   async function canLeave(): Promise<boolean> {
    if (busy) return false;
    if (!isDirty()) return true;
    const discard = await confirmer.ask('You have unsaved work item changes. Leaving discards them.', {
      title: 'Discard unsaved changes?', danger: true, confirmLabel: 'Discard', cancelLabel: 'Keep editing',
    });
    if (discard) editing = false;
    return discard;
  }
  $effect(() => router.guard(() => canLeave()));

  async function load(): Promise<void> {
    const owner = JSON.stringify([wsId, id]);
    if (detailOwner !== owner) {
      detailOwner = owner;
      ++viewGeneration;
      detail = null;
      editing = false;
      editBaseline = null;
      busy = false;
      deciding = null;
    }
    const want = id;
    const workspaceId = wsId;
    const view = viewGeneration;
    const request = ++readGeneration;
    const ownsRead = () => ownsView(workspaceId, want, view) && request === readGeneration;
    loading = true;
    err = '';
    try {
      const d = await missionControlApi.item(workspaceId, want);
      if (!ownsRead()) return;
      detail = d;
      // A read may begin before Edit, or follow an approval while editing.
      // Only the persisted detail changes; the draft keeps its edit baseline.
      if (!editing) resetDraft();
    } catch (e) {
      if (ownsRead()) err = e instanceof ApiError ? e.message : 'Otto couldn’t reach the daemon.';
    } finally {
      if (ownsRead()) loading = false;
    }
  }

  // Reload whenever the selected id changes. `load` reads `detail` before its
  // first await — untracked, or every landed detail re-ran this effect (a
  // back-to-back GET loop, ~200 req/s, while an item was open).
  $effect(() => {
    void id;
    void wsId;
    untrack(() => void load());
  });

  // Live: reload when THIS item changes (or on a reconnect resync) — the
  // pane used to keep showing a stale status while the list updated. Never
  // while the user is editing; approval refreshes may still update the detail.
  let seenTick = untrack(() => missionControlBus.tick);
  $effect(() => {
    const tick = missionControlBus.tick;
    const evItem = missionControlBus.itemId;
    const evWs = missionControlBus.workspaceId;
    if (tick === seenTick) return;
    seenTick = tick;
    const resync = evWs === '' && evItem === '';
    const mine = untrack(() => evItem === id && evWs === wsId);
    if ((resync || mine) && !untrack(() => editing)) untrack(() => void load());
  });

  async function saveEdits(): Promise<void> {
    if (!detail || busy) return;
    const workspaceId = wsId, itemId = id, view = viewGeneration;
    const submitted = draftSnapshot();
    busy = true;
    ++readGeneration; // A pre-save read cannot overwrite the committed response.
    try {
      const saved = await missionControlApi.patch(workspaceId, itemId, submitted);
      if (!ownsView(workspaceId, itemId, view) || !detail) return;
      ++readGeneration; // Also invalidate reads that started during the PATCH.
      detail = { ...detail, ...saved };
      editBaseline = snapshot(saved);
      // Inputs remain editable during Save. Retain anything typed after submit.
      if (sameDraft(draftSnapshot(), submitted)) cancelEdits();
      await load();
      if (ownsView(workspaceId, itemId, view)) onChange?.();
      toasts.success('Work item saved');
    } catch (e) {
      toastError('Couldn’t save the work item', e);
    } finally {
      if (ownsView(workspaceId, itemId, view)) { busy = false; loading = false; }
    }
  }

  async function requestApproval(): Promise<void> {
    if (busy) return;
    const workspaceId = wsId, itemId = id, view = viewGeneration;
    busy = true;
    try {
      await missionControlApi.requestApproval(workspaceId, itemId, { reason: approveReason || undefined });
      if (!ownsView(workspaceId, itemId, view)) return;
      approveReason = '';
      await load();
      if (ownsView(workspaceId, itemId, view)) onChange?.();
      toasts.success('Approval requested');
    } catch (e) {
      toastError('Couldn’t request approval', e);
    } finally {
      if (ownsView(workspaceId, itemId, view)) busy = false;
    }
  }

  /** Which approval is mid-decision, and which way. */
  let deciding = $state<{ id: string; kind: 'approve' | 'deny' } | null>(null);
  async function decide(aid: string, decision: 'approved' | 'rejected', note?: string | null): Promise<void> {
    if (busy) return;
    const workspaceId = wsId, itemId = id, view = viewGeneration;
    busy = true;
    deciding = { id: aid, kind: decision === 'approved' ? 'approve' : 'deny' };
    try {
      // The wire keeps `rejected`; the UI says Deny (one queue vocabulary).
      await missionControlApi.decideApproval(workspaceId, aid, { decision, note: note?.trim() || undefined });
      if (!ownsView(workspaceId, itemId, view)) return;
      await load();
      if (ownsView(workspaceId, itemId, view)) onChange?.();
      toasts.success(decision === 'approved' ? 'Approved' : 'Denied');
    } catch (e) {
      toastError('Couldn’t record the decision', e);
    } finally {
      if (ownsView(workspaceId, itemId, view)) { busy = false; deciding = null; }
    }
  }

  // navigateToSession routes to `agents/<id>`; a following router.go('agents')
  // used to overwrite that and land on Agents without the session.
  function openSession(): void {
    if (!detail) return;
    ws.navigateToSession(detail.source_id);
  }

  // "Open in <module>" — every kind lands on its source row in the owning
  // module (sourceLinks.ts). Kinds whose route needs more than the source id
  // look it up first (a swarm project's swarm, a run's workflow, a review's PR).
  let opening = $state(false);
  async function openSource(): Promise<void> {
    const d = detail;
    if (!d || opening) return;
    if (d.kind === 'session' || d.kind === 'external_trigger') return openSession();
    const direct = directRoute(d.kind, d.source_id);
    if (direct) {
      await router.goChecked(direct);
      return;
    }
    opening = true;
    try {
      // Navigate FIRST and only touch the target page/store once the leave
      // guards let us go: a declined guard used to fall through to a 10 s
      // port wait and a false "Couldn’t open" toast (and, for a swarm, still
      // switch the global swarm store under the page the user stayed on).
      if (d.kind === 'swarm') {
        const p = await missionControlApi.sourceProject(d.source_id);
        if (p.swarm_id) {
          if (!(await router.goChecked(`swarm/${encodeURIComponent(p.swarm_id)}`))) return;
          const { swarm } = await import('../../lib/stores/swarm.svelte');
          await swarm.openProject(wsId, p.swarm_id, p.id);
        } else await router.goChecked('swarm');
      } else if (d.kind === 'workflow') {
        const r = await missionControlApi.sourceWorkflowRun(d.source_id);
        const { workflowsPagePort } = await import('../../lib/uiCommands/workflows');
        if (!(await router.goChecked(`workflows/${encodeURIComponent(r.workflow_id)}`))) return;
        const page = await workflowsPagePort.get(new AbortController().signal);
        if (await page.open(r.workflow_id)) await page.openRun(r.workflow_id, r.id);
      } else if (d.kind === 'review') {
        const r = await missionControlApi.sourceReview(d.source_id);
        await router.goChecked(reviewRoute(r.repo_id, r.pr_number));
      }
    } catch (e) {
      toastError(`Couldn’t open this in ${SOURCE_MODULE[d.kind]}`, e);
    } finally {
      opening = false;
    }
  }

  // Stop — through the owner's own cancel endpoint; the projector moves the
  // item when the owner's status event lands.
  const stopUrl = $derived(detail && isActive(detail.status) ? stopPath(detail.kind, detail.source_id) : null);
  let stopping = $state(false);
  async function stopSource(): Promise<void> {
    const d = detail;
    const path = stopUrl;
    if (!d || !path || stopping) return;
    const ok = await confirmer.ask(
      `Stop “${d.title}”? ${KIND_LABEL[d.kind]} work in progress halts; finished steps are kept.`,
      { title: `Stop ${KIND_LABEL[d.kind].toLowerCase()}`, confirmLabel: 'Stop', danger: true },
    );
    if (!ok) return;
    stopping = true;
    try {
      await missionControlApi.stopSource(path);
      toasts.success('Stop requested');
      onChange?.();
    } catch (e) {
      toastError('Couldn’t stop it', e);
    } finally {
      stopping = false;
    }
  }

  /** A person's label: "You", a name, or a shortened id (full id in title). */
  function who(v: string | null | undefined): string {
    if (!v) return '—';
    return ownerLabel(v, auth.me?.id) ?? shortId(v);
  }

  const APPROVAL_TONE: Record<string, Tone> = { pending: 'warning', approved: 'success', rejected: 'danger' };

  function relLabel(rel: string): string {
    return sentenceCase(rel);
  }

  /** "3m ago" that keeps counting (shared clock); "—" when unknown. */
  const ago = (iso: string | null | undefined): string => (iso && rel(iso)) || '—';
  function localTime(iso: string | null | undefined): string | undefined {
    const t = iso ? Date.parse(iso) : NaN;
    return Number.isFinite(t) ? new Date(t).toLocaleString() : undefined;
  }

  async function copyId(v: string): Promise<void> {
    if (await copyText(v)) toasts.success('Copied ID'); else toasts.error('Couldn’t copy the ID', `The clipboard isn’t available here. Select the ID and copy it manually. ${v}`);
  }

  function isUrl(s: string | null): boolean {
    return !!s && /^https?:\/\//.test(s);
  }
  function payloadPreview(p: unknown): string {
    if (p == null) return '';
    try {
      const s = typeof p === 'string' ? p : JSON.stringify(p);
      return s.length > 140 ? s.slice(0, 139) + '…' : s;
    } catch {
      return '';
    }
  }
;
async () => {

 { svelteHTML.createElement("aside", {   "class":`detail`,"aria-label":`Work item details`,});
   { svelteHTML.createElement("header", { "class":`d-head`,});
     { svelteHTML.createElement("div", { "class":`d-title`,});
      if(detail){ { svelteHTML.createElement("span", { "class":`d-kicon`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":KIND_ICON[detail.kind],"size":16,}});} }}
       { svelteHTML.createElement("div", { "class":`d-titletext`,});
         { svelteHTML.createElement("h2", { "title":detail?.title,});detail?.title ?? 'Work item'; }
        if(detail){
           { svelteHTML.createElement("span", { "class":`d-sub`,});
            KIND_LABEL[detail.kind]; 
             { svelteHTML.createElement("button", {     "class":`id-btn mono`,"title":`${detail.source_id} — click to copy`,"onclick":() => detail && void copyId(detail.source_id),});isUlid(detail.source_id) ? shortId(detail.source_id) : detail.source_id; }
           }
        }
       }
     }
     { svelteHTML.createElement("button", {       "class":`icon-btn detail-back`,"title":`Back`,"aria-label":`Back`,"onclick":onClose,});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`chevronLeft`,"size":14,}});}
       { svelteHTML.createElement("span", { "class":`back-word`,});  }
     }
   }

  if(loading && !detail){
     { svelteHTML.createElement("div", { "class":`d-body`,}); { const $$_notelekS2C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS2C({ target: __sveltets_2_any(), props: {    "rows":6,"label":`the work item`,}});} }
  } else if (err && !detail){
     { svelteHTML.createElement("div", { "class":`d-body`,});
       { svelteHTML.createElement("div", {   "class":`d-err`,"role":`alert`,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":14,}});}
         { svelteHTML.createElement("span", {});     err; }
       }
       { svelteHTML.createElement("div", {}); { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => void load(),}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":12,}});}  } }
     }
  } else if (detail){
     { svelteHTML.createElement("div", { "class":`d-body`,});
      if(err){ {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('div'),({ text: err, assertive: true })));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {   "class":`d-err`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":14,}});} { svelteHTML.createElement("span", {});err; } }}}

      
       { svelteHTML.createElement("div", { "class":`d-banner`,});
         { const $$_egdaBsutatS3C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS3C({ target: __sveltets_2_any(), props: {  "status":workStatus(detail.status),}});}
         { svelteHTML.createElement("span", {   "class":`chip-risk`,"style":`--c:${riskColor(detail.risk_level)}`,});RISK_LABEL[detail.risk_level];  }
        if(detail.needs_approval){ { svelteHTML.createElement("span", { "class":`chip warn`,});  detail.pending_approvals;  }}
         { svelteHTML.createElement("span", { "class":`grow`,}); }
        if(stopUrl){
           { svelteHTML.createElement("button", {       "class":`btn small danger`,"onclick":() => void stopSource(),"disabled":stopping,"aria-busy":stopping,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`stop`,"size":12,}});}stopping ? 'Stopping…' : 'Stop'; }
        }
         { svelteHTML.createElement("button", {       "class":`btn small`,"onclick":() => void openSource(),"disabled":opening,"aria-busy":opening,});
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":KIND_ICON[detail.kind],"size":12,}});}detail.kind === 'session' || detail.kind === 'external_trigger' ? 'Open session' : `Open in ${SOURCE_MODULE[detail.kind]}`;
         }
       }

      
       { svelteHTML.createElement("dl", { "class":`facts`,});
         { svelteHTML.createElement("div", {}); { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", { "title":detail.owner ?? undefined,});who(detail.owner);  { svelteHTML.createElement("span", { "class":`dim`,}); ACTOR_LABEL[detail.owner_kind];  } } }
         { svelteHTML.createElement("div", {}); { svelteHTML.createElement("dt", {});   } { svelteHTML.createElement("dd", { "class":`mono`,});fmtCost(detail.cost_so_far); } }
         { svelteHTML.createElement("div", {}); { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", {   "class":`mono`,"title":detail.repo_id ?? undefined,});detail.repo_id ?? '—'; } }
         { svelteHTML.createElement("div", {}); { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", {   "class":`mono`,"title":detail.branch ?? undefined,});detail.branch ?? '—'; } }
         { svelteHTML.createElement("div", {}); { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", { "title":localTime(detail.created_at),});ago(detail.created_at); } }
         { svelteHTML.createElement("div", {}); { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", { "title":localTime(detail.updated_at),});ago(detail.updated_at); } }
       }

      
       { svelteHTML.createElement("section", { "class":`d-sec`,});
         { svelteHTML.createElement("div", { "class":`sec-head`,});
           { svelteHTML.createElement("h3", { "class":`section-title`,});   }
          if(!editing){ { svelteHTML.createElement("button", {       "class":`btn ghost small`,...__sveltets_2_empty({"data-edit-for":EDIT_KEY}),"aria-label":`Edit goal and context`,"onclick":beginEdit,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`edit`,"size":12,}});}  }}
         }
        if(editing){
           { svelteHTML.createElement("label", { "class":`field`,}); { svelteHTML.createElement("span", { "class":`flabel`,});  } {const $$action_0 = __sveltets_2_ensureAction(focusFirstField(svelteHTML.mapElementTag('textarea')));{ svelteHTML.createElement("textarea", __sveltets_2_union($$action_0), {        "dir":`auto`,"class":`input field-input`,"rows":2,"bind:value":editGoal,});/*Ωignore_startΩ*/() => editGoal = __sveltets_2_any(null);/*Ωignore_endΩ*/ }} }
           { svelteHTML.createElement("label", { "class":`field`,}); { svelteHTML.createElement("span", { "class":`flabel`,});  } { svelteHTML.createElement("textarea", {       "dir":`auto`,"class":`input field-input`,"rows":2,"bind:value":editResult,});/*Ωignore_startΩ*/() => editResult = __sveltets_2_any(null);/*Ωignore_endΩ*/ } }
           { svelteHTML.createElement("label", { "class":`field`,});
             { svelteHTML.createElement("span", { "class":`flabel`,});  }
             { svelteHTML.createElement("select", {   "class":`input field-input`,"bind:value":editRisk,});/*Ωignore_startΩ*/() => editRisk = __sveltets_2_any(null);/*Ωignore_endΩ*/
                 for(let r of __sveltets_2_ensureArray(RISK_LEVELS)){r; { svelteHTML.createElement("option", { "value":r,});RISK_LABEL[r]; }}
             }
           }
           { svelteHTML.createElement("div", { "class":`edit-actions`,});
             { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":busy,"onclick":cancelEdits,});  }
             { svelteHTML.createElement("button", {     "class":`btn primary small`,"disabled":busy,"onclick":saveEdits,});busy ? 'Saving…' : 'Save'; }
           }
        }else{
           { svelteHTML.createElement("p", {  "class":`goal`,});!detail.goal;detail.goal ?? 'No goal recorded.'; }
          if(detail.context_summary){ { svelteHTML.createElement("p", { "class":`ctx dim`,});detail.context_summary; }}
          if(detail.result_summary){ { svelteHTML.createElement("div", { "class":`result`,}); { svelteHTML.createElement("span", { "class":`flabel`,});  } { svelteHTML.createElement("p", {});detail.result_summary; } }}
        }
       }

      
       { svelteHTML.createElement("section", { "class":`d-sec`,});
         { svelteHTML.createElement("h3", { "class":`section-title`,});  }
        if(detail.approvals.length === 0){
           { svelteHTML.createElement("p", { "class":`dim small`,});    }
        }else{
           { svelteHTML.createElement("ul", { "class":`approvals`,});
               for(let a of __sveltets_2_ensureArray(detail.approvals)){a.id;
               { svelteHTML.createElement("li", {  "class":`ap`,});a.status === 'pending';
                 { svelteHTML.createElement("div", { "class":`ap-main`,});
                   { const $$_egdaBsutatS6C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS6C({ target: __sveltets_2_any(), props: {    "tone":APPROVAL_TONE[a.status] ?? 'neutral',"label":a.status === 'rejected' ? 'Denied' : sentenceCase(a.status),}});}
                   { svelteHTML.createElement("span", {   "class":`small ap-reason`,"title":a.reason ?? undefined,});a.reason ?? 'Approval requested'; }
                   { svelteHTML.createElement("span", {   "class":`dim small`,"title":a.requested_by,}); who(a.requested_by); }
                 }
                if(a.status === 'pending'){
                   { const $$_snoitcAlavorppA5C = __sveltets_2_ensureComponent(ApprovalActions); new $$_snoitcAlavorppA5C({ target: __sveltets_2_any(), props: {           "busy":deciding?.id === a.id ? deciding.kind : null,"disabled":busy,"denyTarget":`this work item gate`,"onapprove":() => decide(a.id, 'approved'),"ondeny":(reason) => decide(a.id, 'rejected', reason),}});}
                } else if (a.decided_by){
                   { const $$_emoctuOlavorppA5C = __sveltets_2_ensureComponent(ApprovalOutcome); new $$_emoctuOlavorppA5C({ target: __sveltets_2_any(), props: {         "outcome":a.status === 'approved' ? 'approved' : 'denied',"by":who(a.decided_by),"at":a.decided_at,"note":a.decision_note,}});}
                }
               }
            }
           }
        }
         { svelteHTML.createElement("div", { "class":`ap-req`,});
           { svelteHTML.createElement("input", {            "dir":`auto`,"class":`input req-in`,"placeholder":`Reason (optional)`,"aria-label":`Approval reason (optional)`,"bind:value":approveReason,"onkeydown":(e) => e.key === 'Enter' && !busy && void requestApproval(),});/*Ωignore_startΩ*/() => approveReason = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":busy,"onclick":requestApproval,});  }
         }
       }

      
       { svelteHTML.createElement("section", { "class":`d-sec`,});
         { svelteHTML.createElement("h3", { "class":`section-title`,});  }
        if(detail.edges.length === 0){
           { svelteHTML.createElement("p", { "class":`dim small`,});    }
        }else{
           { svelteHTML.createElement("ul", { "class":`edges`,});
               for(let e of __sveltets_2_ensureArray(detail.edges)){e.direction + e.peer_id + e.relation;
               { svelteHTML.createElement("li", {});
                 { svelteHTML.createElement("button", {     "class":`edge-link`,"onclick":() => onOpen(e.peer_id),"title":`Open ${e.peer_title}`,});
                   { svelteHTML.createElement("span", {   "class":`rel`,"title":e.direction === 'out' ? 'Outgoing' : 'Incoming',});
                     { svelteHTML.createElement("span", { "class":`rel-dir`,}); { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":e.direction === 'out' ? 'chevronRight' : 'chevronLeft',"size":12,}});} }relLabel(e.relation);
                   }
                   { svelteHTML.createElement("span", { "class":`peer-icon`,}); { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":KIND_ICON[e.peer_kind],"size":12,}});} }
                   { svelteHTML.createElement("span", { "class":`peer-title`,});e.peer_title; }
                   { const $$_egdaBsutatS6C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS6C({ target: __sveltets_2_any(), props: {    "status":workStatus(e.peer_status),"variant":`text`,}});}
                 }
               }
            }
           }
        }
       }

      
       { svelteHTML.createElement("section", { "class":`d-sec`,});
         { svelteHTML.createElement("h3", { "class":`section-title`,});  }
        if(detail.artifacts.length === 0){
           { svelteHTML.createElement("p", { "class":`dim small`,});  }
        }else{
           { svelteHTML.createElement("ul", { "class":`artifacts`,});
               for(let a of __sveltets_2_ensureArray(detail.artifacts)){a.id;
               { svelteHTML.createElement("li", {});
                 { svelteHTML.createElement("span", { "class":`chip art-kind`,});ARTIFACT_LABEL[a.kind]; }
                 { svelteHTML.createElement("span", {   "class":`art-title`,"title":a.title,});a.title; }
                if(isUrl(a.ref)){
                   { svelteHTML.createElement("button", {     "class":`btn ghost small`,"onclick":() => void openExternal(a.ref),"title":a.ref ?? undefined,}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`external`,"size":12,}});}  }
                } else if (a.ref){ { svelteHTML.createElement("span", {   "class":`art-ref mono dim`,"title":a.ref,});a.ref; }}
               }
            }
           }
        }
       }

      
       { svelteHTML.createElement("section", { "class":`d-sec`,});
         { svelteHTML.createElement("h3", { "class":`section-title`,});  { svelteHTML.createElement("span", { "class":`count`,}); detail.events.length;  } }
        if(detail.events.length === 0){
           { svelteHTML.createElement("p", { "class":`dim small`,});   }
        }else{
           { svelteHTML.createElement("ul", { "class":`timeline`,});
               for(let ev of __sveltets_2_ensureArray(detail.events)){ev.id;
               { svelteHTML.createElement("li", {});
                 { svelteHTML.createElement("span", {   "class":`tl-dot actor-${ev.actor}`,"aria-hidden":`true`,}); }
                 { svelteHTML.createElement("span", { "class":`tl-type`,});sentenceCase(ev.event_type); }
                 { svelteHTML.createElement("span", { "class":`tl-actor dim small`,});ACTOR_LABEL[ev.actor]; }
                 { svelteHTML.createElement("span", {   "class":`tl-time dim small`,"title":localTime(ev.ts),});ago(ev.ts); }
                if(payloadPreview(ev.payload)){ { svelteHTML.createElement("span", {   "class":`tl-payload mono small dim`,"title":payloadPreview(ev.payload),});payloadPreview(ev.payload); }}
               }
            }
           }
        }
       }
     }
  }
 }


};
return { props: {} as any as Props, exports: {} as any as { canLeave: typeof canLeave }, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const WorkItemDetail__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type WorkItemDetail__SvelteComponent_ = ReturnType<typeof WorkItemDetail__SvelteComponent_>;
/*Ωignore_endΩ*/export default WorkItemDetail__SvelteComponent_;