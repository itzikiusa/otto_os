<script lang="ts">
  // Model breakdown: tokens per (provider, model) over the window, plus a
  // per-day stacked chart by model. Tokens lead; cost is a secondary column.
  // Data comes straight from UsageSummary.models / daily_models.
  import Icon from '../../lib/components/Icon.svelte';
  import { formatCount } from '../../lib/metric-format';
  import type { ModelUsage, DailyModelUsage } from '../../lib/api/usage.svelte';

  let {
    models,
    dailyModels,
    days,
    onexport,
  }: {
    models: ModelUsage[];
    dailyModels: DailyModelUsage[];
    days: number;
    onexport?: () => void;
  } = $props();

  let provider = $state<string>('all');
  const providers = $derived([...new Set(models.map((m) => m.provider))].sort());
  // A filter left over from another window falls back to "all".
  const activeProvider = $derived(provider === 'all' || providers.includes(provider) ? provider : 'all');

  const shown = $derived(activeProvider === 'all' ? models : models.filter((m) => m.provider === activeProvider));
  const shownDaily = $derived(
    activeProvider === 'all' ? dailyModels : dailyModels.filter((d) => d.provider === activeProvider),
  );

  // Chart series: the 5 biggest models get their own colour, the rest fold
  // into "Other" (more than 6 categorical colours stop being distinguishable).
  const PALETTE = ['var(--cat-1)', 'var(--cat-2)', 'var(--cat-3)', 'var(--cat-4)', 'var(--cat-5)', 'var(--cat-6)'];
  const label = (m: { provider: string; model: string }) => `${m.provider} · ${m.model || 'unknown'}`;
  const series = $derived.by(() => {
    const top = [...shown].sort((a, b) => b.total_tokens - a.total_tokens).slice(0, 5).map(label);
    const out = top.map((name, i) => ({ name, color: PALETTE[i] }));
    if (shown.length > top.length) out.push({ name: 'Other', color: PALETTE[5] });
    return out;
  });

  type Day = { day: string; total: number; cost: number; parts: Map<string, number> };
  const byDay = $derived.by(() => {
    const names = new Set(series.map((s) => s.name));
    const map = new Map<string, Day>();
    for (const d of shownDaily) {
      let e = map.get(d.day);
      if (!e) {
        e = { day: d.day, total: 0, cost: 0, parts: new Map() };
        map.set(d.day, e);
      }
      const key = names.has(label(d)) ? label(d) : 'Other';
      e.parts.set(key, (e.parts.get(key) ?? 0) + d.total_tokens);
      e.total += d.total_tokens;
      e.cost += d.cost_usd;
    }
    return [...map.values()].sort((a, b) => a.day.localeCompare(b.day));
  });
  const maxDay = $derived(Math.max(1, ...byDay.map((d) => d.total)));

  const W = 500;
  const H = 120;
  const AXIS_L = 52;
  const AXIS_B = 22;
  const yTicks = $derived.by(() => {
    const raw = Math.max(maxDay / 3, 1);
    const mag = Math.pow(10, Math.floor(Math.log10(raw)));
    const nice = Math.ceil(raw / mag) * mag;
    return [0, nice, nice * 2, nice * 3];
  });
  const yMax = $derived(yTicks[yTicks.length - 1] || 1);
  const plotH = H - AXIS_B;
  function x(i: number, n: number): number {
    return AXIS_L + ((i + 0.5) / Math.max(1, n)) * (W - AXIS_L);
  }
  function showX(i: number, n: number): boolean {
    if (n <= 8) return true;
    const step = Math.ceil(n / 8);
    return i % step === 0 || i === n - 1;
  }
  const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
  function shortDay(iso: string): string {
    const m = iso.match(/^\d{4}-(\d{2})-(\d{2})/);
    return m ? `${MONTHS[parseInt(m[1], 10) - 1] ?? m[1]} ${parseInt(m[2], 10)}` : iso;
  }
  function fmtCost(n: number): string {
    if (n === 0) return '$0';
    if (n < 0.01) return '<$0.01';
    return '$' + n.toFixed(n < 100 ? 2 : 0);
  }
  function dayTitle(d: Day): string {
    const parts = series
      .map((s) => [s.name, d.parts.get(s.name) ?? 0] as const)
      .filter(([, v]) => v > 0)
      .map(([n, v]) => `${n} ${v.toLocaleString()}`);
    return `${shortDay(d.day)} · ${d.total.toLocaleString()} tokens (≈ ${fmtCost(d.cost)})\n${parts.join('\n')}`;
  }
</script>

<section class="panel card" aria-labelledby="usage-model-title" data-testid="usage-models">
  <div class="panel-head">
    <h3 id="usage-model-title">By model</h3>
    <div class="head-tools">
      {#if providers.length > 1}
        <div class="segmented" role="group" aria-label="Provider">
          <button aria-pressed={activeProvider === 'all'} class:active={activeProvider === 'all'} onclick={() => (provider = 'all')}>All</button>
          {#each providers as p (p)}
            <button aria-pressed={activeProvider === p} class:active={activeProvider === p} onclick={() => (provider = p)}>{p}</button>
          {/each}
        </div>
      {/if}
      {#if models.length > 0 && onexport}
        <button class="btn small ghost" onclick={onexport} title="Download models as CSV" aria-label="Download models as CSV">
          <Icon name="download" size={12} /> CSV
        </button>
      {/if}
    </div>
  </div>

  {#if shown.length === 0}
    <p class="dim small">No usage in this window. Each model appears here with its token counts as agents run.</p>
  {:else}
    {#if byDay.length > 0}
      <div class="legend" aria-hidden="true">
        {#each series as s (s.name)}
          <span class="lg"><i style="background: {s.color}"></i>{s.name}</span>
        {/each}
      </div>
      <svg class="model-svg" viewBox="0 0 {W} {H}" role="img" aria-label="Daily tokens by model over the last {days} days, peak {formatCount(maxDay)} tokens">
        {#each yTicks as t (t)}
          {@const y = plotH - (t / yMax) * plotH}
          <line class="grid-line" x1={AXIS_L} y1={y} x2={W} y2={y} />
          <text class="axis-label" x={AXIS_L - 4} y={y + 4} text-anchor="end">{formatCount(t)}</text>
        {/each}
        {#each byDay as d, i (d.day)}
          {@const cx = x(i, byDay.length)}
          {@const bw = Math.max(2, Math.min(40, ((W - AXIS_L) / Math.max(1, byDay.length)) * 0.7))}
          {@const barH = (d.total / yMax) * plotH}
          {@const stack = series.map((s) => ({ s, h: ((d.parts.get(s.name) ?? 0) / Math.max(1, d.total)) * barH }))}
          {#each stack as seg, si (seg.s.name)}
            {#if seg.h > 0}
              <rect
                x={cx - bw / 2}
                y={plotH - stack.slice(0, si + 1).reduce((a, b) => a + b.h, 0)}
                width={bw}
                height={seg.h}
                fill={seg.s.color}
                rx="1"
              />
            {/if}
          {/each}
          <rect class="bar-hit" x={cx - bw / 2} y="0" width={bw} height={plotH}><title>{dayTitle(d)}</title></rect>
          {#if showX(i, byDay.length)}
            <text class="axis-label" x={cx} y={H - 4} text-anchor="middle">{shortDay(d.day)}</text>
          {/if}
        {/each}
        <line class="axis-line" x1={AXIS_L} y1={plotH} x2={W} y2={plotH} />
      </svg>
    {/if}

    <div class="tbl-scroll">
      <table class="model-table">
        <thead>
          <tr>
            <th scope="col">Model</th>
            <th scope="col" class="num">Input</th>
            <th scope="col" class="num">Output</th>
            <th scope="col" class="num">Cache write</th>
            <th scope="col" class="num">Cache read</th>
            <th scope="col" class="num">Total</th>
            <th scope="col" class="num">Cost</th>
          </tr>
        </thead>
        <tbody>
          {#each shown as m (m.provider + '\u0000' + m.model)}
            <tr>
              <td>
                <span class="m-name" title={m.model}>{m.model || 'unknown'}</span>
                <span class="m-prov dim">{m.provider}</span>
              </td>
              <td class="num" title={m.input_tokens.toLocaleString()}>{formatCount(m.input_tokens)}</td>
              <td class="num" title={m.output_tokens.toLocaleString()}>{formatCount(m.output_tokens)}</td>
              <td class="num" title={m.cache_write_tokens.toLocaleString()}>{formatCount(m.cache_write_tokens)}</td>
              <td class="num" title={m.cache_read_tokens.toLocaleString()}>{formatCount(m.cache_read_tokens)}</td>
              <td class="num strong" title={m.total_tokens.toLocaleString()}>{formatCount(m.total_tokens)}</td>
              <td class="num dim">{fmtCost(m.cost_usd)}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {/if}
</section>

<style>
  .panel {
    padding: 14px 16px;
    min-width: 0;
  }
  .panel h3 {
    font-size: var(--fs-m);
    font-weight: 600;
    margin: 0;
    color: var(--text);
  }
  .panel-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    min-height: 24px;
    margin-bottom: 12px;
    gap: 6px 12px;
  }
  .head-tools {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
    min-width: 0;
  }
  .segmented {
    flex-wrap: wrap;
  }
  .legend {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 12px;
    margin-bottom: 6px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .lg {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    min-width: 0;
  }
  .lg i {
    width: 8px;
    height: 8px;
    border-radius: 2px;
    flex-shrink: 0;
  }
  .model-svg {
    width: 100%;
    height: 130px;
    display: block;
    overflow: visible;
    margin-bottom: 10px;
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
  .bar-hit {
    fill: transparent;
    cursor: crosshair;
  }
  .bar-hit:hover {
    fill: var(--hover);
  }
  .tbl-scroll {
    overflow-x: auto;
    max-width: 100%;
  }
  .model-table {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--fs-s);
    font-variant-numeric: tabular-nums;
  }
  .model-table th {
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--text-dim);
    text-align: start;
    padding: 4px 8px;
    border-bottom: 1px solid var(--separator);
    white-space: nowrap;
  }
  .model-table td {
    padding: 6px 8px;
    border-bottom: 1px solid var(--separator);
    white-space: nowrap;
  }
  .model-table .num {
    text-align: end;
  }
  .strong {
    font-weight: 600;
    color: var(--text);
  }
  .m-name {
    color: var(--text);
  }
  .m-prov {
    margin-inline-start: 6px;
    font-size: var(--fs-xs);
  }
  .dim {
    color: var(--text-dim);
  }
  .small {
    font-size: var(--fs-s);
  }
  p.small {
    margin: 0;
    line-height: 1.5;
  }
</style>
