<script lang="ts">
  // Drawer "Pods" tab for a workload (Deployment / StatefulSet / DaemonSet /
  // ReplicaSet / Job / Rollout): the pods its `spec.selector` matches, with
  // ready / status / restarts / CPU / MEM (metrics-server, when it answers) /
  // age, refreshed every 10 s while visible. Each row jumps to that pod's own
  // drawer; Logs / Shell open it straight on those tabs.
  import { untrack } from 'svelte';
  import { pollWhileVisible } from '../../lib/poll';
  import Icon from '../../lib/components/Icon.svelte';
  import Badge from '../../lib/components/Badge.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { isAbortError } from '../../lib/api/client';
  import { k8sApi } from '../../lib/api/k8s';
  import type { K8sRow } from '../../lib/api/types';
  import type { K8sDrawerTab } from '../../lib/stores/k8s.svelte';
  import { formatAge, formatBytes, formatMillicores, healthTone, rowAge } from './k8s-util';

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
      error = loadErrorText(e);
    } finally {
      if (abort === ac) loading = false;
    }
  }

  $effect(() => {
    void clusterId;
    void ns;
    void selector;
    // A different cluster/namespace/selector must not show the previous one's pods.
    untrack(() => {
      pods = [];
      void load();
    });
    const t = pollWhileVisible(() => load(true), { ms: REFRESH_MS, immediate: false });
    return () => {
      t.stop();
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
  <!-- A failed poll never replaces good data: with pods on screen it becomes the
       stale bar; the full error shows only when there is nothing to show. -->
  <LoadState what="pods" {loading} {error} empty={!pods.length} onretry={() => void load()} variant="panel">
    {#snippet emptyView()}
      <div class="dim pad">No pods match <code class="mono">{selector}</code>.</div>
    {/snippet}
      <div class="wp-head" class:metrics={hasMetrics}>
        <span>Pod</span><span class="num">Ready</span><span>Status</span><span class="num" title="Restarts"><Icon name="refresh" size={11} /><span class="sr-only">Restarts</span></span>
        {#if hasMetrics}<span class="num">CPU</span><span class="num">MEM</span>{/if}
        <span class="num">Age</span><span></span>
      </div>
      {#each pods as p (p.name)}
        {@const tone = healthTone(p.health, p.status)}
        <!-- The row itself is not interactive: the pod name is the open control (a
             real <button>, Enter + Space), and Logs / Shell are its siblings —
             never nested inside another interactive element. -->
        <div class="wp-row" class:metrics={hasMetrics}>
          <button type="button" class="wp-name mono ell" title="Open {p.name}" onclick={() => onopenpod(p.name)}>{p.name}</button>
          <span class="num mono">{p.ready ?? ''}</span>
          <span class="wp-status"><Badge {tone} dot live={tone === 'info'}><span class="ell">{p.status}</span></Badge></span>
          <span class="num mono" class:warn={(p.restarts ?? 0) > 0}>{p.restarts ?? ''}</span>
          {#if hasMetrics}
            <span class="num mono">{p.cpu == null ? '' : formatMillicores(p.cpu)}</span>
            <span class="num mono">{p.mem == null ? '' : formatBytes(p.mem)}</span>
          {/if}
          <span class="num mono">{formatAge(rowAge(p))}</span>
          <span class="acts">
            <button class="icon-btn" onclick={() => onopenpod(p.name, 'logs')} title="Logs of {p.name}" aria-label="Logs of {p.name}"><Icon name="file" size={12} /></button>
            {#if canEdit}<button class="icon-btn" onclick={() => onopenpod(p.name, 'terminal')} title="Shell into {p.name}" aria-label="Shell into {p.name}"><Icon name="terminal" size={12} /></button>{/if}
          </span>
        </div>
      {/each}
  
  </LoadState>
</div>

<style>
  .wp {
    display: flex;
    flex-direction: column;
    min-height: 0;
    font-size: var(--fs-s);
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
    font-size: var(--fs-xs);
    font-weight: 600;
    letter-spacing: .06em;
    text-transform: uppercase;
    color: var(--text-dim);
    border-bottom: 1px solid var(--border);
    min-height: 24px;
  }
  .wp-row {
    border-bottom: 1px solid color-mix(in srgb, var(--border) 55%, transparent);
  }
  .wp-row:hover,
  .wp-row:focus-within {
    background: var(--hover);
  }
  .wp-name {
    display: block;
    padding: 0;
    border: none;
    background: none;
    color: inherit;
    font: inherit;
    text-align: start;
    cursor: pointer;
  }
  .wp-name:hover {
    text-decoration: underline;
  }
  .wp-name:focus-visible {
    border-radius: var(--radius-s);
  }
  .num {
    text-align: end;
  }
  .ell {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .mono {
    font-family: var(--font-mono);
    font-size: var(--fs-s);
  }
  .warn {
    color: var(--warning);
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
  /* The status Badge shrinks to its grid column; the inner .ell ellipsizes. */
  .wp-status {
    min-width: 0;
    display: flex;
  }
  .wp-status :global(.badge) {
    max-width: 100%;
  }
  .dim {
    color: var(--text-dim);
  }
  .pad {
    padding: 12px;
  }
</style>
