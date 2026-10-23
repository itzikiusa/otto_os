<script lang="ts">
  // The inspector's run view (architecture-1-08): the run bar (live status,
  // Stop, maximize), the run-level error, the step timeline, the step list,
  // and on phone/tablet the final output and the context files (on desktop
  // those live in RunContextSidebar). Live state comes from the page's
  // RunMonitor (runSync.svelte.ts).
  import { announceShown } from '../../lib/announce.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import StatusBadge from '../../lib/components/StatusBadge.svelte';
  import { runStatus } from '../../lib/status';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import FileTree from '../panels/FileTree.svelte';
  import RunFinalOutput from './RunFinalOutput.svelte';
  import RunSteps from './RunSteps.svelte';
  import { fmtStepMs } from './runProgress';
  import { contextFs, isLive, type RunMonitor } from './runSync.svelte';

  interface Props {
    mon: RunMonitor;
    nodeName: (id: string) => string;
    /** The step selected in the timeline / on the canvas. */
    selectedId: string | null;
    /** "Zoom" the run detail to ~85vh (R6). */
    runDetailMax: boolean;
    onstop: () => void;
    onOpenSession: (sid: string) => void;
  }
  let { mon, nodeName, selectedId = $bindable(), runDetailMax = $bindable(), onstop, onOpenSession }: Props = $props();

  const run = $derived(mon.run);
  // Whether the run is still active (so Stop makes sense even when this
  // editor didn't start it — e.g. opened from the Running list).
  const runActive = $derived(!!run && isLive(run.status));
  const fmtMs = fmtStepMs;
  function runStatusLabel(status: string): string {
    return runStatus(status).label;
  }
  /** Normalised run/step key for the status dot (`succeeded`, `failed`, …). */
  function dotKey(status: string): string {
    return runStatus(status).key;
  }
</script>

{#if run}
  <!-- Run bar: live status + Stop (R7) + maximize/zoom (R6). -->
  <div class="insp-bar">
    <span class="tl-label"><StatusBadge status={runStatus(run.status)} testid="run-status" /></span>
    <span class="grow"></span>
    {#if runActive}
      <button
        class="btn small danger"
        data-testid="run-cancel"
        onclick={onstop}
        title="Stop this run (finishes the current step, then halts)"
      >
        <Icon name="stop" size={12} /> Stop run…
      </button>
    {/if}
    <button
      class="icon-btn"
      data-testid="run-detail-max"
      onclick={() => (runDetailMax = !runDetailMax)}
      aria-pressed={runDetailMax}
      aria-label="Maximize run detail"
      title={runDetailMax ? 'Restore run-detail height' : 'Maximize run detail'}
    >
      <Icon name={runDetailMax ? 'minimize' : 'maximize'} size={13} />
    </button>
  </div>
  <!-- The run-level reason (an invalid graph, an exhausted restart
       resume, a queued run that couldn't start) often has no step
       error to show — without this a failed run said only "Failed". -->
  {#if run.status === 'error' && run.error}
    <p class="run-error" data-testid="run-error" use:announceShown={{ text: `Run failed: ${run.error}`, assertive: true }}><Icon name="warning" size={12} /> {run.error}</p>
  {/if}
  <div class="timeline">
    {#each run.nodes as ns (ns.node_id)}
      <button
        class="tl-step"
        class:active={selectedId === ns.node_id}
        data-status={ns.status}
        onclick={() => (selectedId = ns.node_id)}
      >
        <span class="dot {dotKey(ns.status)}" role="img" aria-label={runStatusLabel(ns.status)}></span>
        <span class="tl-name">{nodeName(ns.node_id)}</span>
        {#if ns.duration_ms != null}<span class="tl-ms">{fmtMs(ns.duration_ms)}</span>{/if}
      </button>
    {/each}
  </div>
  <div class="run-detail"><RunSteps {run} nodeName={(id) => nodeName(id)} onOpenSession={onOpenSession} onRunUpdated={mon.applyRunSnapshot} onRefresh={() => void mon.refetchRun(run.id)} /></div>
  {#if run.context_dir && !viewport.isDesktop}
    <!-- Mobile/tablet: the run's deliverable, above the context-file tree
         below. On desktop this moves into the Context-files sidebar. -->
    <RunFinalOutput {mon} {run} />
    <!-- Mobile/tablet: the run's context files inline (instruction brief,
         repos.json, per-step handoffs). On desktop these move to the
         full-height Context-files sidebar (RunContextSidebar) so they get real
         room — same browsable tree + viewer the agent Files panel uses. -->
    <details class="ctx-files">
      <summary>
        <Icon name="folder" size={13} />
        <span>Context files</span>
        <code class="dim">{run.context_dir}</code>
      </summary>
      <div class="ctx-files-tree"><FileTree root={run.context_dir} {...contextFs(run.id, run.context_dir)} /></div>
    </details>
  {/if}
{/if}

<style>
  /* "Running" sidebar list — in-flight runs across the workspace, live. */
  .run-error {
    margin: 0 0 6px; padding: 6px 10px; border-radius: var(--radius-s);
    background: var(--danger-soft); color: var(--danger); font-size: var(--fs-s);
    display: flex; gap: 6px; align-items: baseline; overflow-wrap: anywhere;
  }
  .grow {
    flex: 1;
  }
  /* Run bar: sticky so status + Cancel + maximize stay reachable while the run
     detail scrolls. (R6/R7) */
  .insp-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    position: sticky;
    top: -10px;
    margin: -10px -12px 4px;
    padding: 8px 12px;
    background: var(--surface);
    border-bottom: 1px solid var(--border);
    z-index: 2;
  }
  .insp-bar .grow {
    flex: 1;
  }

  /* Run timeline */
  .timeline {
    display: flex;
    align-items: center;
    gap: 6px;
    overflow-x: auto;
    padding-bottom: 8px;
    margin-bottom: 8px;
    border-bottom: 1px solid var(--border);
    /* overflow-x zeroes a flex item's automatic min-height, so the flex-column
       inspector would vertically compress (clip) this row. Pin it. (R8) */
    flex-shrink: 0;
  }
  .run-detail {
    margin: 8px 0;
  }
  .ctx-files {
    margin: 8px 0;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
  }
  .ctx-files > summary {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
    font-size: var(--fs-s);
    font-weight: 600;
    cursor: pointer;
    user-select: none;
    min-width: 0;
  }
  .ctx-files > summary code {
    font-size: var(--fs-xs);
    font-weight: 400;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    direction: ltr;
  }
  .ctx-files-tree {
    border-top: 1px solid var(--border);
    max-height: 420px;
    overflow: auto;
  }
  .tl-label {
    display: inline-flex;
    align-items: center;
    flex-shrink: 0;
    padding-inline-end: 4px;
  }
  .tl-step {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    flex-shrink: 0;
    padding: 4px 10px;
    border: 1px solid var(--border);
    border-radius: 999px;
    background: var(--surface-2);
    color: var(--text);
    font-size: var(--fs-s);
    cursor: pointer;
  }
  .tl-step.active {
    border-color: var(--accent);
  }
  .tl-step[data-status='success'] {
    border-color: color-mix(in srgb, var(--success) 55%, var(--border));
  }
  .tl-step[data-status='error'] {
    border-color: color-mix(in srgb, var(--danger) 55%, var(--border));
  }
  /* Running is info-blue (lib/status.ts) — the accent border is `.active`
     (selection), so a running step no longer looks selected. */
  .tl-step[data-status='running'] {
    border-color: color-mix(in srgb, var(--info) 60%, var(--border));
  }
  .tl-name {
    white-space: nowrap;
  }
  .tl-ms {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    font-family: var(--font-mono);
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    display: inline-block;
  }
  /* Run/step dots use the shared run vocabulary (lib/status.ts runStatus):
     running is info-blue and pulses — never the succeeded green. */
  .dot.succeeded {
    background: var(--status-working);
  }
  .dot.failed {
    background: var(--status-exited);
  }
  .dot.waiting {
    background: var(--status-warn);
  }
  .dot.queued,
  .dot.cancelled,
  .dot.skipped {
    background: var(--text-dim);
  }
</style>
