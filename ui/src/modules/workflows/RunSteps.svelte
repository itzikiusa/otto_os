<script lang="ts">
  // Reusable run detail: every step of a WorkflowRun with its status, duration,
  // logs, error, and rendered "work product" (agent reply / JSON).
  import {untrack, onDestroy} from 'svelte';
  import {RunBodyCache, mergeCheckpointPage, fmtStepMs} from './runProgress';
  import {api} from '../../lib/api/client';
  import Icon from '../../lib/components/Icon.svelte';
  import StatusBadge from '../../lib/components/StatusBadge.svelte';
  import { runStatus } from '../../lib/status';
  import Modal from '../../lib/components/Modal.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { proof } from '../../lib/stores/proof.svelte';
  import { router } from '../../lib/router.svelte';
  import { workflowNodeDetail, workflowCheckpointDetail, workflowCheckpointPage, retryRunNode } from '../../lib/api/workflows';
  import type { WorkflowRun, NodeRunState, WorkflowCheckpoint, WorkflowCheckpointPage } from '../../lib/api/types';
  import { copyTextOrThrow } from '../../lib/clipboard';

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
  }
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
      toasts.error('Copy failed');
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
      toasts.error('Retry failed', e instanceof Error ? e.message : String(e));
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
    attempted.add(attempt); bodyLoading[key]=true; delete bodyErrors[key];
    try {
      const result=checkpoint ? await workflowCheckpointDetail(id,summary.node_id) : await workflowNodeDetail(id,summary.node_id);
      if (run.id !== id) return;
      const current=checkpoint ? (run.summary ? loadedCheckpoints : run.checkpoints)?.find(c=>c.node_id===summary.node_id) : run.nodes.find(n=>n.node_id===summary.node_id);
      if (current?.detail_version !== result.detail_version) {onRefresh?.();return;}
      bodies.put(id,key,result.detail_version,result.body); attempted.delete(attempt); bodyTick++;
    } catch(e) {if(run.id===id) bodyErrors[key]=e instanceof Error?e.message:String(e);}
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
    } catch(e) {if(run.id===id && checkpointRequest===request) checkpointError=e instanceof Error?e.message:String(e);}
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
</script>

{#if run.proof_pack_id || run.workflow_version != null}
  <div class="run-meta">
    {#if run.workflow_version != null}
      <span class="rm-ver" title="workflow version this run executed">v{run.workflow_version}</span>
    {/if}
    {#if run.proof_pack_id}
      <button
        class="link-btn"
        title="Open the proof pack assembled for this run"
        onclick={() => { if (run.proof_pack_id) viewProof(run.proof_pack_id); }}
      >
        <Icon name="check" size={11} /> View proof pack
      </button>
    {/if}
  </div>
{/if}

{#if (run.checkpoint_count ?? run.checkpoints?.length ?? 0) > 0}
  <details class="checkpoint-list" open={checkpointOpen} ontoggle={(event)=>checkpointOpen=event.currentTarget.open}>
    <summary>Loop checkpoints · {run.checkpoint_done ?? run.checkpoints?.filter(c=>c.status==='success').length ?? 0}/{run.checkpoint_count ?? run.checkpoints?.length ?? 0} complete</summary>
    {#if checkpointOpen}
      {#if checkpointError}<p class="err">{checkpointError} <button onclick={()=>void loadCheckpointPage()}>Retry</button></p>{/if}
      {#if checkpointLoading}<p>Loading checkpoints…</p>{/if}
      {#each checkpointRows.filter(c=>c.iteration>0) as summary (summary.node_id)}
        {@const checkpoint=displayedCheckpoint(summary)}
        <details open={checkpointExpanded[summary.node_id]??false} ontoggle={(event)=>checkpointExpanded[summary.node_id]=event.currentTarget.open}>
          <summary>{summary.name} · iteration {summary.iteration} · {runStatus(summary.status).label} · {summary.attempts} attempt(s)</summary>
          {#if checkpointExpanded[summary.node_id]}
            {#if bodyLoading[`c:${summary.node_id}`]}<p>Loading details…</p>{/if}
            {#if bodyErrors[`c:${summary.node_id}`]}<p class="err">{bodyErrors[`c:${summary.node_id}`]} <button onclick={()=>void loadBody(summary,true,true)}>Retry</button></p>{/if}
            {#if checkpoint.error}<p class="err">{checkpoint.error}</p>{/if}
            {#if checkpoint.logs.length}<pre>{checkpoint.logs.join('\n')}</pre>{/if}
            {#if !summary.detail_version || checkpoint!==summary}<pre>{JSON.stringify(checkpoint.output??checkpoint.input,null,2)}</pre>{/if}
          {/if}
        </details>
      {/each}
      {#if checkpointPageIndex>0}<button onclick={()=>void loadCheckpointPage(checkpointPageIndex-1)} disabled={checkpointLoading}>Previous checkpoints</button>{/if}
      {#if checkpointPages[checkpointPageIndex]?.page.next_cursor}<button onclick={()=>void loadCheckpointPage(checkpointPageIndex+1)} disabled={checkpointLoading}>More checkpoints</button>{/if}
    {/if}
  </details>
{/if}
<div class="steps">
  {#each run.nodes as summary (summary.node_id)}
    {@const ns = displayedNode(summary)}
    <details
      class="step"
      open={isOpen(ns)}
      ontoggle={(e) => onToggle(ns, e.currentTarget.open)}
      data-status={ns.status}
    >
      <summary>
        <span class="dot {runStatus(ns.status).key}" aria-hidden="true"></span>
        <span class="name">{nodeName(ns.node_id)}</span>
        <StatusBadge status={runStatus(ns.status)} variant="text" dot={false} />
        {#if (ns.attempts ?? 1) > 1}<span class="chip" title="step was retried">×{ns.attempts} attempts</span>{/if}
        <span class="sp-grow"></span>
        {#if ns.duration_ms != null}
          <span class="ms">{fmtMs(ns.duration_ms)}</span>
        {:else if ns.status === 'running' && elapsedMs(ns) != null}
          <span class="ms live">{fmtMs(elapsedMs(ns))}</span>
        {/if}
        {#if ns.status === 'running' && ns.activity && ns.activity.subagents.length}
          {@const running = ns.activity.subagents.filter((s) => s.status === 'running').length}
          {@const done = ns.activity.subagents.filter((s) => s.status === 'done').length}
          <span
            class="chip subagents"
            data-testid="subagent-chip"
            title="Sub-agents / background tasks the step launched"
          >
            {ns.activity.subagents.length} sub-agent{ns.activity.subagents.length === 1 ? '' : 's'} · {running ? `${running} running` : `${done} done`}
          </span>
        {/if}
        <button
          class="zoom-btn"
          title="Zoom in on this step"
          aria-label="Zoom in on this step"
          onclick={(e) => {
            e.preventDefault();
            e.stopPropagation();
            zoomed = summary;
            void loadBody(summary);
          }}
        >
          <Icon name="maximize" size={12} />
        </button>
        {#if ns.status === 'running' && ns.activity?.phase}
          <!-- What the engine is waiting for right now; `hold_reason` wins when
               the step LOOKS idle but is deliberately being held. It belongs to
               the CARD, not the body (design §4 "muted line under the title") —
               a collapsed running step must still say what it is waiting on. -->
          <div class="phase" data-testid="step-phase">{ns.activity.hold_reason ?? ns.activity.phase}</div>
        {/if}
      </summary>
      {#if isOpen(summary)}
      <div class="body">
        {#if bodyLoading[`n:${summary.node_id}`]}<p>Loading details…</p>{/if}
        {#if bodyErrors[`n:${summary.node_id}`]}<p class="err">{bodyErrors[`n:${summary.node_id}`]} <button onclick={()=>void loadBody(summary,false,true)}>Retry</button></p>{/if}
        {#if ns.error}
          <div class="err">{ns.error}</div>
        {/if}

        {#if ns.sessions?.length || reviewIdOf(ns.output) || canRetry(ns) || canRerunFrom(ns)}
          <div class="links">
            {#if canRetry(ns)}
              <button
                class="link-btn"
                title="Re-run ONLY this errored step, keeping this run's files/worktree"
                disabled={retryingId === ns.node_id}
                onclick={() => void retryStep(ns)}
              >
                <Icon name="refresh" size={11} /> {retryingId === ns.node_id ? 'Retrying…' : 'Retry step'}
              </button>
            {/if}
            {#if canRerunFrom(ns)}
              <button
                class="link-btn"
                title="Re-run this step AND everything after it, keeping this run's files/worktree (unlike the canvas Run-from-here, which starts a fresh run with a clean worktree)"
                disabled={retryingId === ns.node_id}
                onclick={() => void retryStep(ns, true)}
              >
                <Icon name="play" size={11} /> Re-run from here
              </button>
            {/if}
            {#each ns.sessions ?? [] as sid (sid)}
              <button class="link-btn" title={`Open session ${sid}`} onclick={() => openSession(sid)}>
                <Icon name="terminal" size={11} /> Open session
              </button>
            {/each}
            {#if reviewIdOf(ns.output)}
              <button
                class="link-btn"
                title={`Open review ${reviewIdOf(ns.output)}`}
                onclick={() => openReview(ns.output)}
              >
                <Icon name="eye" size={11} /> Open review
              </button>
            {/if}
          </div>
        {/if}
        {#if ns.logs?.length}
          <div class="logs">{#each ns.logs as l}<div class:phase-line={isPhaseLine(l)} class:warn-line={l.startsWith('⚠')} class:ok-line={l.startsWith('✓')}>{l}</div>{/each}</div>
        {/if}

        {#if hasOutput(ns)}
          {@const txt = reply(ns.output)}
          <div class="product">
            <div class="product-h">
              <span>Work product</span>
              <span class="ph-grow"></span>
              <button class="copy-btn" title="Copy to clipboard" onclick={() => copy(asText(ns.output), 'output')}>
                <Icon name="copy" size={11} /> Copy
              </button>
            </div>
            {#if txt}
              <pre class="text scrolly">{txt}</pre>
            {:else}
              <pre class="json scrolly">{JSON.stringify(ns.output, null, 2)}</pre>
            {/if}
          </div>
        {:else if ns.status === 'success' && (!summary.detail_version || ns !== summary)}
          <div class="muted">No output.</div>
        {/if}
      </div>
      {/if}
    </details>
  {/each}
</div>

{#if zoomed}
  {@const z = displayedNode(run.nodes.find(n=>n.node_id===zoomed?.node_id)??zoomed)}
  <Modal title={`Step · ${nodeName(z.node_id)}`} width={920} onclose={() => (zoomed = null)}>
    <div class="zoom">
      {#if bodyLoading[`n:${z.node_id}`]}<p>Loading details…</p>{/if}
      {#if bodyErrors[`n:${z.node_id}`]}<p class="err">{bodyErrors[`n:${z.node_id}`]} <button onclick={()=>void loadBody(zoomed!,false,true)}>Retry</button></p>{/if}
      {#if z.error}<div class="err">{z.error}</div>{/if}
      {#if z.logs?.length}
        <div class="zh"><span>Logs</span></div>
        <pre class="logs zbig">{z.logs.join('\n')}</pre>
      {/if}
      {#if hasOutput(z)}
        {@const zt = reply(z.output)}
        <div class="zh">
          <span>Work product</span>
          <span class="ph-grow"></span>
          <button class="copy-btn" title="Copy to clipboard" onclick={() => copy(asText(z.output), 'output')}>
            <Icon name="copy" size={11} /> Copy
          </button>
        </div>
        {#if zt}
          <pre class="text zbig">{zt}</pre>
        {:else}
          <pre class="json zbig">{JSON.stringify(z.output, null, 2)}</pre>
        {/if}
      {:else if !z.error && !z.logs?.length}
        <div class="muted">No output.</div>
      {/if}
    </div>
  </Modal>
{/if}

<style>
  .checkpoint-list { margin: 8px 0; padding: 8px; border: 1px solid var(--border); }
  .checkpoint-list pre { max-height: 240px; overflow: auto; white-space: pre-wrap; }
  .steps {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .run-meta {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 8px;
  }
  .rm-ver {
    font-size: var(--fs-xs);
    font-family: var(--font-mono);
    color: var(--accent-text);
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    padding: 1px 7px;
    border-radius: 99px;
  }
  .chip {
    font-size: var(--fs-xs);
    color: var(--warning);
    background: var(--warning-soft);
    padding: 1px 7px;
    border-radius: 99px;
  }
  /* Sub-agent chip: neutral, not the warn colour the retry chip uses. */
  .chip.subagents {
    color: var(--text-dim);
    background: color-mix(in srgb, var(--accent) 12%, transparent);
  }
  /* Live phase under the step title (or why it's being held). */
  /* Its own row under the title line (the summary wraps), so the phase reads
     as a caption of the step rather than an item in the header row. */
  .phase {
    flex-basis: 100%;
    margin-top: -2px;
    color: var(--text-dim);
    font-size: 11px;
  }
  .links {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .link-btn {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text-dim);
    font-size: var(--fs-xs);
    padding: 2px 8px;
    border-radius: var(--radius-s);
    cursor: pointer;
  }
  .link-btn:hover {
    color: var(--text);
    border-color: color-mix(in srgb, var(--accent) 50%, var(--border));
  }
  .link-btn:disabled {
    opacity: 0.6;
    cursor: default;
  }
  .step {
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
    overflow: hidden;
  }
  .step[data-status='error'] {
    border-color: color-mix(in srgb, var(--danger) 45%, var(--border));
  }
  .step[data-status='success'] {
    border-color: color-mix(in srgb, var(--success) 35%, var(--border));
  }
  summary {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 9px;
    padding: 9px 12px;
    cursor: pointer;
    list-style: none;
    font-size: 12.5px;
  }
  summary::-webkit-details-marker {
    display: none;
  }
  .name {
    font-weight: 600;
    color: var(--text);
  }
  .ms {
    margin-inline-start: auto;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    font-family: var(--font-mono);
  }
  .ms.live {
    color: var(--info);
  }
  .body {
    padding: 0 12px 12px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .err {
    color: var(--danger);
    font-size: 11.5px;
    background: var(--danger-soft);
    padding: 7px 9px;
    border-radius: var(--radius-s);
  }
  .logs,
  .text,
  .json {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--text-dim);
    background: var(--surface);
    border-radius: var(--radius-s);
    padding: 8px;
    margin: 0;
    overflow-x: auto;
    white-space: pre-wrap;
  }
  /* Log-line colouring by prefix: phase lines recede, ⚠/✓ stand out. There is
     no --warn token in this file, so that one keeps a literal fallback. */
  .logs .phase-line {
    color: var(--text-dim);
  }
  .logs .warn-line {
    color: var(--warning);
  }
  .logs .ok-line {
    color: var(--success);
  }
  .product-h {
    display: flex;
    align-items: center;
    font-size: var(--fs-xs);
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-dim);
    margin-bottom: 6px;
  }
  .ph-grow {
    flex: 1;
  }
  .copy-btn {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text-dim);
    font-size: var(--fs-xs);
    text-transform: none;
    letter-spacing: 0;
    padding: 2px 8px;
    border-radius: var(--radius-s);
    cursor: pointer;
  }
  .copy-btn:hover {
    color: var(--text);
    border-color: color-mix(in srgb, var(--accent) 50%, var(--border));
  }
  .scrolly {
    max-height: 340px;
    overflow: auto;
  }
  .sp-grow {
    flex: 1;
  }
  .zoom-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border: none;
    background: transparent;
    color: var(--text-dim);
    padding: 2px;
    border-radius: var(--radius-s);
    cursor: pointer;
    flex-shrink: 0;
  }
  .zoom-btn:hover {
    color: var(--text);
    background: color-mix(in srgb, var(--accent) 14%, transparent);
  }
  /* Zoomed step modal (R6): big, readable logs + work product. */
  .zoom {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .zh {
    display: flex;
    align-items: center;
    font-size: 11px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-dim);
  }
  .zbig {
    font-family: var(--font-mono);
    font-size: 12.5px;
    line-height: 1.5;
    color: var(--text);
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 12px;
    margin: 0;
    max-height: 68vh;
    overflow: auto;
    white-space: pre-wrap;
    word-break: break-word;
  }
  .muted {
    font-size: 11.5px;
    color: var(--text-dim);
  }
  /* Leading step dot — the shared run vocabulary (lib/status.ts): running is
     info-blue, never the succeeded green. */
  .dot {
    width: 9px;
    height: 9px;
    border-radius: 50%;
    flex-shrink: 0;
    background: var(--text-dim);
  }
  .dot.succeeded {
    background: var(--status-working);
  }
  .dot.failed {
    background: var(--status-exited);
  }
  .dot.running {
    background: var(--info);
  }
  .dot.waiting {
    background: var(--status-warn);
  }
</style>
