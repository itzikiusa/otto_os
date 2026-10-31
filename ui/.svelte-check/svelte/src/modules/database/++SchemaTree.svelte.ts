///<reference types="svelte" />
;
import { ListWindow } from '../../lib/listWindow.svelte';
import { announce } from '../../lib/announce.svelte';
import { rowMenu } from '../../lib/rowMenu';
// Lazy recursive schema tree (databases → tables/views → columns; keyspaces →
// keys; collections → fields). Mirrors CollectionsTree: chevron expand, indent
// by depth, an icon per node kind, dimmed `detail`. Clicking a leaf object
// opens its Structure; right-click offers "Explain with agent".
import { tick, untrack } from 'svelte';
import { viewport } from '../../lib/stores/viewport.svelte';
import Icon, { type IconName } from '../../lib/components/Icon.svelte';
import RedisKeyFilter from './RedisKeyFilter.svelte';
import { database } from '../../lib/stores/database.svelte';
import { ctxMenu } from '../../lib/contextmenu.svelte';
import type { DbNodeKind, ObjectHit, SchemaNode } from '../../lib/api/types';
import { copyTextOrThrow } from '../../lib/clipboard';
function $$render() {
 const nodeRow/*Ωignore_positionΩ*/ = (node: SchemaNode, depth: number, pos: number, size: number)/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
  const open = database.isExpanded(node.id);
  const selected = database.selectedObjectPath === node.id;
   {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('div')));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {                            "class":`node`,"style":`padding-inline-start: ${depth * 13 + 4}px`,"role":`treeitem`,"tabindex":-1,"aria-level":depth + 1,"aria-posinset":pos,"aria-setsize":size,"aria-expanded":node.has_children ? open : undefined,"aria-selected":selected,...__sveltets_2_empty({"data-node-id":node.id}),...__sveltets_2_empty({"data-depth":depth}),"oncontextmenu":(e) => showMenu(e, node),});selected;(node.kind === 'database' && node.label === database.activeDb) ||
      (node.kind === 'keyspace' && node.id === database.activeDb);
    if(node.has_children){
       { svelteHTML.createElement("button", {       "class":`caret`,"tabindex":-1,"onclick":() => expandNode(node),"aria-label":open ? `Collapse ${node.label}` : `Expand ${node.label}`,});
        if(database.isLoadingNode(node.id)){
           { svelteHTML.createElement("span", {     "class":`spinner`,"style":`--spinner-size: 10px`,"aria-hidden":`true`,}); }
        }else{
           { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":open ? 'chevronDown' : 'chevronRight',"size":12,}});}
        }
       }
    }else{
       { svelteHTML.createElement("span", { "class":`caret-spacer`,}); }
    }
     { svelteHTML.createElement("span", { "class":`node-icon ${node.kind}`,}); { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":iconFor(node.kind),"size":12,}});} }
    
     { svelteHTML.createElement("button", {         "class":`node-label`,"tabindex":-1,"onclick":() => onClick(node),"title":node.detail ? `${node.label} — ${node.detail}` : node.label,});
       { svelteHTML.createElement("span", { "class":`nl-text ellipsis`,});node.label; }
      if(node.detail){ { svelteHTML.createElement("span", { "class":`nl-detail ellipsis`,});node.detail; }}
     }
   }}
};return __sveltets_2_any(0)};
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  // Top-level schema search / filter. Client-side, this only filters ROOT nodes
  // and already-cached subtrees — it can never find a table inside a schema you
  // have not opened. `searchObjects` (server-side, catalog-backed) is what makes
  // that possible; the scope picker chooses between them.
  let searchTimer: ReturnType<typeof setTimeout> | null = null;

  // Redis has no object namespace to search — its key filter already covers it.
  const searchable = $derived(database.queryLanguage !== 'redis');
  const hits = $derived(database.objectSearchHits);

  /** Debounced server-side lookup — one request per pause, not per keystroke. */
  function onSearchInput(e: Event): void {
    database.objectSearchQuery = (e.currentTarget as HTMLInputElement).value;
    if (searchTimer) clearTimeout(searchTimer);
    const q = database.objectSearchQuery;
    if (!q.trim()) {
      database.objectSearchHits = null;
      return;
    }
    if (!searchable) return; // Redis: the client-side key filter already covers it
    searchTimer = setTimeout(() => {
      void database.searchObjects(q, database.activeDb ?? undefined);
    }, 220);
  }

  function setScope(scope: 'schema' | 'all'): void {
    database.objectSearchScope = scope;
    if (database.objectSearchQuery.trim() && searchable) {
      void database.searchObjects(database.objectSearchQuery, database.activeDb ?? undefined);
    }
  }

  function clearSearch(): void {
    if (searchTimer) clearTimeout(searchTimer);
    database.clearObjectSearch();
  }

  /** Build the SchemaNode a hit stands for, so hits get the SAME context menu
   *  as a browsed node instead of silently swallowing right-click. */
  function hitNode(hit: ObjectHit): SchemaNode {
    return { id: hit.path, label: hit.name, kind: hit.kind, has_children: true };
  }

  /** Open a search hit exactly as a browsed node would open. */
  function openHit(hit: { path: string; name: string; kind: DbNodeKind }): void {
    openStructure({
      id: hit.path,
      label: hit.name,
      kind: hit.kind,
      has_children: true,
    });
  }

  function openStructure(node: SchemaNode): void {
    const page = treeEl?.closest('.db-page');
    void database.openObject(node);
    // Tablet list/detail navigation gives the structure its usable width. The
    // schema stays mounted, retaining its query and selection for the return.
    if (viewport.isTablet && !database.sidebarCollapsed) {
      database.toggleSidebar();
      void tick().then(() => page?.querySelector<HTMLButtonElement>('.view-switch [aria-selected="true"]')?.focus());
    }
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

  function iconFor(kind: DbNodeKind): IconName {
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

  // ── Expand with inline failure state ───────────────────────────────────────
  // The store's expand() reports a failed child-load only as a toast and folds
  // the node back — track the failure here so a "failed — retry" row renders
  // under the node instead of leaving no trace once the toast fades.
  let failedNodes = $state<Set<string>>(new Set());
  async function expandNode(node: SchemaNode): Promise<void> {
    const wasOpen = database.isExpanded(node.id);
    await database.expand(node);
    if (wasOpen) return; // was a collapse toggle — nothing to verify
    // On failure the store collapses the node back and caches no children.
    const failed = !database.isExpanded(node.id) && !database.childrenOf(node.id);
    const next = new Set(failedNodes);
    if (failed) next.add(node.id);
    else next.delete(node.id);
    failedNodes = next;
  }

  function onClick(node: SchemaNode): void {
    if (OBJECT_KINDS.has(node.kind)) {
      openStructure(node);
    } else if (node.kind === 'database') {
      // Clicking a database makes it the active one (queries scope to it, like
      // Workbench's bold default schema) and expands it.
      database.setActiveDb(node.label);
      if (node.has_children) void expandNode(node);
    } else if (node.kind === 'keyspace') {
      // A Redis keyspace (db0/db1/…) IS the active DB — commands run against it.
      // Clicking selects it (so it's clear which DB you're on) and expands.
      database.setActiveDb(node.id);
      if (node.has_children) void expandNode(node);
    } else if (node.has_children) {
      void expandNode(node);
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
      await copyTextOrThrow(node.label);
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

  // The filter the TREE applies. The box (and the debounced server search)
  // follow every keystroke, but walking every cached subtree per key is what
  // made typing lag with thousands of cached nodes: past FILTER_DEBOUNCE_NODES
  // the client-side filter applies once typing pauses.
  const FILTER_DEBOUNCE_NODES = 2000;
  const FILTER_DEBOUNCE_MS = 150;
  let treeFilterQ = $state('');
  function cachedNodeCount(): number {
    let n = database.schemaRoot.length;
    for (const kids of database.childrenCache.values()) n += kids.length;
    return n;
  }
  $effect(() => {
    const q = database.objectSearchQuery.trim().toLowerCase();
    if (!q || untrack(cachedNodeCount) <= FILTER_DEBOUNCE_NODES) {
      treeFilterQ = q;
      return;
    }
    const timer = setTimeout(() => (treeFilterQ = q), FILTER_DEBOUNCE_MS);
    return () => clearTimeout(timer);
  });

  const filteredRoot = $derived.by(() => {
    const q = treeFilterQ;
    if (!q) return database.schemaRoot;
    return database.schemaRoot.filter((n) => nodeMatchesFilter(n, q));
  });

  /** Children a node actually renders: pruned by the filter (keeping ancestors
   *  of matches; a node whose own name matched shows its whole subtree). */
  function visibleChildren(node: SchemaNode, children: SchemaNode[]): SchemaNode[] {
    const q = treeFilterQ;
    if (!q || node.label.toLowerCase().includes(q)) return children;
    return children.filter((k) => nodeMatchesFilter(k, q));
  }

  /** Cap flattened children per node; a tail row reports how many were held
   *  back. The tree is windowed (only rows in view mount), so the cap only
   *  bounds the flat list — it was 1,000, which hid tables 1,001+ of a big
   *  schema behind "refine filter" for no DOM benefit. */
  const CHILD_CAP = 20_000;

  // ── Flattened, windowed rows ────────────────────────────────────────────
  // The visible tree (roots, then each OPEN node's filtered children, depth
  // first) as one flat list. Past WINDOW_MIN rows only the slice in view of
  // the sidebar's scroller (+ overscan) is mounted, between two spacers — a
  // few open 1,000-table schemas were ~8 DOM nodes and 2–3 Icons per row.
  // Every row is ROW_H tall in that mode (`.node` already is). A Redis
  // keyspace's inline key filter isn't fixed-height, so a tree showing one
  // renders unwindowed (Redis key lists are capped server-side anyway).
  // One member per node-bearing tag: a shared `t: 'node' | 'failed' | 'redis'`
  // member never narrows away in the template's {:else} chain.
  type NodeRow<T extends string> = { t: T; node: SchemaNode; depth: number; key: string };
  // `pos` / `size`: the node's 1-based place among its siblings and their
  // count, for aria-posinset / aria-setsize — the flat, windowed list carries
  // no DOM nesting a screen reader could derive the hierarchy from.
  type FlatRow =
    | (NodeRow<'node'> & { pos: number; size: number })
    | NodeRow<'failed'>
    | NodeRow<'redis'>
    | { t: 'empty'; depth: number; text: string; key: string }
    | { t: 'more'; depth: number; shown: number; key: string };
  const flat = $derived.by<FlatRow[]>(() => {
    const out: FlatRow[] = [];
    const walk = (node: SchemaNode, depth: number, pos: number, size: number): void => {
      out.push({ t: 'node', node, depth, key: node.id, pos, size });
      const open = database.isExpanded(node.id);
      if (!open) {
        if (failedNodes.has(node.id)) out.push({ t: 'failed', node, depth, key: `failed:${node.id}` });
        return;
      }
      if (node.kind === 'keyspace') out.push({ t: 'redis', node, depth, key: `redis:${node.id}` });
      const children = database.childrenOf(node.id);
      if (!children) return;
      const shown = visibleChildren(node, children);
      if (shown.length === 0) {
        const text = treeFilterQ && children.length > 0 ? 'no match' : 'empty';
        out.push({ t: 'empty', depth, text, key: `empty:${node.id}` });
      }
      const kids = shown.length > CHILD_CAP ? shown.slice(0, CHILD_CAP) : shown;
      kids.forEach((child, i) => walk(child, depth + 1, i + 1, shown.length));
      if (shown.length > CHILD_CAP) out.push({ t: 'more', depth, shown: shown.length, key: `more:${node.id}` });
    };
    filteredRoot.forEach((n, i) => walk(n, 0, i + 1, filteredRoot.length));
    return out;
  });

  const ROW_H = 25; // == .node height
  const WINDOW_MIN = 400;
  const OVERSCAN = 20;
  const windowed = $derived(flat.length > WINDOW_MIN && !flat.some((r) => r.t === 'redis'));
  // The shared outer-scroller window (lib/listWindow): it follows the sidebar
  // body's scroll and size, measures the `.node` pitch, and sizes the spacers.
  const lw = new ListWindow('.node', { min: WINDOW_MIN, overscan: OVERSCAN, rowH: ROW_H });
  const win = $derived(windowed ? lw.range(flat.length) : { start: 0, end: flat.length, top: 0, bottom: 0 });
  const visibleFlat = $derived(windowed ? flat.slice(win.start, win.end) : flat);

  // ── Keyboard navigation (roving focus over the rendered treeitems) ─────────
  let treeEl = $state<HTMLElement | null>(null);
  function visibleItems(): HTMLElement[] {
    return Array.from(treeEl?.querySelectorAll<HTMLElement>('[role="treeitem"]') ?? []);
  }
  /** Resolve a rendered row back to its SchemaNode via the cached tree. */
  function findNode(id: string | undefined, list: SchemaNode[] = database.schemaRoot): SchemaNode | null {
    if (!id) return null;
    for (const n of list) {
      if (n.id === id) return n;
      const kids = database.childrenOf(n.id);
      if (kids) {
        const f = findNode(id, kids);
        if (f) return f;
      }
    }
    return null;
  }
  function onTreeKey(e: KeyboardEvent): void {
    // Keys typed in the Redis key filter, or pressed on a failed row's Retry,
    // belong to that control: ↑/↓ must move the caret, not jump to the top.
    if ((e.target as HTMLElement | null)?.closest('input, textarea, select, button:not(.node-label)')) return;
    const items = visibleItems();
    if (items.length === 0) return;
    // Focus usually sits on a row's inner label button (a click, or a
    // programmatic focus), not the treeitem itself: resolve it to its row, or
    // ↑/↓ restarted at the top and ←/→/Enter did nothing.
    const active = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const row = active?.closest<HTMLElement>('[role="treeitem"]') ?? null;
    const cur = row ? items.indexOf(row) : -1;
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      const next =
        e.key === 'ArrowDown'
          ? Math.min(items.length - 1, cur < 0 ? 0 : cur + 1)
          : Math.max(0, cur < 0 ? 0 : cur - 1);
      items[next]?.focus();
      return;
    }
    if (cur < 0) return;
    const el = items[cur];
    const node = findNode(el.dataset.nodeId);
    if (!node) return;
    const open = database.isExpanded(node.id);
    if (e.key === 'ArrowRight') {
      e.preventDefault();
      if (node.has_children && !open) void expandNode(node);
      else if (open) items[cur + 1]?.focus(); // into the first child
      return;
    }
    if (e.key === 'ArrowLeft') {
      e.preventDefault();
      if (node.has_children && open) {
        void expandNode(node); // collapse
        return;
      }
      // Collapsed / leaf → jump to the parent (the previous shallower row).
      const depth = Number(el.dataset.depth ?? 0);
      for (let i = cur - 1; i >= 0; i--) {
        if (Number(items[i].dataset.depth ?? 0) < depth) {
          items[i].focus();
          return;
        }
      }
      return;
    }
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      onClick(node);
      return;
    }
    // Keyboard path to the same context menu as right-click.
    if (e.key === 'ContextMenu' || (e.shiftKey && e.key === 'F10')) {
      e.preventDefault();
      showMenu(e, node);
    }
  }

  function showMenu(e: MouseEvent | KeyboardEvent, node: SchemaNode): void {
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
      items.push({ label: 'Open structure', icon: 'eye', action: () => openStructure(node) });
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
  $effect(() => {
    const st = database.activeConnStatus;
    if (st?.phase === 'error') announce(`Couldn’t connect. ${st.error ?? ''}`.trim(), { assertive: true });
  });
  $effect(() => { if (database.schemaLoading || database.activeConnStatus?.phase === 'connecting') announce('Loading schema…'); });
  $effect(() => { if (database.objectSearching) announce('Searching…'); });
;
async () => {


 { const $$_div0 = svelteHTML.createElement("div", {  "class":`schema-tree`,});treeEl = $$_div0;
  if(!database.schemaLoading && database.schemaRoot.length > 0){
     { svelteHTML.createElement("div", { "class":`tree-search`,});
       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`search`,"size":12,}});}
       { svelteHTML.createElement("input", {                  "dir":`auto`,"class":`tree-search-input`,"type":`text`,"value":database.objectSearchQuery,"oninput":onSearchInput,"placeholder":searchable ? 'Find a table…' : 'Filter schema…',"onkeydown":(e) => {
          if (e.key === 'Escape') {
            e.preventDefault();
            clearSearch();
          }
        },"spellcheck":`false`,"aria-label":`Find an object`,});}
      if(database.objectSearchQuery || hits !== null){
         { svelteHTML.createElement("button", {       "class":`tree-search-clear`,"onclick":clearSearch,"aria-label":`Clear filter`,"title":`Clear filter`,});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});}
         }
      }
     }
     { svelteHTML.createElement("div", { "class":`tree-opts`,});
      if(searchable){
         { svelteHTML.createElement("select", {           "class":`scope-pick`,"value":database.objectSearchScope,"onchange":(e) => setScope(e.currentTarget.value as 'schema' | 'all'),"aria-label":`Search scope`,"title":database.selectedConn?.kind === 'postgres'
            ? 'Postgres cannot cross databases — “All schemas” covers every schema in the connected database'
            : 'Where to look for the object',});
           { svelteHTML.createElement("option", { "value":`schema`,});  }
           { svelteHTML.createElement("option", { "value":`all`,});  }
         }
      }
       { svelteHTML.createElement("label", {   "class":`counts-toggle`,"title":`Show approximate row counts. Off by default — collecting the estimate is what makes expanding a large server slow.`,});
         { svelteHTML.createElement("input", {       "type":`checkbox`,"checked":database.showCounts,"onchange":(e) => database.setShowCounts(e.currentTarget.checked),});}
         
       }
     }
  }
  if(database.schemaLoading || database.activeConnStatus?.phase === 'connecting'){
     { svelteHTML.createElement("div", { "class":`tree-loading`,});
       { svelteHTML.createElement("span", {     "class":`spinner`,"style":`--spinner-size: 13px`,"aria-hidden":`true`,}); }
       
     }
  } else if (database.activeConnStatus?.phase === 'error'){
     { svelteHTML.createElement("div", { "class":`tree-error`,});
       { svelteHTML.createElement("div", { "class":`tree-error-head`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});}  }
       { svelteHTML.createElement("div", { "class":`tree-error-msg`,});database.activeConnStatus.error; }
       { svelteHTML.createElement("button", {   "class":`btn small tree-error-retry`,"onclick":() => database.retryConnection(),});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":12,}});}
       }
     }
  } else if (database.schemaRoot.length === 0){
     { svelteHTML.createElement("div", { "class":`tree-empty`,});       }
  } else if (hits !== null){
    
    if(database.objectSearching && hits.length === 0){
       { svelteHTML.createElement("div", { "class":`tree-loading`,}); { svelteHTML.createElement("span", {     "class":`spinner`,"style":`--spinner-size: 13px`,"aria-hidden":`true`,}); } { svelteHTML.createElement("span", {});  } }
    } else if (hits.length === 0){
       { svelteHTML.createElement("div", { "class":`tree-empty`,});
           database.objectSearchQuery; database.objectSearchScope === 'schema'
          ? ' in this schema — try All schemas.'
          : '.';
       }
    }else{
       { svelteHTML.createElement("div", { "class":`hit-head`,});
         { svelteHTML.createElement("button", {     "class":`back-btn`,"onclick":clearSearch,"title":`Back to the schema tree (Esc)`,});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`chevronLeft`,"size":12,}});}
         }
         { svelteHTML.createElement("span", {});
          hits.length;database.objectSearchTruncated ? '+' : ''; hits.length === 1
            ? ''
            : 'es';
          if(database.objectSearchScope === 'all' && database.objectSearchScanned > 0){
             { svelteHTML.createElement("span", {  "class":`hit-scan`,}); database.objectSearchScanned;
              database.objectSearchScanned === 1 ? 'schema' : 'schemas';  }
          }
         }
       }
         for(let hit of __sveltets_2_ensureArray(hits)){hit.path;
         {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('button')));{ svelteHTML.createElement("button", __sveltets_2_union($$action_0), {               "class":`hit`,"aria-current":database.selectedObjectPath === hit.path ? 'true' : undefined,...__sveltets_2_empty({"data-node-id":hit.path}),"onclick":() => openHit(hit),"oncontextmenu":(e) => showMenu(e, hitNode(hit)),"title":hit.path,});database.selectedObjectPath === hit.path;
           { svelteHTML.createElement("span", { "class":`node-icon ${hit.kind}`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":iconFor(hit.kind),"size":12,}});} }
           { svelteHTML.createElement("span", { "class":`hit-name`,});hit.name; }
           { svelteHTML.createElement("span", { "class":`hit-schema`,});hit.schema; }
         }}
      }
      if(database.objectSearchTruncated){
         { svelteHTML.createElement("div", { "class":`tree-empty`,});       }
      }
    }
  } else if (filteredRoot.length === 0){
     { svelteHTML.createElement("div", { "class":`tree-empty`,});   database.objectSearchQuery;  }
  }else{
     { svelteHTML.createElement("div", {          "class":`tree-rows`,"role":`tree`,"aria-label":`Schema`,"tabindex":0,[Symbol("@attach")]:lw.attach,"onkeydown":onTreeKey,});
      if(win.top > 0){
         { svelteHTML.createElement("div", {     "class":`tree-pad`,"style":`height: ${win.top}px`,"aria-hidden":`true`,}); }
      }
         for(let row of __sveltets_2_ensureArray(visibleFlat)){row.key;
        if(row.t === 'node'){
          ;__sveltets_2_ensureSnippet(nodeRow(row.node, row.depth, row.pos, row.size));
        } else if (row.t === 'failed'){
           { svelteHTML.createElement("div", {        "class":`node-failed`,"role":`group`,"aria-label":`${row.node.label}: couldn’t load`,"style":`padding-inline-start: ${(row.depth + 1) * 13 + 18}px`,});windowed;
             { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});}
             { svelteHTML.createElement("span", {});  }
             { svelteHTML.createElement("button", {   "class":`node-failed-retry`,"onclick":() => expandNode(row.node),});  }
           }
        } else if (row.t === 'redis'){
           { const $$_retliFyeKsideR2C = __sveltets_2_ensureComponent(RedisKeyFilter); new $$_retliFyeKsideR2C({ target: __sveltets_2_any(), props: {    "node":row.node,"depth":row.depth,}});}
        } else if (row.t === 'empty'){
           { svelteHTML.createElement("div", {      "class":`node-empty`,"role":`group`,"style":`padding-inline-start: ${(row.depth + 1) * 13 + 18}px`,});windowed;
            row.text;
           }
        } else if (row.t === 'more'){
           { svelteHTML.createElement("div", {      "class":`node-more`,"role":`group`,"style":`padding-inline-start: ${(row.depth + 1) * 13 + 18}px`,});windowed;
             CHILD_CAP.toLocaleString();  row.shown.toLocaleString();   
           }
        }
      }
      if(win.bottom > 0){
         { svelteHTML.createElement("div", {     "class":`tree-pad`,"style":`height: ${win.bottom}px`,"aria-hidden":`true`,}); }
      }
     }
  }
 }




};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const SchemaTree__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type SchemaTree__SvelteComponent_ = ReturnType<typeof SchemaTree__SvelteComponent_>;
/*Ωignore_endΩ*/export default SchemaTree__SvelteComponent_;