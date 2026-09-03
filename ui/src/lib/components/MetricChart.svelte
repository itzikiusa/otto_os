<script lang="ts">
  // Generic time-series chart — hand-rolled inline SVG, no charting dependency
  // (sibling of modules/database/Chart.svelte, which stays QueryResult-bound).
  // Multi-series line/area over a shared time axis: y ticks in human units
  // (bytes / percent / seconds / rates), x ticks in local time, gaps (`null`)
  // break the line, hover snaps to the nearest sample and shows time + every
  // series' value. Width is fluid (`viewBox` + `width:100%`), so it fits a
  // phone; colours come from the theme's CSS vars. Shapes + formatters live
  // in `lib/metric-format.ts` so stat rows can share them.
  import {
    formatMetric,
    formatTimeTick,
    type MetricChartPoint,
    type MetricChartSeries,
    type MetricChartUnit,
  } from '../metric-format';

  interface Props {
    series: MetricChartSeries[];
    unit?: MetricChartUnit;
    /** Pixel height of the plot (the width follows the container). */
    height?: number;
    showLegend?: boolean;
    /** Fill under each line. */
    area?: boolean;
    /** Shown when every series is empty / all-null. */
    emptyText?: string;
  }
  let {
    series,
    unit = 'count',
    height = 160,
    showLegend = true,
    area = true,
    emptyText = 'No data',
  }: Props = $props();

  const PALETTE = ['var(--accent)', '#28c840', '#d2691e', '#bf5af2', '#0e8a8a', '#ff5f57', '#febc2e'];
  const W = 600;
  const PAD = { l: 44, r: 10, t: 10, b: 22 };

  // ── formatting ─────────────────────────────────────────────────────────────
  const fmt = (v: number) => formatMetric(v, unit);
  const fmtTick = formatTimeTick;
  function fmtFull(t: number): string {
    return new Date(t).toLocaleString();
  }

  // ── domain ─────────────────────────────────────────────────────────────────
  const colorOf = (i: number) => series[i]?.color ?? PALETTE[i % PALETTE.length];

  const hasData = $derived(series.some((s) => s.points.some((p) => p.v != null)));

  const xDomain = $derived.by(() => {
    let lo = Infinity;
    let hi = -Infinity;
    for (const s of series)
      for (const p of s.points) {
        if (p.t < lo) lo = p.t;
        if (p.t > hi) hi = p.t;
      }
    if (!Number.isFinite(lo)) return { lo: 0, hi: 1 };
    if (hi === lo) hi = lo + 60_000;
    return { lo, hi };
  });

  /** "Nice" y ceiling so ticks land on round numbers (binary steps for bytes). */
  function niceCeil(v: number): number {
    if (v <= 0) return 1;
    if (unit === 'bytes' || unit === 'bytes_per_sec') {
      let base = 1;
      while (v / base >= 1024) base *= 1024;
      const m = v / base;
      const step = m <= 1 ? 1 : m <= 2 ? 2 : m <= 4 ? 4 : m <= 8 ? 8 : m <= 16 ? 16 : m <= 32 ? 32 : m <= 64 ? 64 : m <= 128 ? 128 : m <= 256 ? 256 : m <= 512 ? 512 : 1024;
      return step * base;
    }
    const exp = Math.floor(Math.log10(v));
    const base = 10 ** exp;
    const m = v / base;
    const step = m <= 1 ? 1 : m <= 2 ? 2 : m <= 2.5 ? 2.5 : m <= 5 ? 5 : 10;
    return step * base;
  }

  const yDomain = $derived.by(() => {
    let lo = 0;
    let hi = 0;
    for (const s of series)
      for (const p of s.points) {
        if (p.v == null || !Number.isFinite(p.v)) continue;
        if (p.v < lo) lo = p.v;
        if (p.v > hi) hi = p.v;
      }
    if (unit === 'percent' && hi <= 100 && hi > 50) hi = 100;
    else hi = niceCeil(hi);
    if (lo < 0) lo = -niceCeil(-lo);
    if (hi === lo) hi = lo + 1;
    return { lo, hi };
  });

  const H = $derived(Math.max(80, height));
  const plotW = $derived(W - PAD.l - PAD.r);
  const plotH = $derived(H - PAD.t - PAD.b);

  function x(t: number): number {
    const { lo, hi } = xDomain;
    return PAD.l + (plotW * (t - lo)) / (hi - lo);
  }
  function y(v: number): number {
    const { lo, hi } = yDomain;
    return PAD.t + plotH - (plotH * (v - lo)) / (hi - lo);
  }
  const y0 = $derived(y(Math.max(0, yDomain.lo)));

  /** Line path per series — a `null` breaks the segment (gap). */
  function linePath(points: MetricChartPoint[]): string {
    let d = '';
    let pen = false;
    for (const p of points) {
      if (p.v == null || !Number.isFinite(p.v)) {
        pen = false;
        continue;
      }
      d += `${pen ? 'L' : 'M'} ${x(p.t).toFixed(1)} ${y(p.v).toFixed(1)} `;
      pen = true;
    }
    return d;
  }
  /** Area path: one closed polygon per contiguous run. */
  function areaPath(points: MetricChartPoint[]): string {
    let d = '';
    let run: MetricChartPoint[] = [];
    const flush = () => {
      if (run.length === 0) return;
      const first = run[0];
      const last = run[run.length - 1];
      d += `M ${x(first.t).toFixed(1)} ${y0.toFixed(1)} `;
      for (const p of run) d += `L ${x(p.t).toFixed(1)} ${y(p.v as number).toFixed(1)} `;
      d += `L ${x(last.t).toFixed(1)} ${y0.toFixed(1)} Z `;
      run = [];
    };
    for (const p of points) {
      if (p.v == null || !Number.isFinite(p.v)) flush();
      else run.push(p);
    }
    flush();
    return d;
  }

  // Single isolated points would be invisible as a line — draw dots for them.
  function lonePoints(points: MetricChartPoint[]): MetricChartPoint[] {
    const out: MetricChartPoint[] = [];
    for (let i = 0; i < points.length; i++) {
      const p = points[i];
      if (p.v == null) continue;
      const prev = points[i - 1]?.v ?? null;
      const next = points[i + 1]?.v ?? null;
      if (prev == null && next == null) out.push(p);
    }
    return out;
  }

  const yTicks = $derived.by(() => {
    const { lo, hi } = yDomain;
    const n = 4;
    const out: number[] = [];
    for (let i = 0; i <= n; i++) out.push(lo + ((hi - lo) * i) / n);
    return out;
  });

  const xTicks = $derived.by(() => {
    const { lo, hi } = xDomain;
    const span = hi - lo;
    const n = 5;
    const out: { t: number; label: string }[] = [];
    for (let i = 0; i <= n; i++) {
      const t = lo + (span * i) / n;
      out.push({ t, label: fmtTick(t, span) });
    }
    return out;
  });

  // ── hover ──────────────────────────────────────────────────────────────────
  let svgEl = $state<SVGSVGElement | null>(null);
  let hover = $state<{ t: number; px: number; values: (number | null)[] } | null>(null);

  /** All distinct sample times (sorted) — hover snaps to these. */
  const times = $derived.by(() => {
    const set = new Set<number>();
    for (const s of series) for (const p of s.points) set.add(p.t);
    return [...set].sort((a, b) => a - b);
  });

  function onMove(e: PointerEvent): void {
    if (!svgEl || times.length === 0) return;
    const rect = svgEl.getBoundingClientRect();
    const fx = ((e.clientX - rect.left) / rect.width) * W;
    const { lo, hi } = xDomain;
    const t = lo + ((fx - PAD.l) / plotW) * (hi - lo);
    // Nearest sample by binary search.
    let a = 0;
    let b = times.length - 1;
    while (a < b) {
      const mid = (a + b) >> 1;
      if (times[mid] < t) a = mid + 1;
      else b = mid;
    }
    const cand = [times[a], times[a - 1]].filter((v): v is number => v != null);
    const nearest = cand.reduce((best, c) => (Math.abs(c - t) < Math.abs(best - t) ? c : best), cand[0]);
    hover = {
      t: nearest,
      px: x(nearest),
      values: series.map((s) => s.points.find((p) => p.t === nearest)?.v ?? null),
    };
  }
  function onLeave(): void {
    hover = null;
  }

  // Tooltip sits left of the cursor once past the midpoint so it never leaves
  // the chart's own box (which is what the parent's overflow clips to).
  const tipLeft = $derived(hover ? (hover.px / W) * 100 : 0);
  const tipFlip = $derived(hover ? hover.px > W * 0.55 : false);
</script>

<div class="mc" style="--mc-h:{H}px">
  {#if !hasData}
    <div class="mc-empty" style="height:{H}px">{emptyText}</div>
  {:else}
    <div class="mc-plot">
      <svg
        bind:this={svgEl}
        viewBox="0 0 {W} {H}"
        preserveAspectRatio="none"
        class="mc-svg"
        style="height:{H}px"
        role="img"
        aria-label={series.map((s) => s.label).join(', ')}
        onpointermove={onMove}
        onpointerleave={onLeave}
        onpointercancel={onLeave}
      >
        <!-- grid + y ticks -->
        {#each yTicks as v, i (i)}
          <line x1={PAD.l} x2={W - PAD.r} y1={y(v).toFixed(1)} y2={y(v).toFixed(1)} class="grid" />
          <text x={PAD.l - 6} y={y(v).toFixed(1)} class="tick y" text-anchor="end" dominant-baseline="middle">{fmt(v)}</text>
        {/each}
        <!-- x ticks -->
        {#each xTicks as tk, i (i)}
          <text
            x={x(tk.t).toFixed(1)}
            y={H - 6}
            class="tick x"
            text-anchor={i === 0 ? 'start' : i === xTicks.length - 1 ? 'end' : 'middle'}
          >{tk.label}</text>
        {/each}
        <!-- series -->
        {#each series as s, i (s.label)}
          {#if area}
            <path d={areaPath(s.points)} fill={colorOf(i)} fill-opacity="0.12" stroke="none" />
          {/if}
          <path d={linePath(s.points)} fill="none" stroke={colorOf(i)} stroke-width="1.6" stroke-linejoin="round" vector-effect="non-scaling-stroke" />
          {#each lonePoints(s.points) as p (p.t)}
            <circle cx={x(p.t).toFixed(1)} cy={y(p.v as number).toFixed(1)} r="2" fill={colorOf(i)} vector-effect="non-scaling-stroke" />
          {/each}
        {/each}
        <!-- hover cursor -->
        {#if hover}
          <line x1={hover.px.toFixed(1)} x2={hover.px.toFixed(1)} y1={PAD.t} y2={PAD.t + plotH} class="cursor" />
          {#each hover.values as v, i (i)}
            {#if v != null}
              <circle cx={hover.px.toFixed(1)} cy={y(v).toFixed(1)} r="3" fill={colorOf(i)} stroke="var(--surface)" stroke-width="1.5" vector-effect="non-scaling-stroke" />
            {/if}
          {/each}
        {/if}
      </svg>
      {#if hover}
        <div class="mc-tip" class:flip={tipFlip} style="left:{tipLeft}%">
          <div class="mc-tip-t">{fmtFull(hover.t)}</div>
          {#each series as s, i (s.label)}
            <div class="mc-tip-row">
              <span class="sw" style="background:{colorOf(i)}"></span>
              <span class="lbl">{s.label}</span>
              <span class="val mono">{hover.values[i] == null ? '—' : fmt(hover.values[i] as number)}</span>
            </div>
          {/each}
        </div>
      {/if}
    </div>
    {#if showLegend && series.length > 1}
      <ul class="mc-legend">
        {#each series as s, i (s.label)}
          <li><span class="sw" style="background:{colorOf(i)}"></span>{s.label}</li>
        {/each}
      </ul>
    {/if}
  {/if}
</div>

<style>
  .mc {
    width: 100%;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .mc-plot {
    position: relative;
    width: 100%;
  }
  .mc-svg {
    display: block;
    width: 100%;
    touch-action: pan-y;
    overflow: visible;
  }
  .grid {
    stroke: var(--border);
    stroke-width: 0.75;
    vector-effect: non-scaling-stroke;
  }
  .tick {
    fill: var(--text-dim);
    font-size: 10px;
    font-family: var(--font-mono, ui-monospace, monospace);
  }
  .cursor {
    stroke: var(--text-dim);
    stroke-width: 1;
    stroke-dasharray: 3 3;
    vector-effect: non-scaling-stroke;
  }
  .mc-empty {
    display: grid;
    place-items: center;
    color: var(--text-dim);
    font-size: 11.5px;
    border: 1px dashed var(--border);
    border-radius: var(--radius-m, 6px);
  }
  .mc-tip {
    position: absolute;
    top: 6px;
    transform: translateX(8px);
    pointer-events: none;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-m, 6px);
    box-shadow: 0 4px 14px rgba(0, 0, 0, 0.18);
    padding: 6px 8px;
    font-size: 11px;
    min-width: 120px;
    max-width: 240px;
    z-index: 2;
  }
  .mc-tip.flip {
    transform: translateX(calc(-100% - 8px));
  }
  .mc-tip-t {
    color: var(--text-dim);
    margin-bottom: 4px;
    white-space: nowrap;
  }
  .mc-tip-row {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .mc-tip-row .lbl {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .mc-tip-row .val {
    font-variant-numeric: tabular-nums;
  }
  .mc-legend {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-wrap: wrap;
    gap: 2px 12px;
    font-size: 11px;
    color: var(--text);
  }
  .mc-legend li {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .sw {
    width: 9px;
    height: 9px;
    border-radius: 2px;
    flex-shrink: 0;
  }
  .mono {
    font-family: var(--font-mono, ui-monospace, monospace);
  }
</style>
