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
  // null = in-flight, 'err' = failed (retry available), TopicStats = loaded.
  let stats = $state<Record<string, TopicStats | null | 'err'>>({});
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
      .filter((t) => {
        if (!cleanupFilter) return true;
        const s = stats[t.name];
        return s != null && s !== 'err' && s.cleanup_policy === cleanupFilter;
      })
      .sort((a, b) => a.name.localeCompare(b.name)),
  );
  const pageCount = $derived(Math.max(1, Math.ceil(filtered.length / PAGE_SIZE)));
  const pageStart = $derived((Math.min(page, pageCount) - 1) * PAGE_SIZE);
  const visible = $derived(filtered.slice(pageStart, pageStart + PAGE_SIZE));
  // Cleanup-policy values discovered so far (for the filter dropdown).
  const cleanupOptions = $derived(
    [...new Set(
      Object.values(stats)
        .filter((s): s is TopicStats => s != null && s !== 'err')
        .map((s) => s.cleanup_policy)
        .filter(Boolean)
    )] as string[],
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

  // Background-fill stats for the topics currently on screen via the batch
  // endpoint (one HTTP call vs N×1). Bounded by STATS_CONCURRENCY batches of
  // PAGE_SIZE so very large topic lists don't hammer the server with huge
  // request bodies.
  let statsToken = 0;
  async function fillStats(names: string[], retry = false) {
    // Skip already-loaded (TopicStats) and in-flight (null) entries unless retrying errors.
    const pending = names.filter((n) => {
      const v = stats[n];
      if (v === undefined) return true;
      if (retry && v === 'err') return true;
      return false;
    });
    if (pending.length === 0) return;
    const token = ++statsToken;
    // Mark as in-flight (null) so parallel calls don't duplicate fetches.
    stats = { ...stats, ...Object.fromEntries(pending.map((n) => [n, null as null])) };
    try {
      // Single batch call to replace per-topic N×1 round-trips.
      const result = await api.post<Record<string, TopicStats>>(
        `/brokers/clusters/${cluster.id}/topics/stats`,
        { names: pending },
      );
      if (token !== statsToken) return;
      const update: Record<string, TopicStats | 'err'> = {};
      for (const name of pending) {
        update[name] = result[name] ?? ('err' as const);
      }
      stats = { ...stats, ...update };
    } catch {
      // Batch endpoint unavailable (older server); fall back to per-topic calls.
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
            if (token === statsToken) stats = { ...stats, [name]: 'err' as const };
          }
        }
      };
      await Promise.all(Array.from({ length: STATS_CONCURRENCY }, worker));
    }
  }

  function retryStats() {
    void fillStats(visible.map((t) => t.name), true);
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
    if (s === 'err') return '—';
    return s.message_count.toLocaleString();
  }

  // Per-topic production rate (msg/s), derived server-side from the high-watermark
  // delta between consecutive `topics/stats` polls. `—` until a second sample
  // lands (or when the count is unavailable).
  function rateText(name: string): string {
    const s = stats[name];
    if (s == null || s === 'err' || s.msg_per_sec == null) return '—';
    const r = s.msg_per_sec;
    if (r < 0.05) return '0';
    if (r < 10) return `${r.toFixed(1)}/s`;
    return `${Math.round(r).toLocaleString()}/s`;
  }

  // Periodically re-fetch the visible topics' stats so the msg/s rate refreshes
  // (the backend needs two samples to compute it). One batch call per tick.
  async function refreshRates() {
    if (loading || selected) return;
    const names = visible.map((t) => t.name).filter((n) => {
      const v = stats[n];
      return v != null && v !== 'err';
    });
    if (names.length === 0) return;
    try {
      const result = await api.post<Record<string, TopicStats>>(
        `/brokers/clusters/${cluster.id}/topics/stats`,
        { names },
      );
      stats = { ...stats, ...result };
    } catch {
      // Transient — keep the last values; the next tick retries.
    }
  }

  $effect(() => {
    void cluster.id;
    const h = setInterval(() => void refreshRates(), 5000);
    return () => clearInterval(h);
  });

  // True when any visible topic's stats failed — show the retry button.
  const hasStatErrors = $derived(visible.some((t) => stats[t.name] === 'err'));

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
      {#if hasStatErrors}
        <button class="btn small" onclick={retryStats} title="Retry failed message-count fetches">
          <Icon name="refreshCw" size={12} /> Retry counts
        </button>
      {/if}
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
              <th class="num" title="Production rate (messages/second), from the high-watermark delta between polls">Msg/s</th>
              <th class="num">Size</th>
            </tr>
          </thead>
          <tbody>
            {#each visible as t (t.name)}
              <tr onclick={() => (selected = t.name)}>
                <td class="tname" class:internal={t.internal}>{t.name}</td>
                <td class="num">{t.partitions}</td>
                <td class="num">{t.replication_factor}</td>
                <td
                  class="num"
                  class:err-cell={stats[t.name] === 'err'}
                  title={stats[t.name] === 'err' ? 'Count unavailable — click "Retry counts" to try again' : undefined}
                >{countText(t.name)}</td>
                <td class="num" title="Production rate (msg/s) — high-watermark delta between polls">{rateText(t.name)}</td>
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
    text-align: start;
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
    text-align: end;
    width: 110px;
  }
  table.grid td {
    padding: 7px 14px;
    border-bottom: 1px solid var(--border);
  }
  table.grid td.num {
    text-align: end;
    font-variant-numeric: tabular-nums;
  }
  table.grid td.err-cell {
    color: var(--text-dim);
    cursor: help;
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
