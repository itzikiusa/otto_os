<script lang="ts">
  // A goal run's progress (ux.md §4.3, goal.md §3.5): how many criteria are
  // met, one row per criterion with its word, method and evidence (the
  // latest settled check's output), and the loop's budget. On the run page
  // when nothing more urgent has a card; in the Result's Criteria tab otherwise.
  // Reads revision-/generation-aware checkpoints shared through RunSync (ARCH-1).
  import { elapsed } from '../../../lib/statusWords';
  import type { Criterion, GoalProgress, WorkflowCheckpoint, WorkflowRun } from '../../../lib/api/types';
  import CriteriaTable from '../goal/CriteriaTable.svelte';
  import {
    activeMs,
    checksByStep,
    criterionRows,
    metCount,
    type ScopedCriterion,
  } from '../goal/goalCheck';

  interface Props {
    goal: GoalProgress;
    run: WorkflowRun;
    /** The check step(s)' criteria (from the run's graph). */
    criteria?: readonly (Criterion | ScopedCriterion)[];
    /** Shared live checkpoints from RunSync (falls back to `run.checkpoints`). */
    checkpoints?: readonly WorkflowCheckpoint[];
  }
  let { goal, run, criteria = [], checkpoints: sharedCheckpoints }: Props = $props();

  const checkpoints = $derived(sharedCheckpoints ?? run.checkpoints);
  const checks = $derived(checksByStep(run.nodes, checkpoints, criteria));
  const rows = $derived(criterionRows(criteria, checks, goal.awaiting ?? [], goal, checkpoints));
  const count = $derived(metCount(rows, goal));
  const settled = $derived(run.status !== 'pending' && run.status !== 'running');
  const kept = $derived.by(() => {
    const input = (run.input ?? {}) as { research_dir?: unknown; repos?: unknown[] };
    if (input.research_dir) return 'Findings are kept in the run’s research folder (Files).';
    return input.repos?.length ? `Work kept on branch otto-wf/${run.id}; nothing was pushed.` : null;
  });
</script>

<h3 class="title">Criteria {count.met} of {count.total} met</h3>
{#if rows.length}<CriteriaTable {rows} />{/if}
<p class="meta" data-testid="wf-goal-budget">
  Iteration {goal.iteration} of {goal.max_iterations} · {elapsed(activeMs(goal, checkpoints))}{goal.max_active_ms ? ` of ${elapsed(goal.max_active_ms)}` : ''} active
</p>
{#if settled && kept}<p class="meta" data-testid="wf-goal-kept">{kept}</p>{/if}

<style>
  .title {
    margin: 0;
    font-size: var(--fs-m);
    font-weight: 600;
  }
  .meta {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
</style>
