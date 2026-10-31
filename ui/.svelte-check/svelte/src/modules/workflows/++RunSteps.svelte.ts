///<reference types="svelte" />
;// Reusable run detail: every step of a WorkflowRun with its status, duration,
// logs, error, and rendered "work product" (agent reply / JSON).

import {untrack, onDestroy} from 'svelte';
import { toastError } from '../../lib/toastError';
import {RunBodyCache, mergeCheckpointPage, fmtStepMs, sharedNodeBodies} from './runProgress';
import {api} from '../../lib/api/client';
import Icon from '../../lib/components/Icon.svelte';
import StatusBadge from '../../lib/components/StatusBadge.svelte';
import Badge from '../../lib/components/Badge.svelte';
import { runStatus } from '../../lib/status';
import Modal from '../../lib/components/Modal.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../lib/loadError';
import { plural } from '../../lib/plural';
import { toasts } from '../../lib/toast.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { proof } from '../../lib/stores/proof.svelte';
import { router } from '../../lib/router.svelte';
import { workflowNodeDetail, workflowCheckpointDetail, workflowCheckpointPage, retryRunNode } from '../../lib/api/workflows';
import type { WorkflowRun, NodeRunState, WorkflowCheckpoint, WorkflowCheckpointPage } from '../../lib/api/types';
import { copyTextOrThrow } from '../../lib/clipboard';
;

  interface Props {
    run: WorkflowRun;
    /** Resolve a node id to a friendly label. */
    nodeName?: (id: string) => string;
    /** Open a session INLINE (WF page's Agents tab) instead of navigating to the
     *  global Agents panel. When omitted, falls back to router navigation. */
    onOpenSession?: (id: string) => void;
    /** Merge a fresh run snapshot into the viewed run (e.g. after a step retry
     *  flips it back to running). When omitted, the WS/poll sync catches up. */
    onRunUpdated?: (run: WorkflowRun) => void;
    onRefresh?: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { run, nodeName = (id) => id, onOpenSession, onRunUpdated, onRefresh }: Props = $props();

  // Expansion is USER-owned, id-keyed state: a step that errors auto-opens once
  // (error visibility), but a manual toggle always wins afterward — live run
  // updates must never fight what the user opened or closed. Reset per run so
  // a freshly viewed run starts from its own defaults.
  let expanded = $state<Record<string, boolean>>({});
  let expandedRunId: string | null = null;
  $effect(() => {
    if (run.id !== expandedRunId) {
      expandedRunId = run.id;
      expanded = {};
    }
  });
  function isOpen(ns: NodeRunState): boolean {
    return expanded[ns.node_id] ?? ns.status === 'error';
  }
  function onToggle(ns: NodeRunState, open: boolean): void {
    // A toggle event also fires for our own programmatic open (error
    // auto-open); only a value that DIFFERS from the computed one is the user.
    if (open !== isOpen(ns)) expanded[ns.node_id] = open;
  }

  // Live elapsed time on running steps: a 1s client-side ticker while any step
  // runs (no network involved).
  let now = $state(Date.now());
  $effect(() => {
    if (!run.nodes.some((n) => n.status === 'running')) return;
    const iv = setInterval(() => (now = Date.now()), 1000);
    return () => clearInterval(iv);
  });
  function elapsedMs(ns: NodeRunState): number | null {
    if (!ns.started_at) return null;
    const t = new Date(ns.started_at).getTime();
    return Number.isFinite(t) ? Math.max(0, now - t) : null;
  }

  const fmtMs = fmtStepMs;

  // Phase lines the step engine emits (turn oracle): muted, so the ▶/✓/⚠/↻
  // lines around them stay the ones that read as events.
  const PHASE_PREFIXES = ['⏳', '✉', '⚙', '🧩', '⏸', '📄'];
  function isPhaseLine(l: string): boolean {
    return PHASE_PREFIXES.some((p) => l.startsWith(p));
  }

  function reply(out: unknown): string | null {
    if (out && typeof out === 'object' && typeof (out as { reply?: unknown }).reply === 'string') {
      return (out as { reply: string }).reply;
    }
    return null;
  }
  function hasOutput(ns: NodeRunState): boolean {
    return ns.output !== undefined && ns.output !== null;
  }

  /** A review id surfaced by a review_run step's output (if any). */
  function reviewIdOf(out: unknown): string | null {
    if (out && typeof out === 'object') {
      const r = (out as { review_id?: unknown }).review_id;
      if (typeof r === 'string' && r) return r;
    }
    return null;
  }

  /** Best-effort repo id for a step (step output, falling back to run input). */
  function repoIdOf(out: unknown): string | null {
    const fromOut = out && typeof out === 'object' ? (out as { repo_id?: unknown }).repo_id : undefined;
    if (typeof fromOut === 'string' && fromOut) return fromOut;
    const inp = run.input;
    const fromIn = inp && typeof inp === 'object' ? (inp as { repo_id?: unknown }).repo_id : undefined;
    return typeof fromIn === 'string' && fromIn ? fromIn : null;
  }

  /** Open an agent session this step drove. Prefers the inline handler (WF page
   *  Agents tab); falls back to router nav when used outside the WF page. */
  function openSession(id: string): void {
    if (onOpenSession) onOpenSession(id);
    else ws.navigateToSession(id);
  }

  /** Open the proof pack assembled for this run in the Proof module. */
  function viewProof(id: string): void {
    void proof.open(id);
    router.go('proof');
  }

  /** Open the review a step produced. There's no standalone review-by-id route,
   *  so land the user in the repo's git view (which surfaces its reviews) when
   *  the repo is resolvable; otherwise the git module. The review id is in the
   *  link tooltip. */
  async function openReview(out: unknown): Promise<void> {
    if (!repoIdOf(out) && run.summary) {
      try {const full = await api.get<WorkflowRun>(`/workflow-runs/${encodeURIComponent(run.id)}`); if (full.id === run.id) run.input = full.input;} catch { /* Fall back to the Git overview. */ }
    }
    const repo = repoIdOf(out);
    router.go(repo ? `git/${repo}` : 'git');
  }

  async function copy(text: string, label = 'output'): Promise<void> {
    try {
      await copyTextOrThrow(text);
      toasts.success(`Copied ${label}`);
    } catch {
      toasts.error('Couldn’t copy to the clipboard', 'Select the text and copy it manually.');
    }
  }
  function asText(out: unknown): string {
    return typeof out === 'string' ? out : JSON.stringify(out, null, 2);
  }

  // "Retry step": re-run a single ERRORED step of a FINISHED run. Never offered
  // while the run is still pending/running (the server 409s), nor on steps that
  // ended any other way (done/skipped — the server 400s those).
  const runFinished = $derived(
    run.status === 'success' || run.status === 'error' || run.status === 'canceled',
  );
  function canRetry(ns: NodeRunState): boolean {
    return ns.status === 'error' && runFinished;
  }
  // "Re-run from here": re-enter THIS run at a settled step and re-execute it
  // plus everything downstream — same run id, so the run's context dir and
  // otto-wf worktree (the files earlier steps produced) are reused. This is
  // the stateful counterpart of the canvas "Run from here" (which mints a
  // fresh run with a clean worktree).
  function canRerunFrom(ns: NodeRunState): boolean {
    return runFinished && ns.status !== 'pending' && ns.status !== 'running';
  }
  let retryingId = $state<string | null>(null);
  async function retryStep(ns: NodeRunState, includeDownstream = false): Promise<void> {
    if (retryingId) return; // one retry in flight at a time
    retryingId = ns.node_id;
    try {
      const nr = await retryRunNode(run.id, ns.node_id, includeDownstream);
      onRunUpdated?.(nr); // flips the run back to running; WS keeps it live
      toasts.info(includeDownstream ? 'Re-running from step…' : 'Step retrying…', nodeName(ns.node_id));
    } catch (e) {
      toastError('Couldn’t retry the step', e);
    } finally {
      retryingId = null;
    }
  }

  // "Zoom in on a specific step" (R6): open the step's full logs + work product
  // in a large modal, so a big JSON config/output is actually readable.
  let zoomed = $state<NodeRunState | null>(null);

  const bodies = new RunBodyCache();
  let bodyTick = $state(0);
  let bodyLoading = $state<Record<string,boolean>>({});
  let bodyErrors = $state<Record<string,string>>({});
  const attempted = new Set<string>();
  let detailRun: string | null = null;
  let detailGeneration: number | undefined;
  let checkpointOpen = $state(false);
  let checkpointPages = $state<{cursor?:string; page:WorkflowCheckpointPage}[]>([]);
  let checkpointPageIndex = $state(0);
  let checkpointLoading = $state(false);
  let checkpointError = $state('');
  let checkpointExpanded = $state<Record<string,boolean>>({});
  let loadedCheckpoints = $state<WorkflowCheckpoint[]>([]);
  const checkpointRowVersions = new Map<string, number>();
  let checkpointRefreshTimer: ReturnType<typeof setTimeout> | null = null;
  function queueCheckpointRefresh(): void {
    if (checkpointRefreshTimer || !checkpointOpen) return;
    checkpointRefreshTimer = setTimeout(() => {
      checkpointRefreshTimer = null;
      if (checkpointOpen) void loadCheckpointPage();
    }, 1000);
  }
  onDestroy(() => { if (checkpointRefreshTimer) clearTimeout(checkpointRefreshTimer); checkpointRequest++; });
  let checkpointRefreshQueued = false;
  let checkpointRequest = 0;
  const checkpointRows = $derived(checkpointPages[checkpointPageIndex]?.page.items ?? run.checkpoints ?? []);

  function displayedNode(summary:NodeRunState):NodeRunState {
    void bodyTick;
    if (!summary.detail_version) return summary;
    return bodies.get<NodeRunState>(run.id,`n:${summary.node_id}`,summary.detail_version) ?? summary;
  }
  function displayedCheckpoint(summary:WorkflowCheckpoint):WorkflowCheckpoint {
    void bodyTick;
    return summary.detail_version ? bodies.get<WorkflowCheckpoint>(run.id,`c:${summary.node_id}`,summary.detail_version) ?? summary : summary;
  }
  async function loadBody(summary:NodeRunState | WorkflowCheckpoint, checkpoint=false, retry=false):Promise<void> {
    const version=summary.detail_version;
    if (!version) return;
    const id=run.id, key=`${checkpoint?'c':'n'}:${summary.node_id}`, attempt=`${id}:${key}:${version}`;
    if (bodies.get(id,key,version) || (!retry && attempted.has(attempt))) return;
    // The inspector may already hold this exact body (shared fetch path).
    const shared=checkpoint ? null : sharedNodeBodies.peek(id,summary.node_id,version);
    if (shared) {bodies.put(id,key,version,shared); bodyTick++; return;}
    attempted.add(attempt); bodyLoading[key]=true; delete bodyErrors[key];
    try {
      const result=checkpoint ? await workflowCheckpointDetail(id,summary.node_id) : await sharedNodeBodies.fetch(id,summary.node_id,(signal)=>workflowNodeDetail(id,summary.node_id,signal));
      if (run.id !== id) return;
      const current=checkpoint ? (run.summary ? loadedCheckpoints : run.checkpoints)?.find(c=>c.node_id===summary.node_id) : run.nodes.find(n=>n.node_id===summary.node_id);
      if (current?.detail_version !== result.detail_version) {onRefresh?.();return;}
      bodies.put(id,key,result.detail_version,result.body); attempted.delete(attempt); bodyTick++;
    } catch(e) {if(run.id===id) bodyErrors[key]=loadErrorText(e);}
    finally {if(run.id===id) bodyLoading[key]=false;}
  }
  async function loadCheckpointPage(index=checkpointPageIndex):Promise<void> {
    if (!run.summary) return;
    if (checkpointLoading) {checkpointRefreshQueued=true;return;}
    const id=run.id, generation=run.checkpoint_generation;
    const cursor=index===0?undefined:checkpointPages[index]?.cursor ?? checkpointPages[index-1]?.page.next_cursor ?? undefined;
    if(index>0 && !cursor) return;
    const request=++checkpointRequest;
    checkpointLoading=true;checkpointError='';
    try {
      const page=await workflowCheckpointPage(id,cursor);
      if(run.id!==id || run.checkpoint_generation!==generation || checkpointRequest!==request) return;
      if(page.generation !== generation) return;
      const merged=mergeCheckpointPage(page,loadedCheckpoints,checkpointRowVersions);
      checkpointPages[index]={cursor,page:{...page,items:merged.items}}; checkpointPageIndex=index;
      loadedCheckpoints=merged.known;
      if(page.checkpoint_rev < (run.checkpoint_rev??0)) checkpointRefreshQueued=true;
    } catch(e) {if(run.id===id && checkpointRequest===request) checkpointError=loadErrorText(e);}
    finally {
      if(run.id===id && checkpointRequest===request) {checkpointLoading=false;if(checkpointRefreshQueued && checkpointOpen){checkpointRefreshQueued=false;queueCheckpointRefresh();}}
    }
  }
  $effect(() => {
    const id=run.id, generation=run.checkpoint_generation;
    if(detailRun!==id || detailGeneration!==generation) {
      const newRun=detailRun!==id;
      detailRun=id;detailGeneration=generation;
      bodies.clear();attempted.clear();bodyLoading={};bodyErrors={};
      if(checkpointRefreshTimer) clearTimeout(checkpointRefreshTimer); checkpointRefreshTimer=null;
      loadedCheckpoints=[];checkpointRowVersions.clear();
      checkpointPages=[];checkpointPageIndex=0;checkpointLoading=false;checkpointRequest++;checkpointRefreshQueued=false;
      if(newRun) {zoomed=null;checkpointExpanded={};checkpointOpen=run.status==='running';}
    }
  });
  $effect(() => {
    const id=run.id, revision=run.checkpoint_rev, open=checkpointOpen;
    void id;void revision;
    if(open && run.summary) untrack(()=>queueCheckpointRefresh());
  });
  $effect(() => {
    const nodes=run.nodes.filter(node=>isOpen(node));
    const zoom=run.nodes.find(node=>node.node_id===zoomed?.node_id);
    if(zoom && !nodes.some(node=>node.node_id===zoom.node_id)) nodes.push(zoom);
    const checkpoints=checkpointOpen ? checkpointRows.filter(cp=>checkpointExpanded[cp.node_id]) : [];
    // Version changes refresh expanded bodies; collapsed records stay metadata.
    const versions=[...nodes,...checkpoints].map(value=>value.detail_version);
    void versions;
    untrack(()=>{
      bodies.pin(run.id,[...nodes.map(n=>`n:${n.node_id}`),...checkpoints.map(cp=>`c:${cp.node_id}`),...(zoomed?[`n:${zoomed.node_id}`]:[])]);
      for(const node of nodes) void loadBody(node);
      for(const checkpoint of checkpoints) void loadBody(checkpoint,true);
    });
  });
;
async () => {

if(run.proof_pack_id || run.workflow_version != null){
   { svelteHTML.createElement("div", { "class":`run-meta`,});
    if(run.workflow_version != null){
       { svelteHTML.createElement("span", {   "class":`rm-ver`,"title":`workflow version this run executed`,}); run.workflow_version; }
    }
    if(run.proof_pack_id){
       { svelteHTML.createElement("button", {       "class":`link-btn`,"title":`Open the proof pack assembled for this run`,"onclick":() => { if (run.proof_pack_id) viewProof(run.proof_pack_id); },});
         { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}   
       }
    }
   }
}

if((run.checkpoint_count ?? run.checkpoints?.length ?? 0) > 0){
   { svelteHTML.createElement("details", {     "class":`checkpoint-list`,"open":checkpointOpen,"ontoggle":(event)=>checkpointOpen=event.currentTarget.open,});
     { svelteHTML.createElement("summary", {});   run.checkpoint_done ?? run.checkpoints?.filter(c=>c.status==='success').length ?? 0; run.checkpoint_count ?? run.checkpoints?.length ?? 0;  }
    if(checkpointOpen){
      if(checkpointError || checkpointLoading){ { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {            "what":`checkpoints`,"variant":`compact`,"loading":checkpointLoading,"error":checkpointError,"empty":true,"onretry":()=>void loadCheckpointPage(),}});}}
         for(let summary of __sveltets_2_ensureArray(checkpointRows.filter(c=>c.iteration>0))){summary.node_id;
        const checkpoint=displayedCheckpoint(summary);
         { svelteHTML.createElement("details", {   "open":checkpointExpanded[summary.node_id]??false,"ontoggle":(event)=>checkpointExpanded[summary.node_id]=event.currentTarget.open,});
           { svelteHTML.createElement("summary", {});summary.name;   summary.iteration;  runStatus(summary.status).label;  plural(summary.attempts,'attempt'); }
          if(checkpointExpanded[summary.node_id]){
            if(bodyLoading[`c:${summary.node_id}`] || bodyErrors[`c:${summary.node_id}`]){ { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {            "what":`step details`,"variant":`compact`,"loading":bodyLoading[`c:${summary.node_id}`],"error":bodyErrors[`c:${summary.node_id}`],"empty":true,"onretry":()=>void loadBody(summary,true,true),}});}}
            if(checkpoint.error){ { svelteHTML.createElement("p", { "class":`err`,});checkpoint.error; }}
            if(checkpoint.logs.length){ { svelteHTML.createElement("pre", {});checkpoint.logs.join('\n'); }}
            if(!summary.detail_version || checkpoint!==summary){ { svelteHTML.createElement("pre", {});JSON.stringify(checkpoint.output??checkpoint.input,null,2); }}
          }
         }
      }
      if(checkpointPageIndex>0){ { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":()=>void loadCheckpointPage(checkpointPageIndex-1),"disabled":checkpointLoading,});checkpointLoading ? 'Loading checkpoints…' : 'Previous checkpoints'; }}
      if(checkpointPages[checkpointPageIndex]?.page.next_cursor){ { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":()=>void loadCheckpointPage(checkpointPageIndex+1),"disabled":checkpointLoading,});checkpointLoading ? 'Loading checkpoints…' : 'More checkpoints'; }}
    }
   }
}
 { svelteHTML.createElement("div", { "class":`steps`,});
     for(let summary of __sveltets_2_ensureArray(run.nodes)){summary.node_id;
    const ns = displayedNode(summary);
     { svelteHTML.createElement("div", { "class":`step-wrap`,});
     { svelteHTML.createElement("details", {         "class":`step`,"open":isOpen(ns),"ontoggle":(e) => onToggle(ns, e.currentTarget.open),...__sveltets_2_empty({"data-status":ns.status}),});
       { svelteHTML.createElement("summary", {});
         { svelteHTML.createElement("span", {   "class":`dot ${runStatus(ns.status).key}`,"aria-hidden":`true`,}); }
         { svelteHTML.createElement("span", { "class":`name`,});nodeName(ns.node_id); }
         { const $$_egdaBsutatS4C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS4C({ target: __sveltets_2_any(), props: {      "status":runStatus(ns.status),"variant":`text`,"dot":false,}});}
        if((ns.attempts ?? 1) > 1){ { const $$_egdaB4C = __sveltets_2_ensureComponent(Badge); new $$_egdaB4C({ target: __sveltets_2_any(), props: {      "tone":`warn`,"title":`This step was retried`,"label":plural(ns.attempts ?? 1, 'attempt'),}});}}
         { svelteHTML.createElement("span", { "class":`sp-grow`,}); }
        if(ns.duration_ms != null){
           { svelteHTML.createElement("span", { "class":`ms`,});fmtMs(ns.duration_ms); }
        } else if (ns.status === 'running' && elapsedMs(ns) != null){
           { svelteHTML.createElement("span", { "class":`ms live`,});fmtMs(elapsedMs(ns)); }
        }
        if(ns.status === 'running' && ns.activity && ns.activity.subagents.length){
          const subs = ns.activity.subagents;
          const running = subs.filter((s) => s.status === 'running').length;
          const done = subs.filter((s) => s.status === 'done').length;
          const failed = subs.filter((s) => s.status === 'failed').length;
          const subParts = [
            plural(subs.length, 'sub-agent'),
            running ? `${running} running` : '',
            done ? `${done} done` : '',
            failed ? `${failed} failed` : '',
          ].filter(Boolean);
           { const $$_egdaB4C = __sveltets_2_ensureComponent(Badge); new $$_egdaB4C({ target: __sveltets_2_any(), props: {       "testid":`subagent-chip`,"title":`Sub-agents / background tasks the step launched`,"label":subParts.join(' · '),}});}
        }
        if(ns.status === 'running' && ns.activity?.phase){
          
           { svelteHTML.createElement("div", {   "class":`phase`,...__sveltets_2_empty({"data-testid":`step-phase`}),});ns.activity.hold_reason ?? ns.activity.phase; }
        }
       }
      if(isOpen(summary)){
       { svelteHTML.createElement("div", { "class":`body`,});
        if(bodyLoading[`n:${summary.node_id}`] || bodyErrors[`n:${summary.node_id}`]){ { const $$_etatSdaoL4C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL4C({ target: __sveltets_2_any(), props: {            "what":`step details`,"variant":`compact`,"loading":bodyLoading[`n:${summary.node_id}`],"error":bodyErrors[`n:${summary.node_id}`],"empty":true,"onretry":()=>void loadBody(summary,false,true),}});}}
        if(ns.error){
           { svelteHTML.createElement("div", { "class":`err`,});ns.error; }
        }

        if(ns.sessions?.length || reviewIdOf(ns.output) || canRetry(ns) || canRerunFrom(ns)){
           { svelteHTML.createElement("div", { "class":`links`,});
            if(canRetry(ns)){
               { svelteHTML.createElement("button", {         "class":`link-btn`,"title":`Re-run ONLY this errored step, keeping this run’s files/worktree`,"disabled":retryingId === ns.node_id,"onclick":() => void retryStep(ns),});
                 { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":12,}});} retryingId === ns.node_id ? 'Retrying…' : 'Retry step';
               }
            }
            if(canRerunFrom(ns)){
               { svelteHTML.createElement("button", {         "class":`link-btn`,"title":`Re-run this step AND everything after it, keeping this run’s files/worktree (unlike the canvas Run-from-here, which starts a fresh run with a clean worktree)`,"disabled":retryingId === ns.node_id,"onclick":() => void retryStep(ns, true),});
                 { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`play`,"size":12,}});}   
               }
            }
               for(let sid of __sveltets_2_ensureArray(ns.sessions ?? [])){sid;
               { svelteHTML.createElement("button", {     "class":`link-btn`,"title":`Open session ${sid}`,"onclick":() => openSession(sid),});
                 { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`terminal`,"size":12,}});}  
               }
            }
            if(reviewIdOf(ns.output)){
               { svelteHTML.createElement("button", {       "class":`link-btn`,"title":`Open review ${reviewIdOf(ns.output)}`,"onclick":() => openReview(ns.output),});
                 { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`eye`,"size":12,}});}  
               }
            }
           }
        }
        if(ns.logs?.length){
           { svelteHTML.createElement("div", { "class":`logs`,});  for(let l of __sveltets_2_ensureArray(ns.logs)){ { svelteHTML.createElement("div", {   });isPhaseLine(l);l.startsWith('⚠');l.startsWith('✓');l; }} }
        }

        if(hasOutput(ns)){
          const txt = reply(ns.output);
           { svelteHTML.createElement("div", { "class":`product`,});
             { svelteHTML.createElement("div", { "class":`product-h`,});
               { svelteHTML.createElement("span", {});  }
               { svelteHTML.createElement("span", { "class":`ph-grow`,}); }
               { svelteHTML.createElement("button", {     "class":`copy-btn`,"title":`Copy to clipboard`,"onclick":() => copy(asText(ns.output), 'output'),});
                 { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`copy`,"size":12,}});} 
               }
             }
            if(txt){
               { svelteHTML.createElement("pre", { "class":`text scrolly`,});txt; }
            }else{
               { svelteHTML.createElement("pre", { "class":`json scrolly`,});JSON.stringify(ns.output, null, 2); }
            }
           }
        } else if (ns.status === 'success' && (!summary.detail_version || ns !== summary)){
           { svelteHTML.createElement("div", { "class":`muted`,});  }
        }
       }
      }
     }
     { svelteHTML.createElement("button", {         "class":`zoom-btn`,"title":`Zoom in on this step`,"aria-label":`Zoom in on ${nodeName(summary.node_id)}`,"onclick":(e) => {
        e.preventDefault();
        e.stopPropagation();
        zoomed = summary;
        void loadBody(summary);
      },});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`maximize`,"size":12,}});}
     }
     }
  }
 }

if(zoomed){
  const z = displayedNode(run.nodes.find(n=>n.node_id===zoomed?.node_id)??zoomed);
   { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); new $$_ladoM0C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"title":`Step · ${nodeName(z.node_id)}`,"width":920,"onclose":() => (zoomed = null),}});
     { svelteHTML.createElement("div", { "class":`zoom`,});
      if(bodyLoading[`n:${z.node_id}`] || bodyErrors[`n:${z.node_id}`]){ { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {            "what":`step details`,"variant":`compact`,"loading":bodyLoading[`n:${z.node_id}`],"error":bodyErrors[`n:${z.node_id}`],"empty":true,"onretry":()=>void loadBody(zoomed!,false,true),}});}}
      if(z.error){ { svelteHTML.createElement("div", { "class":`err`,});z.error; }}
      if(z.logs?.length){
         { svelteHTML.createElement("div", { "class":`zh`,}); { svelteHTML.createElement("span", {});  } }
         { svelteHTML.createElement("pre", { "class":`logs zbig`,});z.logs.join('\n'); }
      }
      if(hasOutput(z)){
        const zt = reply(z.output);
         { svelteHTML.createElement("div", { "class":`zh`,});
           { svelteHTML.createElement("span", {});  }
           { svelteHTML.createElement("span", { "class":`ph-grow`,}); }
           { svelteHTML.createElement("button", {     "class":`copy-btn`,"title":`Copy to clipboard`,"onclick":() => copy(asText(z.output), 'output'),});
             { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`copy`,"size":12,}});} 
           }
         }
        if(zt){
           { svelteHTML.createElement("pre", { "class":`text zbig`,});zt; }
        }else{
           { svelteHTML.createElement("pre", { "class":`json zbig`,});JSON.stringify(z.output, null, 2); }
        }
      } else if (!z.error && !z.logs?.length){
         { svelteHTML.createElement("div", { "class":`muted`,});  }
      }
     }
   Modal}
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const RunSteps__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type RunSteps__SvelteComponent_ = ReturnType<typeof RunSteps__SvelteComponent_>;
/*Ωignore_endΩ*/export default RunSteps__SvelteComponent_;