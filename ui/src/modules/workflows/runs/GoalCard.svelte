<script lang="ts">
  // A goal run's progress (ux.md §4.3, goal.md §3.5): how many criteria are
  // met, one row per criterion with its word, method and evidence (the
  // latest settled check's output), and the loop's budget. On the run page
  // when nothing more urgent has a card; in the Result's Criteria tab otherwise.
  import { untrack } from 'svelte';
  import { workflowCheckpointDetail, workflowCheckpointPage } from '../../../lib/api/workflows';
  import { elapsed } from '../../../lib/statusWords';
  import type { Criterion, GoalProgress, WorkflowCheckpoint, WorkflowRun } from '../../../lib/api/types';
  import CriteriaTable from '../goal/CriteriaTable.svelte';
  import {
    activeMs,
    checksByStep,
    criterionRows,
    isCheckOutput,
    metCount,
    newestCheckpointsByStep,
    readsCheckpoints,
    type ScopedCriterion,
  } from '../goal/goalCheck';

  interface Props {
    goal: GoalProgress;
    run: WorkflowRun;
    /** The check step(s)' criteria (from the run's graph). */
    criteria?: readonly (Criterion | ScopedCriterion)[];
  }
  let { goal, run, criteria = [] }: Props = $props();

  // The run read carries the loop's checkpoints as they were when it loaded;
  // a live run's go stale, so read them when the goal moves.
  let fetched = $state.raw<WorkflowCheckpoint[] | null>(null);
  $effect(() => {
    if (!readsCheckpoints(run, untrack(() => fetched !== null))) return;
    const id = run.id;
    void [goal.iteration, goal.criteria_met, goal.awaiting?.length, goal.criteria?.length, run.status];
    let alive = true;
    (async () => {
      const all: WorkflowCheckpoint[] = [];
      let cursor: string | undefined;
      do {
        const page = await workflowCheckpointPage(id, cursor);
        all.push(...page.items);
        cursor = page.next_cursor ?? undefined;
      } while (cursor && alive);
      // Fetch full output for each check step's newest checkpoint when missing.
      for (const cp of newestCheckpointsByStep(all).values()) {
        if (!alive) break;
        if (!isCheckOutput(cp.output)) {
          const idx = all.indexOf(cp);
          if (idx >= 0) all[idx] = (await workflowCheckpointDetail(id, cp.node_id)).body;
        }
      }
      if (alive) fetched = all;
    })().catch(() => {});
    return () => {
      alive = false;
    };
  });

  const checkpoints = $derived(fetched ?? run.checkpoints);
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
  Iteration {goal.iteration} of {goal.max_iterations} · {elapsed(activeMs(goal, fetched ?? run.checkpoints))}{goal.max_active_ms ? ` of ${elapsed(goal.max_active_ms)}` : ''} active
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
