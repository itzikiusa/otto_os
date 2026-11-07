<script lang="ts">
  import type { AcceptanceCriterion, GoalGraphNodeState, GoalLoopGraph, GoalLoopStatus } from '../../lib/api/types';
  import ExecutionDiagram from './ExecutionDiagram.svelte';
  import type { DiagramNode } from './diagramLayout';
  let { graph, states, criteria, loopStatus }: { graph: GoalLoopGraph; states: GoalGraphNodeState[]; criteria: AcceptanceCriterion[]; loopStatus: GoalLoopStatus } = $props();
  const stateById = $derived(new Map(states.map(state => [state.node_id, state])));
  const titleById = $derived(new Map(graph.nodes.map(node => [node.id, node.title])));
  const criteriaById = $derived(new Map(criteria.map(criterion => [criterion.id, criterion.text])));
  const labels = { pending: 'Pending', running: 'Running', passed: 'Passed', blocked: 'Blocked' };
  let selected = $state('');
  const active = $derived(graph.nodes.find(node => node.id === selected) ?? graph.nodes.find(node => stateById.get(node.id)?.status === 'running') ?? graph.nodes.find(node => stateById.get(node.id)?.status === 'blocked') ?? graph.nodes[0]);
  const nodes = $derived<DiagramNode[]>(graph.nodes.map(node => {
    const state = stateById.get(node.id);
    return { id: node.id, title: node.title, note: stateLabel(state), tone: state?.status === 'running' ? (loopStatus === 'running' ? 'running' : undefined) : state?.status === 'passed' || state?.status === 'blocked' ? state.status : undefined };
  }));
  const edges = $derived(graph.nodes.flatMap(node => node.depends_on.map(id => ({ from: id, to: node.id }))));
  function stateLabel(state: GoalGraphNodeState | undefined): string {
    if (!state) return 'Not started';
    if (state.status === 'running' && loopStatus !== 'running') return loopStatus === 'paused' ? 'Paused' : 'Interrupted';
    return labels[state.status];
  }
</script>

<section class="graph-view" aria-label="Engineering graph" data-testid="goal-graph-state">
  <h3>Engineering graph</h3>
  <p class="hint">One ready task runs per iteration after its prerequisites pass. Work is sequential; earlier evidence is rechecked when work changes.</p>
  <ExecutionDiagram {nodes} {edges} selected={active?.id ?? ''} onselect={(id) => selected = id} label="Engineering graph progress" />
    {#if active}
      {@const state = stateById.get(active.id)}
      <div class="task-detail" data-testid="graph-task-details">
          <h4>{active.title} <span class="hint">— {stateLabel(state)}</span></h4>
          {#if active.depends_on.length}<p class="hint">After: {active.depends_on.map(id => titleById.get(id) ?? id).join(', ')}</p>{/if}
          <p>{active.instructions}</p>
          <ul>{#each active.criterion_ids as id (id)}<li>{criteriaById.get(id) ?? id}</li>{/each}</ul>
          {#if state}
            {#if state.iteration > 0}<p class="hint">Last attempted in iteration {state.iteration}</p>{/if}
            {#if state.evidence}<p class="evidence">{state.evidence}</p>{/if}
            {#each state.criteria as criterion (criterion.id)}
              <p class="evidence"><strong>{criterion.met ? 'Met' : 'Not met'} — {criteriaById.get(criterion.id) ?? criterion.id}:</strong> {criterion.evidence || 'No evidence recorded'}</p>
            {/each}
          {/if}
      </div>
    {/if}
</section>

<style>
  .graph-view { min-width: 0; }
  h3 { font-size: var(--fs-m); font-weight: 600; margin: 0; }
  .hint { color: var(--text-dim); font-size: var(--fs-s); line-height: 1.5; }
  .task-detail { margin-block-start: 12px; }
  h4 { font-size: var(--fs-s); font-weight: 500; margin: 0; overflow-wrap: anywhere; }
  p, ul { font-size: var(--fs-s); line-height: 1.5; overflow-wrap: anywhere; }
  ul { padding-inline-start: 20px; }
  .evidence { white-space: pre-wrap; }
</style>
