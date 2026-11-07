<script lang="ts">
  import type { AcceptanceCriterion, GoalGraphNode, GoalLoopGraph } from '../../lib/api/types';
  import Icon from '../../lib/components/Icon.svelte';
  import { announceShown } from '../../lib/announce.svelte';
  import { graphError, newGraphNode, removeGraphNode } from './graph';
  import ExecutionDiagram from './ExecutionDiagram.svelte';

  let { graph, criteria, onchange }: {
    graph: GoalLoopGraph | null | undefined;
    criteria: AcceptanceCriterion[];
    onchange: (graph: GoalLoopGraph | null) => void;
  } = $props();
  let selected = $state('');
  const active = $derived(graph?.nodes.find(node => node.id === selected) ?? graph?.nodes[0]);
  const error = $derived(graphError(graph, criteria));
  const nodes = $derived((graph?.nodes ?? []).map(node => ({ id: node.id, title: node.title || 'Untitled task', note: `${node.criterion_ids.length} ${node.criterion_ids.length === 1 ? 'criterion' : 'criteria'}` })));
  const edges = $derived((graph?.nodes ?? []).flatMap(node => node.depends_on.map(id => ({ from: id, to: node.id }))));

  function update(id: string, patch: Partial<GoalGraphNode>): void {
    if (graph) onchange({ nodes: graph.nodes.map(node => node.id === id ? { ...node, ...patch } : node) });
  }
  function toggle(node: GoalGraphNode, field: 'depends_on' | 'criterion_ids', id: string): void {
    update(node.id, { [field]: node[field].includes(id) ? node[field].filter(value => value !== id) : [...node[field], id] });
  }
  function add(): void {
    const current = graph ?? { nodes: [] };
    const node = newGraphNode(current, criteria);
    if (!graph) node.criterion_ids = criteria.map(criterion => criterion.id);
    selected = node.id;
    onchange({ nodes: [...current.nodes, node] });
  }
  function remove(id: string): void {
    if (!graph) return;
    const next = removeGraphNode(graph, id);
    selected = next.nodes[0]?.id ?? '';
    onchange(next);
  }
</script>

<section class="graph-editor" aria-label="Engineering graph">
  <h2>Engineering graph</h2>
  {#if !graph}
    <p class="hint">Add tasks and connect their prerequisites. Each task verifies its linked acceptance criteria.</p>
    <button type="button" class="btn small" onclick={add}>Add first task</button>
  {:else}
    <p class="hint">Arrows show prerequisites. One ready task runs per iteration, sequentially. Select a task to edit its work and evidence criteria.</p>
    <ExecutionDiagram {nodes} {edges} selected={active?.id ?? ''} onselect={(id) => selected = id} label="Engineering graph tasks" />
    {#if active}
      <div class="node-fields" data-testid="graph-node-editor">
        <label>Task title<input class="input graph-input" dir="auto" value={active.title} oninput={(e) => update(active.id, { title: e.currentTarget.value })} /></label>
        <label>Task instructions<textarea class="input graph-input" dir="auto" rows="4" value={active.instructions} oninput={(e) => update(active.id, { instructions: e.currentTarget.value })}></textarea></label>
        <fieldset><legend>Depends on</legend>
          {#each graph.nodes.filter(node => node.id !== active.id) as node (node.id)}
            <label class="check"><input type="checkbox" checked={active.depends_on.includes(node.id)} onchange={() => toggle(active, 'depends_on', node.id)} /><span>{node.title || 'Untitled task'}</span></label>
          {:else}<p class="hint">Add another task to create a dependency.</p>{/each}
        </fieldset>
        <fieldset><legend>Acceptance criteria</legend>
          {#each criteria as criterion (criterion.id)}
            <label class="check"><input type="checkbox" checked={active.criterion_ids.includes(criterion.id)} onchange={() => toggle(active, 'criterion_ids', criterion.id)} /><span>{criterion.text || 'Untitled criterion'}</span></label>
          {:else}<p class="hint">Add an acceptance criterion to the goal.</p>{/each}
        </fieldset>
        <button type="button" class="btn small" disabled={graph.nodes.length === 1} title={graph.nodes.length === 1 ? 'A graph needs one task. Choose Goal loop in Execution to use whole-goal iterations.' : undefined} onclick={() => remove(active.id)}><Icon name="trash" size={12} /> Remove task</button>
      </div>
    {/if}
    <button type="button" class="btn small" disabled={graph.nodes.length >= 16} onclick={add}><Icon name="plus" size={12} /> Add task</button>
    {#if error}<p class="error" use:announceShown={{ text: error, assertive: true }}>{error}</p>{/if}
  {/if}
</section>

<style>
  .graph-editor { min-width: 0; }
  h2 { font-size: var(--fs-m); font-weight: 600; margin: 0; }
  .hint { color: var(--text-dim); font-size: var(--fs-s); line-height: 1.5; margin: 8px 0; }
  .node-fields { display: flex; flex-direction: column; gap: 12px; border: 1px solid var(--border); border-radius: var(--radius-s); padding: 12px; margin-block: 12px; }
  .node-fields > label { display: flex; flex-direction: column; gap: 4px; font-size: var(--fs-s); }
  .graph-input { width: 100%; box-sizing: border-box; min-width: 0; font: inherit; }
  fieldset { border: 0; padding: 0; margin: 0; min-width: 0; }
  legend { font-size: var(--fs-s); font-weight: 500; padding: 0; margin-block-end: 6px; }
  .check { display: flex; align-items: start; gap: 6px; font-size: var(--fs-s); margin-block: 6px; overflow-wrap: anywhere; }
  .check input { flex: none; margin-block-start: 2px; }
  .node-fields > button { align-self: flex-start; }
  .error { color: var(--danger); font-size: var(--fs-s); }
  @media (max-width: 640px) { .check { min-height: var(--hit-min); } }
</style>
