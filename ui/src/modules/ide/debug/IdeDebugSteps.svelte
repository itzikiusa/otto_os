<script lang="ts">
  // The Debug All step line under the debugger toolbar: the step that runs now,
  // a result chip per other step (click: its place in the console), and at the
  // end the run's summary with up to 5 failed tests, each with Debug.
  import type { IdeDebugSessionSnapshot } from '../ideStore.svelte';
  import {
    currentStepLine,
    failedTests,
    runSummary,
    stepChips,
  } from './ideDebugSteps';

  interface Props {
    dbg: IdeDebugSessionSnapshot;
    onjump: (index: number) => void;
    ondebugtest: (stepIndex: number, name: string) => void;
  }

  let { dbg, onjump, ondebugtest }: Props = $props();

  const steps = $derived(dbg.steps ?? []);
  const live = $derived(dbg.state !== 'terminated' && dbg.state !== 'error');
  const line = $derived(currentStepLine(steps, dbg.current_step));
  const chips = $derived(stepChips(steps, dbg.current_step));
  const failed = $derived(live ? [] : failedTests(steps));
</script>

<section class="steps" aria-label="Debug all steps" data-testid="ide-debug-steps">
  <div class="steps-row">
    <!-- One always-mounted region: the live step, then the run's result. -->
    <span class="steps-now" role="status" data-testid="ide-debug-step-now">
      {#if line}<span class="spinner" aria-hidden="true"></span>{line}{:else if !live}{runSummary(steps)}{/if}
    </span>
    {#each chips as chip (chip.index)}
      <button
        type="button"
        class="steps-chip is-{chip.state}"
        title={chip.title}
        aria-label={chip.title}
        disabled={chip.state === 'pending'}
        onclick={() => onjump(chip.index)}
        data-testid="ide-debug-step-chip"
      ><span class="steps-mark" aria-hidden="true">{chip.mark}</span>{chip.text}</button>
    {/each}
  </div>
  {#if failed.length > 0}
    <ul class="steps-failed" aria-label="Failed tests">
      {#each failed as t (t.key)}
        <li>
          <span class="steps-failed-name mono" title={`${t.name} · ${t.step}`}>{t.name}</span>
          <button
            type="button"
            class="btn small ghost"
            aria-label={`Debug ${t.name}`}
            title={`Debug ${t.name} alone`}
            onclick={() => ondebugtest(t.stepIndex, t.name)}
            data-testid="ide-debug-step-failed-debug"
          >Debug</button>
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  .steps { display: flex; flex-direction: column; gap: 2px; padding-block: 4px; padding-inline: 8px; border-block-end: 1px solid var(--border); background: var(--surface); flex-shrink: 0; font-size: var(--fs-xs); }
  .steps-row { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; min-inline-size: 0; }
  .steps-now { display: inline-flex; align-items: center; gap: 6px; color: var(--text); font-weight: 600; min-inline-size: 0; overflow-wrap: anywhere; --spinner-size: 10px; }
  .steps-now:empty { display: none; }
  .steps-chip { display: inline-flex; align-items: center; gap: 4px; min-block-size: 22px; padding-inline: 6px; border: 1px solid var(--border); border-radius: var(--radius-s); background: var(--surface-2); color: var(--text-dim); font-size: var(--fs-xs); cursor: pointer; max-inline-size: 100%; overflow-wrap: anywhere; text-align: start; }
  .steps-chip:hover:not(:disabled) { background: var(--hover); color: var(--text); }
  .steps-chip:disabled { cursor: default; }
  .steps-mark { font-weight: 600; }
  .steps-chip.is-passed .steps-mark { color: var(--success); }
  .steps-chip.is-failed .steps-mark { color: var(--danger); }
  .steps-chip.is-error .steps-mark { color: var(--warning); }
  .steps-failed { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 2px; }
  .steps-failed li { display: flex; align-items: center; gap: 6px; min-inline-size: 0; }
  .steps-failed-name { flex: 1; min-inline-size: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--danger); }
  :global(.dbg-root.is-compact) .steps-chip { min-block-size: 36px; }
</style>
