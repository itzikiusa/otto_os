<script lang="ts">
  // Tiny inline-SVG sparkline (no library). `points` are plotted left→right
  // against their own min/max; a flat series draws a mid line.
  interface Props {
    points: number[];
    width?: number;
    height?: number;
    stroke?: string;
    label?: string;
  }
  let { points, width = 110, height = 26, stroke = 'var(--accent)', label = '' }: Props = $props();

  const path = $derived.by(() => {
    const n = points.length;
    if (n === 0) return '';
    if (n === 1) return `M0,${height / 2} L${width},${height / 2}`;
    let min = Math.min(...points);
    let max = Math.max(...points);
    if (!Number.isFinite(min) || !Number.isFinite(max)) return '';
    if (max === min) {
      max = min + 1;
      min = min - 1;
    }
    const pad = 2;
    const dx = (width - 2 * pad) / (n - 1);
    return points
      .map((v, i) => {
        const x = pad + i * dx;
        const y = pad + (height - 2 * pad) * (1 - (v - min) / (max - min));
        return `${i === 0 ? 'M' : 'L'}${x.toFixed(1)},${y.toFixed(1)}`;
      })
      .join(' ');
  });
</script>

{#if points.length}
  <svg class="spark" {width} {height} viewBox="0 0 {width} {height}" role="img" aria-label={label || 'trend'}>
    <path d={path} fill="none" {stroke} stroke-width="1.5" stroke-linejoin="round" stroke-linecap="round" />
  </svg>
{:else}
  <span class="spark-empty" title="No samples in this window">—</span>
{/if}

<style>
  .spark {
    display: block;
    overflow: visible;
  }
  .spark-empty {
    color: var(--text-dim);
    font-size: 11px;
  }
</style>
