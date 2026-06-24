<script lang="ts">
  // Freehand node — renders a stored stroke (perfect-freehand-style point list)
  // as an SVG polyline that fills the node box. Drawing new strokes is out of
  // scope for this build (the AI/import paths populate `freehand.points`); we
  // render whatever points exist, scaled into the box. Not connectable (it's
  // decorative ink), so no Handles — matching frame/freehand per the spec.
  import type { CanvasNode } from '../types';

  interface Props {
    data: { node: CanvasNode };
    selected?: boolean;
    width?: number;
    height?: number;
  }
  let { data, selected, width, height }: Props = $props();

  const node = $derived(data.node);
  const points = $derived(node.freehand?.points ?? []);
  const color = $derived(node.freehand?.color ?? 'var(--text)');
  const size = $derived(node.freehand?.size ?? 4);
  const w = $derived(width ?? node.w ?? 160);
  const h = $derived(height ?? node.h ?? 90);

  // Project the raw point list into the box's local coordinate space so the
  // stroke is visible regardless of the absolute coords it was captured at.
  const path = $derived.by(() => {
    if (points.length < 2) return '';
    const xs = points.map((p) => p[0]);
    const ys = points.map((p) => p[1]);
    const minX = Math.min(...xs);
    const maxX = Math.max(...xs);
    const minY = Math.min(...ys);
    const maxY = Math.max(...ys);
    const sx = maxX - minX || 1;
    const sy = maxY - minY || 1;
    const pad = 6;
    return points
      .map((p, i) => {
        const x = pad + ((p[0] - minX) / sx) * (w - pad * 2);
        const y = pad + ((p[1] - minY) / sy) * (h - pad * 2);
        return `${i === 0 ? 'M' : 'L'}${x.toFixed(1)},${y.toFixed(1)}`;
      })
      .join(' ');
  });
</script>

<div class="freehand" class:selected>
  <svg width={w} height={h} viewBox={`0 0 ${w} ${h}`}>
    {#if path}
      <path
        d={path}
        fill="none"
        stroke={color}
        stroke-width={size}
        stroke-linecap="round"
        stroke-linejoin="round"
      />
    {:else}
      <text x={w / 2} y={h / 2} text-anchor="middle" class="empty">freehand</text>
    {/if}
  </svg>
</div>

<style>
  .freehand {
    width: 100%;
    height: 100%;
  }
  .freehand.selected {
    outline: 1px dashed var(--accent);
    border-radius: var(--radius-s);
  }
  svg {
    display: block;
    width: 100%;
    height: 100%;
  }
  .empty {
    fill: var(--text-dim);
    font-size: 12px;
  }
</style>
