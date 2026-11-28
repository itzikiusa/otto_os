<script lang="ts">
  // The editor on a phone (ux.md §10): the steps as a read-only outline in
  // layout order (parallel steps indented); a tap opens the step in the
  // editor's inspector (a full-width sheet whose Done saves that step). The
  // graph itself — adding, deleting or connecting steps — is edited on a
  // larger screen.
  import Icon from '../../../lib/components/Icon.svelte';
  import type { WorkflowGraph, WorkflowTrigger } from '../../../lib/api/types';
  import { autoLayout } from './autoLayout';
  import { nodeIcon, nodeSummary } from './nodeSummary';

  interface Props {
    graph: WorkflowGraph;
    triggers?: readonly WorkflowTrigger[];
    defaultProvider: string;
    onopen: (id: string) => void;
  }
  let { graph, triggers = [], defaultProvider, onopen }: Props = $props();
  const layout = $derived(autoLayout(graph));
  const byId = $derived(new Map(graph.nodes.map((n) => [n.id, n])));
  const ctx = $derived({ nodes: graph.nodes, defaultProvider, triggerKinds: triggers.map((t) => t.kind) });
  const rows = $derived([
    ...layout.layers.flatMap((l) => l.map((id) => ({ id, nested: l.length > 1 }))),
    ...layout.loose.map((id) => ({ id, nested: false })),
  ]);
</script>

<div class="po" data-testid="wf-phone-outline">
  <p class="po-note"><Icon name="layout" size={14} />The graph is edited on a larger screen. Tap a step to change its settings.</p>
  <ol class="po-list">
    {#each rows as r (r.id)}
      {@const n = byId.get(r.id)}
      {#if n}
        <li class:nested={r.nested} class:loose={layout.loose.includes(r.id)}>
          <button type="button" class="po-row" data-node-id={n.id} onclick={() => onopen(n.id)}>
            <span class="po-tile"><Icon name={nodeIcon(n.kind)} size={16} /></span>
            <span class="po-text" dir="auto">
              <span class="po-name">{n.name || n.kind}</span>
              <span class="po-sum">{layout.loose.includes(r.id) ? 'Not connected' : nodeSummary(n, ctx)}</span>
            </span>
            <Icon name="chevronRight" size={14} />
          </button>
        </li>
      {/if}
    {/each}
  </ol>
</div>

<style>
  .po {
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
    padding: var(--sp-3) var(--sp-4);
  }
  .po-note {
    display: flex;
    align-items: flex-start;
    gap: var(--sp-2);
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .po-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
  }
  .po-list li.nested {
    padding-inline-start: var(--sp-5);
  }
  .po-row {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    inline-size: 100%;
    min-block-size: 56px;
    padding: var(--sp-2) 0;
    border: none;
    border-block-end: 1px solid var(--border);
    background: none;
    color: var(--text-dim);
    font: inherit;
    text-align: start;
    cursor: pointer;
  }
  .po-tile {
    display: grid;
    place-items: center;
    flex: none;
    inline-size: 32px;
    block-size: 32px;
    border-radius: var(--radius-s);
    background: var(--surface-2);
  }
  .po-text {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-inline-size: 0;
  }
  .po-name {
    color: var(--text);
    font-size: var(--fs-m);
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .po-sum {
    font-size: var(--fs-s);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .loose .po-sum {
    color: var(--warning);
  }
  .po-row :global(svg:last-child) {
    flex: none;
  }
</style>
