<script lang="ts">
  import { diagramLayout, type DiagramNode, type DiagramEdge } from './diagramLayout';
  let { nodes, edges, selected, onselect, label, loop = false }: {
    nodes: DiagramNode[]; edges: DiagramEdge[]; selected: string; onselect: (id: string) => void; label: string; loop?: boolean;
  } = $props();
  let width = $state(320);
  const componentId = $props.id();
  const marker = `${componentId}-arrow`;
  const layout = $derived(diagramLayout(nodes, edges, width || 320, loop));

  function moveFocus(event: KeyboardEvent): void {
    if (!['ArrowDown', 'ArrowRight', 'ArrowUp', 'ArrowLeft', 'Home', 'End'].includes(event.key)) return;
    const button = event.currentTarget as HTMLButtonElement;
    const buttons = Array.from(button.parentElement?.querySelectorAll<HTMLButtonElement>('[data-diagram-node]') ?? []);
    const current = buttons.indexOf(button);
    const offset = event.key === 'ArrowUp' || event.key === 'ArrowLeft' ? -1 : 1;
    const next = event.key === 'Home' ? 0 : event.key === 'End' ? buttons.length - 1 : (current + offset + buttons.length) % buttons.length;
    event.preventDefault();
    buttons[next]?.focus();
  }
</script>

<div class="diagram-scroll" bind:clientWidth={width}>
  <div class="diagram-stage" role="group" aria-label={label} dir="ltr" style:height={`${layout.height}px`} data-testid="execution-diagram">
    <svg class="diagram-edges" width="100%" height={layout.height} aria-hidden="true">
      <defs><marker id={marker} viewBox="0 0 8 8" refX="7" refY="4" markerWidth="6" markerHeight="6" orient="auto-start-reverse"><path d="M 0 0 L 8 4 L 0 8 z" /></marker></defs>
      {#each layout.edges as edge, i (`${edge.from}:${edge.to}:${i}`)}
        <path class="dependency" class:feedback={edge.feedback} d={edge.path} marker-end={`url(#${marker})`} data-edge={`${edge.from}:${edge.to}`} />
      {/each}
    </svg>
    {#each layout.nodes as node (node.id)}
      <button type="button" class="diagram-node" class:selected={selected === node.id} class:running={node.tone === 'running'} class:passed={node.tone === 'passed'} class:blocked={node.tone === 'blocked'}
        data-diagram-node={node.id} aria-pressed={selected === node.id} aria-label={`${node.title}${node.note ? ` — ${node.note}` : ''}`} title={node.title}
        style:inset-inline-start={`${node.x}px`} style:top={`${node.y}px`} style:width={`${node.width}px`} style:height={`${node.height}px`}
        onclick={() => onselect(node.id)} onkeydown={moveFocus}>
        <span class="diagram-title" dir="auto">{node.title}</span>
        {#if node.note}<span class="diagram-note" dir="auto">{node.note}</span>{/if}
      </button>
    {/each}
  </div>
</div>

<style>
  .diagram-scroll { width: 100%; max-height: 560px; overflow: auto; border: 1px solid var(--border); border-radius: var(--radius-m); background: var(--surface); }
  .diagram-stage { position: relative; width: 100%; min-width: 0; }
  .diagram-edges { position: absolute; inset: 0; pointer-events: none; overflow: visible; }
  marker path { fill: var(--text-dim); }
  .dependency { fill: none; stroke: var(--text-dim); stroke-width: 1.5; stroke-linejoin: round; }
  .feedback { stroke-dasharray: 5 4; }
  .diagram-node { position: absolute; display: flex; flex-direction: column; justify-content: center; gap: 4px; padding: 8px; border: 1px solid var(--border); border-radius: var(--radius-m); background: var(--surface); color: var(--text); font: inherit; text-align: center; cursor: pointer; }
  .diagram-node.selected { border-color: var(--accent-solid); background: var(--accent-soft); }
  .diagram-node:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 2px; }
  .diagram-title { font-size: var(--fs-s); font-weight: 500; overflow: hidden; display: -webkit-box; line-clamp: 2; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow-wrap: anywhere; }
  .diagram-note { color: var(--text-dim); font-size: var(--fs-xs); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .running .diagram-note { color: var(--info); }
  .passed .diagram-note { color: var(--success); }
  .blocked .diagram-note { color: var(--warning); }
</style>
