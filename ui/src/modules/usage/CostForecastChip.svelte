<script lang="ts">
  // Pre-launch cost-forecast chip (B1): small inline widget that calls
  // POST /usage/forecast and shows a projected cost estimate before a run.
  // Usage: <CostForecastChip feature="review" provider="claude" />
  //        <CostForecastChip feature="agent" provider="claude" estTokens={4000} />
  import { untrack } from 'svelte';
  import { api } from '../../lib/api/client';
  import type { ForecastReq, ForecastResp } from './types';

  interface Props {
    /** Otto feature label ("review" | "product" | "agent" | "channel" | …). */
    feature: string;
    /** Provider name ("claude" | "codex" | "shell" | …). */
    provider: string;
    /**
     * Optional explicit token estimate — when supplied the forecast is priced
     * directly rather than from historical averages.
     */
    estTokens?: number;
  }

  let { feature, provider, estTokens }: Props = $props();

  let resp: ForecastResp | null = $state(null);
  let loading = $state(false);
  let expanded = $state(false);

  // Request token: a newer load supersedes any in-flight one, so a slow
  // response for stale inputs can never overwrite the current forecast.
  let seq = 0;

  async function load(f: string, p: string, est: number | undefined): Promise<void> {
    const mine = ++seq;
    loading = true;
    try {
      const req: ForecastReq = { feature: f, provider: p };
      if (est && est > 0) req.est_tokens = est;
      const r = await api.post<ForecastResp>('/usage/forecast', req);
      if (mine === seq) resp = r;
    } catch {
      if (mine === seq) resp = null;
    } finally {
      if (mine === seq) loading = false;
    }
  }

  // Reload whenever inputs change. Only the three props are dependencies:
  // `load` reads and writes `loading`/`resp`, so it runs untracked — a
  // tracked `loading` re-ran this effect on every `finally`, which fired
  // POST /usage/forecast back-to-back for as long as the chip was mounted.
  $effect(() => {
    const f = feature;
    const p = provider;
    const est = estTokens;
    untrack(() => void load(f, p, est));
  });

  function fmtCost(n: number): string {
    if (n === 0) return '$0';
    if (n < 0.001) return '<$0.001';
    if (n < 0.01) return '$' + n.toFixed(4);
    return '$' + n.toFixed(2);
  }
</script>

{#if loading}
  <span class="forecast-chip loading" title="Estimating run cost…">
    ≈ …
  </span>
{:else if resp && resp.projected_cost_usd > 0}
  <button
    class="forecast-chip ready"
    onclick={() => (expanded = !expanded)}
    title={resp.basis}
    aria-expanded={expanded}
  >
    ≈ {fmtCost(resp.projected_cost_usd)}
  </button>
  {#if expanded}
    <div class="forecast-tooltip" role="tooltip">
      <span class="forecast-label">Estimated cost</span>
      <span class="forecast-value">{fmtCost(resp.projected_cost_usd)}</span>
      <p class="forecast-basis">{resp.basis}</p>
    </div>
  {/if}
{:else if resp && resp.projected_cost_usd === 0}
  <span class="forecast-chip no-data" title={resp.basis}>
    ≈ no data
  </span>
{/if}

<style>
  .forecast-chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 11px;
    padding: 2px 7px;
    border-radius: 10px;
    font-family: var(--font-mono);
    vertical-align: middle;
    line-height: 1.4;
    cursor: default;
    user-select: none;
  }
  .forecast-chip.loading {
    background: var(--surface-3);
    color: var(--text-dim);
    border: 1px solid var(--border);
    animation: pulse 1.2s ease-in-out infinite;
  }
  .forecast-chip.ready {
    background: var(--surface-3);
    color: var(--accent-text);
    border: 1px solid var(--accent-soft);
    cursor: pointer;
    position: relative;
  }
  .forecast-chip.ready:hover {
    border-color: var(--accent);
    background: var(--accent-soft);
  }
  .forecast-chip.no-data {
    background: var(--surface-3);
    color: var(--text-dim);
    border: 1px dashed var(--border);
  }

  .forecast-tooltip {
    position: absolute;
    z-index: 100;
    margin-top: 4px;
    padding: 10px 12px;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 6px;
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.4);
    min-width: 200px;
    max-width: 320px;
  }
  .forecast-label {
    display: block;
    font-size: var(--fs-xs);
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-dim);
    margin-bottom: 2px;
  }
  .forecast-value {
    display: block;
    font-size: 18px;
    font-weight: 700;
    color: var(--text);
    font-family: var(--font-mono);
  }
  .forecast-basis {
    margin: 6px 0 0;
    font-size: 11px;
    color: var(--text-dim);
    line-height: 1.4;
  }

  @keyframes pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.4; }
  }
</style>
