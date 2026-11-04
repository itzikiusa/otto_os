<script lang="ts">
  // The sidebar's "Running" list: every in-flight run of the workspace with
  // its status, the step it is on and its step progress. Moved out of
  // WorkflowsPage; same markup and test ids.
  import Icon from '../../lib/components/Icon.svelte';
  import RelTime from '../../lib/components/RelTime.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { runStatus } from '../../lib/status';

  interface Props {
    /** The run open in the page (highlighted). */
    activeRunId: string | null;
    onopen: (workflowId: string, runId: string) => void;
  }
  let { activeRunId, onopen }: Props = $props();
  const run = $derived(activeRunId ? { id: activeRunId } : null);
  const openRunById = (workflowId: string, runId: string) => onopen(workflowId, runId);

  function shortRunId(id: string): string {
    return id.length > 6 ? id.slice(-6) : id;
  }
  // A `pending` run is PARKED in the daemon's FIFO run queue — the shared run
  // vocabulary (lib/status.ts) says "Queued".
  function runStatusLabel(status: string): string {
    return runStatus(status).label;
  }
  function dotKey(status: string): string {
    return runStatus(status).key;
  }
  const activeRunOrdinals = $derived.by(() => {
    const counts: Record<string, number> = {};
    const map: Record<string, number> = {};
    const sorted = [...ws.activeWorkflowRuns].sort((a, b) => a.started_at.localeCompare(b.started_at));
    for (const r of sorted) {
      counts[r.workflow_id] = (counts[r.workflow_id] ?? 0) + 1;
      map[r.run_id] = counts[r.workflow_id];
    }
    return map;
  });
  const activeWfRunCounts = $derived.by(() => {
    const c: Record<string, number> = {};
    for (const r of ws.activeWorkflowRuns) c[r.workflow_id] = (c[r.workflow_id] ?? 0) + 1;
    return c;
  });

</script>

{#if ws.activeWorkflowRuns.length > 0}
  <div class="running" data-testid="running-workflows">
    <div class="list-h">
      Running
      <span class="run-count">{ws.activeWorkflowRuns.length}</span>
    </div>
    {#each ws.activeWorkflowRuns as r (r.run_id)}
      <button
        class="run-row"
        class:active={run?.id === r.run_id}
        onclick={() => openRunById(r.workflow_id, r.run_id)}
        title={`${r.workflow_name} — ${runStatusLabel(r.status)}${r.current_step ? ` · ${r.current_step}` : ''}`}
      >
        <span class="dot {dotKey(r.status)}" aria-hidden="true"></span>
        <span class="sr-only">{runStatusLabel(r.status)}:</span>
        <span class="run-name">{r.workflow_name}</span>
        {#if activeWfRunCounts[r.workflow_id] > 1}
          <span class="run-ord" title={`run #${activeRunOrdinals[r.run_id]} of this workflow`}>#{activeRunOrdinals[r.run_id]}</span>
        {/if}
        {#if r.waiting_approval}
          <span class="run-badge" role="img" title="Waiting for your approval" aria-label="Waiting for your approval"><Icon name="userCheck" size={12} /></span>
        {/if}
        {#if !r.current_step}<code class="run-id" title={r.run_id}>{shortRunId(r.run_id)}</code>{/if}
        {#if r.current_step}
          <span class="run-step" data-testid="running-step" title={`On step: ${r.current_step}`}>{r.current_step}</span>
        {/if}
        <span class="run-prog" title={`${r.nodes_done} of ${r.nodes_total} steps done`}>{r.nodes_done}/{r.nodes_total}</span>
        <!-- Ticks with the shared clock (a one-shot "5m ago" froze at load). -->
        <span class="run-when"><RelTime iso={r.started_at} fallback="" /></span>
      </button>
    {/each}
  </div>
{/if}

<style>
  .running {
    flex-shrink: 0;
    max-height: 38%;
    overflow-y: auto;
    padding: 8px;
    border-bottom: 1px solid var(--border);
  }
  .running .list-h {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .run-count {
    display: inline-grid;
    place-items: center;
    min-width: 16px;
    height: 16px;
    padding: 0 4px;
    border-radius: var(--radius-m);
    font-size: var(--fs-xs);
    background: var(--accent-soft-strong);
    color: var(--text);
  }
  .run-row {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    min-width: 0;
    overflow: hidden;
    padding: 6px 8px;
    background: none;
    border: none;
    border-radius: var(--radius-s);
    cursor: pointer;
    text-align: start;
    color: var(--text);
  }
  .run-row:hover {
    background: var(--hover);
  }
  .run-row.active {
    background: var(--accent-soft);
  }
  .run-name {
    font-size: var(--fs-m);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    flex: 1;
    min-width: 0;
  }
  .run-badge {
    display: inline-flex;
    color: var(--warning);
    flex-shrink: 0;
  }
  /* Disambiguators for concurrent runs of the same workflow (item 8). */
  .run-ord {
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--accent-text);
    background: var(--accent-soft);
    padding: 0 4px;
    border-radius: 999px;
    flex-shrink: 0;
  }
  .run-id {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    color: var(--text-dim);
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .run-step {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    max-width: 40%;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .run-prog {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
    flex-shrink: 0;
    white-space: nowrap;
  }
  .run-when {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    margin-inline-start: 6px;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  /* 18-03: a 220px list cannot hold a name, an id, and a relative time. */
  @container wf-side (max-width: 280px) {
    .running .run-id,
    .running .run-when {
      display: none;
    }
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    display: inline-block;
    flex-shrink: 0;
  }
  .dot.succeeded {
    background: var(--status-working);
  }
  .dot.failed {
    background: var(--status-exited);
  }
  .dot.running {
    background: var(--info);
    animation: otto-pulse 1.6s ease-in-out infinite;
  }
  .dot.waiting {
    background: var(--status-warn);
  }
  .dot.queued,
  .dot.cancelled,
  .dot.skipped {
    background: var(--text-dim);
  }
  .list-h {
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: .06em;
    color: var(--text-dim);
    padding: 4px 6px;
  }
  @media (prefers-reduced-motion: reduce) {
    .dot.running {
      animation: none;
    }
  }
</style>
