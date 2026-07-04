<script lang="ts">
  // The pipeline rail — the fixed stage machine drawn as a strip of steps.
  // Three renderings of ONE truth (runStatus.STAGES):
  //   • static  (no status): the launcher's "how a run executes" explainer
  //   • full    (status, labels): RunDetail header — live, current step pulses
  //   • mini    (status, no labels): run-list rows — 8 slim segments
  import Icon from '../../lib/components/Icon.svelte';
  import { STAGES, stageIndex, isTerminal } from './runStatus';

  interface Props {
    /** Current run status; omit for the static explainer rendering. */
    status?: string;
    /** Slim segment bar (list rows). */
    mini?: boolean;
  }
  let { status, mini = false }: Props = $props();

  const cur = $derived(status ? stageIndex(status) : -1);
  const failed = $derived(status === 'failed');
  const cancelled = $derived(status === 'cancelled');
  const rejected = $derived(status === 'rejected');
  const done = $derived(status === 'completed');
  const live = $derived(!!status && !isTerminal(status));

  /** Per-step visual state class. */
  function stepState(i: number): string {
    if (!status) return 'idle';
    if (failed || cancelled) return 'off';
    if (done) return 'done';
    if (i < cur) return 'done';
    if (i === cur) return rejected ? 'bad' : live ? 'now' : 'done';
    return 'todo';
  }
</script>

{#if mini}
  <div class="rail mini" class:dead={failed || cancelled} role="img"
    aria-label={status ? `Pipeline stage: ${STAGES[Math.max(cur, 0)]?.label ?? status}` : 'Pipeline'}>
    {#each STAGES as s, i (s.key)}
      <span class="seg {stepState(i)}" title={s.label}></span>
    {/each}
  </div>
{:else}
  <ol class="rail full" class:dead={failed || cancelled}>
    {#each STAGES as s, i (s.key)}
      <li class="step {stepState(i)}" class:gate={!status && s.key === 'approve'} title="{s.label} — {s.hint}">
        <span class="bubble"><Icon name={s.icon} size={12} /></span>
        <span class="lbl">{s.label}</span>
        {#if i < STAGES.length - 1}<span class="arrow" aria-hidden="true"></span>{/if}
      </li>
    {/each}
  </ol>
{/if}

<style>
  /* ── full rail ─────────────────────────────────────────────────────────── */
  .rail.full {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    row-gap: 0.4rem;
  }
  .step {
    display: inline-flex;
    align-items: center;
    gap: 0.32rem;
    color: var(--text-dim);
    font-size: 0.72rem;
  }
  .bubble {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 22px;
    height: 22px;
    border-radius: 999px;
    border: 1px solid var(--border);
    background: var(--bg);
    color: var(--text-dim);
    flex: none;
  }
  .lbl { white-space: nowrap; }
  .arrow {
    width: 14px;
    height: 1px;
    margin: 0 0.45rem;
    background: linear-gradient(90deg, var(--border), color-mix(in srgb, var(--accent) 45%, var(--border)));
    position: relative;
    flex: none;
  }
  .arrow::after {
    content: '';
    position: absolute;
    right: -1px;
    top: -2.5px;
    border-left: 4px solid color-mix(in srgb, var(--accent) 45%, var(--border));
    border-top: 3px solid transparent;
    border-bottom: 3px solid transparent;
  }

  /* per-step states */
  .step.done .bubble {
    border-color: color-mix(in srgb, var(--status-working) 55%, var(--border));
    background: color-mix(in srgb, var(--status-working) 14%, var(--bg));
    color: var(--status-working);
  }
  .step.done { color: var(--text); }
  .step.now .bubble {
    border-color: var(--accent);
    background: color-mix(in srgb, var(--accent) 18%, var(--bg));
    color: var(--accent);
    animation: rail-pulse 1.6s ease-in-out infinite;
  }
  .step.now { color: var(--accent); font-weight: 600; }
  .step.bad .bubble {
    border-color: color-mix(in srgb, var(--status-exited) 60%, var(--border));
    background: color-mix(in srgb, var(--status-exited) 15%, var(--bg));
    color: var(--status-exited);
  }
  .step.bad { color: var(--status-exited); }
  /* the approval step reads amber in the static explainer — the one human gate */
  .step.gate .bubble {
    border-color: color-mix(in srgb, var(--status-warn) 55%, var(--border));
    background: color-mix(in srgb, var(--status-warn) 12%, var(--bg));
    color: var(--status-warn);
  }
  .rail.dead .step .bubble { opacity: 0.45; }
  .rail.dead .step { opacity: 0.7; }

  @keyframes rail-pulse {
    0%, 100% { box-shadow: 0 0 0 0 color-mix(in srgb, var(--accent) 35%, transparent); }
    50% { box-shadow: 0 0 0 4px color-mix(in srgb, var(--accent) 12%, transparent); }
  }
  @media (prefers-reduced-motion: reduce) {
    .step.now .bubble { animation: none; }
  }

  /* ── mini rail (list rows) ─────────────────────────────────────────────── */
  .rail.mini {
    display: inline-flex;
    align-items: center;
    gap: 3px;
  }
  .seg {
    width: 14px;
    height: 4px;
    border-radius: 2px;
    background: color-mix(in srgb, var(--text-dim) 22%, transparent);
  }
  .seg.done { background: var(--status-working); }
  .seg.now { background: var(--accent); animation: rail-blink 1.4s ease-in-out infinite; }
  .seg.bad { background: var(--status-exited); }
  .rail.mini.dead .seg { background: color-mix(in srgb, var(--status-exited) 30%, transparent); }
  @keyframes rail-blink {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.45; }
  }
  @media (prefers-reduced-motion: reduce) {
    .seg.now { animation: none; }
  }
</style>
