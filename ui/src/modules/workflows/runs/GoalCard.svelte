<script lang="ts">
  // A goal run's progress (ux.md §4.3, goal.md §3.5): how many criteria are
  // met, one row per criterion (its method and, while it waits on a person,
  // Verify), and the loop's budget. On the run page when nothing more urgent
  // has a card; in the Result's Criteria tab otherwise.
  import { elapsed } from '../../../lib/statusWords';
  import type { Criterion, GoalProgress, WorkflowRun } from '../../../lib/api/types';

  interface Props {
    goal: GoalProgress;
    run: WorkflowRun;
    /** The check step's criteria (from the run's graph). */
    criteria?: readonly Criterion[];
  }
  let { goal, criteria = [] }: Props = $props();

  const METHOD: Record<string, string> = { command: 'Command', agent: 'Agent', human: 'You', expr: 'Rule' };
  const all = $derived(goal.criteria_met === goal.criteria_total && goal.criteria_total > 0);
</script>

<h3 class="title">Criteria {goal.criteria_met} of {goal.criteria_total} met</h3>
{#if criteria.length}
  <ul class="crit">
    {#each criteria as c (c.id)}
      {@const verify = goal.awaiting?.includes(c.id)}
      <li data-testid="wf-goal-criterion" data-criterion-id={c.id} data-met={verify ? 'verify' : all ? 'true' : undefined} data-method={c.verify}>
        <span class="text" dir="auto">{c.text}</span>
        <span class="chip">{METHOD[c.verify] ?? c.verify}</span>
        {#if verify}<span class="w warning">Verify</span>{:else if all}<span class="w success">Met</span>{/if}
      </li>
    {/each}
  </ul>
{/if}
<p class="meta" data-testid="wf-goal-budget">
  Iteration {goal.iteration} of {goal.max_iterations} · {elapsed(goal.active_ms)}{goal.max_active_ms ? ` of ${elapsed(goal.max_active_ms)}` : ''} active
</p>

<style>
  .title {
    margin: 0;
    font-size: var(--fs-m);
    font-weight: 600;
  }
  .crit {
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .crit li {
    display: flex;
    align-items: center;
    gap: var(--sp-4);
    padding-block: var(--sp-3);
    border-block-start: 1px solid var(--border);
    font-size: var(--fs-m);
  }
  .text {
    flex: 1;
    min-inline-size: 0;
  }
  .w {
    font-size: var(--fs-s);
    font-weight: 600;
  }
  .w.warning {
    color: var(--warning);
  }
  .w.success {
    color: var(--success);
  }
  .meta {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
</style>
