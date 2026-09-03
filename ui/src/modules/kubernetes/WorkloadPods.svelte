<script lang="ts">
  // Drawer "Pods" tab for a workload (Deployment / StatefulSet / DaemonSet /
  // ReplicaSet / Job / Rollout): the pods its `spec.selector` matches, with
  // ready / status / restarts / CPU / MEM (metrics-server, when it answers) /
  // age, refreshed every 10 s while visible. Each row jumps to that pod's own
  // drawer; Logs / Shell open it straight on those tabs.
  import { untrack } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import { isAbortError } from '../../lib/api/client';
  import { k8sApi } from '../../lib/api/k8s';
  import type { K8sRow } from '../../lib/api/types';
  import type { K8sDrawerTab } from '../../lib/stores/k8s.svelte';
  import { formatAge, formatBytes, formatMillicores, healthClass } from './k8s-util';

  interface Props {
    clusterId: string;
    ns: string;
    selector: string;
    canEdit: boolean;
    onopenpod: (pod: string, tab?: K8sDrawerTab) => void;
  }
  let { clusterId, ns, selector, canEdit, onopenpod }: Props = $props();

  const REFRESH_MS = 10_000;
  let pods: K8sRow[] = $state([]);
  let hasMetrics = $state(false);
  let loading = $state(true);
  let error = $state('');
  let abort: AbortController | null = null;

  async function load(quiet = false): Promise<void> {
    abort?.abort();
    const ac = new AbortController();
    abort = ac;
    if (!quiet) loading = true;
    try {
      const r = await k8sApi.resources(clusterId, 'pods', { ns, label: selector }, ac.signal);
      if (ac.signal.aborted) return;
      pods = [...r.items].sort((a, b) => a.name.localeCompare(b.name));
      hasMetrics = r.has_metrics;
      error = '';
    } catch (e) {
      if (ac.signal.aborted || isAbortError(e)) return;
      error = e instanceof Error ? e.message : String(e);
    } finally {
      if (abort === ac) loading = false;
    }
  }

  $effect(() => {
    void clusterId;
    void ns;
    void selector;
    untrack(() => void load());
    const t = setInterval(() => void load(true), REFRESH_MS);
    return () => {
      clearInterval(t);
      abort?.abort();
    };
  });

  const totals = $derived.by(() => {
    let ready = 0;
    let restarts = 0;
    let cpu = 0;
    let mem = 0;
    for (const p of pods) {
      const [r, t] = (p.ready ?? '0/0').split('/').map(Number);
      if (r === t && t > 0) ready++;
      restarts += p.restarts ?? 0;
      cpu += p.cpu ?? 0;
      mem += p.mem ?? 0;
    }
    return { ready, restarts, cpu, mem };
  });
</script>

<div class="wp">
  <div class="wp-sum">
    <span><b>{pods.length}</b> pods</span>
    <span><b>{totals.ready}</b> ready</span>
    <span class:warn={totals.restarts > 0}><b>{totals.restarts}</b> restarts</span>
    {#if hasMetrics}<span class="mono">{formatMillicores(totals.cpu)} · {formatBytes(totals.mem)}</span>{/if}
    <span class="spacer"></span>
    <button class="icon-btn" onclick={() => void load(true)} title="Refresh" aria-label="Refresh pods"><Icon name="refresh" size={13} /></button>
  </div>
  {#if error}
    <div class="err">{error}</div>
  {:else if loading && !pods.length}
    <div class="pad"><Skeleton rows={4} height={22} /></div>
  {:else if !pods.length}
    <div class="dim pad">No pods match <code class="mono">{selector}</code>.</div>
  {:else}
    <div class="wp-head" class:metrics={hasMetrics}>
      <span>Pod</span><span class="num">Ready</span><span>Status</span><span class="num" title="Restarts">↻</span>
      {#if hasMetrics}<span class="num">CPU</span><span class="num">MEM</span>{/if}
      <span class="num">Age</span><span></span>
    </div>
    {#each pods as p (p.name)}
      <div class="wp-row {healthClass(p.health, p.status)}" class:metrics={hasMetrics} role="button" tabindex="0" onclick={() => onopenpod(p.name)} onkeydown={(e) => { if (e.key === 'Enter') onopenpod(p.name); }} title={p.name}>
        <span class="mono ell">{p.name}</span>
        <span class="num mono">{p.ready ?? ''}</span>
        <span class="status-pill ell"><span class="hdot"></span>{p.status}</span>
        <span class="num mono" class:warn={(p.restarts ?? 0) > 0}>{p.restarts ?? ''}</span>
        {#if hasMetrics}
          <span class="num mono">{p.cpu == null ? '' : formatMillicores(p.cpu)}</span>
          <span class="num mono">{p.mem == null ? '' : formatBytes(p.mem)}</span>
        {/if}
        <span class="num mono">{formatAge(p.age_seconds)}</span>
        <span class="acts">
          <button class="icon-btn" onclick={(e) => { e.stopPropagation(); onopenpod(p.name, 'logs'); }} title="Logs" aria-label="Logs of {p.name}"><Icon name="file" size={12} /></button>
          {#if canEdit}<button class="icon-btn" onclick={(e) => { e.stopPropagation(); onopenpod(p.name, 'terminal'); }} title="Shell (exec)" aria-label="Shell into {p.name}"><Icon name="terminal" size={12} /></button>{/if}
        </span>
      </div>
    {/each}
  {/if}
</div>

<style>
  .wp {
    display: flex;
    flex-direction: column;
    min-height: 0;
    font-size: 12px;
  }
  .wp-sum {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 8px 12px;
    border-bottom: 1px solid var(--border);
    color: var(--text-dim);
  }
  .wp-sum b {
    color: var(--text);
    font-weight: 600;
  }
  .spacer {
    flex: 1;
  }
  .wp-head,
  .wp-row {
    display: grid;
    grid-template-columns: minmax(120px, 1fr) 44px minmax(90px, 0.8fr) 30px 52px 56px;
    align-items: center;
    column-gap: 8px;
    padding: 0 12px;
    min-height: 28px;
  }
  .wp-head.metrics,
  .wp-row.metrics {
    grid-template-columns: minmax(120px, 1fr) 44px minmax(90px, 0.8fr) 30px 56px 60px 52px 56px;
  }
  .wp-head {
    font-size: 10.5px;
    font-weight: 600;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    color: var(--text-dim);
    border-bottom: 1px solid var(--border);
    min-height: 24px;
  }
  .wp-row {
    border-bottom: 1px solid color-mix(in srgb, var(--border) 55%, transparent);
    cursor: pointer;
    outline: none;
  }
  .wp-row:hover,
  .wp-row:focus-visible {
    background: var(--surface-2);
  }
  .num {
    text-align: right;
  }
  .ell {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .mono {
    font-family: var(--font-mono);
    font-size: 11.5px;
  }
  .warn {
    color: var(--status-warn, #e0a000);
  }
  .acts {
    display: inline-flex;
    justify-content: flex-end;
    gap: 2px;
    opacity: 0.55;
  }
  .wp-row:hover .acts,
  .wp-row:focus-within .acts {
    opacity: 1;
  }
  .status-pill {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .hdot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--text-dim);
    flex-shrink: 0;
  }
  .health-ok {
    color: var(--status-ready, #3fb950);
  }
  .health-ok .hdot {
    background: var(--status-ready, #3fb950);
  }
  .health-bad {
    color: var(--status-exited, #f85149);
  }
  .health-bad .hdot {
    background: var(--status-exited, #f85149);
  }
  .health-progressing {
    color: var(--status-working, #d29922);
  }
  .health-progressing .hdot {
    background: var(--status-working, #d29922);
  }
  .health-warn {
    color: var(--status-warn, #e0a000);
  }
  .health-warn .hdot {
    background: var(--status-warn, #e0a000);
  }
  .err {
    padding: 8px 12px;
    color: var(--status-exited);
    white-space: pre-wrap;
  }
  .dim {
    color: var(--text-dim);
  }
  .pad {
    padding: 12px;
  }
</style>
