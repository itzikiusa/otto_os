<script lang="ts">
  import { api } from '../../lib/api/client';
  import { toasts } from '../../lib/toast.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import TopicDetail from './TopicDetail.svelte';
  import type { BrokerCluster, CreateTopicReq, TopicSummary } from '../../lib/api/types';

  interface Props {
    cluster: BrokerCluster;
  }
  let { cluster }: Props = $props();

  let topics = $state<TopicSummary[]>([]);
  let loading = $state(true);
  let query = $state('');
  let showInternal = $state(false);
  let selected = $state<string | null>(null);

  let creating = $state(false);
  let newName = $state('');
  let newParts = $state(1);
  let newRf = $state(1);

  const guarded = $derived(cluster.read_only || cluster.environment === 'prod');

  const filtered = $derived(
    topics
      .filter((t) => showInternal || !t.internal)
      .filter((t) => t.name.toLowerCase().includes(query.toLowerCase()))
      .sort((a, b) => a.name.localeCompare(b.name)),
  );

  function load() {
    loading = true;
    api
      .get<TopicSummary[]>(`/brokers/clusters/${cluster.id}/topics`)
      .then((t) => {
        topics = t;
        if (selected && !t.some((x) => x.name === selected)) selected = null;
      })
      .catch((e) => toasts.error('Failed to load topics', String(e)))
      .finally(() => (loading = false));
  }

  $effect(() => {
    void cluster.id;
    selected = null;
    load();
  });

  async function createTopic() {
    if (!newName.trim()) return;
    const req: CreateTopicReq = {
      name: newName.trim(),
      partitions: Number(newParts),
      replication_factor: Number(newRf),
      confirm: guarded,
    };
    try {
      await api.post(`/brokers/clusters/${cluster.id}/topics`, req);
      toasts.success(`Created ${req.name}`);
      creating = false;
      newName = '';
      load();
      selected = req.name;
    } catch (e) {
      toasts.error('Create failed', String(e));
    }
  }
</script>

<div class="topics">
  <div class="list">
    <div class="list-head">
      <input class="search" bind:value={query} placeholder="Search topics…" />
      <button class="btn small" onclick={() => (creating = !creating)} title="New topic">
        <Icon name="plus" size={13} />
      </button>
    </div>
    {#if creating}
      <div class="create">
        <input bind:value={newName} placeholder="topic name" />
        <div class="cr-row">
          <label>Parts <input type="number" min="1" bind:value={newParts} /></label>
          <label>RF <input type="number" min="1" bind:value={newRf} /></label>
          <button class="btn primary small" onclick={createTopic}>Create</button>
        </div>
      </div>
    {/if}
    <label class="internal-toggle">
      <input type="checkbox" bind:checked={showInternal} /> show internal
    </label>
    <div class="rows">
      {#if loading}
        <p class="muted pad">Loading…</p>
      {:else}
        {#each filtered as t (t.name)}
          <button class="trow" class:sel={selected === t.name} onclick={() => (selected = t.name)}>
            <span class="tn" class:internal={t.internal}>{t.name}</span>
            <span class="meta">{t.partitions}p · {t.message_count < 0 ? '—' : t.message_count.toLocaleString()}</span>
          </button>
        {/each}
        {#if filtered.length === 0}<p class="muted pad">No topics.</p>{/if}
      {/if}
    </div>
  </div>

  <div class="detail">
    {#if selected}
      {#key selected}
        <TopicDetail
          {cluster}
          topic={selected}
          ondeleted={() => {
            selected = null;
            load();
          }}
        />
      {/key}
    {:else}
      <div class="empty">
        <Icon name="box" size={26} />
        <p>Select a topic to browse messages, partitions, configs and produce.</p>
      </div>
    {/if}
  </div>
</div>

<style>
  .topics {
    display: flex;
    height: 100%;
    min-height: 0;
  }
  .list {
    width: 280px;
    border-right: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .list-head {
    display: flex;
    gap: 6px;
    padding: 10px;
  }
  .search {
    flex: 1;
    padding: 6px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--bg);
    color: var(--text);
    font-size: 12px;
  }
  .create {
    padding: 0 10px 8px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .create input {
    padding: 6px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--bg);
    color: var(--text);
    font-size: 12px;
  }
  .cr-row {
    display: flex;
    gap: 6px;
    align-items: center;
  }
  .cr-row label {
    font-size: 11px;
    color: var(--text-dim);
    display: flex;
    gap: 4px;
    align-items: center;
  }
  .cr-row input {
    width: 48px;
  }
  .internal-toggle {
    font-size: 11px;
    color: var(--text-dim);
    padding: 0 10px 8px;
    display: flex;
    align-items: center;
    gap: 5px;
  }
  .rows {
    flex: 1;
    overflow: auto;
  }
  .trow {
    width: 100%;
    text-align: left;
    border: none;
    background: transparent;
    padding: 7px 12px;
    display: flex;
    flex-direction: column;
    gap: 1px;
    cursor: pointer;
    border-left: 2px solid transparent;
  }
  .trow:hover {
    background: color-mix(in srgb, var(--text-dim) 8%, transparent);
  }
  .trow.sel {
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    border-left-color: var(--accent);
  }
  .tn {
    font-family: var(--font-mono);
    font-size: 12.5px;
    color: var(--text);
    word-break: break-all;
  }
  .tn.internal {
    color: var(--text-dim);
  }
  .meta {
    font-size: 11px;
    color: var(--text-dim);
  }
  .detail {
    flex: 1;
    min-width: 0;
  }
  .empty {
    height: 100%;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    color: var(--text-dim);
  }
  .muted {
    color: var(--text-dim);
  }
  .pad {
    padding: 12px;
  }
</style>
