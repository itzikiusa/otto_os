<script lang="ts">
  // Reusable run detail: every step of a WorkflowRun with its status, duration,
  // logs, error, and rendered "work product" (agent reply / JSON).
  import Icon from '../../lib/components/Icon.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { proof } from '../../lib/stores/proof.svelte';
  import { router } from '../../lib/router.svelte';
  import { retryRunNode } from '../../lib/api/workflows';
  import type { WorkflowRun, NodeRunState } from '../../lib/api/types';
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
  }
  let { run, nodeName = (id) => id, onOpenSession, onRunUpdated }: Props = $props();

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

  function fmtMs(ms?: number | null): string {
    if (ms == null) return '';
    return ms >= 1000 ? `${(ms / 1000).toFixed(1)}s` : `${ms}ms`;
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
  function openReview(out: unknown): void {
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

<div class="steps">
  {#each run.nodes as ns (ns.node_id)}
    <details
      class="step"
      open={isOpen(ns)}
      ontoggle={(e) => onToggle(ns, e.currentTarget.open)}
      data-status={ns.status}
    >
      <summary>
        <span class="dot {ns.status}"></span>
        <span class="name">{nodeName(ns.node_id)}</span>
        <span class="status">{ns.status}</span>
        {#if (ns.attempts ?? 1) > 1}<span class="chip" title="step was retried">×{ns.attempts} attempts</span>{/if}
        <span class="sp-grow"></span>
        {#if ns.duration_ms != null}
          <span class="ms">{fmtMs(ns.duration_ms)}</span>
        {:else if ns.status === 'running' && elapsedMs(ns) != null}
          <span class="ms live">{fmtMs(elapsedMs(ns))}</span>
        {/if}
        <button
          class="zoom-btn"
          title="Zoom in on this step"
          aria-label="Zoom in on this step"
          onclick={(e) => {
            e.preventDefault();
            e.stopPropagation();
            zoomed = ns;
          }}
        >
          <Icon name="maximize" size={12} />
        </button>
      </summary>
      <div class="body">
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
          <div class="logs">{#each ns.logs as l}<div>{l}</div>{/each}</div>
        {/if}

        {#if hasOutput(ns)}
          {@const txt = reply(ns.output)}
          <div class="product">
            <div class="product-h">
              <span>Work product</span>
              <span class="ph-grow"></span>
              <button class="copy-btn" title="Copy to clipboard" onclick={() => copy(asText(ns.output), 'output')}>
                <Icon name="file" size={11} /> Copy
              </button>
            </div>
            {#if txt}
              <pre class="text scrolly">{txt}</pre>
            {:else}
              <pre class="json scrolly">{JSON.stringify(ns.output, null, 2)}</pre>
            {/if}
          </div>
        {:else if ns.status === 'success'}
          <div class="muted">No output.</div>
        {/if}
      </div>
    </details>
  {/each}
</div>

{#if zoomed}
  {@const z = zoomed}
  <Modal title={`Step · ${nodeName(z.node_id)}`} width={920} onclose={() => (zoomed = null)}>
    <div class="zoom">
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
            <Icon name="file" size={11} /> Copy
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
    font-size: 10.5px;
    font-family: var(--font-mono);
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    padding: 1px 7px;
    border-radius: 99px;
  }
  .chip {
    font-size: 10px;
    color: var(--status-warn, #b07a00);
    background: color-mix(in srgb, var(--status-warn, #b07a00) 16%, transparent);
    padding: 1px 7px;
    border-radius: 99px;
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
    font-size: 10px;
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
    border-color: color-mix(in srgb, var(--status-exited) 45%, var(--border));
  }
  .step[data-status='success'] {
    border-color: color-mix(in srgb, var(--status-working, #28c840) 35%, var(--border));
  }
  summary {
    display: flex;
    align-items: center;
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
  .status {
    text-transform: capitalize;
    color: var(--text-dim);
    font-size: 11.5px;
  }
  .ms {
    margin-inline-start: auto;
    font-size: 10.5px;
    color: var(--text-dim);
    font-family: var(--font-mono);
  }
  .ms.live {
    color: var(--status-working, #28c840);
  }
  .body {
    padding: 0 12px 12px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .err {
    color: var(--status-exited);
    font-size: 11.5px;
    background: color-mix(in srgb, var(--status-exited) 10%, transparent);
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
  .product-h {
    display: flex;
    align-items: center;
    font-size: 10.5px;
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
    font-size: 10px;
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
  .dot {
    width: 9px;
    height: 9px;
    border-radius: 50%;
    flex-shrink: 0;
  }
  .dot.success {
    background: var(--status-working, #28c840);
  }
  .dot.error {
    background: var(--status-exited);
  }
  .dot.running {
    background: var(--status-working, #28c840);
  }
  .dot.pending,
  .dot.skipped {
    background: var(--text-dim);
  }
</style>
