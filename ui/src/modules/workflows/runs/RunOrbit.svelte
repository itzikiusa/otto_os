<script lang="ts">
  // The run ring (ux.md §6): 104 px, on the run page only. Arcs come from
  // runOrbit.ts (the step list's own entries), colour is runState only, the
  // centre is time (elapsed "of ~typical" while live, the total when done).
  // Not mirrored in RTL (a clock keeps its direction). Hovering an arc names
  // its step; the step list is the way to open one.
  import { buildOrbit, orbitLabel, VIEW } from './runOrbit';
  import { entrySteps, type StepEntry } from './stepOrder';
  import type { RunState } from '../../../lib/api/types';

  interface Props {
    entries: readonly StepEntry[];
    runState: RunState;
    /** The run's word ("Running", "Failed") for the label. */
    word: string;
    centre: string;
    sub?: string;
  }
  let { entries, runState, word, centre, sub = '' }: Props = $props();
  const orbit = $derived(buildOrbit(entries, runState));
  const label = $derived(orbitLabel(entries, word));
  const nameOf = (slot: number) => entrySteps(entries[slot]).map((s) => s.name).join(', ');
</script>

<div class="orbit" data-testid="wf-run-orbit" role="img" aria-label={label} data-slots={orbit.slots} data-children={orbit.children} data-grouped={orbit.grouped}>
  <svg viewBox="0 0 {VIEW} {VIEW}" aria-hidden="true">
    <circle class="track" cx={VIEW / 2} cy={VIEW / 2} r="54" />
    <line class="noon" x1={VIEW / 2} y1="2" x2={VIEW / 2} y2="8" />
    {#each orbit.arcs as arc, i (i)}
      <path class="arc {arc.state}" d={arc.d} stroke-width={arc.width}><title>{nameOf(arc.slot)}</title></path>
    {/each}
  </svg>
  <span class="centre"><span class="t">{centre}</span>{#if sub}<span class="of">{sub}</span>{/if}</span>
</div>

<style>
  .orbit {
    position: relative;
    flex: none;
    inline-size: 104px;
    block-size: 104px;
    direction: ltr;
  }
  svg {
    display: block;
    inline-size: 100%;
    block-size: 100%;
  }
  .track {
    fill: none;
    stroke: var(--surface-2);
    stroke-width: 10;
  }
  .noon {
    stroke: var(--text-dim);
    stroke-width: 1.5;
  }
  .arc {
    fill: none;
    stroke: var(--border-strong);
  }
  .arc.done {
    stroke: var(--success);
  }
  .arc.working {
    stroke: var(--accent);
    animation: otto-pulse 1.8s ease-in-out infinite;
  }
  .arc.waiting {
    stroke: var(--accent);
    stroke-opacity: 0.45;
    stroke-dasharray: 3 2;
  }
  .arc.needs {
    stroke: var(--warning);
  }
  .arc.failed {
    stroke: var(--danger);
  }
  .arc.skipped {
    stroke: var(--border);
    stroke-opacity: 0.7;
  }
  .arc.canceled {
    stroke: var(--text-dim);
    stroke-opacity: 0.5;
  }
  @media (prefers-reduced-motion: reduce) {
    .arc.working {
      animation: none;
    }
  }
  .centre {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    pointer-events: none;
  }
  .t {
    font-size: var(--fs-xl);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }
  .of {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
</style>
