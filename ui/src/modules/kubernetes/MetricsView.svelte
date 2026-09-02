<script lang="ts">
  // Drawer "Metrics" tab: per-container CPU / memory bars for one pod from
  // `GET …/metrics?ns=` (metrics-server via `kubectl top`). Bars are relative
  // to the pod total (requests/limits aren't in the payload); refreshed every
  // 10 s while the tab is open.
  import { untrack } from 'svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import { k8sApi } from '../../lib/api/k8s';
  import type { K8sPodMetrics } from '../../lib/api/types';
  import { formatBytes, formatMillicores } from './k8s-util';

  interface Props {
    clusterId: string;
    ns: string;
    pod: string;
  }
  let { clusterId, ns, pod }: Props = $props();

  let metrics = $state<K8sPodMetrics | null>(null);
  let available = $state(true);
  let loading = $state(true);
  let error = $state('');

  async function load(): Promise<void> {
    try {
      const r = await k8sApi.metrics(clusterId, ns);
      available = r.available;
      metrics = r.pods.find((p) => p.name === pod && p.namespace === ns) ?? null;
      error = '';
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    // Re-arm when the pod changes; the fetches themselves are untracked.
    void clusterId;
    void ns;
    void pod;
    loading = true;
    untrack(() => void load());
    const t = setInterval(() => untrack(() => void load()), 10_000);
    return () => clearInterval(t);
  });

  const maxCpu = $derived(Math.max(1, ...(metrics?.containers.map((c) => c.cpu_millicores) ?? [1])));
  const maxMem = $derived(Math.max(1, ...(metrics?.containers.map((c) => c.mem_bytes) ?? [1])));
</script>

<div class="metrics">
  {#if loading && !metrics}
    <Skeleton rows={3} height={40} />
  {:else if error}
    <div class="err">{error}</div>
  {:else if !available}
    <div class="dim">metrics-server isn't installed in this cluster, so <span class="mono">kubectl top</span> has nothing to report.</div>
  {:else if !metrics}
    <div class="dim">No metrics for this pod yet (new pods take a minute to show up in metrics-server).</div>
  {:else}
    <div class="totals">
      <div class="tot"><span class="lbl">CPU</span><span class="val mono">{formatMillicores(metrics.cpu_millicores)}</span></div>
      <div class="tot"><span class="lbl">Memory</span><span class="val mono">{formatBytes(metrics.mem_bytes)}</span></div>
    </div>
    <div class="containers">
      {#each metrics.containers as c (c.name)}
        <div class="ctr">
          <div class="ctr-name mono">{c.name}</div>
          <div class="bar-row">
            <span class="lbl">cpu</span>
            <div class="bar" role="meter" aria-label="{c.name} CPU" aria-valuemin={0} aria-valuemax={maxCpu} aria-valuenow={c.cpu_millicores}>
              <div class="fill cpu" style="width:{(100 * c.cpu_millicores) / maxCpu}%"></div>
            </div>
            <span class="val mono">{formatMillicores(c.cpu_millicores)}</span>
          </div>
          <div class="bar-row">
            <span class="lbl">mem</span>
            <div class="bar" role="meter" aria-label="{c.name} memory" aria-valuemin={0} aria-valuemax={maxMem} aria-valuenow={c.mem_bytes}>
              <div class="fill mem" style="width:{(100 * c.mem_bytes) / maxMem}%"></div>
            </div>
            <span class="val mono">{formatBytes(c.mem_bytes)}</span>
          </div>
        </div>
      {/each}
    </div>
    <div class="dim small">Bars are relative to the busiest container in this pod. Refreshes every 10 s.</div>
  {/if}
</div>

<style>
  .metrics {
    padding: 14px 16px;
    display: flex;
    flex-direction: column;
    gap: 14px;
    font-size: 12.5px;
  }
  .totals {
    display: flex;
    gap: 24px;
  }
  .tot {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .tot .val {
    font-size: 16px;
    font-weight: 600;
  }
  .lbl {
    font-size: 10.5px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-dim);
    width: 32px;
  }
  .containers {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .ctr-name {
    font-size: 12px;
    margin-bottom: 4px;
  }
  .bar-row {
    display: grid;
    grid-template-columns: 32px 1fr 90px;
    gap: 8px;
    align-items: center;
    margin-bottom: 3px;
  }
  .bar {
    height: 8px;
    border-radius: 999px;
    background: var(--surface-2);
    overflow: hidden;
  }
  .fill {
    height: 100%;
    border-radius: 999px;
    transition: width 300ms ease-out;
  }
  .fill.cpu {
    background: var(--accent);
  }
  .fill.mem {
    background: var(--status-working);
  }
  .val {
    text-align: right;
    font-size: 12px;
  }
  .dim {
    color: var(--text-dim);
    line-height: 1.5;
  }
  .small {
    font-size: 11px;
  }
  .err {
    color: var(--status-exited);
  }
  .mono {
    font-family: var(--font-mono);
  }
</style>
