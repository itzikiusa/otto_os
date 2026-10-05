<script lang="ts">
  // Opt-in cross-check against ccusage (root only). Nothing runs until the
  // user clicks: the daemon then runs `npx ccusage` locally with a timeout and
  // returns its numbers next to Otto's for the same dates, every session
  // (external included) on both sides.
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { formatCount } from '../../lib/metric-format';
  import { usage, type TokenTotals } from '../../lib/api/usage.svelte';

  const RANGES = [7, 14, 30];
  let days = $state(7);

  const check = $derived(usage.ccusage);

  function diff(ours: number, theirs: number): string {
    const d = ours - theirs;
    if (d === 0) return '0';
    return (d > 0 ? '+' : '−') + formatCount(Math.abs(d));
  }
  function pct(ours: number, theirs: number): string {
    if (theirs === 0) return '';
    const p = ((ours - theirs) / theirs) * 100;
    if (Math.abs(p) < 0.5) return 'match';
    return `${p > 0 ? '+' : '−'}${Math.abs(p).toFixed(0)}%`;
  }
  function fmtCost(n: number): string {
    if (n === 0) return '$0';
    if (n < 0.01) return '<$0.01';
    return '$' + n.toFixed(2);
  }
  const BUCKETS: { key: keyof TokenTotals; label: string }[] = [
    { key: 'input_tokens', label: 'Input' },
    { key: 'output_tokens', label: 'Output' },
    { key: 'cache_write_tokens', label: 'Cache write' },
    { key: 'cache_read_tokens', label: 'Cache read' },
    { key: 'total_tokens', label: 'Total' },
  ];
</script>

<section class="panel card" aria-labelledby="usage-ccusage-title" data-testid="usage-ccusage">
  <div class="panel-head">
    <h3 id="usage-ccusage-title">Compare with ccusage</h3>
    <div class="head-tools">
      <div class="segmented" role="group" aria-label="Days to compare">
        {#each RANGES as r (r)}
          <button aria-pressed={days === r} class:active={days === r} disabled={usage.ccusageRunning} onclick={() => (days = r)}>{r}d</button>
        {/each}
      </div>
      <button class="btn small" disabled={usage.ccusageRunning} onclick={() => usage.runCcusage(days)}>
        <Icon name="refresh" size={12} />
        {usage.ccusageRunning ? 'Running ccusage…' : check ? 'Run again' : 'Compare with ccusage'}
      </button>
    </div>
  </div>
  <p class="dim small intro">
    Runs <span class="mono">npx ccusage</span> on this Mac and puts its numbers next to Otto's for the same days, with
    every session counted on both sides. The first run may download ccusage through npx and take up to 2 minutes.
    Nothing is sent anywhere. ccusage also counts tools Otto doesn't track (for example Hermes), so those rows only
    appear on its side.
  </p>

  {#if usage.ccusageRunning}
    <LoadState what="the ccusage comparison" loading empty rows={3} />
  {:else if usage.ccusageError}
    <LoadState what="the ccusage comparison" error={usage.ccusageError} empty onretry={() => usage.runCcusage(days)} />
  {:else if check && !check.ran}
    <div class="cc-error" role="alert">
      <Icon name="warning" size={14} />
      <div>
        <div>ccusage didn't run: {check.error ?? 'no output'}</div>
        {#if check.command}<div class="dim small">Command: <span class="mono">{check.command}</span></div>{/if}
      </div>
    </div>
  {:else if check}
    <p class="dim small">
      {check.since} to {check.until} · ran in {(check.duration_ms / 1000).toFixed(1)} s ·
      <span class="mono" title={check.command}>{check.command}</span>
    </p>
    <div class="tbl-scroll">
      <table class="cc-table">
        <caption class="sr-only">Totals: Otto versus ccusage</caption>
        <thead>
          <tr><th scope="col">Tokens</th><th scope="col" class="num">Otto</th><th scope="col" class="num">ccusage</th><th scope="col" class="num">Difference</th></tr>
        </thead>
        <tbody>
          {#each BUCKETS as b (b.key)}
            <tr>
              <th scope="row">{b.label}</th>
              <td class="num">{formatCount(check.totals_ours[b.key])}</td>
              <td class="num">{formatCount(check.totals_theirs[b.key])}</td>
              <td class="num">{diff(check.totals_ours[b.key], check.totals_theirs[b.key])} <span class="dim">{pct(check.totals_ours[b.key], check.totals_theirs[b.key])}</span></td>
            </tr>
          {/each}
          <tr class="cost-row">
            <th scope="row">Cost (est.)</th>
            <td class="num dim">{fmtCost(check.totals_ours.cost_usd)}</td>
            <td class="num dim">{fmtCost(check.totals_theirs.cost_usd)}</td>
            <td class="num dim">{pct(check.totals_ours.cost_usd, check.totals_theirs.cost_usd)}</td>
          </tr>
        </tbody>
      </table>
    </div>

    <h4>By agent and model</h4>
    {#if check.rows.length === 0}
      <p class="dim small">Neither side recorded usage in this range.</p>
    {:else}
      <div class="tbl-scroll">
        <table class="cc-table">
          <thead>
            <tr>
              <th scope="col">Agent · model</th>
              <th scope="col" class="num">Otto total</th>
              <th scope="col" class="num">ccusage total</th>
              <th scope="col" class="num">Difference</th>
              <th scope="col" class="num">Output (Otto / ccusage)</th>
            </tr>
          </thead>
          <tbody>
            {#each check.rows as r (r.provider + '\u0000' + r.model)}
              <tr>
                <td><span class="m-name">{r.model || 'unknown'}</span> <span class="dim">{r.provider}</span></td>
                <td class="num">{formatCount(r.ours.total_tokens)}</td>
                <td class="num">{formatCount(r.theirs.total_tokens)}</td>
                <td class="num">{diff(r.ours.total_tokens, r.theirs.total_tokens)} <span class="dim">{pct(r.ours.total_tokens, r.theirs.total_tokens)}</span></td>
                <td class="num dim">{formatCount(r.ours.output_tokens)} / {formatCount(r.theirs.output_tokens)}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {/if}

    {#if check.daily.length > 0}
      <h4>By day</h4>
      <div class="tbl-scroll">
        <table class="cc-table">
          <thead>
            <tr><th scope="col">Date</th><th scope="col" class="num">Otto total</th><th scope="col" class="num">ccusage total</th><th scope="col" class="num">Difference</th></tr>
          </thead>
          <tbody>
            {#each check.daily as d (d.day)}
              <tr>
                <td>{d.day}</td>
                <td class="num">{formatCount(d.ours.total_tokens)}</td>
                <td class="num">{formatCount(d.theirs.total_tokens)}</td>
                <td class="num">{diff(d.ours.total_tokens, d.theirs.total_tokens)} <span class="dim">{pct(d.ours.total_tokens, d.theirs.total_tokens)}</span></td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {/if}
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
  h4 {
    font-size: var(--fs-s);
    font-weight: 600;
    margin: 14px 0 6px;
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
  }
  .intro {
    margin: -6px 0 10px;
  }
  .cc-error {
    display: flex;
    gap: 8px;
    align-items: flex-start;
    padding: 8px 10px;
    border-radius: var(--radius-s);
    background: var(--warning-soft);
    color: var(--text);
    font-size: var(--fs-s);
    overflow-wrap: anywhere;
  }
  .tbl-scroll {
    overflow-x: auto;
    max-width: 100%;
  }
  .cc-table {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--fs-s);
    font-variant-numeric: tabular-nums;
  }
  .cc-table th,
  .cc-table td {
    padding: 4px 8px;
    border-bottom: 1px solid var(--separator);
    text-align: start;
    white-space: nowrap;
  }
  .cc-table thead th {
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--text-dim);
  }
  .cc-table tbody th {
    font-weight: 500;
    color: var(--text);
  }
  .cc-table .num {
    text-align: end;
  }
  .m-name {
    color: var(--text);
  }
  .mono {
    font-family: var(--font-mono);
    overflow-wrap: anywhere;
  }
  .dim {
    color: var(--text-dim);
  }
  .small {
    font-size: var(--fs-s);
  }
  p.small {
    margin: 0 0 8px;
    line-height: 1.5;
  }
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
  }
</style>
