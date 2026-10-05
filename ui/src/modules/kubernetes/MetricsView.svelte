<script lang="ts">
  // Drawer "Metrics" tab: per-container CPU / memory bars for one pod from
  // `GET …/metrics?ns=` (metrics-server via `kubectl top`). Bars are relative
  // to the pod total (requests/limits aren't in the payload); refreshed every
  // 10 s while the tab is open. Below the snapshot, "History (Monitor)" reads
  // this pod's memory / request-rate series from the Monitor (K-2) and links
  // to the workload's row there.
  import { untrack } from 'svelte';
  import { router } from '../../lib/router.svelte';
  import { k8s } from '../../lib/stores/k8s.svelte';
  import Sparkline from './monitor/Sparkline.svelte';
  import { monitorPath } from './viewState';
  import { pollWhileVisible } from '../../lib/poll';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { k8sApi } from '../../lib/api/k8s';
  import type { K8sMonitorSeries, K8sPodMetrics } from '../../lib/api/types';
  import { formatBytes, formatMillicores } from './k8s-util';

  interface Props {
    clusterId: string;
    ns: string;
    pod: string;
    /** The pod's owning workload (from its ownerReferences), when known. */
    workload?: string;
  }
  let { clusterId, ns, pod, workload = '' }: Props = $props();

  // --- History (Monitor) ------------------------------------------------------
  let history = $state<{ mem: K8sMonitorSeries | null; rps: K8sMonitorSeries | null } | null>(null);
  let historyError = $state('');
  let historyLoading = $state(true);
  let historySeq = 0;
  async function loadHistory(): Promise<void> {
    const seq = ++historySeq;
    historyLoading = true;
    try {
      const [ws, sys, rps] = await Promise.all([
        k8sApi.monitorSeries(clusterId, { metric: 'mem_working_set_bytes', pod, window: '1h' }),
        k8sApi.monitorSeries(clusterId, { metric: 'mem_sys_bytes', pod, window: '1h' }),
        k8sApi.monitorSeries(clusterId, { metric: 'http_requests_total', pod, window: '1h' }),
      ]);
      if (seq !== historySeq) return;
      history = { mem: ws.points.length ? ws : sys.points.length ? sys : null, rps: rps.points.length ? rps : null };
      historyError = '';
    } catch (e) {
      if (seq !== historySeq) return;
      historyError = e instanceof Error ? e.message : String(e);
    } finally {
      if (seq === historySeq) historyLoading = false;
    }
  }
  $effect(() => {
    void clusterId;
    void pod;
    untrack(() => void loadHistory());
    return () => {
      historySeq++;
    };
  });
  function openInMonitor(): void {
    if (workload) k8s.saveMonitorUi(clusterId, { expanded: `${ns}/${workload}`, filter: workload });
    router.go(monitorPath(clusterId));
  }

  let metrics = $state<K8sPodMetrics | null>(null);
  let available = $state(true);
  let loading = $state(true);
  let error = $state('');

  // perf K8s R5: ask for THIS pod only (`?pod=`), abort with the poller and
  // drop an answer that belongs to a pod the drawer has since left.
  let metricsSeq = 0;
  let poller: { now(): void } | null = null;
  async function load(signal?: AbortSignal): Promise<void> {
    const seq = ++metricsSeq;
    const [c, n, p] = [clusterId, ns, pod];
    try {
      const r = await k8sApi.metrics(c, n, signal, p);
      if (seq !== metricsSeq) return;
      available = r.available;
      metrics = r.pods.find((m) => m.name === p && m.namespace === n) ?? null;
      error = '';
    } catch (e) {
      if (seq !== metricsSeq || signal?.aborted) return;
      error = loadErrorText(e);
    } finally {
      if (seq === metricsSeq) loading = false;
    }
  }

  $effect(() => {
    // Re-arm when the pod changes; the fetches themselves are untracked.
    void clusterId;
    void ns;
    void pod;
    loading = true;
    metrics = null;
    const t = untrack(() => pollWhileVisible((signal) => load(signal), { ms: 10_000 }));
    poller = t;
    return () => {
      t.stop();
      metricsSeq++;
      poller = null;
    };
  });

  const maxCpu = $derived(Math.max(1, ...(metrics?.containers.map((c) => c.cpu_millicores) ?? [1])));
  const maxMem = $derived(Math.max(1, ...(metrics?.containers.map((c) => c.mem_bytes) ?? [1])));
</script>

<div class="metrics">
  <!-- A failed 10 s poll keeps the last good bars (stale bar + Retry); the full
       error shows only when there is nothing to draw. -->
  <LoadState what="metrics" {loading} {error} empty={!metrics} rows={3} onretry={() => (poller ? void poller.now() : void load())}>
    {#snippet emptyView()}
      {#if !available}
        <div class="dim">metrics-server isn’t installed in this cluster, so <span class="mono">kubectl top</span> has nothing to report.</div>
      {:else}
        <div class="dim">No metrics for this pod yet (new pods take a minute to show up in metrics-server).</div>
      {/if}
    {/snippet}
    {#if metrics}
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
  </LoadState>

  <section class="history" aria-label="History from the Monitor" data-testid="k8s-metrics-history">
    <div class="hist-head">
      <span class="hist-title">History (Monitor) · last hour</span>
      <button class="btn small ghost" onclick={openInMonitor} title={workload ? `Open ${workload} in the Monitor` : 'Open the Monitor for this cluster'} data-testid="k8s-metrics-open-monitor">Open in Monitor</button>
    </div>
    {#if historyLoading && !history}
      <Skeleton rows={2} height={30} />
    {:else if historyError}
      <div class="dim">The Monitor has no history for this cluster ({historyError}). <button class="btn small" onclick={() => void loadHistory()}>Retry</button></div>
    {:else if !history?.mem && !history?.rps}
      <div class="dim">No Monitor samples for this pod in the last hour. Enable monitoring for this cluster (Monitor › Settings) to keep history.</div>
    {:else}
      {#if history?.mem}
        <div class="hist-row"><span class="lbl">mem</span><Sparkline points={history.mem.points.map((p) => p.v)} width={260} height={36} label="memory history" /><span class="val mono">{formatBytes(history.mem.points[history.mem.points.length - 1]?.v ?? 0)}</span></div>
      {/if}
      {#if history?.rps}
        <div class="hist-row"><span class="lbl">req/s</span><Sparkline points={history.rps.points.map((p) => p.v)} width={260} height={36} stroke="var(--status-working)" label="request-rate history" /><span class="val mono">{(history.rps.points[history.rps.points.length - 1]?.v ?? 0).toFixed(2)}</span></div>
      {/if}
    {/if}
  </section>
</div>

<style>
  .metrics {
    padding: 14px 16px;
    display: flex;
    flex-direction: column;
    gap: 14px;
    font-size: var(--fs-m);
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
    font-size: var(--fs-l);
    font-weight: 600;
  }
  .lbl {
    font-size: var(--fs-xs);
    text-transform: uppercase;
    letter-spacing: .06em;
    color: var(--text-dim);
    width: 32px;
  }
  .containers {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .ctr-name {
    font-size: var(--fs-s);
    margin-bottom: 4px;
  }
  .bar-row {
    display: grid;
    grid-template-columns: 32px 1fr 90px;
    gap: 8px;
    align-items: center;
    margin-bottom: 2px;
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
    /* Data-driven width: no transition (a poll would animate every tick). */
  }
  .fill.cpu {
    background: var(--accent);
  }
  .fill.mem {
    background: var(--status-working);
  }
  .val {
    text-align: end;
    font-size: var(--fs-s);
  }
  .dim {
    color: var(--text-dim);
    line-height: 1.5;
  }
  .small {
    font-size: var(--fs-xs);
  }
  .history {
    border-block-start: 1px solid var(--border);
    padding-block-start: 12px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .hist-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }
  .hist-title {
    font-size: var(--fs-s);
    font-weight: 600;
  }
  .hist-row {
    display: grid;
    grid-template-columns: 40px minmax(0, 1fr) 90px;
    gap: 8px;
    align-items: center;
  }
  .mono {
    font-family: var(--font-mono);
  }
</style>
