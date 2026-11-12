<script lang="ts">
  // History side tab → "All connections": the recent query history of every
  // database connection (history-all.ts), newest first, with a connection
  // filter. Opening a row switches to its connection and opens the statement
  // in a new tab (never runs it). Connections this user cannot read are
  // skipped and counted, never shown as an error for the whole list.
  import { onMount } from 'svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { api } from '../../lib/api/client';
  import type { DbHistoryEntry } from '../../lib/api/types';
  import { database } from '../../lib/stores/database.svelte';
  import { toastError } from '../../lib/toastError';
  import { stmtPreview } from '../../lib/database/sql-util';
  import { rel } from '../../lib/stores/now.svelte';
  import { plural } from '../../lib/plural';
  import { ALL_HISTORY_PER_CONN, historyConnections, mergeHistory, type AllHistoryRow, type ConnLike } from './history-all';

  let { search = '' }: { search?: string } = $props();

  let lists = $state.raw<{ conn: ConnLike; rows: DbHistoryEntry[] }[]>([]);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let skipped = $state(0);
  let connId = $state('');

  async function load(): Promise<void> {
    if (loading) return;
    loading = true;
    const conns = historyConnections(database.connections);
    const got = await Promise.allSettled(
      conns.map((c) => api.get<DbHistoryEntry[]>(`/connections/${c.id}/db/history?limit=${ALL_HISTORY_PER_CONN}`)),
    );
    lists = conns.flatMap((conn, i) => {
      const r = got[i];
      return r.status === 'fulfilled' ? [{ conn, rows: r.value }] : [];
    });
    skipped = got.filter((r) => r.status === 'rejected').length;
    error = conns.length > 0 && lists.length === 0 ? 'No connection’s history could be read.' : null;
    loading = false;
  }
  // Once per mount (Refresh reloads): load() reads its own state.
  onMount(() => void load());

  const rows = $derived(mergeHistory(lists, connId, search));

  async function open(h: AllHistoryRow): Promise<void> {
    try {
      let statement = h.statement;
      if (h.statement_len !== undefined && h.statement_len > h.statement.length) {
        statement = (await api.get<DbHistoryEntry>(`/connections/${h.connection_id}/db/history/${encodeURIComponent(h.id)}`)).statement;
      }
      if (database.selectedConnId !== h.connection_id) await database.openConnection(h.connection_id);
      if (database.selectedConnId === h.connection_id) database.newTab(statement);
    } catch (e) {
      toastError('Couldn’t open this history entry', e);
    }
  }
</script>

<div class="ha-bar">
  <select class="ha-conn" bind:value={connId} aria-label="Show history of" title="Show history of one connection or all">
    <option value="">All connections</option>
    {#each lists as l (l.conn.id)}<option value={l.conn.id}>{l.conn.name}</option>{/each}
  </select>
  <button class="icon-btn" type="button" onclick={() => void load()} disabled={loading} aria-label="Refresh history" title="Refresh history"><Icon name="refresh" size={12} /></button>
</div>
{#if skipped > 0 && lists.length > 0}
  <p class="ha-note">{plural(skipped, 'connection')} could not be read and {skipped === 1 ? 'is' : 'are'} not listed.</p>
{/if}
<LoadState what="query history" variant="compact" {loading} {error} empty={lists.length === 0} onretry={() => void load()}>
  {#snippet emptyView()}<EmptyState icon="clock" title="No query history yet" body="Queries you run on any connection show up here." />{/snippet}
  {#if rows.length === 0}
    <EmptyState icon="search" title="No matches" body="No history matches this filter." />
  {:else}
    {#each rows as h (h.connection_id + h.id)}
      <button class="ha-row" class:bad={!h.ok} onclick={() => void open(h)} title={h.error ? stmtPreview(h.error, 1000) : stmtPreview(h.statement, 1000)} data-testid="db-history-all-row">
        <span class="ha-dot" class:ok={h.ok}></span>
        <span class="ha-main">
          <span class="ha-stmt mono" dir="ltr">{stmtPreview(h.statement)}</span>
          <span class="ha-meta">{h.connName} · {h.ok ? `${h.row_count}r` : 'err'} · {rel(h.created_at, { compact: true })}</span>
        </span>
      </button>
    {/each}
  {/if}
</LoadState>

<style>
  .ha-bar {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 6px 6px;
  }
  .ha-conn {
    flex: 1;
    min-width: 0;
    font-size: var(--fs-s);
  }
  .ha-note {
    margin: 0;
    padding: 0 8px 6px;
    color: var(--text-dim);
    font-size: var(--fs-xs);
  }
  .ha-row {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    border: none;
    background: transparent;
    border-radius: var(--radius-s);
    cursor: pointer;
    color: var(--text);
    text-align: start;
    padding: 4px 8px;
  }
  .ha-row:hover {
    background: var(--hover);
  }
  .ha-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--status-exited);
    flex-shrink: 0;
  }
  .ha-dot.ok {
    background: var(--status-working);
  }
  .ha-main {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-width: 0;
  }
  .ha-stmt,
  .ha-meta {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ha-stmt {
    font-size: var(--fs-xs);
  }
  .ha-meta {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
</style>
