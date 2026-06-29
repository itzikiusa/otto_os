<script lang="ts">
  // Reusable run detail: every step of a WorkflowRun with its status, duration,
  // logs, error, and rendered "work product" (agent reply / JSON).
  import Icon from '../../lib/components/Icon.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { proof } from '../../lib/stores/proof.svelte';
  import { router } from '../../lib/router.svelte';
  import type { WorkflowRun, NodeRunState } from '../../lib/api/types';

  interface Props {
    run: WorkflowRun;
    /** Resolve a node id to a friendly label. */
    nodeName?: (id: string) => string;
  }
  let { run, nodeName = (id) => id }: Props = $props();

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

  /** Navigate to an agent session this step drove (reuses the real router nav). */
  function openSession(id: string): void {
    ws.navigateToSession(id);
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
      await navigator.clipboard.writeText(text);
      toasts.success(`Copied ${label}`);
    } catch {
      toasts.error('Copy failed');
    }
  }
  function asText(out: unknown): string {
    return typeof out === 'string' ? out : JSON.stringify(out, null, 2);
  }
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
    <details class="step" open={ns.status === 'error'} data-status={ns.status}>
      <summary>
        <span class="dot {ns.status}"></span>
        <span class="name">{nodeName(ns.node_id)}</span>
        <span class="status">{ns.status}</span>
        {#if (ns.attempts ?? 1) > 1}<span class="chip" title="step was retried">×{ns.attempts} attempts</span>{/if}
        {#if ns.duration_ms != null}<span class="ms">{fmtMs(ns.duration_ms)}</span>{/if}
      </summary>
      <div class="body">
        {#if ns.error}
          <div class="err">{ns.error}</div>
        {/if}

        {#if ns.sessions?.length || reviewIdOf(ns.output)}
          <div class="links">
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
