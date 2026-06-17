<script lang="ts">
  // DB Explorer page (mirrors ApiPage): left sidebar = connection picker +
  // SchemaTree + a Saved/History switch; main = a tab strip (Query / Builder /
  // Structure / Dashboards) over the active view.
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import SchemaTree from './SchemaTree.svelte';
  import QueryEditor from './QueryEditor.svelte';
  import QueryBuilder from './QueryBuilder.svelte';
  import StructureView from './StructureView.svelte';
  import Dashboards from './Dashboards.svelte';
  import ConnectionForm from '../connections/ConnectionForm.svelte';
  import { database, engineGlyph, type DbMainTab } from '../../lib/stores/database.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { api } from '../../lib/api/client';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import type { Connection, ConnectionKind } from '../../lib/api/types';

  // DB connections are created/managed here (hidden from the Connections page).
  const DB_KINDS: ConnectionKind[] = ['mysql', 'redis', 'mongodb', 'clickhouse'];
  let connFormOpen = $state(false);
  let editingConn = $state<Connection | null>(null);

  function newConnection(): void {
    editingConn = null;
    connFormOpen = true;
  }
  function editConnection(c: Connection): void {
    editingConn = c;
    connFormOpen = true;
  }
  async function onConnSaved(c: Connection): Promise<void> {
    connFormOpen = false;
    await database.loadConnections();
    void database.openConnection(c.id);
  }
  async function deleteConnection(c: Connection): Promise<void> {
    if (
      !(await confirmer.ask(`Delete connection “${c.name}”? Its Keychain secret is removed too.`, {
        title: 'Delete connection',
      }))
    )
      return;
    try {
      await api.del(`/connections/${c.id}`);
      if (database.openConnIds.includes(c.id)) database.closeConnection(c.id);
      await database.loadConnections();
    } catch (e) {
      toasts.error('Delete failed', e instanceof Error ? e.message : String(e));
    }
  }

  // Load connections + workspace-scoped saved/dashboards when the workspace changes.
  $effect(() => {
    if (ws.currentId) {
      void database.loadConnections();
      void database.loadSavedQueries();
      void database.loadDashboards();
    }
  });

  const mainTabs: { id: DbMainTab; label: string; show: () => boolean }[] = [
    { id: 'query', label: 'Query', show: () => true },
    { id: 'builder', label: 'Builder', show: () => database.supportsBuilder },
    { id: 'structure', label: 'Structure', show: () => true },
    { id: 'dashboards', label: 'Dashboards', show: () => true },
  ];
  const visibleTabs = $derived(mainTabs.filter((t) => t.show()));

  // Open connections as top-level tabs (Workbench-style), resolved to their
  // Connection records for name + engine glyph.
  const openConns = $derived(
    database.openConnIds
      .map((id) => database.connections.find((c) => c.id === id))
      .filter((c): c is NonNullable<typeof c> => c != null),
  );

  function fmtAgo(iso: string): string {
    const ms = Date.now() - new Date(iso).getTime();
    const s = Math.floor(ms / 1000);
    if (s < 60) return `${s}s`;
    if (s < 3600) return `${Math.floor(s / 60)}m`;
    if (s < 86400) return `${Math.floor(s / 3600)}h`;
    return `${Math.floor(s / 86400)}d`;
  }
</script>

<div class="db-page">
  <aside class="db-side">
    <!-- Connection picker + management -->
    <div class="conn-head">
      <span class="conn-head-title">Connections</span>
      <button class="icon-btn" onclick={newConnection} aria-label="New connection" title="New connection">
        <Icon name="plus" size={13} />
      </button>
    </div>
    <div class="conn-list">
      {#if database.connections.length === 0}
        <div class="conn-empty">
          No database connections.
          <button class="link" onclick={newConnection}>New connection →</button>
        </div>
      {:else}
        {#each database.connections as c (c.id)}
          <div
            class="conn-row"
            class:active={database.selectedConnId === c.id}
            class:open={database.openConnIds.includes(c.id)}
          >
            <button class="conn-item" onclick={() => database.openConnection(c.id)} title={c.name}>
              <span class="conn-glyph {c.kind}"><Icon name={engineGlyph(c.kind)} size={13} /></span>
              <span class="conn-name ellipsis">{c.name}</span>
              <span class="conn-kind">{c.kind}</span>
            </button>
            <div class="conn-actions">
              <button class="icon-btn" aria-label="Edit connection" title="Edit" onclick={() => editConnection(c)}>
                <Icon name="edit" size={11} />
              </button>
              <button class="icon-btn" aria-label="Delete connection" title="Delete" onclick={() => deleteConnection(c)}>
                <Icon name="trash" size={11} />
              </button>
            </div>
          </div>
        {/each}
      {/if}
    </div>

    {#if database.selectedConnId}
      <!-- Schema / Saved / History switch -->
      <div class="side-switch" role="tablist">
        <button class="ss" class:active={database.sideTab === 'schema'} role="tab" aria-selected={database.sideTab === 'schema'} onclick={() => (database.sideTab = 'schema')}>Schema</button>
        <button class="ss" class:active={database.sideTab === 'saved'} role="tab" aria-selected={database.sideTab === 'saved'} onclick={() => (database.sideTab = 'saved')}>Saved</button>
        <button class="ss" class:active={database.sideTab === 'history'} role="tab" aria-selected={database.sideTab === 'history'} onclick={() => (database.sideTab = 'history')}>History</button>
        {#if database.sideTab === 'schema'}
          <span class="grow"></span>
          <button class="icon-btn" onclick={() => database.refreshSchema()} title="Refresh schema" aria-label="Refresh schema"><Icon name="refresh" size={12} /></button>
        {/if}
      </div>

      <div class="side-body">
        {#if database.sideTab === 'schema'}
          <SchemaTree />
        {:else if database.sideTab === 'saved'}
          {#if database.savedQueries.length === 0}
            <div class="list-empty">No saved queries. Save one from the Query tab.</div>
          {:else}
            {#each database.savedQueries as q (q.id)}
              <div class="saved-row">
                <button class="saved-open" onclick={() => database.openSavedQuery(q)} title={q.statement}>
                  <Icon name="file" size={12} />
                  <span class="ellipsis">{q.name}</span>
                </button>
                <button class="icon-btn row-del" onclick={() => database.deleteSavedQuery(q.id)} aria-label="Delete saved query"><Icon name="trash" size={11} /></button>
              </div>
            {/each}
          {/if}
        {:else}
          {#if database.history.length === 0}
            <div class="list-empty">No query history yet.</div>
          {:else}
            {#each database.history as h (h.id)}
              <button class="hist-row" class:bad={!h.ok} onclick={() => database.openHistory(h)} title={h.error ?? h.statement}>
                <span class="hist-dot" class:ok={h.ok}></span>
                <span class="hist-stmt ellipsis mono">{h.statement}</span>
                <span class="hist-meta">{h.ok ? `${h.row_count}r` : 'err'} · {fmtAgo(h.created_at)}</span>
              </button>
            {/each}
          {/if}
        {/if}
      </div>
    {/if}
  </aside>

  <div class="db-main">
    {#if !database.selectedConnId}
      <EmptyState
        icon="db"
        title="Pick a database connection"
        body={database.connections.length === 0
          ? 'No MySQL, Redis, MongoDB or ClickHouse connections in this workspace yet.'
          : 'Choose a connection on the left to browse its schema and run queries.'}
        actionLabel={database.connections.length === 0 ? 'New connection' : undefined}
        onaction={database.connections.length === 0 ? newConnection : undefined}
      />
    {:else}
      <!-- Top-level connection tabs (one per open connection) -->
      <div class="conn-tabs" role="tablist" aria-label="Open connections">
        {#each openConns as c (c.id)}
          <div class="conn-tab" class:active={database.selectedConnId === c.id} role="tab" aria-selected={database.selectedConnId === c.id}>
            <button class="conn-tab-main" onclick={() => database.openConnection(c.id)} title={c.name}>
              <span class="conn-tab-glyph {c.kind}"><Icon name={engineGlyph(c.kind)} size={12} /></span>
              <span class="conn-tab-name ellipsis">{c.name}</span>
            </button>
            <button
              class="conn-tab-close"
              onclick={(e) => {
                e.stopPropagation();
                database.closeConnection(c.id);
              }}
              aria-label="Close connection tab"
              title="Close"
            >
              <Icon name="x" size={11} />
            </button>
          </div>
        {/each}
      </div>

      <div class="main-tabs">
        {#each visibleTabs as t (t.id)}
          <button class="mt" class:active={database.mainTab === t.id} role="tab" aria-selected={database.mainTab === t.id} onclick={() => (database.mainTab = t.id)}>
            {t.label}
          </button>
        {/each}
        <span class="grow"></span>
        <div class="conn-status">
          {#if database.capabilities}
            <span class="cap-chip mono" title="Engine">{database.capabilities.engine}</span>
          {/if}
          <button class="btn small ghost" onclick={() => database.testConnection()} disabled={database.testing}>
            <Icon name="plug" size={11} />{database.testing ? 'Testing…' : 'Test'}
          </button>
          {#if database.testResult}
            <span class="test-dot" class:ok={database.testResult.ok} title={database.testResult.message}></span>
          {/if}
        </div>
      </div>

      <div class="main-body">
        {#if database.mainTab === 'query'}
          <QueryEditor />
        {:else if database.mainTab === 'builder'}
          <QueryBuilder />
        {:else if database.mainTab === 'structure'}
          <StructureView />
        {:else}
          <Dashboards />
        {/if}
      </div>
    {/if}
  </div>
</div>

{#if connFormOpen}
  <ConnectionForm
    existing={editingConn}
    kinds={DB_KINDS}
    onclose={() => (connFormOpen = false)}
    onsaved={onConnSaved}
  />
{/if}

<style>
  .db-page {
    height: 100%;
    display: flex;
    min-height: 0;
  }
  .db-side {
    width: 280px;
    flex-shrink: 0;
    border-right: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .conn-list {
    display: flex;
    flex-direction: column;
    gap: 1px;
    padding: 10px 8px;
    border-bottom: 1px solid var(--border);
    max-height: 35%;
    overflow-y: auto;
  }
  .conn-empty,
  .list-empty {
    font-size: 11.5px;
    color: var(--text-dim);
    padding: 8px 6px;
    line-height: 1.5;
  }
  .link {
    border: none;
    background: none;
    color: var(--accent);
    cursor: pointer;
    font-size: 11.5px;
    padding: 0;
  }
  .conn-item {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 30px;
    padding: 0 8px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text);
    cursor: pointer;
    text-align: left;
  }
  .conn-item:hover {
    background: color-mix(in srgb, var(--text-dim) 10%, transparent);
  }
  .conn-row.open:not(.active) .conn-item {
    background: color-mix(in srgb, var(--text-dim) 7%, transparent);
  }
  .conn-row.active .conn-item {
    background: color-mix(in srgb, var(--accent) 15%, transparent);
  }
  .conn-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 6px 8px 2px;
  }
  .conn-head-title {
    font-size: 10.5px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-dim);
  }
  .conn-row {
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .conn-row .conn-item {
    flex: 1;
    min-width: 0;
  }
  .conn-actions {
    display: flex;
    gap: 1px;
    flex-shrink: 0;
    opacity: 0;
    padding-right: 2px;
  }
  .conn-row:hover .conn-actions {
    opacity: 1;
  }
  .conn-glyph {
    display: grid;
    place-items: center;
    flex-shrink: 0;
    color: var(--text-dim);
  }
  .conn-glyph.mysql,
  .conn-glyph.clickhouse {
    color: var(--accent);
  }
  .conn-glyph.redis {
    color: #d2691e;
  }
  .conn-glyph.mongodb {
    color: var(--status-working);
  }
  .conn-row.active .conn-item .conn-glyph {
    color: var(--accent);
  }
  .conn-name {
    flex: 1;
    min-width: 0;
    font-size: 12.5px;
    font-weight: 500;
  }
  .conn-kind {
    font-size: 9.5px;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    color: var(--text-dim);
  }
  .side-switch {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 8px 8px 6px;
    border-bottom: 1px solid var(--border);
  }
  .ss {
    height: 24px;
    padding: 0 9px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font-size: 11.5px;
    font-weight: 500;
    cursor: pointer;
  }
  .ss:hover {
    background: var(--surface-2);
  }
  .ss.active {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    color: var(--accent);
  }
  .side-body {
    flex: 1;
    overflow-y: auto;
    overflow-x: hidden;
    padding: 8px;
    min-height: 0;
  }
  .saved-row,
  .hist-row {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    border: none;
    background: transparent;
    border-radius: var(--radius-s);
    cursor: pointer;
    color: var(--text);
    text-align: left;
    padding: 0 6px;
  }
  .saved-row {
    padding: 0;
  }
  .saved-open {
    flex: 1;
    display: flex;
    align-items: center;
    gap: 7px;
    min-width: 0;
    border: none;
    background: transparent;
    color: var(--text);
    cursor: pointer;
    height: 28px;
    padding: 0 6px;
    border-radius: var(--radius-s);
    font-size: 12px;
  }
  .saved-row:hover,
  .hist-row:hover {
    background: color-mix(in srgb, var(--text-dim) 9%, transparent);
  }
  .row-del {
    opacity: 0;
  }
  .saved-row:hover .row-del {
    opacity: 1;
  }
  .hist-row {
    height: 32px;
  }
  .hist-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--status-exited);
    flex-shrink: 0;
  }
  .hist-dot.ok {
    background: var(--status-working);
  }
  .hist-stmt {
    flex: 1;
    min-width: 0;
    font-size: 11px;
  }
  .hist-meta {
    font-size: 10px;
    color: var(--text-dim);
    flex-shrink: 0;
    font-variant-numeric: tabular-nums;
  }
  .db-main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  /* Top-level connection tabs (Workbench-style), above the main tab row. */
  .conn-tabs {
    display: flex;
    align-items: center;
    gap: 3px;
    height: 36px;
    padding: 0 10px;
    border-bottom: 1px solid var(--border);
    background: var(--bg);
    overflow-x: auto;
    scrollbar-width: none;
    flex-shrink: 0;
  }
  .conn-tabs::-webkit-scrollbar {
    display: none;
  }
  .conn-tab {
    display: flex;
    align-items: center;
    height: 26px;
    padding: 0 3px 0 9px;
    border-radius: var(--radius-s);
    border: 1px solid transparent;
    color: var(--text-dim);
    cursor: pointer;
    white-space: nowrap;
    max-width: 200px;
    flex-shrink: 0;
    transition: background 120ms ease-out, color 120ms ease-out;
  }
  .conn-tab:hover {
    background: var(--surface-2);
  }
  .conn-tab.active {
    background: var(--surface);
    border-color: var(--border);
    color: var(--text);
  }
  .conn-tab-main {
    display: flex;
    align-items: center;
    gap: 7px;
    min-width: 0;
    border: none;
    background: transparent;
    color: inherit;
    cursor: pointer;
    font-size: 12.5px;
    font-weight: 500;
    padding: 0;
    height: 100%;
  }
  .conn-tab-glyph {
    display: grid;
    place-items: center;
    flex-shrink: 0;
    color: var(--text-dim);
  }
  .conn-tab-glyph.mysql,
  .conn-tab-glyph.clickhouse {
    color: var(--accent);
  }
  .conn-tab-glyph.redis {
    color: #d2691e;
  }
  .conn-tab-glyph.mongodb {
    color: var(--status-working);
  }
  .conn-tab.active .conn-tab-glyph {
    color: var(--accent);
  }
  .conn-tab-name {
    min-width: 0;
    max-width: 140px;
  }
  .conn-tab-close {
    display: grid;
    place-items: center;
    width: 17px;
    height: 17px;
    margin-left: 5px;
    border: none;
    border-radius: 3px;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    opacity: 0;
    flex-shrink: 0;
    transition: opacity 120ms ease-out, background 120ms ease-out, color 120ms ease-out;
  }
  .conn-tab:hover .conn-tab-close,
  .conn-tab.active .conn-tab-close {
    opacity: 1;
  }
  .conn-tab-close:hover {
    background: color-mix(in srgb, var(--text-dim) 22%, transparent);
    color: var(--text);
  }
  .main-tabs {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 8px 14px 0;
    border-bottom: 1px solid var(--border);
  }
  .mt {
    height: 30px;
    padding: 0 13px;
    border: none;
    background: transparent;
    color: var(--text-dim);
    font-size: 12.5px;
    font-weight: 500;
    cursor: pointer;
    border-bottom: 2px solid transparent;
    margin-bottom: -1px;
  }
  .mt:hover {
    color: var(--text);
  }
  .mt.active {
    color: var(--accent);
    border-bottom-color: var(--accent);
  }
  .conn-status {
    display: flex;
    align-items: center;
    gap: 8px;
    padding-bottom: 4px;
  }
  .cap-chip {
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
    background: var(--surface-2);
    padding: 1px 7px;
    border-radius: 999px;
  }
  .test-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--status-exited);
  }
  .test-dot.ok {
    background: var(--status-working);
  }
  .main-body {
    flex: 1;
    min-height: 0;
    padding: 12px 16px 16px;
    display: flex;
    flex-direction: column;
  }
  .grow {
    flex: 1;
  }
  .ellipsis {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
