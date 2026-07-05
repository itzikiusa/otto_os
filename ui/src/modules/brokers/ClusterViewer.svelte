<script lang="ts">
  // Embeddable Kafka cluster viewer: the header + per-cluster tab strip
  // (Overview / Topics / Consumer Groups / Schema Registry / Replay / Lag Alerts)
  // that the standalone Message Brokers page renders, factored out so the unified
  // Connections workbench can open a cluster as a first-class tab (next to the DB
  // connections) instead of navigating away. Self-contained: it owns its sub-tab
  // + warm-tunnel state and does the Test call itself; Edit/Remove are delegated
  // to the host via callbacks so each page can wire them to its own flow.
  import Icon from '../../lib/components/Icon.svelte';
  import { api } from '../../lib/api/client';
  import { toasts } from '../../lib/toast.svelte';
  import type { BrokerCluster, TestClusterResp } from '../../lib/api/types';
  import OverviewTab from './OverviewTab.svelte';
  import TopicsTab from './TopicsTab.svelte';
  import GroupsTab from './GroupsTab.svelte';
  import SchemaTab from './SchemaTab.svelte';
  import ReplayPanel from './ReplayPanel.svelte';
  import LagAlertsPanel from './LagAlertsPanel.svelte';

  interface Props {
    cluster: BrokerCluster;
    onEdit: (c: BrokerCluster) => void;
    onRemove: (c: BrokerCluster) => void;
  }
  let { cluster, onEdit, onRemove }: Props = $props();

  type Tab = 'overview' | 'topics' | 'groups' | 'schema' | 'replay' | 'alerts';
  let tab = $state<Tab>('overview');
  let testing = $state(false);

  // Reset to Overview whenever the shown cluster changes (tab switch in the host).
  $effect(() => {
    void cluster.id;
    tab = 'overview';
  });

  // Warm the SSH tunnel (if any) in the background so the pill flips to "Tunnel"
  // quickly — mirrors the standalone Message Brokers page.
  let warmingId = $state<string | null>(null);
  let tunnelReady = $state(false);
  $effect(() => {
    const c = cluster;
    tunnelReady = false;
    if (!c.ssh) return;
    warmingId = c.id;
    const id = c.id;
    void api
      .post<TestClusterResp>(`/brokers/clusters/${id}/test`, {})
      .then((r) => {
        if (warmingId === id) tunnelReady = r.ok;
      })
      .catch(() => {
        // silent — the pill stays grey until a real op succeeds
      });
  });

  async function testConn(): Promise<void> {
    testing = true;
    try {
      const r = await api.post<TestClusterResp>(`/brokers/clusters/${cluster.id}/test`, {});
      if (r.ok) toasts.success('Connected', `${r.message} · ${r.latency_ms}ms`);
      else toasts.error('Connection failed', r.message);
    } catch (e) {
      toasts.error('Test failed', String(e));
    } finally {
      testing = false;
    }
  }
</script>

<div class="cv">
  <header class="cv-head">
    <div class="cv-title">
      <span class="dot" style="background: {cluster.color || 'var(--accent)'}"></span>
      <span class="name ellipsis">{cluster.name}</span>
      <span class="env {cluster.environment}">{cluster.environment}</span>
      {#if cluster.read_only}<span class="ro">read-only</span>{/if}
      {#if cluster.ssh}
        <span
          class="tunnel-pill"
          class:ready={tunnelReady}
          title={tunnelReady ? 'SSH tunnel connected' : 'SSH tunnel warming…'}
        >
          <Icon name="zap" size={10} /> {tunnelReady ? 'Tunnel' : 'Connecting…'}
        </span>
      {/if}
      <span class="boot mono ellipsis">{cluster.bootstrap_servers}</span>
    </div>
    <div class="cv-actions">
      <button class="btn small" onclick={testConn} disabled={testing}>
        {testing ? 'Testing…' : 'Test'}
      </button>
      <button class="btn small" onclick={() => onEdit(cluster)}>Edit</button>
      <button class="btn small danger" onclick={() => onRemove(cluster)}>Remove</button>
    </div>
  </header>

  <div class="cv-tabs" role="tablist" aria-label="Kafka cluster views">
    <button class:on={tab === 'overview'} role="tab" aria-selected={tab === 'overview'} onclick={() => (tab = 'overview')}>Overview</button>
    <button class:on={tab === 'topics'} role="tab" aria-selected={tab === 'topics'} onclick={() => (tab = 'topics')}>Topics</button>
    <button class:on={tab === 'groups'} role="tab" aria-selected={tab === 'groups'} onclick={() => (tab = 'groups')}>Consumer Groups</button>
    <button class:on={tab === 'schema'} role="tab" aria-selected={tab === 'schema'} onclick={() => (tab = 'schema')}>Schema Registry</button>
    <button class:on={tab === 'replay'} role="tab" aria-selected={tab === 'replay'} onclick={() => (tab = 'replay')}>Replay</button>
    <button class:on={tab === 'alerts'} role="tab" aria-selected={tab === 'alerts'} onclick={() => (tab = 'alerts')}>Lag Alerts</button>
  </div>

  <div class="cv-body">
    {#key cluster.id}
      {#if tab === 'overview'}
        <OverviewTab clusterId={cluster.id} />
      {:else if tab === 'topics'}
        <TopicsTab {cluster} />
      {:else if tab === 'groups'}
        <GroupsTab {cluster} />
      {:else if tab === 'schema'}
        <SchemaTab {cluster} />
      {:else if tab === 'replay'}
        <ReplayPanel {cluster} />
      {:else if tab === 'alerts'}
        <LagAlertsPanel {cluster} />
      {/if}
    {/key}
  </div>
</div>

<style>
  .cv {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    overflow: hidden;
  }
  .cv-head {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 12px;
    border-bottom: 1px solid var(--border);
    flex: 0 0 auto;
  }
  .cv-title {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    flex: 1 1 auto;
  }
  .cv-title .name {
    font-weight: 600;
    max-width: 240px;
  }
  .dot {
    width: 9px;
    height: 9px;
    border-radius: 50%;
    flex: 0 0 auto;
  }
  .env {
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    padding: 1px 6px;
    border-radius: 999px;
    border: 1px solid var(--border);
    color: var(--muted);
  }
  .env.prod {
    color: #ff9800;
    border-color: #ff980055;
  }
  .ro {
    font-size: 10px;
    color: var(--muted);
    border: 1px solid var(--border);
    border-radius: 999px;
    padding: 1px 6px;
  }
  .tunnel-pill {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    font-size: 10px;
    color: var(--muted);
    border: 1px solid var(--border);
    border-radius: 999px;
    padding: 1px 6px;
  }
  .tunnel-pill.ready {
    color: #7ee787;
    border-color: #7ee78755;
  }
  .boot {
    font-size: 11px;
    color: var(--muted);
    max-width: 380px;
  }
  .cv-actions {
    display: flex;
    gap: 6px;
    flex: 0 0 auto;
  }
  .cv-tabs {
    display: flex;
    gap: 2px;
    padding: 4px 10px 0;
    border-bottom: 1px solid var(--border);
    flex: 0 0 auto;
    overflow-x: auto;
  }
  .cv-tabs button {
    background: none;
    border: none;
    border-bottom: 2px solid transparent;
    color: var(--muted);
    font: inherit;
    font-size: 12px;
    padding: 6px 10px;
    cursor: pointer;
    white-space: nowrap;
  }
  .cv-tabs button:hover {
    color: var(--fg);
  }
  .cv-tabs button.on {
    color: var(--fg);
    border-bottom-color: var(--accent);
  }
  .cv-body {
    flex: 1 1 auto;
    min-height: 0;
    overflow: auto;
  }
  .ellipsis {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .btn {
    background: var(--surface-2, var(--surface));
    color: var(--fg);
    border: 1px solid var(--border);
    border-radius: 6px;
    font: inherit;
    cursor: pointer;
  }
  .btn.small {
    font-size: 12px;
    padding: 3px 10px;
  }
  .btn:hover {
    border-color: var(--accent);
  }
  .btn:disabled {
    opacity: 0.6;
    cursor: default;
  }
  .btn.danger:hover {
    border-color: #ff6b6b;
    color: #ff6b6b;
  }
</style>
