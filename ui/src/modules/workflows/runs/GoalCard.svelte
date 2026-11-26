<script lang="ts">
  // A goal run's progress (ux.md §4.3, goal.md §3.5): how many criteria are
  // met, one row per criterion with its word, method and evidence (the
  // latest settled check's output), and the loop's budget. On the run page
  // when nothing more urgent has a card; in the Result's Criteria tab otherwise.
  import { workflowCheckpointDetail, workflowCheckpointPage } from '../../../lib/api/workflows';
  import { elapsed } from '../../../lib/statusWords';
  import type { Criterion, GoalProgress, WorkflowCheckpoint, WorkflowRun } from '../../../lib/api/types';
  import CriteriaTable from '../goal/CriteriaTable.svelte';
  import { criterionRows, isCheckOutput, latestCheck, newestCheckpoint } from '../goal/goalCheck';

  interface Props {
    goal: GoalProgress;
    run: WorkflowRun;
    /** The check step's criteria (from the run's graph). */
    criteria?: readonly Criterion[];
  }
  let { goal, run, criteria = [] }: Props = $props();

  // The run read carries the loop's checkpoints; a live run (progress
  // projection) does not, so read them when the goal moves.
  let fetched = $state.raw<WorkflowCheckpoint[] | null>(null);
  $effect(() => {
    if (run.checkpoints?.length) return;
    const id = run.id;
    void [goal.iteration, goal.criteria_met, goal.awaiting?.length, run.status];
    let alive = true;
    (async () => {
      const all: WorkflowCheckpoint[] = [];
      let cursor: string | undefined;
      do {
        const page = await workflowCheckpointPage(id, cursor);
        all.push(...page.items);
        cursor = page.next_cursor ?? undefined;
      } while (cursor && alive);
      const cp = newestCheckpoint(all);
      if (cp && !isCheckOutput(cp.output)) all[all.indexOf(cp)] = (await workflowCheckpointDetail(id, cp.node_id)).body;
      if (alive) fetched = all;
    })().catch(() => {});
    return () => {
      alive = false;
    };
  });

  const check = $derived(latestCheck(run.nodes, run.checkpoints?.length ? run.checkpoints : (fetched ?? undefined)));
  const rows = $derived(criterionRows(criteria, check, goal.awaiting ?? []));
  const settled = $derived(run.status !== 'pending' && run.status !== 'running');
  const kept = $derived.by(() => {
    const input = (run.input ?? {}) as { research_dir?: unknown; repos?: unknown[] };
    if (input.research_dir) return 'Findings are kept in the run’s research folder (Files).';
    return input.repos?.length ? `Work kept on branch otto-wf/${run.id}; nothing was pushed.` : null;
  });
</script>

<h3 class="title">Criteria {goal.criteria_met} of {goal.criteria_total} met</h3>
{#if rows.length}<CriteriaTable {rows} />{/if}
<p class="meta" data-testid="wf-goal-budget">
  Iteration {goal.iteration} of {goal.max_iterations} · {elapsed(goal.active_ms)}{goal.max_active_ms ? ` of ${elapsed(goal.max_active_ms)}` : ''} active
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
