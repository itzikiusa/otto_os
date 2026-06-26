<script lang="ts">
  import type { GraphView, GraphNode } from '../../lib/api/types';
  import { KIND_LABEL, WORK_KINDS, statusColor } from './lib';

  interface Props {
    graph: GraphView;
    selectedId: string | null;
    onOpen: (id: string) => void;
  }
  let { graph, selectedId, onOpen }: Props = $props();

  const COL_W = 200;
  const ROW_H = 74;
  const PAD = 48;

  type Placed = { x: number; y: number; r: number; node: GraphNode };

  const layout = $derived.by(() => {
    const byKind = new Map<string, GraphNode[]>();
    for (const n of graph.nodes) {
      const arr = byKind.get(n.kind) ?? [];
      arr.push(n);
      byKind.set(n.kind, arr);
    }
    const cols = WORK_KINDS.filter((k) => byKind.has(k));
    const pos = new Map<string, Placed>();
    let maxRows = 0;
    cols.forEach((k, ci) => {
      const arr = byKind.get(k) ?? [];
      maxRows = Math.max(maxRows, arr.length);
      arr.forEach((n, ri) => {
        const cost = n.cost_so_far > 0 ? n.cost_so_far : 0;
        const r = 13 + Math.min(9, Math.sqrt(cost) * 3.2);
        pos.set(n.id, { x: PAD + ci * COL_W + 66, y: PAD + 34 + ri * ROW_H, r, node: n });
      });
    });
    const width = Math.max(440, cols.length * COL_W + PAD * 2);
    const height = Math.max(260, maxRows * ROW_H + PAD * 2);
    const edges = graph.edges.filter((e) => pos.has(e.from_item_id) && pos.has(e.to_item_id));
    return { cols, pos: Array.from(pos.values()), posMap: pos, width, height, edges };
  });

  function trunc(s: string, n = 20): string {
    return s.length > n ? s.slice(0, n - 1) + '…' : s;
  }
</script>

<div class="graph-wrap">
  {#if graph.nodes.length === 0}
    <div class="graph-empty dim">No work items match the current filters.</div>
  {:else}
    <svg
      class="graph-svg"
      viewBox={`0 0 ${layout.width} ${layout.height}`}
      preserveAspectRatio="xMidYMin meet"
      role="img"
      aria-label="Work graph"
    >
      <!-- column headers -->
      {#each layout.cols as k, ci (k)}
        <text class="col-head" x={PAD + ci * COL_W + 66} y={26} text-anchor="middle">{KIND_LABEL[k]}</text>
      {/each}

      <!-- edges -->
      {#each layout.edges as e (e.from_item_id + e.to_item_id + e.relation)}
        {@const a = layout.posMap.get(e.from_item_id)}
        {@const b = layout.posMap.get(e.to_item_id)}
        {#if a && b}
          <line class="edge" x1={a.x} y1={a.y} x2={b.x} y2={b.y} />
          <text class="edge-label" x={(a.x + b.x) / 2} y={(a.y + b.y) / 2 - 3} text-anchor="middle">
            {e.relation.replace('_', ' ')}
          </text>
        {/if}
      {/each}

      <!-- nodes -->
      {#each layout.pos as p (p.node.id)}
        <g
          class="node"
          class:selected={p.node.id === selectedId}
          role="button"
          tabindex="0"
          onclick={() => onOpen(p.node.id)}
          onkeydown={(ev) => {
            if (ev.key === 'Enter' || ev.key === ' ') {
              ev.preventDefault();
              onOpen(p.node.id);
            }
          }}
        >
          <title>{p.node.title} — {p.node.status}</title>
          <circle cx={p.x} cy={p.y} r={p.r} fill={statusColor(p.node.status)} />
          {#if p.node.needs_approval}
            <circle class="approve-ring" cx={p.x} cy={p.y} r={p.r + 4} />
          {/if}
          <text class="node-label" x={p.x} y={p.y + p.r + 13} text-anchor="middle">{trunc(p.node.title)}</text>
        </g>
      {/each}
    </svg>
  {/if}
</div>

<style>
  .graph-wrap {
    width: 100%;
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius-m, 8px);
    background:
      radial-gradient(circle at 1px 1px, color-mix(in srgb, var(--border) 60%, transparent) 1px, transparent 0)
      0 0 / 22px 22px;
    min-height: 260px;
  }
  .graph-svg {
    display: block;
    width: 100%;
    height: auto;
    min-height: 260px;
  }
  .graph-empty {
    padding: 48px 16px;
    text-align: center;
    min-height: 200px;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .col-head {
    fill: var(--text-dim);
    font-size: 11px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .edge {
    stroke: var(--text-dim);
    stroke-width: 1.2;
    opacity: 0.45;
  }
  .edge-label {
    fill: var(--text-dim);
    font-size: 9px;
    opacity: 0.8;
  }
  .node {
    cursor: pointer;
  }
  .node circle {
    stroke: var(--bg);
    stroke-width: 2;
    transition: r 0.1s;
  }
  .node:hover circle {
    stroke: var(--text);
  }
  .node.selected > circle {
    stroke: #7ee787;
    stroke-width: 3;
  }
  .approve-ring {
    fill: none;
    stroke: #ffd33d;
    stroke-width: 1.6;
    stroke-dasharray: 3 3;
  }
  .node-label {
    fill: var(--text);
    font-size: 10px;
    pointer-events: none;
  }
</style>
