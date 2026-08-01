<script lang="ts">
  // Lazy recursive schema tree (databases → tables/views → columns; keyspaces →
  // keys; collections → fields). Mirrors CollectionsTree: chevron expand, indent
  // by depth, an icon per node kind, dimmed `detail`. Clicking a leaf object
  // opens its Structure; right-click offers "Explain with agent".
  import Icon from '../../lib/components/Icon.svelte';
  import RedisKeyFilter from './RedisKeyFilter.svelte';
  import { database } from '../../lib/stores/database.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import type { DbNodeKind, SchemaNode } from '../../lib/api/types';

  // Top-level schema search / filter. Client-side, this only filters ROOT nodes
  // and already-cached subtrees — it can never find a table inside a schema you
  // have not opened. `searchObjects` (server-side, catalog-backed) is what makes
  // that possible; the scope picker chooses between them.
  let schemaFilter = $state('');
  let searchTimer: ReturnType<typeof setTimeout> | null = null;

  // Redis has no object namespace to search — its key filter already covers it.
  const searchable = $derived(database.queryLanguage !== 'redis');
  const hits = $derived(database.objectSearchHits);

  /** Debounced server-side lookup — one request per pause, not per keystroke. */
  function onSearchInput(): void {
    if (searchTimer) clearTimeout(searchTimer);
    const q = schemaFilter;
    if (!q.trim()) {
      database.clearObjectSearch();
      return;
    }
    if (!searchable) return; // Redis: the client-side key filter already covers it
    searchTimer = setTimeout(() => {
      void database.searchObjects(q, database.activeDb ?? undefined);
    }, 220);
  }

  function setScope(scope: 'schema' | 'all'): void {
    database.objectSearchScope = scope;
    if (schemaFilter.trim() && searchable) {
      void database.searchObjects(schemaFilter, database.activeDb ?? undefined);
    }
  }

  function clearSearch(): void {
    schemaFilter = '';
    if (searchTimer) clearTimeout(searchTimer);
    database.clearObjectSearch();
  }

  /** Open a search hit exactly as a browsed node would open. */
  function openHit(hit: { path: string; name: string; kind: DbNodeKind }): void {
    void database.openObject({
      id: hit.path,
      label: hit.name,
      kind: hit.kind,
      has_children: true,
    });
  }

  // Node kinds that, when clicked, open the Structure view (vs. just expanding).
  const OBJECT_KINDS = new Set<DbNodeKind>([
    'table',
    'view',
    'procedure',
    'function',
    'collection',
    'key',
  ]);

  function iconFor(kind: DbNodeKind): string {
    switch (kind) {
      case 'database':
      case 'schema':
        return 'db';
      case 'table':
        return 'grid';
      case 'view':
        return 'eye';
      case 'procedure':
        return 'procedure';
      case 'function':
        return 'function';
      case 'column':
      case 'field':
        return 'dot';
      case 'index':
        return 'key';
      case 'collection':
        return 'box';
      case 'keyspace':
      case 'key_namespace':
        return 'folder';
      case 'key':
        return 'key';
      default:
        return 'file';
    }
  }

  function onClick(node: SchemaNode): void {
    if (OBJECT_KINDS.has(node.kind)) {
      void database.openObject(node);
    } else if (node.kind === 'database') {
      // Clicking a database makes it the active one (queries scope to it, like
      // Workbench's bold default schema) and expands it.
      database.setActiveDb(node.label);
      if (node.has_children) void database.expand(node);
    } else if (node.kind === 'keyspace') {
      // A Redis keyspace (db0/db1/…) IS the active DB — commands run against it.
      // Clicking selects it (so it's clear which DB you're on) and expands.
      database.setActiveDb(node.id);
      if (node.has_children) void database.expand(node);
    } else if (node.has_children) {
      void database.expand(node);
    }
  }

  function explain(node: SchemaNode): void {
    void database.explainWithAgent(
      `Database object: ${node.label} (${node.kind})\nPath: ${node.id}`,
      `Explain this ${node.kind} and how it is typically used.`,
      `Explain ${node.label}`,
    );
  }

  async function copyName(node: SchemaNode): Promise<void> {
    try {
      await navigator.clipboard.writeText(node.label);
    } catch {
      /* clipboard unavailable — ignore */
    }
  }

  // Pretty number for the "Select Rows (Limit N)" label.
  const fmtNum = (n: number): string => n.toLocaleString();

  /**
   * Return true when a node or any of its cached descendants match the filter.
   * Only inspects already-expanded subtrees; nodes whose children haven't been
   * loaded yet are always included (we don't fetch on behalf of the filter).
   */
  function nodeMatchesFilter(node: SchemaNode, q: string): boolean {
    if (node.label.toLowerCase().includes(q)) return true;
    const kids = database.childrenOf(node.id);
    if (kids) {
      return kids.some((k) => nodeMatchesFilter(k, q));
    }
    return false;
  }

  const filteredRoot = $derived.by(() => {
    const q = schemaFilter.trim().toLowerCase();
    if (!q) return database.schemaRoot;
    return database.schemaRoot.filter((n) => nodeMatchesFilter(n, q));
  });

  function showMenu(e: MouseEvent, node: SchemaNode): void {
    const isObject = OBJECT_KINDS.has(node.kind);
    const isSqlTable =
      database.capabilities?.sql === true && (node.kind === 'table' || node.kind === 'view');
    // Stored procedure / function — SQL routines that carry a SHOW CREATE DDL.
    const isSqlRoutine =
      database.capabilities?.sql === true &&
      (node.kind === 'procedure' || node.kind === 'function');
    const isMongoCollection = node.kind === 'collection';

    const items = [];

    // Database node: set/clear the active database (queries scope to it). For
    // Mongo this is required so `db.<coll>...` resolves to the right database.
    if (node.kind === 'database') {
      if (database.activeDb === node.label) {
        items.push({ label: 'Clear active database', icon: 'db', action: () => database.setActiveDb(null) });
      } else {
        items.push({ label: 'Set as active database', icon: 'db', action: () => database.setActiveDb(node.label) });
      }
      items.push({ separator: true });
    }

    // Redis keyspace (db0/db1/…): the active DB commands run against.
    if (node.kind === 'keyspace' && database.activeDb !== node.id) {
      items.push({ label: 'Set as active database', icon: 'db', action: () => database.setActiveDb(node.id) });
      items.push({ separator: true });
    }

    // Redis key: read its value with the TYPE-correct command (GET only works on
    // strings; a hash needs HGETALL, a list LRANGE, …). This is the reliable way
    // to query a key — no guessing the command or retyping the full key name.
    if (node.kind === 'key') {
      const verb = database.redisReadCommand(node.detail, '').trim().split(' ')[0];
      items.push({
        label: `Get value (${verb})`,
        icon: 'play',
        action: () => void database.getRedisValue(node, { run: true }),
      });
      items.push({
        label: 'Send to editor',
        icon: 'send',
        action: () => void database.getRedisValue(node, { run: false }),
      });
      items.push({ separator: true });
    }

    // Workbench-style data actions for SQL tables/views.
    if (isSqlTable) {
      items.push({
        label: `Select Rows (Limit ${fmtNum(database.rowLimit)})`,
        icon: 'play',
        action: () => void database.selectRows(node),
      });
      items.push({
        label: 'Send to SQL Editor',
        icon: 'send',
        action: () => void database.sendSelectToEditor(node),
      });
      // Import a local file (CSV/TSV/NDJSON/JSON) into this table, prefilling its
      // name. Routes through the same guarded write path as a query.
      if (node.kind === 'table') {
        items.push({
          label: 'Import into…',
          icon: 'arrowDown',
          action: () => database.openImportDialog(node),
        });
      }
      items.push({ separator: true });
    }

    // Data actions for Mongo collections: find({}) capped at the row limit.
    if (isMongoCollection) {
      items.push({
        label: `Find Rows (Limit ${fmtNum(database.rowLimit)})`,
        icon: 'play',
        action: () => void database.findRows(node),
      });
      items.push({
        label: 'Send to Editor',
        icon: 'send',
        action: () => void database.sendFindToEditor(node),
      });
      // Import a local file into this collection (insertMany batches through the
      // same guarded write path), prefilling its name — parity with SQL tables.
      items.push({
        label: 'Import into…',
        icon: 'arrowDown',
        action: () => database.openImportDialog(node),
      });
      items.push({ separator: true });
    }

    if (isObject) {
      items.push({ label: 'Open structure', icon: 'eye', action: () => database.openObject(node) });
    }
    items.push({ label: 'Explain with agent', icon: 'zap', action: () => explain(node) });

    items.push({ separator: true });
    items.push({ label: 'Copy name', icon: 'file', action: () => void copyName(node) });
    // CREATE statement (DDL) — SQL tables/views AND stored procedures/functions
    // (MySQL + ClickHouse via SHOW CREATE TABLE/VIEW/PROCEDURE/FUNCTION);
    // Mongo/Redis have no DDL.
    if (isSqlTable || isSqlRoutine) {
      items.push({
        label: 'Copy create statement',
        icon: 'file',
        action: () => void database.copyCreateStatement(node),
      });
    }
    if (node.has_children) {
      items.push({ label: 'Refresh', icon: 'refresh', action: () => void database.refreshSchema() });
    }

    // Destructive SQL actions — pre-fill a tab (NOT auto-run); user reviews + runs.
    if (isSqlTable) {
      items.push({ separator: true });
      if (node.kind === 'table') {
        items.push({
          label: 'Truncate Table…',
          icon: 'trash',
          danger: true,
          action: () => void database.truncateTable(node),
        });
      }
      items.push({
        label: node.kind === 'view' ? 'Drop View…' : 'Drop Table…',
        icon: 'trash',
        danger: true,
        action: () => void database.dropObject(node),
      });
    }

    ctxMenu.show(e, items);
  }
</script>

<div class="schema-tree">
  {#if !database.schemaLoading && database.schemaRoot.length > 0}
    <div class="tree-search">
      <Icon name="search" size={11} />
      <input
        class="tree-search-input"
        type="text"
        bind:value={schemaFilter}
        oninput={onSearchInput}
        placeholder={searchable ? 'Find a table…' : 'Filter schema…'}
        spellcheck="false"
        aria-label="Find an object"
      />
      {#if schemaFilter}
        <button class="tree-search-clear" onclick={clearSearch} aria-label="Clear filter">
          <Icon name="x" size={10} />
        </button>
      {/if}
    </div>
    <div class="tree-opts">
      {#if searchable}
        <select
          class="scope-pick"
          value={database.objectSearchScope}
          onchange={(e) => setScope(e.currentTarget.value as 'schema' | 'all')}
          aria-label="Search scope"
          title={database.selectedConn?.kind === 'postgres'
            ? 'Postgres cannot cross databases — “All schemas” covers every schema in the connected database'
            : 'Where to look for the object'}
        >
          <option value="schema">This schema</option>
          <option value="all">All schemas</option>
        </select>
      {/if}
      <label class="counts-toggle" title="Show approximate row counts. Off by default — collecting the estimate is what makes expanding a large server slow.">
        <input
          type="checkbox"
          checked={database.showCounts}
          onchange={(e) => database.setShowCounts(e.currentTarget.checked)}
        />
        Counts
      </label>
    </div>
  {/if}
  {#if database.schemaLoading || database.activeConnStatus?.phase === 'connecting'}
    <div class="tree-loading">
      <Icon name="refresh" size={13} />
      <span>Loading schema…</span>
    </div>
  {:else if database.activeConnStatus?.phase === 'error'}
    <div class="tree-error" role="status" aria-live="polite">
      <div class="tree-error-head"><Icon name="x" size={12} />Couldn't connect</div>
      <div class="tree-error-msg">{database.activeConnStatus.error}</div>
      <button class="tree-error-retry" onclick={() => database.retryConnection()}>
        <Icon name="refresh" size={11} />Retry
      </button>
    </div>
  {:else if database.schemaRoot.length === 0}
    <div class="tree-empty">No objects. Test the connection or refresh.</div>
  {:else if hits !== null}
    <!-- Server-side results: a flat list, each hit labelled with its schema so
         you can tell two same-named tables apart. -->
    {#if database.objectSearching && hits.length === 0}
      <div class="tree-loading"><Icon name="refresh" size={13} /><span>Searching…</span></div>
    {:else if hits.length === 0}
      <div class="tree-empty">
        No object matching "{schemaFilter}"{database.objectSearchScope === 'schema'
          ? ' in this schema — try All schemas.'
          : '.'}
      </div>
    {:else}
      <div class="hit-head">
        {hits.length}{database.objectSearchTruncated ? '+' : ''} match{hits.length === 1 ? '' : 'es'}
        {#if database.objectSearchScope === 'all' && database.objectSearchScanned > 0}
          <span class="hit-scan">· {database.objectSearchScanned} schemas scanned</span>
        {/if}
      </div>
      {#each hits as hit (hit.path)}
        <button class="hit" onclick={() => openHit(hit)} title={hit.path}>
          <span class="node-icon {hit.kind}"><Icon name={iconFor(hit.kind)} size={12} /></span>
          <span class="hit-name">{hit.name}</span>
          <span class="hit-schema">{hit.schema}</span>
        </button>
      {/each}
      {#if database.objectSearchTruncated}
        <div class="tree-empty">More matches exist — narrow the search.</div>
      {/if}
    {/if}
  {:else if filteredRoot.length === 0}
    <div class="tree-empty">No match for "{schemaFilter}".</div>
  {:else}
    {#each filteredRoot as node (node.id)}
      {@render treeNode(node, 0)}
    {/each}
  {/if}
</div>

{#snippet treeNode(node: SchemaNode, depth: number)}
  {@const open = database.isExpanded(node.id)}
  {@const selected = database.selectedObjectPath === node.id}
  <!-- svelte-ignore a11y_no_static_element_interactions a11y_click_events_have_key_events -->
  <div
    class="node"
    class:selected
    class:active-db={(node.kind === 'database' && node.label === database.activeDb) ||
      (node.kind === 'keyspace' && node.id === database.activeDb)}
    style="padding-inline-start: {depth * 13 + 4}px"
    oncontextmenu={(e) => showMenu(e, node)}
  >
    {#if node.has_children}
      <button class="caret" onclick={() => database.expand(node)} aria-label="Toggle">
        {#if database.isLoadingNode(node.id)}
          <span class="spin"><Icon name="refresh" size={10} /></span>
        {:else}
          <Icon name={open ? 'chevronDown' : 'chevronRight'} size={11} />
        {/if}
      </button>
    {:else}
      <span class="caret-spacer"></span>
    {/if}
    <span class="node-icon {node.kind}"><Icon name={iconFor(node.kind)} size={12} /></span>
    <button
      class="node-label"
      onclick={() => onClick(node)}
      title={node.detail ? `${node.label} — ${node.detail}` : node.label}
    >
      <span class="nl-text ellipsis">{node.label}</span>
      {#if node.detail}<span class="nl-detail ellipsis">{node.detail}</span>{/if}
    </button>
  </div>
  {#if open}
    {#if node.kind === 'keyspace'}
      <RedisKeyFilter {node} {depth} />
    {/if}
    {@const children = database.childrenOf(node.id)}
    {#if children}
      {#each children as child (child.id)}
        {@render treeNode(child, depth + 1)}
      {:else}
        <div class="node-empty" style="padding-inline-start: {(depth + 1) * 13 + 18}px">empty</div>
      {/each}
    {/if}
  {/if}
{/snippet}

<style>
  .schema-tree {
    display: flex;
    flex-direction: column;
    gap: 0;
    min-width: 0;
  }
  .tree-loading,
  .tree-empty {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 6px;
    font-size: 11.5px;
    color: var(--text-dim);
  }
  .tree-error {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 6px;
    padding: 10px 8px;
    font-size: 11.5px;
  }
  .tree-error-head {
    display: flex;
    align-items: center;
    gap: 5px;
    color: var(--status-exited);
    font-weight: 600;
  }
  .tree-error-msg {
    color: var(--text-dim);
    line-height: 1.4;
    word-break: break-word;
  }
  .tree-error-retry {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 3px 9px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text);
    cursor: pointer;
    font-size: 11.5px;
  }
  .tree-error-retry:hover {
    background: var(--surface);
    border-color: color-mix(in srgb, var(--accent) 40%, transparent);
  }
  .node {
    display: flex;
    align-items: center;
    gap: 4px;
    height: 25px;
    padding-inline-end: 4px;
    border-radius: var(--radius-s);
    min-width: 0;
  }
  .node:hover {
    background: color-mix(in srgb, var(--text-dim) 9%, transparent);
  }
  .node.selected {
    background: color-mix(in srgb, var(--accent) 15%, transparent);
  }
  /* Active database = bold, like Workbench's default schema. */
  .node.active-db .nl-text {
    font-weight: 700;
    color: var(--text);
  }
  .node.active-db .node-icon {
    color: var(--accent);
  }
  .caret {
    display: grid;
    place-items: center;
    width: 15px;
    height: 15px;
    border: none;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    flex-shrink: 0;
  }
  .caret-spacer {
    width: 15px;
    flex-shrink: 0;
  }
  .spin {
    display: grid;
    place-items: center;
    animation: spin 0.9s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .node-icon {
    display: grid;
    place-items: center;
    flex-shrink: 0;
    color: var(--text-dim);
  }
  .node-icon.table,
  .node-icon.view,
  .node-icon.collection {
    color: var(--accent);
  }
  /* Routines get a muted-accent tone so they read as distinct from data objects. */
  .node-icon.procedure,
  .node-icon.function {
    color: color-mix(in srgb, var(--accent) 45%, var(--text));
  }
  .node-icon.database,
  .node-icon.schema {
    color: color-mix(in srgb, var(--accent) 80%, var(--text));
  }
  .node-label {
    display: flex;
    align-items: baseline;
    gap: 7px;
    min-width: 0;
    flex: 1;
    border: none;
    background: transparent;
    color: var(--text);
    cursor: pointer;
    text-align: start;
    height: 100%;
    padding: 0;
  }
  .nl-text {
    font-size: 12px;
    min-width: 0;
    /* Name is the primary value: it only shrinks as a last resort. */
    flex: 0 1 auto;
  }
  .nl-detail {
    font-size: 10.5px;
    color: var(--text-dim);
    min-width: 0;
    /* Engine/detail is secondary: shrinks (and ellipsises away) ~100× faster
       than the name, so a long engine never crowds out the table name. */
    flex: 0 100 auto;
  }
  .node-empty {
    font-size: 10.5px;
    color: var(--text-dim);
    font-style: italic;
    padding-top: 2px;
    padding-bottom: 2px;
  }
  .ellipsis {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tree-opts {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 8px 5px;
    font-size: 10.5px;
    color: var(--text-dim, #98989f);
  }
  .scope-pick {
    flex: 1;
    min-width: 0;
    font-size: 10.5px;
    padding: 1px 4px;
    background: var(--surface-2, #323238);
    border: 1px solid var(--border, rgba(255, 255, 255, 0.1));
    border-radius: var(--radius-s, 5px);
    color: var(--text, #f2f2f5);
  }
  .counts-toggle {
    display: flex;
    align-items: center;
    gap: 4px;
    cursor: pointer;
    white-space: nowrap;
  }
  .counts-toggle input {
    margin: 0;
  }

  .hit-head {
    padding: 4px 8px;
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-dim, #98989f);
  }
  .hit-scan {
    text-transform: none;
    letter-spacing: 0;
  }
  .hit {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    padding: 3px 8px;
    background: none;
    border: none;
    color: var(--text, #f2f2f5);
    font-size: 11.5px;
    text-align: left;
    cursor: pointer;
  }
  .hit:hover {
    background: color-mix(in srgb, var(--accent, #0a84ff) 16%, transparent);
  }
  .hit-name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .hit-schema {
    flex: 0 0 auto;
    max-width: 45%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-dim, #98989f);
    font-size: 10px;
  }

  /* Schema-tree filter bar */
  .tree-search {
    display: flex;
    align-items: center;
    gap: 5px;
    padding: 4px 6px;
    border-bottom: 1px solid color-mix(in srgb, var(--border) 60%, transparent);
    color: var(--text-dim);
    flex-shrink: 0;
  }
  .tree-search-input {
    flex: 1;
    border: none;
    background: transparent;
    color: var(--text);
    font-size: 11.5px;
    outline: none;
    min-width: 0;
  }
  .tree-search-input::placeholder {
    color: var(--text-dim);
  }
  .tree-search-clear {
    display: grid;
    place-items: center;
    border: none;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    padding: 0;
    flex-shrink: 0;
  }
  .tree-search-clear:hover {
    color: var(--text);
  }

  /* Phone: larger node rows + readable labels (the desktop sizes are too small
     to tap/read on a device). */
  @media (max-width: 640px) {
    .node {
      height: 36px;
    }
    .nl-text {
      font-size: 14px;
    }
    .nl-detail {
      font-size: 12px;
    }
    .tree-loading,
    .tree-empty {
      font-size: 13.5px;
    }
    .tree-search-input {
      font-size: 14px;
    }
  }
</style>
