<script lang="ts">
  import { resourceAccess } from '../../lib/stores/resource-access.svelte';
  // CloudWatch metrics for ONE resource (an SQS queue, EC2 instance or RDS
  // instance): range picker (1h … 30d), refresh + auto-refresh every 60 s
  // while mounted, and a grid of MetricChart cards laid out per
  // `metrics-groups.ts`, each with a current / min / max / sum-or-avg stat
  // row. Errors follow the module's pattern: AccessDenied → "needs
  // cloudwatch:GetMetricData"; expired credentials → Sign in.
  import { untrack } from 'svelte';
  import { awsApi, isLoginRequired } from '../../lib/api/aws';
  import { ApiError, isAbortError } from '../../lib/api/client';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import MetricChart from '../../lib/components/MetricChart.svelte';
  import { formatMetric } from '../../lib/metric-format';
  import { buildCards } from './metrics-groups';
  import type { MetricsNamespace, MetricsRange, MetricsResp } from '../../lib/api/types';

  interface Props {
    accountId: string;
    namespace: MetricsNamespace;
    /** Queue name / instance id / DB identifier. */
    dimValue: string;
    region?: string;
    /** EC2 only — lets the daemon skip credit metrics on non-burstable types. */
    instanceType?: string | null;
    onsignin?: () => void;
  }
  let { accountId, namespace, dimValue, region, instanceType = null, onsignin }: Props = $props();

  const RANGES: { id: MetricsRange; label: string }[] = [
    { id: '1h', label: '1h' },
    { id: '6h', label: '6h' },
    { id: '24h', label: '24h' },
    { id: '7d', label: '7d' },
    { id: '30d', label: '30d' },
  ];
  const AUTO_MS = 60_000;

  let range = $state<MetricsRange>('1h');
  let resp = $state<MetricsResp | null>(null);
  let loading = $state(false);
  let error = $state('');
  let denied = $state(false);
  let updatedAt = $state<number | null>(null);
  let current: AbortController | null = null;

  async function load(): Promise<void> {
    await resourceAccess.load('aws_account', accountId);
    if (!resourceAccess.can('aws_account', accountId, 'metrics', 'aws', 'view')) {
      resp = null; denied = true; error = 'Metrics access is not granted for this account.'; return;
    }
    current?.abort();
    const ac = new AbortController();
    current = ac;
    loading = true;
    try {
      const r = await awsApi.metrics(accountId, namespace, dimValue, range, {
        region,
        instanceType,
        signal: ac.signal,
      });
      if (ac.signal.aborted) return;
      resp = r;
      error = '';
      denied = false;
      updatedAt = Date.now();
    } catch (e) {
      if (ac.signal.aborted || isAbortError(e)) return;
      denied = e instanceof ApiError && e.status === 403;
      error = e instanceof Error ? e.message : String(e);
    } finally {
      if (!ac.signal.aborted) loading = false;
    }
  }

  // Reload when the target or range changes; auto-refresh every 60 s while
  // mounted (the panel only exists while its tab is visible).
  $effect(() => {
    void accountId;
    void namespace;
    void dimValue;
    void region;
    void instanceType;
    void range;
    untrack(() => {
      resp = null;
      void load();
    });
    const t = setInterval(() => untrack(() => void load()), AUTO_MS);
    return () => {
      clearInterval(t);
      current?.abort();
    };
  });

  const cards = $derived(resp ? buildCards(resp) : []);
  const loginNeeded = $derived(isLoginRequired(new Error(error)));
  const periodLabel = $derived.by(() => {
    const p = resp?.period_seconds ?? 0;
    if (!p) return '';
    return p >= 3600 ? `${p / 3600} h` : p >= 60 ? `${p / 60} min` : `${p} s`;
  });

  function fmtUpdated(t: number | null): string {
    if (!t) return '';
    const d = new Date(t);
    const p = (n: number) => String(n).padStart(2, '0');
    return `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
  }
</script>

<div class="mp" data-testid="aws-metrics">
  <div class="mp-bar">
    <div class="ranges" role="radiogroup" aria-label="Time range">
      {#each RANGES as r (r.id)}
        <button
          role="radio"
          aria-checked={range === r.id}
          class:on={range === r.id}
          onclick={() => (range = r.id)}
        >{r.label}</button>
      {/each}
    </div>
    <span class="meta dim">
      {#if resp}period {periodLabel}{/if}
      {#if updatedAt} · updated {fmtUpdated(updatedAt)}{/if}
      · auto-refresh 60 s
    </span>
    <button class="icon-btn" class:spin={loading} onclick={() => void load()} disabled={loading} title="Refresh" aria-label="Refresh metrics">
      <Icon name="refresh" size={13} />
    </button>
  </div>

  {#if error && !resp}
    {#if denied}
      <EmptyState
        icon="lock"
        title="CloudWatch access denied"
        body={`This account's IAM identity needs cloudwatch:GetMetricData. ${error}`}
        actionLabel="Retry"
        onaction={() => void load()}
      />
    {:else}
      <EmptyState
        icon="chart"
        title="Couldn't load metrics"
        body={error}
        actionLabel={loginNeeded && onsignin ? 'Sign in' : 'Retry'}
        onaction={loginNeeded && onsignin ? onsignin : () => void load()}
      />
    {/if}
  {:else if !resp}
    <div class="pad"><Skeleton rows={4} height={120} /></div>
  {:else}
    {#if error}
      <p class="stale">Showing the last successful load — refresh failed: {error}</p>
    {/if}
    <div class="grid">
      {#each cards as c (c.group.id)}
        <section class="card">
          <h4>{c.group.title}</h4>
          <MetricChart series={c.chart} unit={c.series[0].unit} height={150} />
          <table class="stats">
            <thead>
              <tr>
                <th></th><th>Current</th><th>Min</th><th>Max</th><th>{c.series[0].stat === 'Sum' ? 'Sum' : 'Avg'}</th>
              </tr>
            </thead>
            <tbody>
              {#each c.series as s (s.id)}
                <tr>
                  <td class="lbl" title={`${s.metric} (${s.stat})`}>{s.label}</td>
                  <td class="mono">{formatMetric(s.current, s.unit)}</td>
                  <td class="mono">{formatMetric(s.min, s.unit)}</td>
                  <td class="mono">{formatMetric(s.max, s.unit)}</td>
                  <td class="mono">{formatMetric(s.stat === 'Sum' ? s.sum : s.avg, s.unit)}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        </section>
      {/each}
    </div>
  {/if}
</div>

<style>
  .mp {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 10px 12px 14px;
    min-width: 0;
  }
  .mp-bar {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
    font-size: 12px;
  }
  .ranges {
    display: inline-flex;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    overflow: hidden;
  }
  .ranges button {
    border: 0;
    background: transparent;
    color: var(--text-dim);
    padding: 4px 9px;
    font: inherit;
    font-size: 12px;
    cursor: pointer;
  }
  .ranges button + button {
    border-left: 1px solid var(--border);
  }
  .ranges button.on {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    color: var(--text);
    font-weight: 600;
  }
  .meta {
    font-size: 11.5px;
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .dim {
    color: var(--text-dim);
  }
  .icon-btn {
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
    color: var(--text);
    cursor: pointer;
  }
  .icon-btn:disabled {
    opacity: 0.6;
    cursor: default;
  }
  .icon-btn.spin :global(svg) {
    animation: spin 0.9s linear infinite;
  }
  .pad {
    padding: 4px 0;
  }
  .stale {
    margin: 0;
    font-size: 11.5px;
    color: var(--status-warn);
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(min(100%, 300px), 1fr));
    gap: 10px;
  }
  .card {
    min-width: 0;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    padding: 8px 10px 6px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  h4 {
    margin: 0;
    font-size: 12px;
    font-weight: 600;
  }
  .stats {
    width: 100%;
    border-collapse: collapse;
    font-size: 11px;
    table-layout: fixed;
  }
  .stats th {
    text-align: right;
    font-weight: 500;
    color: var(--text-dim);
    padding: 2px 4px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    font-size: 9.5px;
  }
  .stats th:first-child {
    width: 34%;
  }
  .stats td {
    text-align: right;
    padding: 2px 4px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    font-variant-numeric: tabular-nums;
  }
  .stats td.lbl {
    text-align: left;
    color: var(--text-dim);
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
