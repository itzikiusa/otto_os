<script lang="ts">
  // The Usage page's one chart: spend per day (cost by default, tokens one tap
  // away). Hand-rolled SVG, no dependencies: labelled y ticks, thinned day
  // ticks, one keyboard-reachable hit target per bar (arrows / Home / End move,
  // chartKeys.ts) and a Day picker that says the selected day in words — the
  // same details on a phone, where there is no hover.
  import { formatCount, formatUsd } from '../../lib/metric-format';
  import type { DailyUsage } from '../../lib/api/usage.svelte';
  import { handleBarChartKey } from './chartKeys';
  import { breakdownTitle, shortDay, tokenSegs } from './tokenParts';
  import { usageUi } from './usageUiState.svelte';

  let { daily, days }: { daily: DailyUsage[]; days: number } = $props();

  const cost = $derived(usageUi.chartMetric === 'cost');
  const value = (d: DailyUsage): number => (cost ? d.cost_usd : d.total_tokens);
  const fmt = (v: number): string => (cost ? formatUsd(v) : formatCount(v));
  const peak = $derived(Math.max(0, ...daily.map(value)));
  const selected = $derived(daily.find((d) => d.day === usageUi.selectedDailyKey) ?? daily.at(-1));

  let width = $state(500);
  const SVG_W = $derived(Math.max(240, width || 500));
  const SVG_H = 120;
  const AXIS_L = 52;
  const AXIS_B = 22;
  const plotH = SVG_H - AXIS_B;
  // Four ticks from 0 to a "nice" ceiling (never below one unit, so a
  // near-empty window cannot repeat a label).
  const yTicks = $derived.by(() => {
    const raw = Math.max(peak / 3, cost ? 0.01 : 1);
    const mag = Math.pow(10, Math.floor(Math.log10(raw)));
    const nice = Math.ceil(raw / mag) * mag;
    return [0, nice, nice * 2, nice * 3];
  });
  const yMax = $derived(yTicks[3] || 1);
  // Bars sit at the CENTRE of equal bands, so the first and last never cross
  // the axis labels or the right edge.
  const x = (i: number, n: number): number => AXIS_L + ((i + 0.5) / Math.max(1, n)) * (SVG_W - AXIS_L);
  // Keep day labels at least 52 px apart.
  function showX(i: number, n: number): boolean {
    return i % Math.max(1, Math.ceil((52 * n) / Math.max(1, SVG_W - AXIS_L))) === 0;
  }
  const dayText = (d: DailyUsage): string => `${shortDay(d.day)} · ≈ ${formatUsd(d.cost_usd)} · ${d.total_tokens.toLocaleString()} tokens`;
</script>

<section class="chart" aria-labelledby="usage-chart-title">
  <div class="chart-head">
    <h3 id="usage-chart-title">{cost ? 'Cost per day' : 'Tokens per day'}</h3>
    <div class="segmented" role="group" aria-label="Chart shows">
      <button aria-pressed={cost} onclick={() => (usageUi.chartMetric = 'cost')}>Cost</button>
      <button aria-pressed={!cost} onclick={() => (usageUi.chartMetric = 'tokens')}>Tokens</button>
    </div>
  </div>
  {#if daily.length === 0 || peak === 0}
    <p class="dim small" data-testid="daily-cost-empty">Nothing recorded in this window.</p>
  {:else}
    {@const n = daily.length}
    {@const barW = Math.max(2, Math.min(40, ((SVG_W - AXIS_L) / Math.max(1, n)) * 0.7))}
    <div class="usage-chart" bind:clientWidth={width}>
      <svg class="daily-svg" viewBox="0 0 {SVG_W} {SVG_H}" role="group" aria-label="{cost ? 'Cost' : 'Tokens'} per day over the last {days} days, peak {fmt(peak)}">
        {#each yTicks as tick (tick)}
          {@const y = plotH - (tick / yMax) * plotH}
          <line class="grid-line" x1={AXIS_L} y1={y} x2={SVG_W} y2={y} />
          <text class="axis-label y-label" x={AXIS_L - 4} y={y + 4} text-anchor="end">{fmt(tick)}</text>
        {/each}
        {#each daily as d, i (d.day)}
          {@const cx = x(i, n)}
          {@const barH = (value(d) / yMax) * plotH}
          {#if cost}
            {#if barH > 0}<rect class="cost-bar" x={cx - barW / 2} y={plotH - barH} width={barW} height={barH} rx="1" />{/if}
          {:else}
            <!-- Top-to-bottom matches the token legend: input, cache write, cache read, output. -->
            {@const segs = tokenSegs(d)}
            {#each segs as s, si (s.label)}
              {#if s.pct > 0}
                {@const segH = (s.v / Math.max(1, d.total_tokens)) * barH}
                {@const off = segs.slice(0, si).reduce((a, p) => a + (p.v / Math.max(1, d.total_tokens)) * barH, 0)}
                <rect x={cx - barW / 2} y={plotH - barH + off} width={barW} height={segH} fill={s.color} rx="1" />
              {/if}
            {/each}
          {/if}
          <!-- Invisible hit target OVER the bar: hovering the bar shows this day. -->
          <rect class="bar-hit" data-daily-bar={d.day} role="button"
            tabindex={selected?.day === d.day ? 0 : -1}
            aria-label={dayText(d)}
            aria-pressed={selected?.day === d.day}
            onclick={() => (usageUi.selectedDailyKey = d.day)}
            onfocus={() => (usageUi.selectedDailyKey = d.day)}
            onkeydown={(event) => handleBarChartKey(event, daily.map((v) => v.day), i, 'data-daily-bar', (k) => (usageUi.selectedDailyKey = k))}
            x={cx - barW / 2} y={0} width={barW} height={plotH}>
            <title>{dayText(d)} · {breakdownTitle(d)}</title>
          </rect>
          {#if showX(i, n)}<text class="axis-label" x={cx} y={SVG_H - 4} text-anchor="middle">{shortDay(d.day)}</text>{/if}
        {/each}
        <line class="axis-line" x1={AXIS_L} y1={plotH} x2={SVG_W} y2={plotH} />
      </svg>
    </div>
    {#if selected}
      <div class="usage-chart-details" data-testid="usage-daily-details">
        <label>Day <select class="input" aria-label="Chart day" value={selected.day} onchange={(event) => (usageUi.selectedDailyKey = event.currentTarget.value)}>
          {#each daily as d (d.day)}<option value={d.day}>{d.day}</option>{/each}
        </select></label>
        <p role="status">{selected.day} · ≈ {formatUsd(selected.cost_usd)} · {selected.total_tokens.toLocaleString()} tokens · {breakdownTitle(selected)}</p>
      </div>
    {/if}
  {/if}
</section>

<style>
  .chart {
    display: flex;
    flex-direction: column;
    gap: var(--sp-4);
    min-inline-size: 0;
  }
  .chart-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-4);
  }
  h3 {
    margin: 0;
    font-size: var(--fs-m);
    font-weight: 600;
  }
  .usage-chart {
    min-inline-size: 0;
  }
  .daily-svg {
    inline-size: 100%;
    block-size: 130px;
    display: block;
    overflow: visible;
  }
  .cost-bar {
    fill: var(--accent-solid);
  }
  .grid-line {
    stroke: var(--border);
    stroke-width: 1;
    stroke-dasharray: 3 3;
  }
  .axis-line {
    stroke: var(--border);
    stroke-width: 1;
  }
  .axis-label {
    fill: var(--text-dim);
    font-size: var(--fs-xs);
    font-variant-numeric: tabular-nums;
  }
  .y-label {
    dominant-baseline: middle;
  }
  .bar-hit {
    fill: transparent;
    cursor: crosshair;
  }
  .bar-hit:hover,
  .bar-hit:focus-visible,
  .bar-hit[aria-pressed='true'] {
    stroke: var(--accent-text);
    stroke-width: 1;
    outline: none;
  }
  .usage-chart-details {
    font-size: var(--fs-s);
    overflow-wrap: anywhere;
    color: var(--text-dim);
  }
  .usage-chart-details label {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--sp-4);
  }
  .usage-chart-details select {
    max-inline-size: 100%;
  }
  .usage-chart-details p {
    margin-block: var(--sp-3) 0;
    line-height: 1.5;
  }
  .dim {
    color: var(--text-dim);
  }
  .small {
    font-size: var(--fs-s);
    margin: 0;
  }
  @media (pointer: coarse) {
    .usage-chart-details select,
    .segmented > button {
      min-block-size: 44px;
    }
  }
</style>
