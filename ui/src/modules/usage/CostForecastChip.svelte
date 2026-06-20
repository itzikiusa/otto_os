<script lang="ts">
  // Pre-launch cost-forecast chip (B1): small inline widget that calls
  // POST /usage/forecast and shows a projected cost estimate before a run.
  // Usage: <CostForecastChip feature="review" provider="claude" />
  //        <CostForecastChip feature="agent" provider="claude" estTokens={4000} />
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

  async function load(): Promise<void> {
    if (loading) return;
    loading = true;
    try {
      const req: ForecastReq = { feature, provider };
      if (estTokens && estTokens > 0) req.est_tokens = estTokens;
      resp = await api.post<ForecastResp>('/usage/forecast', req);
    } catch {
      resp = null;
    } finally {
      loading = false;
    }
  }

  // Reload whenever inputs change.
  $effect(() => {
    void load();
    // eslint-disable-next-line @typescript-eslint/no-unused-expressions
    feature; provider; estTokens;
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
    font-family: var(--font-mono, monospace);
    vertical-align: middle;
    line-height: 1.4;
    cursor: default;
    user-select: none;
  }
  .forecast-chip.loading {
    background: var(--surface-3, #0d1117);
    color: var(--fg-muted, #8b949e);
    border: 1px solid var(--border-subtle, #21262d);
    animation: pulse 1.2s ease-in-out infinite;
  }
  .forecast-chip.ready {
    background: var(--surface-3, #0d1117);
    color: var(--accent, #388bfd);
    border: 1px solid var(--accent-subtle, #1f3a5f);
    cursor: pointer;
    position: relative;
  }
  .forecast-chip.ready:hover {
    border-color: var(--accent, #388bfd);
    background: var(--accent-muted, #0d2136);
  }
  .forecast-chip.no-data {
    background: var(--surface-3, #0d1117);
    color: var(--fg-muted, #8b949e);
    border: 1px dashed var(--border, #30363d);
  }

  .forecast-tooltip {
    position: absolute;
    z-index: 100;
    margin-top: 4px;
    padding: 10px 12px;
    background: var(--surface-2, #161b22);
    border: 1px solid var(--border, #30363d);
    border-radius: 6px;
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.4);
    min-width: 200px;
    max-width: 320px;
  }
  .forecast-label {
    display: block;
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--fg-muted, #8b949e);
    margin-bottom: 2px;
  }
  .forecast-value {
    display: block;
    font-size: 18px;
    font-weight: 700;
    color: var(--fg, #e6edf3);
    font-family: var(--font-mono, monospace);
  }
  .forecast-basis {
    margin: 6px 0 0;
    font-size: 11px;
    color: var(--fg-muted, #8b949e);
    line-height: 1.4;
  }

  @keyframes pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.4; }
  }
</style>
