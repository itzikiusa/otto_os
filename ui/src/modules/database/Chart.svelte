<script lang="ts">
  // Hand-rolled inline-SVG charts (no chart dependency). Maps a QueryResult onto
  // a viz: number (first cell), table (mini grid), line/bar/area/pie. `mapping.x`
  // picks the label column; `mapping.y[]` pick the numeric series.
  import ResultsGrid from './ResultsGrid.svelte';
  import type { DbViz, DbWidgetMapping, QueryResult } from '../../lib/api/types';

  interface Props {
    result: QueryResult | null;
    viz: DbViz;
    mapping?: DbWidgetMapping;
  }
  let { result, viz, mapping = {} }: Props = $props();

  // A palette derived from the accent + complementary hues for multi-series.
  const COLORS = ['var(--accent)', '#28c840', '#d2691e', '#bf5af2', '#0e8a8a', '#ff5f57', '#febc2e'];

  function colIndex(name: string | undefined): number {
    if (!result || !name) return -1;
    return result.columns.findIndex((c) => c.name === name);
  }

  function toNum(v: unknown): number {
    if (typeof v === 'number') return v;
    if (typeof v === 'string') {
      const n = Number(v);
      return Number.isFinite(n) ? n : NaN;
    }
    return NaN;
  }

  // Resolve the x (label) column and y (series) columns, with sensible defaults:
  // first non-numeric column = x; remaining numeric columns = y series.
  const resolved = $derived.by(() => {
    if (!result || result.columns.length === 0) return null;
    let xIdx = colIndex(mapping.x);
    let yIdxs = (mapping.y ?? []).map(colIndex).filter((i) => i >= 0);

    if (xIdx < 0 || yIdxs.length === 0) {
      // Guess: first column with mostly non-numeric values is the label.
      const sample = result.rows[0] ?? [];
      if (xIdx < 0) {
        xIdx = result.columns.findIndex((_c, i) => Number.isNaN(toNum(sample[i])));
        if (xIdx < 0) xIdx = 0;
      }
      if (yIdxs.length === 0) {
        yIdxs = result.columns
          .map((_c, i) => i)
          .filter((i) => i !== xIdx && !Number.isNaN(toNum(sample[i])));
        if (yIdxs.length === 0) yIdxs = result.columns.map((_c, i) => i).filter((i) => i !== xIdx).slice(0, 1);
      }
    }
    const labels = result.rows.map((r) => String(r[xIdx] ?? ''));
    const series = yIdxs.map((yi) => ({
      name: result.columns[yi]?.name ?? `s${yi}`,
      values: result.rows.map((r) => toNum(r[yi])),
    }));
    return { labels, series };
  });

  // ── Number ─────────────────────────────────────────────────────────────────
  const bigNumber = $derived.by(() => {
    if (!result || result.rows.length === 0 || result.columns.length === 0) return null;
    const vi = mapping.value ? colIndex(mapping.value) : result.columns.length - 1;
    const idx = vi >= 0 ? vi : 0;
    const raw = result.rows[0][idx];
    return { value: raw, label: result.columns[idx]?.name ?? '' };
  });

  function fmtNum(v: unknown): string {
    const n = toNum(v);
    if (Number.isNaN(n)) return String(v ?? '');
    return n.toLocaleString(undefined, { maximumFractionDigits: 2 });
  }

  // ── Geometry helpers (viewBox 0..W x 0..H) ─────────────────────────────────
  const W = 300;
  const H = 160;
  const PAD = { l: 4, r: 4, t: 8, b: 18 };

  // Real value domain across all series — negatives supported, NaN ignored.
  // Always spans 0 so bars keep a baseline; a flat/empty domain widens by 1 so
  // the scale never divides by zero.
  const domain = $derived.by(() => {
    let lo = 0;
    let hi = 0;
    if (resolved) {
      for (const s of resolved.series) {
        for (const v of s.values) {
          if (Number.isNaN(v)) continue;
          if (v < lo) lo = v;
          if (v > hi) hi = v;
        }
      }
    }
    if (hi === lo) hi = lo + 1;
    return { lo, hi };
  });

  function x(i: number, n: number): number {
    const span = W - PAD.l - PAD.r;
    return n <= 1 ? PAD.l + span / 2 : PAD.l + (span * i) / (n - 1);
  }
  function y(v: number): number {
    const span = H - PAD.t - PAD.b;
    const { lo, hi } = domain;
    const val = Number.isNaN(v) ? 0 : v;
    return PAD.t + span - (span * (val - lo)) / (hi - lo);
  }
  /** y of the zero line — the baseline; bars grow from here in both directions. */
  const y0 = $derived(y(0));
  function linePath(values: number[]): string {
    return values.map((v, i) => `${i === 0 ? 'M' : 'L'} ${x(i, values.length).toFixed(1)} ${y(v).toFixed(1)}`).join(' ');
  }
  function areaPath(values: number[]): string {
    const top = linePath(values);
    const x0 = x(0, values.length).toFixed(1);
    const xn = x(values.length - 1, values.length).toFixed(1);
    const yb = y0.toFixed(1);
    return `${top} L ${xn} ${yb} L ${x0} ${yb} Z`;
  }

  // ── Pie ────────────────────────────────────────────────────────────────────
  const pieSlices = $derived.by(() => {
    if (!resolved || resolved.series.length === 0) return [];
    // NaN and negative values can't be pie fractions — treat both as 0 (a
    // negative slice would rewind `acc` and corrupt every arc after it).
    const vals = resolved.series[0].values.map((v) => (Number.isNaN(v) || v < 0 ? 0 : v));
    const total = vals.reduce((a, b) => a + b, 0) || 1;
    let acc = 0;
    const cx = 80;
    const cy = 80;
    const r = 70;
    return vals.map((v, i) => {
      const frac = v / total;
      const a0 = acc * 2 * Math.PI - Math.PI / 2;
      acc += frac;
      const a1 = acc * 2 * Math.PI - Math.PI / 2;
      const large = frac > 0.5 ? 1 : 0;
      const p0 = [cx + r * Math.cos(a0), cy + r * Math.sin(a0)];
      const p1 = [cx + r * Math.cos(a1), cy + r * Math.sin(a1)];
      // A slice covering (almost) the whole pie degenerates: start ≈ end, so the
      // arc collapses to nothing. Draw a full circle (two half-arcs) instead.
      const d =
        frac >= 0.9995
          ? `M ${cx} ${cy - r} A ${r} ${r} 0 1 1 ${cx} ${cy + r} A ${r} ${r} 0 1 1 ${cx} ${cy - r} Z`
          : `M ${cx} ${cy} L ${p0[0].toFixed(1)} ${p0[1].toFixed(1)} A ${r} ${r} 0 ${large} 1 ${p1[0].toFixed(1)} ${p1[1].toFixed(1)} Z`;
      return {
        d,
        color: COLORS[i % COLORS.length],
        label: resolved.labels[i],
        pct: Math.round(frac * 100),
      };
    });
  });
</script>

{#if viz === 'number'}
  <div class="num">
    {#if bigNumber}
      <span class="num-value">{fmtNum(bigNumber.value)}</span>
      <span class="num-label">{bigNumber.label}</span>
    {:else}
      <span class="num-empty">—</span>
    {/if}
  </div>
{:else if viz === 'table'}
  <ResultsGrid {result} mini={true} />
{:else if !resolved || resolved.series.length === 0 || resolved.labels.length === 0}
  <div class="chart-empty">No chartable data</div>
{:else if viz === 'pie'}
  <div class="pie-wrap">
    <svg viewBox="0 0 160 160" class="pie-svg" preserveAspectRatio="xMidYMid meet">
      {#each pieSlices as s, i (i)}
        <path d={s.d} fill={s.color} stroke="var(--surface)" stroke-width="1" />
      {/each}
    </svg>
    <ul class="legend">
      {#each pieSlices as s, i (i)}
        <li><span class="sw" style="background:{s.color}"></span>{s.label} <span class="dim">{s.pct}%</span></li>
      {/each}
    </ul>
  </div>
{:else}
  <div class="svg-wrap">
    <svg viewBox="0 0 {W} {H}" class="chart-svg" preserveAspectRatio="none">
      <!-- baseline (the zero line — sits mid-chart when values go negative) -->
      <line x1={PAD.l} y1={y0} x2={W - PAD.r} y2={y0} stroke="var(--border)" stroke-width="0.75" />
      {#if viz === 'bar'}
        {@const n = resolved.labels.length}
        {@const groupW = (W - PAD.l - PAD.r) / Math.max(n, 1)}
        {@const barW = (groupW * 0.74) / resolved.series.length}
        {#each resolved.series as s, si (si)}
          {#each s.values as v, i (i)}
            {@const vy = Number.isNaN(v) ? y0 : y(v)}
            <rect
              x={(PAD.l + i * groupW + groupW * 0.13 + si * barW).toFixed(1)}
              y={Math.min(vy, y0).toFixed(1)}
              width={Math.max(barW - 1, 1).toFixed(1)}
              height={Math.abs(y0 - vy).toFixed(1)}
              fill={COLORS[si % COLORS.length]}
              rx="1"
            />
          {/each}
        {/each}
      {:else if viz === 'area'}
        {#each resolved.series as s, si (si)}
          <path d={areaPath(s.values)} fill={COLORS[si % COLORS.length]} fill-opacity="0.18" stroke="none" />
          <path d={linePath(s.values)} fill="none" stroke={COLORS[si % COLORS.length]} stroke-width="1.5" vector-effect="non-scaling-stroke" />
        {/each}
      {:else}
        <!-- line -->
        {#each resolved.series as s, si (si)}
          <path d={linePath(s.values)} fill="none" stroke={COLORS[si % COLORS.length]} stroke-width="1.5" vector-effect="non-scaling-stroke" />
        {/each}
      {/if}
    </svg>
    {#if resolved.series.length > 1}
      <ul class="legend inline">
        {#each resolved.series as s, si (si)}
          <li><span class="sw" style="background:{COLORS[si % COLORS.length]}"></span>{s.name}</li>
        {/each}
      </ul>
    {/if}
  </div>
{/if}

<style>
  .num {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    height: 100%;
    gap: 4px;
  }
  .num-value {
    font-size: 34px;
    font-weight: 700;
    letter-spacing: -0.02em;
    color: var(--text);
    font-variant-numeric: tabular-nums;
    line-height: 1;
  }
  .num-label {
    font-size: 11px;
    color: var(--text-dim);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .num-empty {
    font-size: 28px;
    color: var(--text-dim);
  }
  .chart-empty {
    display: grid;
    place-items: center;
    height: 100%;
    color: var(--text-dim);
    font-size: 11.5px;
  }
  .svg-wrap {
    width: 100%;
    height: 100%;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .chart-svg {
    width: 100%;
    flex: 1;
    min-height: 0;
  }
  .pie-wrap {
    display: flex;
    align-items: center;
    gap: 14px;
    height: 100%;
    padding: 6px;
  }
  .pie-svg {
    width: 130px;
    height: 130px;
    flex-shrink: 0;
  }
  .legend {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 11px;
    color: var(--text);
    overflow: auto;
    max-height: 100%;
  }
  .legend.inline {
    flex-direction: row;
    flex-wrap: wrap;
    gap: 4px 12px;
    padding-top: 6px;
  }
  .legend li {
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
  .dim {
    color: var(--text-dim);
  }
</style>
