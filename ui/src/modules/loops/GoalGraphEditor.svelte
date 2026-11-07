<script lang="ts">
  import type { AcceptanceCriterion, GoalGraphNode, GoalLoopGraph } from '../../lib/api/types';
  import Icon from '../../lib/components/Icon.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { announceShown } from '../../lib/announce.svelte';
  import { graphError, graphLayers, newGraphNode, removeGraphNode } from './graph';

  let { graph, criteria, onchange }: {
    graph: GoalLoopGraph | null | undefined;
    criteria: AcceptanceCriterion[];
    onchange: (graph: GoalLoopGraph | null) => void;
  } = $props();
  let selected = $state('');
  const active = $derived(graph?.nodes.find(node => node.id === selected) ?? graph?.nodes[0]);
  const error = $derived(graphError(graph, criteria));
  const layers = $derived(graph ? graphLayers(graph) : []);

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
  async function standardLoop(): Promise<void> {
    if (await confirmer.ask('Remove the execution graph? Acceptance criteria stay in the draft. Each iteration will work toward the whole goal.', { title: 'Use a standard loop', confirmLabel: 'Remove graph', danger: true })) onchange(null);
  }
</script>

<section class="graph-editor" aria-label="Execution graph">
  <div class="heading"><h2>Execution graph</h2>
    {#if graph}<button type="button" class="btn small" onclick={standardLoop}>Use standard loop</button>{/if}
  </div>
  {#if !graph}
    <p class="hint">A standard loop works toward every criterion each iteration. Add a graph to enforce task prerequisites.</p>
    <button type="button" class="btn small" onclick={add}>Add execution graph</button>
  {:else}
    <p class="hint">One ready task runs per iteration using the configured executors. Dependencies must pass before a task starts.</p>
    <div class="nodes" aria-label="Graph tasks">
      {#each graph.nodes as node, i (node.id)}
        <button type="button" class="node-select" class:selected={active?.id === node.id} aria-pressed={active?.id === node.id} onclick={() => selected = node.id}>
          <span class="index">{i + 1}</span><span class="node-title">{node.title || 'Untitled task'}</span>
        </button>
      {/each}
    </div>
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
          {:else}<p class="hint">Add an acceptance criterion above.</p>{/each}
        </fieldset>
        <button type="button" class="btn small" disabled={graph.nodes.length === 1} title={graph.nodes.length === 1 ? 'A graph needs one task. Use standard loop to remove the graph.' : undefined} onclick={() => remove(active.id)}><Icon name="trash" size={12} /> Remove task</button>
      </div>
    {/if}
    <button type="button" class="btn small" disabled={graph.nodes.length >= 16} onclick={add}><Icon name="plus" size={12} /> Add task</button>
    {#if error}<p class="error" use:announceShown={{ text: error, assertive: true }}>{error}</p>{/if}
    <details class="preview"><summary>Preview dependencies</summary>
      {#if !error}
        <ol>{#each layers as layer, i (i)}<li>{layer.map(node => node.title).join(' · ')}</li>{/each}</ol>
        <p class="hint">Tasks on the same level share no prerequisites with each other; execution remains sequential.</p>
      {:else}<p class="hint">Resolve the graph error to preview the order.</p>{/if}
    </details>
  {/if}
</section>

<style>
  .graph-editor { min-width: 0; }
  .heading { display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 8px; }
  h2 { font-size: var(--fs-m); font-weight: 600; margin: 0; }
  .hint { color: var(--text-dim); font-size: var(--fs-s); line-height: 1.5; margin: 8px 0; }
  .nodes { display: flex; flex-wrap: wrap; gap: 6px; margin-block: 12px; }
  .node-select { display: flex; align-items: center; gap: 6px; max-width: 100%; border: 1px solid var(--border); border-radius: var(--radius-s); padding: 8px; background: var(--surface); color: var(--text); text-align: start; font: inherit; font-size: var(--fs-s); cursor: pointer; }
  .node-select.selected { border-color: var(--accent-solid); background: var(--accent-soft); }
  .node-title { min-width: 0; overflow-wrap: anywhere; }
  .index { color: var(--text-dim); }
  .node-fields { display: flex; flex-direction: column; gap: 12px; border: 1px solid var(--border); border-radius: var(--radius-s); padding: 12px; margin-block-end: 12px; }
  .node-fields > label { display: flex; flex-direction: column; gap: 4px; font-size: var(--fs-s); }
  .graph-input { width: 100%; box-sizing: border-box; min-width: 0; font: inherit; }
  fieldset { border: 0; padding: 0; margin: 0; min-width: 0; }
  legend { font-size: var(--fs-s); font-weight: 500; padding: 0; margin-block-end: 6px; }
  .check { display: flex; align-items: start; gap: 6px; font-size: var(--fs-s); margin-block: 6px; overflow-wrap: anywhere; }
  .check input { flex: none; margin-block-start: 2px; }
  .node-fields > button { align-self: flex-start; }
  .error { color: var(--danger); font-size: var(--fs-s); }
  .preview { margin-block-start: 12px; font-size: var(--fs-s); }
  summary { cursor: pointer; }
  ol { padding-inline-start: 24px; overflow-wrap: anywhere; }
  li { padding-block: 4px; }
  @media (max-width: 640px) { .node-select, .check { min-height: var(--hit-min); } }
</style>
