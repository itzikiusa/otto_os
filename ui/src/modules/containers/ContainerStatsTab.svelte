<script lang="ts">
  // Container Stats tab: polls `GET /containers/{id}/stats` ONLY while this tab
  // is mounted and the document is visible via `pollWhileVisible` (never raw
  // setInterval), rendering CPU %, memory, network I/O, block I/O, PIDs, and
  // rolling CPU/memory sparklines.
  import { untrack } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import { containersApi } from '../../lib/api/containers';
  import { pollWhileVisible, type Poller } from '../../lib/poll';
  import { loadErrorText } from '../../lib/loadError';
  import { formatBytes, formatCount } from '../../lib/metric-format';
  import type { ContainerStatsResp } from '../../lib/api/types/containers';

  interface Props {
    containerId: string;
    containerName: string;
    running: boolean;
  }
  let { containerId, containerName, running }: Props = $props();

  const MAX_POINTS = 30;

  let latest = $state<ContainerStatsResp | null>(null);
  let cpuHistory = $state.raw<number[]>([]);
  let memHistory = $state.raw<number[]>([]);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let poller: Poller | null = null;

  function sparkPoints(values: readonly number[], maxVal: number): string {
    if (values.length === 0) return '';
    const w = 220;
    const h = 44;
    const ceil = Math.max(maxVal, 1);
    if (values.length === 1) {
      const y = h - Math.min(1, Math.max(0, values[0] / ceil)) * (h - 4) - 2;
      return `0,${y.toFixed(1)} ${w},${y.toFixed(1)}`;
    }
    const step = w / (values.length - 1);
    return values
      .map((v, i) => {
        const x = i * step;
        const y = h - Math.min(1, Math.max(0, v / ceil)) * (h - 4) - 2;
        return `${x.toFixed(1)},${y.toFixed(1)}`;
      })
      .join(' ');
  }

  $effect(() => {
    const id = containerId;
    const isRunning = running;
    untrack(() => {
      latest = null;
      cpuHistory = [];
      memHistory = [];
      error = null;
    });

    if (!isRunning) {
      loading = false;
      return;
    }

    loading = true;
    const p = pollWhileVisible(
      async (signal) => {
        try {
          const snap = await containersApi.stats(id, signal);
          if (signal.aborted) return;
          latest = snap;
          error = null;
          loading = false;
          cpuHistory = [...cpuHistory.slice(-(MAX_POINTS - 1)), snap.cpu_percent];
          memHistory = [...memHistory.slice(-(MAX_POINTS - 1)), snap.memory_percent];
          return true;
        } catch (e) {
          if (signal.aborted) return;
          error = loadErrorText(e);
          loading = false;
          return false;
        }
      },
      { ms: 2500, floorMs: 1500, maxBackoff: 6, hidden: 'pause' },
    );
    poller = p;

    return () => {
      p.stop();
      if (poller === p) poller = null;
    };
  });

  const cpuMax = $derived(Math.max(100, ...cpuHistory));
  const cpuLine = $derived(sparkPoints(cpuHistory, cpuMax));
  const memLine = $derived(sparkPoints(memHistory, 100));
</script>

<div class="ct-stats">
  {#if !running}
    <EmptyState
      icon="gauge"
      title="Container isn’t running"
      body={`Start ${containerName} to inspect live CPU, memory, network, and block I/O statistics.`}
    />
  {:else}
    <LoadState
      what="container stats"
      {loading}
      {error}
      empty={!latest}
      onretry={() => poller?.now()}
    >
      {#if latest}
        <div class="ct-stats-grid">
          <div class="ct-stat-card">
            <div class="ct-stat-head">
              <span class="ct-stat-label">CPU</span>
              <span class="ct-stat-val">{latest.cpu_percent.toFixed(1)}%</span>
            </div>
            <svg class="ct-spark" viewBox="0 0 220 44" preserveAspectRatio="none" aria-hidden="true">
              <polyline class="ct-spark-cpu" points={cpuLine} />
            </svg>
          </div>

          <div class="ct-stat-card">
            <div class="ct-stat-head">
              <span class="ct-stat-label">Memory</span>
              <span class="ct-stat-val">{latest.memory_percent.toFixed(1)}%</span>
            </div>
            <div class="ct-stat-sub">
              {formatBytes(latest.memory_usage_bytes)} / {formatBytes(latest.memory_limit_bytes)}
            </div>
            <svg class="ct-spark" viewBox="0 0 220 44" preserveAspectRatio="none" aria-hidden="true">
              <polyline class="ct-spark-mem" points={memLine} />
            </svg>
          </div>

          <div class="ct-stat-card">
            <div class="ct-stat-head">
              <span class="ct-stat-label">Network I/O</span>
              <Icon name="globe" size={13} />
            </div>
            <div class="ct-io-pair">
              <div>
                <span class="ct-io-k">RX</span>
                <span class="ct-io-v">{formatBytes(latest.net_rx_bytes)}</span>
              </div>
              <div>
                <span class="ct-io-k">TX</span>
                <span class="ct-io-v">{formatBytes(latest.net_tx_bytes)}</span>
              </div>
            </div>
          </div>

          <div class="ct-stat-card">
            <div class="ct-stat-head">
              <span class="ct-stat-label">Block I/O</span>
              <Icon name="db" size={13} />
            </div>
            <div class="ct-io-pair">
              <div>
                <span class="ct-io-k">Read</span>
                <span class="ct-io-v">{formatBytes(latest.block_read_bytes)}</span>
              </div>
              <div>
                <span class="ct-io-k">Write</span>
                <span class="ct-io-v">{formatBytes(latest.block_write_bytes)}</span>
              </div>
            </div>
          </div>

          <div class="ct-stat-card">
            <div class="ct-stat-head">
              <span class="ct-stat-label">Processes</span>
              <span class="ct-stat-val">{formatCount(latest.pids)}</span>
            </div>
            <div class="ct-stat-sub">Active PIDs inside container cgroup</div>
          </div>
        </div>
      {/if}
    </LoadState>
  {/if}
</div>

<style>
  .ct-stats {
    flex: 1;
    min-block-size: 0;
    overflow-y: auto;
    padding: 16px;
  }
  .ct-stats-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    gap: 12px;
  }
  .ct-stat-card {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 12px 14px;
    border-radius: var(--radius-m);
    border: 1px solid var(--border);
    background: var(--surface);
  }
  .ct-stat-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    color: var(--text-dim);
  }
  .ct-stat-label {
    font-size: var(--fs-xs);
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }
  .ct-stat-val {
    font-size: var(--fs-xl);
    font-weight: 600;
    color: var(--text);
    font-variant-numeric: tabular-nums;
  }
  .ct-stat-sub {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .ct-spark {
    inline-size: 100%;
    block-size: 44px;
    display: block;
  }
  .ct-spark polyline {
    fill: none;
    stroke-width: 1.75;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .ct-spark-cpu {
    stroke: var(--cat-1);
  }
  .ct-spark-mem {
    stroke: var(--cat-3);
  }
  .ct-io-pair {
    display: flex;
    gap: 16px;
    flex-wrap: wrap;
  }
  .ct-io-pair > div {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .ct-io-k {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .ct-io-v {
    font-size: var(--fs-m);
    font-weight: 500;
    color: var(--text);
    font-variant-numeric: tabular-nums;
  }
</style>
