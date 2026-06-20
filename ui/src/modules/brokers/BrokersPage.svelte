<script lang="ts">
  import Icon from '../../lib/components/Icon.svelte';
  import { api } from '../../lib/api/client';
  import { brokers } from '../../lib/stores/brokers.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import type { BrokerCluster, TestClusterResp } from '../../lib/api/types';
  import ClusterForm from './ClusterForm.svelte';
  import OverviewTab from './OverviewTab.svelte';
  import TopicsTab from './TopicsTab.svelte';
  import GroupsTab from './GroupsTab.svelte';
  import SchemaTab from './SchemaTab.svelte';

  type Tab = 'overview' | 'topics' | 'groups' | 'schema';
  let tab = $state<Tab>('overview');
  let formOpen = $state(false);
  let editTarget = $state<BrokerCluster | null>(null);
  let testing = $state(false);

  $effect(() => {
    const id = ws.currentId;
    if (id) void brokers.load(id);
  });

  $effect(() => {
    // reset to overview when the selected cluster changes
    void brokers.selectedId;
    tab = 'overview';
  });

  const selected = $derived(brokers.selected);

  function envBadge(c: BrokerCluster): string {
    return c.environment === 'prod' ? 'prod' : c.environment === 'staging' ? 'stg' : 'dev';
  }

  async function testConn(c: BrokerCluster) {
    testing = true;
    try {
      const r = await api.post<TestClusterResp>(`/brokers/clusters/${c.id}/test`, {});
      if (r.ok) toasts.success('Connected', `${r.message} · ${r.latency_ms}ms`);
      else toasts.error('Connection failed', r.message);
    } catch (e) {
      toasts.error('Test failed', String(e));
    } finally {
      testing = false;
    }
  }

  async function removeCluster(c: BrokerCluster) {
    if (!confirm(`Remove cluster "${c.name}"? (Topics on the broker are not touched.)`)) return;
    try {
      await brokers.remove(c.id);
      toasts.success('Cluster removed');
    } catch (e) {
      toasts.error('Remove failed', String(e));
    }
  }

  function openEdit(c: BrokerCluster) {
    editTarget = c;
    formOpen = true;
  }
  function openAdd() {
    editTarget = null;
    formOpen = true;
  }
</script>

<div class="brokers-page">
  <aside class="clusters">
    <div class="aside-head">
      <span class="title">Clusters</span>
      <button class="btn small" onclick={openAdd} title="Add cluster"><Icon name="plus" size={13} /></button>
    </div>
    <div class="cluster-list">
      {#if brokers.loading && brokers.clusters.length === 0}
        <p class="muted pad">Loading…</p>
      {:else}
        {#each brokers.clusters as c (c.id)}
          <button
            class="cluster"
            class:sel={brokers.selectedId === c.id}
            onclick={() => brokers.select(c.id)}
          >
            <span class="dot" style="background: {c.color || 'var(--accent)'}"></span>
            <span class="cn">{c.name}</span>
            <span class="env {c.environment}">{envBadge(c)}</span>
          </button>
        {/each}
        {#if brokers.clusters.length === 0}
          <p class="muted pad small">No clusters yet. Add one to connect to Kafka.</p>
        {/if}
      {/if}
    </div>
  </aside>

  <main class="cluster-main">
    {#if selected}
      <header class="cluster-head">
        <div class="ch-title">
          <span class="dot" style="background: {selected.color || 'var(--accent)'}"></span>
          <span class="name">{selected.name}</span>
          <span class="env {selected.environment}">{selected.environment}</span>
          {#if selected.read_only}<span class="ro">read-only</span>{/if}
          <span class="boot mono">{selected.bootstrap_servers}</span>
        </div>
        <div class="actions">
          <button class="btn small" onclick={() => testConn(selected)} disabled={testing}>
            {testing ? 'Testing…' : 'Test'}
          </button>
          <button class="btn small" onclick={() => openEdit(selected)}>Edit</button>
          <button class="btn small danger" onclick={() => removeCluster(selected)}>Remove</button>
        </div>
      </header>

      <nav class="tabs">
        <button class:on={tab === 'overview'} onclick={() => (tab = 'overview')}>Overview</button>
        <button class:on={tab === 'topics'} onclick={() => (tab = 'topics')}>Topics</button>
        <button class:on={tab === 'groups'} onclick={() => (tab = 'groups')}>Consumer Groups</button>
        <button class:on={tab === 'schema'} onclick={() => (tab = 'schema')}>Schema Registry</button>
      </nav>

      <div class="tab-body">
        {#key selected.id}
          {#if tab === 'overview'}
            <OverviewTab clusterId={selected.id} />
          {:else if tab === 'topics'}
            <TopicsTab cluster={selected} />
          {:else if tab === 'groups'}
            <GroupsTab cluster={selected} />
          {:else if tab === 'schema'}
            <SchemaTab cluster={selected} />
          {/if}
        {/key}
      </div>
    {:else}
      <div class="empty">
        <Icon name="box" size={30} />
        <h3>Message Brokers</h3>
        <p>Connect a Kafka cluster to browse topics, peek messages, inspect consumer-group lag, and watch broker CPU / RAM.</p>
        <button class="btn primary" onclick={openAdd}>Add a cluster</button>
      </div>
    {/if}
  </main>
</div>

{#if formOpen}
  <ClusterForm cluster={editTarget} onclose={() => (formOpen = false)} />
{/if}

<style>
  .brokers-page {
    display: flex;
    height: 100%;
    min-height: 0;
  }
  .clusters {
    width: 220px;
    border-right: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .aside-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 12px 12px 8px;
  }
  .aside-head .title {
    font-size: 12px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
  }
  .cluster-list {
    flex: 1;
    overflow: auto;
  }
  .cluster {
    width: 100%;
    text-align: left;
    border: none;
    background: transparent;
    padding: 8px 12px;
    display: flex;
    align-items: center;
    gap: 8px;
    cursor: pointer;
    border-left: 2px solid transparent;
  }
  .cluster:hover {
    background: color-mix(in srgb, var(--text-dim) 8%, transparent);
  }
  .cluster.sel {
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    border-left-color: var(--accent);
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex: none;
  }
  .cn {
    flex: 1;
    font-size: 13px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .env {
    font-size: 9px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    padding: 1px 5px;
    border-radius: 4px;
    background: color-mix(in srgb, var(--text-dim) 16%, transparent);
    color: var(--text-dim);
  }
  .env.prod {
    background: color-mix(in srgb, var(--status-exited, #ff5f57) 22%, transparent);
    color: var(--status-exited, #ff5f57);
  }
  .cluster-main {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
  }
  .cluster-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 12px 16px;
    border-bottom: 1px solid var(--border);
    gap: 12px;
  }
  .ch-title {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
  }
  .ch-title .name {
    font-size: 15px;
    font-weight: 600;
  }
  .ch-title .ro {
    font-size: 10px;
    color: var(--status-exited, #ff5f57);
    border: 1px solid currentColor;
    border-radius: 4px;
    padding: 0 5px;
  }
  .boot {
    font-size: 11px;
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .actions {
    display: flex;
    gap: 6px;
    flex: none;
  }
  .tabs {
    display: flex;
    gap: 2px;
    padding: 6px 14px 0;
    border-bottom: 1px solid var(--border);
  }
  .tabs button {
    border: none;
    background: transparent;
    color: var(--text-dim);
    padding: 8px 14px;
    cursor: pointer;
    font-size: 13px;
    border-bottom: 2px solid transparent;
  }
  .tabs button.on {
    color: var(--text);
    border-bottom-color: var(--accent);
  }
  .tab-body {
    flex: 1;
    min-height: 0;
    overflow: hidden;
  }
  .empty {
    height: 100%;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    color: var(--text-dim);
    text-align: center;
    padding: 24px;
  }
  .empty h3 {
    margin: 4px 0 0;
    color: var(--text);
  }
  .empty p {
    max-width: 440px;
  }
  .mono {
    font-family: var(--font-mono);
  }
  .muted {
    color: var(--text-dim);
  }
  .pad {
    padding: 12px;
  }
  .small {
    font-size: 11px;
  }
  .btn.danger {
    color: var(--status-exited, #ff5f57);
  }
</style>
