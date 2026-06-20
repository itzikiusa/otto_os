<script lang="ts">
  import { api } from '../../lib/api/client';
  import { toasts } from '../../lib/toast.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import TopicDetail from './TopicDetail.svelte';
  import type {
    BrokerCluster,
    CreateTopicReq,
    TopicStats,
    TopicSummary,
  } from '../../lib/api/types';

  interface Props {
    cluster: BrokerCluster;
  }
  let { cluster }: Props = $props();

  const PAGE_SIZE = 50;
  const STATS_CONCURRENCY = 4;

  let topics = $state<TopicSummary[]>([]);
  // Lazily-filled per-topic stats (count + cleanup policy), keyed by name.
  let stats = $state<Record<string, TopicStats>>({});
  let loading = $state(true);
  let query = $state('');
  let showInternal = $state(false);
  let cleanupFilter = $state('');
  let page = $state(1);
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
      .filter((t) => !cleanupFilter || stats[t.name]?.cleanup_policy === cleanupFilter)
      .sort((a, b) => a.name.localeCompare(b.name)),
  );
  const pageCount = $derived(Math.max(1, Math.ceil(filtered.length / PAGE_SIZE)));
  const pageStart = $derived((Math.min(page, pageCount) - 1) * PAGE_SIZE);
  const visible = $derived(filtered.slice(pageStart, pageStart + PAGE_SIZE));
  // Cleanup-policy values discovered so far (for the filter dropdown).
  const cleanupOptions = $derived(
    [...new Set(Object.values(stats).map((s) => s.cleanup_policy).filter(Boolean))] as string[],
  );

  function load() {
    loading = true;
    stats = {};
    api
      .get<TopicSummary[]>(`/brokers/clusters/${cluster.id}/topics`)
      .then((t) => {
        topics = t;
        if (selected && !t.some((x) => x.name === selected)) selected = null;
      })
      .catch((e) => toasts.error('Failed to load topics', String(e)))
      .finally(() => (loading = false));
  }

  // Background-fill stats for the topics currently on screen (cached; bounded
  // concurrency so a slow/tunnelled cluster stays responsive).
  let statsToken = 0;
  async function fillStats(names: string[]) {
    const pending = names.filter((n) => stats[n] === undefined);
    if (pending.length === 0) return;
    const token = ++statsToken;
    // Mark as in-flight (null) so we don't refetch.
    stats = { ...stats, ...Object.fromEntries(pending.map((n) => [n, null as unknown as TopicStats])) };
    const queue = [...pending];
    const worker = async () => {
      while (queue.length) {
        const name = queue.shift()!;
        try {
          const s = await api.get<TopicStats>(
            `/brokers/clusters/${cluster.id}/topics/${encodeURIComponent(name)}/stats`,
          );
          if (token === statsToken) stats = { ...stats, [name]: s };
        } catch {
          // leave as in-flight/null; count shows "—"
        }
      }
    };
    await Promise.all(Array.from({ length: STATS_CONCURRENCY }, worker));
  }

  $effect(() => {
    void cluster.id;
    selected = null;
    page = 1;
    load();
  });

  // Lazily load stats for the visible page (and refetch as you paginate/filter).
  $effect(() => {
    if (loading || selected) return;
    void fillStats(visible.map((t) => t.name));
  });

  // Reset to page 1 when the filters change.
  $effect(() => {
    void query;
    void showInternal;
    void cleanupFilter;
    page = 1;
  });

  function countText(name: string): string {
    const s = stats[name];
    if (s === undefined) return '';
    if (s === null) return '…';
    return s.message_count.toLocaleString();
  }

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

{#if selected}
  <div class="detail-wrap">
    <button class="crumb" onclick={() => (selected = null)}>
      <Icon name="chevronLeft" size={13} /> Topics
      <span class="sep">/</span>
      <span class="cur">{selected}</span>
    </button>
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
  </div>
{:else}
  <div class="topics">
    <div class="toolbar">
      <input class="search" bind:value={query} placeholder="Search topics…" />
      <label class="chk"><input type="checkbox" bind:checked={showInternal} /> Show internal</label>
      {#if cleanupOptions.length}
        <select bind:value={cleanupFilter} title="Cleanup policy">
          <option value="">Any cleanup policy</option>
          {#each cleanupOptions as p (p)}<option value={p}>{p}</option>{/each}
        </select>
      {/if}
      <span class="spacer"></span>
      <span class="count">{filtered.length} topic{filtered.length === 1 ? '' : 's'}</span>
      <button class="btn small" onclick={() => (creating = !creating)} title="New topic">
        <Icon name="plus" size={13} /> New
      </button>
    </div>

    {#if creating}
      <div class="create">
        <input bind:value={newName} placeholder="topic name" />
        <label>Parts <input type="number" min="1" bind:value={newParts} /></label>
        <label>RF <input type="number" min="1" bind:value={newRf} /></label>
        <button class="btn primary small" onclick={createTopic}>Create</button>
        <button class="btn small" onclick={() => (creating = false)}>Cancel</button>
      </div>
    {/if}

    <div class="grid-wrap">
      {#if loading}
        <p class="muted pad">Loading…</p>
      {:else if filtered.length === 0}
        <p class="muted pad">No topics.</p>
      {:else}
        <table class="grid">
          <thead>
            <tr>
              <th class="tname">Topic</th>
              <th class="num">Partitions</th>
              <th class="num">RF</th>
              <th class="num">Count</th>
              <th class="num">Size</th>
            </tr>
          </thead>
          <tbody>
            {#each visible as t (t.name)}
              <tr onclick={() => (selected = t.name)}>
                <td class="tname" class:internal={t.internal}>{t.name}</td>
                <td class="num">{t.partitions}</td>
                <td class="num">{t.replication_factor}</td>
                <td class="num">{countText(t.name)}</td>
                <td class="num muted" title="On-disk size isn't exposed by this Kafka client">—</td>
              </tr>
            {/each}
          </tbody>
        </table>
      {/if}
    </div>

    {#if !loading && pageCount > 1}
      <div class="pager">
        <span class="muted"
          >{pageStart + 1}–{Math.min(pageStart + PAGE_SIZE, filtered.length)} of {filtered.length}</span
        >
        <button class="btn tiny" disabled={page <= 1} onclick={() => (page = Math.max(1, page - 1))}>
          <Icon name="chevronLeft" size={12} />
        </button>
        <span class="muted">{Math.min(page, pageCount)} / {pageCount}</span>
        <button
          class="btn tiny"
          disabled={page >= pageCount}
          onclick={() => (page = Math.min(pageCount, page + 1))}
        >
          <Icon name="chevronRight" size={12} />
        </button>
      </div>
    {/if}
  </div>
{/if}

<style>
  .topics {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .detail-wrap {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .crumb {
    display: flex;
    align-items: center;
    gap: 4px;
    border: none;
    background: transparent;
    color: var(--text-dim);
    padding: 8px 12px;
    cursor: pointer;
    font-size: 12.5px;
    border-bottom: 1px solid var(--border);
  }
  .crumb:hover {
    color: var(--text);
  }
  .crumb .sep {
    opacity: 0.5;
  }
  .crumb .cur {
    color: var(--text);
    font-family: var(--font-mono);
  }
  .toolbar {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 12px;
    border-bottom: 1px solid var(--border);
  }
  .search {
    width: 240px;
    padding: 6px 9px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--bg);
    color: var(--text);
    font-size: 12.5px;
  }
  .chk {
    display: flex;
    align-items: center;
    gap: 5px;
    font-size: 12px;
    color: var(--text-dim);
    white-space: nowrap;
  }
  .toolbar select {
    padding: 5px 7px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--bg);
    color: var(--text);
    font-size: 12px;
  }
  .spacer {
    flex: 1;
  }
  .count {
    font-size: 12px;
    color: var(--text-dim);
  }
  .create {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    border-bottom: 1px solid var(--border);
    background: color-mix(in srgb, var(--accent) 5%, transparent);
  }
  .create input {
    padding: 6px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--bg);
    color: var(--text);
    font-size: 12px;
  }
  .create label {
    font-size: 11px;
    color: var(--text-dim);
    display: flex;
    gap: 4px;
    align-items: center;
  }
  .create label input {
    width: 52px;
  }
  .grid-wrap {
    flex: 1;
    overflow: auto;
    min-height: 0;
  }
  table.grid {
    width: 100%;
    border-collapse: collapse;
    font-size: 12.5px;
  }
  table.grid th {
    text-align: left;
    font-weight: 500;
    color: var(--text-dim);
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    padding: 8px 14px;
    position: sticky;
    top: 0;
    background: var(--surface);
    border-bottom: 1px solid var(--border);
  }
  table.grid th.num {
    text-align: right;
    width: 110px;
  }
  table.grid td {
    padding: 7px 14px;
    border-bottom: 1px solid var(--border);
  }
  table.grid td.num {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  table.grid td.tname {
    font-family: var(--font-mono);
    color: var(--text);
    word-break: break-all;
  }
  table.grid td.tname.internal {
    color: var(--text-dim);
  }
  table.grid tbody tr {
    cursor: pointer;
  }
  table.grid tbody tr:hover {
    background: color-mix(in srgb, var(--text-dim) 8%, transparent);
  }
  .pager {
    display: flex;
    align-items: center;
    gap: 10px;
    justify-content: flex-end;
    padding: 8px 14px;
    border-top: 1px solid var(--border);
    font-size: 12px;
  }
  .muted {
    color: var(--text-dim);
  }
  .pad {
    padding: 14px;
  }
  .btn.tiny {
    padding: 3px 7px;
  }
</style>
