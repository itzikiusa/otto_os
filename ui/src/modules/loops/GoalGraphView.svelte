<script lang="ts">
  import type { AcceptanceCriterion, GoalGraphNodeState, GoalLoopGraph } from '../../lib/api/types';
  let { graph, states, criteria }: { graph: GoalLoopGraph; states: GoalGraphNodeState[]; criteria: AcceptanceCriterion[] } = $props();
  const stateById = $derived(new Map(states.map(state => [state.node_id, state])));
  const titleById = $derived(new Map(graph.nodes.map(node => [node.id, node.title])));
  const criteriaById = $derived(new Map(criteria.map(criterion => [criterion.id, criterion.text])));
  const labels = { pending: 'Pending', running: 'Running', passed: 'Passed', blocked: 'Blocked' };
</script>

<section class="graph-view" aria-label="Execution graph" data-testid="goal-graph-state">
  <h3>Execution graph</h3>
  <p class="hint">Dependencies gate execution. Earlier results are rechecked when work changes.</p>
  <ol>
    {#each graph.nodes as node (node.id)}
      {@const state = stateById.get(node.id)}
      <li data-testid="goal-graph-node" data-node-id={node.id}>
        <details>
          <summary><span class="title">{node.title}</span><span class="state" class:passed={state?.status === 'passed'} class:blocked={state?.status === 'blocked'} class:running={state?.status === 'running'}>{state ? labels[state.status] : 'Not started'}</span></summary>
          {#if node.depends_on.length}<p class="hint">After: {node.depends_on.map(id => titleById.get(id) ?? id).join(', ')}</p>{/if}
          <p>{node.instructions}</p>
          <ul>{#each node.criterion_ids as id (id)}<li>{criteriaById.get(id) ?? id}</li>{/each}</ul>
          {#if state}
            {#if state.iteration > 0}<p class="hint">Last evaluated in iteration {state.iteration}</p>{/if}
            {#if state.evidence}<p class="evidence">{state.evidence}</p>{/if}
            {#each state.criteria as criterion (criterion.id)}
              <p class="evidence"><strong>{criterion.met ? 'Met' : 'Not met'} — {criteriaById.get(criterion.id) ?? criterion.id}:</strong> {criterion.evidence || 'No evidence recorded'}</p>
            {/each}
          {/if}
        </details>
      </li>
    {/each}
  </ol>
</section>

<style>
  .graph-view { min-width: 0; }
  h3 { font-size: var(--fs-m); font-weight: 600; margin: 0; }
  .hint { color: var(--text-dim); font-size: var(--fs-s); line-height: 1.5; }
  ol { list-style: none; padding: 0; margin: 12px 0 0; }
  ol > li + li { border-top: 1px solid var(--border); }
  summary { display: flex; align-items: baseline; gap: 12px; padding-block: 10px; cursor: pointer; }
  .title { flex: 1; font-size: var(--fs-m); font-weight: 500; min-width: 0; overflow-wrap: anywhere; }
  .state { color: var(--text-dim); font-size: var(--fs-s); flex: none; }
  .state.passed { color: var(--success); }
  .state.blocked { color: var(--warning); }
  .state.running { color: var(--info); }
  p, ul { font-size: var(--fs-s); line-height: 1.5; overflow-wrap: anywhere; }
  ul { padding-inline-start: 20px; }
  .evidence { white-space: pre-wrap; }
</style>
