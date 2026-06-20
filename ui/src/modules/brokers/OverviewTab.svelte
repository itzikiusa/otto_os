<script lang="ts">
  import { api } from '../../lib/api/client';
  import { toasts } from '../../lib/toast.svelte';
  import type { ClusterMetrics, ClusterOverview, Id } from '../../lib/api/types';

  interface Props {
    clusterId: Id;
  }
  let { clusterId }: Props = $props();

  let overview = $state<ClusterOverview | null>(null);
  let metrics = $state<ClusterMetrics | null>(null);
  let loading = $state(true);

  function fmtBytes(n: number | null): string {
    if (n === null || !isFinite(n)) return '—';
    const u = ['B', 'KB', 'MB', 'GB', 'TB'];
    let v = n;
    let i = 0;
    while (v >= 1024 && i < u.length - 1) {
      v /= 1024;
      i++;
    }
    return `${v.toFixed(v < 10 && i > 0 ? 1 : 0)} ${u[i]}`;
  }

  function fmtNum(n: number): string {
    return n.toLocaleString();
  }

  $effect(() => {
    const id = clusterId;
    loading = true;
    overview = null;
    metrics = null;
    let alive = true;

    void api
      .get<ClusterOverview>(`/brokers/clusters/${id}/overview`)
      .then((o) => {
        if (alive) overview = o;
      })
      .catch((e) => toasts.error('Overview failed', String(e)))
      .finally(() => {
        if (alive) loading = false;
      });

    const poll = () =>
      api
        .get<ClusterMetrics>(`/brokers/clusters/${id}/metrics`)
        .then((m) => {
          if (alive) metrics = m;
        })
        .catch(() => {});
    void poll();
    const timer = setInterval(() => void poll(), 4000);

    return () => {
      alive = false;
      clearInterval(timer);
    };
  });

  const peakRate = $derived(
    Math.max(1, ...(metrics?.throughput.map((p) => p.messages_per_sec) ?? [1])),
  );
</script>

<div class="overview">
  {#if loading && !overview}
    <p class="muted">Connecting to cluster…</p>
  {:else if overview}
    <div class="cards">
      <div class="card">
        <span class="k">Brokers</span><span class="v">{overview.brokers.length}</span>
      </div>
      <div class="card">
        <span class="k">Topics</span>
        <span class="v">{overview.topic_count - overview.internal_topic_count}</span>
        <span class="sub">+{overview.internal_topic_count} internal</span>
      </div>
      <div class="card">
        <span class="k">Partitions</span><span class="v">{fmtNum(overview.partition_count)}</span>
      </div>
      <div class="card">
        <span class="k">Consumer groups</span><span class="v">{overview.consumer_group_count}</span>
      </div>
      <div class="card">
        <span class="k">Throughput</span>
        <span class="v">{Math.round(metrics?.messages_per_sec ?? 0)}<small>/s</small></span>
        <span class="sub">{fmtNum(metrics?.total_messages ?? 0)} total</span>
      </div>
      {#if overview.under_replicated_partitions != null}
        <div class="card" class:warn-card={overview.under_replicated_partitions > 0}>
          <span class="k">Under-replicated</span>
          <span class="v" class:warn-v={overview.under_replicated_partitions > 0}>
            {overview.under_replicated_partitions}
          </span>
          <span class="sub">partition{overview.under_replicated_partitions === 1 ? '' : 's'}</span>
        </div>
      {/if}
      {#if overview.leadership_imbalance != null}
        <div class="card" class:warn-card={overview.leadership_imbalance > 0.2}>
          <span class="k">Leader skew</span>
          <span class="v" class:warn-v={overview.leadership_imbalance > 0.2}>
            {(overview.leadership_imbalance * 100).toFixed(0)}%
          </span>
          <span class="sub">CoV of leader counts</span>
        </div>
      {/if}
      {#if overview.cluster_id}
        <div class="card wide">
          <span class="k">Cluster ID</span><span class="mono">{overview.cluster_id}</span>
        </div>
      {/if}
    </div>

    <section>
      <h4>Throughput (messages/sec)</h4>
      {#if metrics && metrics.throughput.length > 1}
        <div class="spark">
          {#each metrics.throughput.slice(-60) as p (p.ts_ms)}
            <div
              class="bar"
              style="height: {Math.max(2, (p.messages_per_sec / peakRate) * 100)}%"
              title="{Math.round(p.messages_per_sec)}/s"
            ></div>
          {/each}
        </div>
      {:else}
        <p class="muted small">Sampling… the chart fills in as you watch.</p>
      {/if}
    </section>

    <section>
      <h4>Brokers</h4>
      <div class="brokers">
        {#each overview.brokers as b (b.id)}
          {@const bm = metrics?.brokers.find(
            (m) => m.instance === String(b.id) || m.instance === b.host,
          ) ?? metrics?.brokers[0]}
          <div class="broker">
            <div class="broker-head">
              <span class="mono">#{b.id}</span>
              <span class="host">{b.host}:{b.port}</span>
              <span class="leaders">{b.partition_leaders} leaders</span>
            </div>
            {#if metrics?.prometheus_available && bm}
              <div class="metric">
                <span class="ml">CPU</span>
                <div class="track">
                  <div
                    class="fill cpu"
                    style="width: {Math.min(100, bm.cpu_percent ?? 0)}%"
                  ></div>
                </div>
                <span class="mv">{bm.cpu_percent !== null ? `${bm.cpu_percent.toFixed(0)}%` : '—'}</span>
              </div>
              <div class="metric">
                <span class="ml">RAM</span>
                <div class="track">
                  <div
                    class="fill ram"
                    style="width: {bm.memory_total_bytes && bm.memory_used_bytes
                      ? Math.min(100, (bm.memory_used_bytes / bm.memory_total_bytes) * 100)
                      : 0}%"
                  ></div>
                </div>
                <span class="mv">
                  {fmtBytes(bm.memory_used_bytes)}{bm.memory_total_bytes
                    ? ` / ${fmtBytes(bm.memory_total_bytes)}`
                    : ''}
                </span>
              </div>
            {/if}
          </div>
        {/each}
      </div>
      {#if metrics && !metrics.prometheus_available}
        <p class="muted small">
          Set a Prometheus <em>Metrics URL</em> on the cluster (e.g. Redpanda <code
            >:9644/public_metrics</code
          > or a Kafka JMX exporter) to see per-broker CPU / RAM.
        </p>
      {/if}
    </section>
  {/if}
</div>

<style>
  .overview {
    padding: 16px;
    display: flex;
    flex-direction: column;
    gap: 22px;
    overflow: auto;
  }
  .cards {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
    gap: 10px;
  }
  .card {
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    padding: 12px 14px;
    display: flex;
    flex-direction: column;
    gap: 2px;
    background: var(--surface);
  }
  .card.wide {
    grid-column: 1 / -1;
  }
  .card.warn-card {
    border-color: color-mix(in srgb, #f5a623 50%, transparent);
  }
  .card .k {
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
  }
  .card .v {
    font-size: 26px;
    font-weight: 600;
  }
  .card .v.warn-v {
    color: #f5a623;
  }
  .card .v small {
    font-size: 13px;
    color: var(--text-dim);
    font-weight: 400;
  }
  .card .sub {
    font-size: 11px;
    color: var(--text-dim);
  }
  h4 {
    margin: 0 0 8px;
    font-size: 13px;
  }
  .spark {
    display: flex;
    align-items: flex-end;
    gap: 2px;
    height: 90px;
    padding: 6px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
  }
  .spark .bar {
    flex: 1;
    min-width: 2px;
    background: var(--accent);
    border-radius: 1px;
    opacity: 0.85;
  }
  .brokers {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
    gap: 10px;
  }
  .broker {
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    background: var(--surface);
  }
  .broker-head {
    display: flex;
    align-items: baseline;
    gap: 8px;
  }
  .broker-head .host {
    font-size: 13px;
  }
  .broker-head .leaders {
    margin-left: auto;
    font-size: 11px;
    color: var(--text-dim);
  }
  .metric {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .metric .ml {
    width: 30px;
    font-size: 11px;
    color: var(--text-dim);
  }
  .metric .track {
    flex: 1;
    height: 7px;
    border-radius: 4px;
    background: color-mix(in srgb, var(--text-dim) 16%, transparent);
    overflow: hidden;
  }
  .metric .fill {
    height: 100%;
  }
  .metric .fill.cpu {
    background: var(--status-working, #28c840);
  }
  .metric .fill.ram {
    background: var(--accent);
  }
  .metric .mv {
    font-size: 11px;
    color: var(--text-dim);
    min-width: 80px;
    text-align: right;
  }
  .mono {
    font-family: var(--font-mono);
    font-size: 12px;
  }
  .muted {
    color: var(--text-dim);
  }
  .muted.small,
  .small {
    font-size: 12px;
  }
  code {
    font-family: var(--font-mono);
  }
</style>
