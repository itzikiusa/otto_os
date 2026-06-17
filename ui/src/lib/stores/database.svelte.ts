// Database Explorer store — workspace-scoped DB connections + schema browsing,
// query tabs, saved queries, history, and Superset-style dashboards/widgets.
// Reads `ws.currentId` only (never mutates it), mirroring apiClient.svelte.ts.

import { api, isAbortError } from '../api/client';
import type {
  Connection,
  DbCapabilities,
  DbCompletionItem,
  DbDashboard,
  DbHistoryEntry,
  DbSavedQuery,
  DbTestResult,
  DbViz,
  DbWidget,
  DbWidgetMapping,
  Id,
  ObjectDetail,
  QueryResult,
  SchemaNode,
  Session,
} from '../api/types';
import { ws } from './workspace.svelte';
import { toasts } from '../toast.svelte';

/** Connection kinds the explorer can browse (the four DB engines). */
export const DB_KINDS = ['mysql', 'redis', 'mongodb', 'clickhouse'] as const;
export type DbKind = (typeof DB_KINDS)[number];

function isDbKind(k: string): k is DbKind {
  return (DB_KINDS as readonly string[]).includes(k);
}

function errMsg(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/** Persisted default row cap applied when a statement has no explicit LIMIT. */
const ROW_LIMIT_KEY = 'otto_db_row_limit';
const DEFAULT_ROW_LIMIT = 1000;
/** Sentinel "no cap" value sent as max_rows for the "All" option. */
export const ROW_LIMIT_ALL = 1_000_000;

function loadRowLimit(): number {
  if (typeof localStorage === 'undefined') return DEFAULT_ROW_LIMIT;
  const v = Number(localStorage.getItem(ROW_LIMIT_KEY));
  return Number.isFinite(v) && v > 0 ? v : DEFAULT_ROW_LIMIT;
}

/**
 * Extract a trailing explicit `LIMIT` from a SQL statement so we honor what the
 * user wrote instead of clipping it. Handles `LIMIT n`, `LIMIT offset, count`,
 * and `LIMIT n OFFSET m`. Returns the row count, or null when there's no
 * trailing LIMIT.
 */
export function parseExplicitLimit(sql: string): number | null {
  const m = sql.match(/\blimit\s+(\d+)(?:\s*,\s*(\d+))?(?:\s+offset\s+\d+)?\s*;?\s*$/i);
  if (!m) return null;
  return m[2] !== undefined ? Number(m[2]) : Number(m[1]);
}

/** Glyph (Icon name) for a connection engine. */
export function engineGlyph(kind: string): string {
  switch (kind) {
    case 'redis':
      return 'key';
    case 'mongodb':
      return 'box';
    case 'clickhouse':
      return 'grid';
    default:
      return 'db';
  }
}

/** An open query tab: an editable statement + its last result. */
export interface QueryTab {
  id: number;
  name: string;
  statement: string;
  result: QueryResult | null;
  running: boolean;
  error: string | null;
}

let nextTabId = 1;
function blankTab(statement = ''): QueryTab {
  return { id: nextTabId++, name: 'Query', statement, result: null, running: false, error: null };
}

/** Main-pane tabs of the DB page. */
export type DbMainTab = 'query' | 'builder' | 'structure' | 'dashboards';
/** Sidebar lower switch (below the schema tree). */
export type DbSideTab = 'schema' | 'saved' | 'history';

/**
 * Point-in-time copy of a connection's active-session working set, kept so we
 * can switch between open connection tabs without re-fetching. Each field holds
 * a reference to the array/Map/Set that was current when captured; those
 * collections are replaced wholesale on mutation elsewhere, so a reference
 * snapshot is a correct point-in-time view.
 *
 * Workspace/global fields (`connections`, `dashboards`, `widgets`,
 * `selectedDashboardId`, `rowLimit`) are intentionally NOT snapshotted — they
 * are shared across all open connections.
 */
interface ConnSnapshot {
  capabilities: DbCapabilities | null;
  testResult: DbTestResult | null;
  schemaRoot: SchemaNode[];
  childrenCache: Map<string, SchemaNode[]>;
  expanded: Set<string>;
  loadingNodes: Set<string>;
  schemaLoading: boolean;
  selectedObjectPath: string | null;
  objectDetail: ObjectDetail | null;
  builderTablesCache: Map<string, { label: string; path: string; kind: string }[]>;
  tabs: QueryTab[];
  activeTab: number;
  savedQueries: DbSavedQuery[];
  history: DbHistoryEntry[];
  mainTab: DbMainTab;
  sideTab: DbSideTab;
}

class DatabaseStore {
  // ── Connections ────────────────────────────────────────────────────────────
  /** All workspace connections, filtered to the four DB engines. */
  connections: Connection[] = $state([]);
  selectedConnId: Id | null = $state(null);
  /** Connections currently open as top-level tabs, in display order. */
  openConnIds: Id[] = $state([]);
  capabilities: DbCapabilities | null = $state(null);
  testResult: DbTestResult | null = $state(null);
  testing = $state(false);
  /** Default row cap for statements without an explicit LIMIT (persisted). */
  rowLimit = $state(loadRowLimit());

  /**
   * Per-connection working-set snapshots, keyed by connection id. Deliberately
   * NON-reactive (plain Map, not `$state`): it's an internal cache read only
   * via capture/restore, and the singleton fields it feeds ARE reactive, so
   * reassigning them on restore is what drives the UI.
   */
  private snapshots = new Map<Id, ConnSnapshot>();

  selectedConn: Connection | null = $derived(
    this.connections.find((c) => c.id === this.selectedConnId) ?? null,
  );

  // ── Schema tree ──────────────────────────────────────────────────────────
  schemaRoot: SchemaNode[] = $state([]);
  /** Lazy children cache keyed by node id. */
  childrenCache: Map<string, SchemaNode[]> = $state(new Map());
  /** Expanded node ids. */
  expanded: Set<string> = $state(new Set());
  /** Nodes whose children are currently loading. */
  loadingNodes: Set<string> = $state(new Set());
  schemaLoading = $state(false);

  // ── Selected object (Structure view) ────────────────────────────────────
  selectedObjectPath: string | null = $state(null);
  objectDetail: ObjectDetail | null = $state(null);
  objectLoading = $state(false);

  // ── Builder catalog cache (palette table lists, keyed by db path) ─────────
  // The schema tree is lazy/partial; the visual builder needs the full catalog
  // on demand. Cached so re-opening the palette is instant.
  private builderTablesCache: Map<string, { label: string; path: string; kind: string }[]> = $state(
    new Map(),
  );

  // ── Query tabs ────────────────────────────────────────────────────────────
  tabs: QueryTab[] = $state([blankTab()]);
  activeTab = $state(0);
  get tab(): QueryTab {
    return this.tabs[this.activeTab] ?? this.tabs[0];
  }

  /**
   * In-flight query AbortControllers, keyed by tab id. Non-reactive (plain Map):
   * lets us cancel a running query (`abortQuery`) without storing a
   * non-serializable controller inside the reactive `$state` tab objects.
   */
  private runControllers = new Map<number, AbortController>();

  // ── UI tabs ────────────────────────────────────────────────────────────────
  mainTab: DbMainTab = $state('query');
  sideTab: DbSideTab = $state('schema');

  // ── Saved queries / history ─────────────────────────────────────────────
  savedQueries: DbSavedQuery[] = $state([]);
  history: DbHistoryEntry[] = $state([]);

  // ── Dashboards / widgets ──────────────────────────────────────────────────
  dashboards: DbDashboard[] = $state([]);
  widgets: DbWidget[] = $state([]);
  selectedDashboardId: Id | null = $state(null);

  selectedDashboard: DbDashboard | null = $derived(
    this.dashboards.find((d) => d.id === this.selectedDashboardId) ?? null,
  );

  // ── Path helpers ────────────────────────────────────────────────────────
  private connBase(id: Id): string {
    return `/connections/${id}/db`;
  }
  private wsBase(): string | null {
    return ws.currentId ? `/workspaces/${ws.currentId}/db` : null;
  }

  /** CodeMirror editor language for the active engine. */
  get queryLanguage(): 'sql' | 'redis' | 'mongo' {
    return this.capabilities?.query_language ?? 'sql';
  }
  /** Whether the visual JOIN builder applies (SQL engines with joins). */
  get supportsBuilder(): boolean {
    return !!this.capabilities?.joins;
  }

  // ── Tab management ──────────────────────────────────────────────────────
  newTab(statement = ''): void {
    this.tabs = [...this.tabs, blankTab(statement)];
    this.activeTab = this.tabs.length - 1;
    this.mainTab = 'query';
    this.persistTabs();
  }
  switchTab(i: number): void {
    if (i >= 0 && i < this.tabs.length) {
      this.activeTab = i;
      this.persistTabs();
    }
  }
  closeTab(i: number): void {
    if (this.tabs.length === 1) {
      this.tabs = [blankTab()];
      this.activeTab = 0;
    } else {
      this.tabs = this.tabs.filter((_, idx) => idx !== i);
      if (this.activeTab >= this.tabs.length) this.activeTab = this.tabs.length - 1;
      else if (i < this.activeTab) this.activeTab -= 1;
    }
    this.persistTabs();
  }
  setStatement(value: string): void {
    const t = this.tab;
    if (t) t.statement = value;
    this.persistTabs();
  }

  // ── Tab persistence (survive reload / a cut-off session) ──────────────────
  // Open query tabs (statement + name, NOT results) are saved per
  // (workspace, connection) so reopening a connection restores in-progress work.
  private tabsKey(connId: Id): string | null {
    return ws.currentId ? `otto_db_tabs:${ws.currentId}:${connId}` : null;
  }
  private persistTabs(): void {
    if (typeof localStorage === 'undefined' || !this.selectedConnId) return;
    const key = this.tabsKey(this.selectedConnId);
    if (!key) return;
    try {
      localStorage.setItem(
        key,
        JSON.stringify({
          tabs: this.tabs.map((t) => ({ name: t.name, statement: t.statement })),
          activeTab: this.activeTab,
        }),
      );
    } catch {
      /* storage full / unavailable — non-fatal */
    }
  }
  private restoreTabs(connId: Id): { tabs: QueryTab[]; activeTab: number } | null {
    if (typeof localStorage === 'undefined') return null;
    const key = this.tabsKey(connId);
    if (!key) return null;
    const raw = localStorage.getItem(key);
    if (!raw) return null;
    try {
      const p = JSON.parse(raw) as {
        tabs?: { name?: string; statement?: string }[];
        activeTab?: number;
      };
      const tabs = (p.tabs ?? []).map((t) => ({
        ...blankTab(t.statement ?? ''),
        name: t.name || 'Query',
      }));
      if (!tabs.length) return null;
      const activeTab = Math.min(Math.max(0, p.activeTab ?? 0), tabs.length - 1);
      return { tabs, activeTab };
    } catch {
      return null;
    }
  }

  /** Set + persist the default row cap (used when a query has no own LIMIT). */
  setRowLimit(n: number): void {
    this.rowLimit = n > 0 ? n : DEFAULT_ROW_LIMIT;
    if (typeof localStorage !== 'undefined') {
      localStorage.setItem(ROW_LIMIT_KEY, String(this.rowLimit));
    }
  }

  // ── Loading ───────────────────────────────────────────────────────────────

  /** Load DB-kind connections for the current workspace. */
  async loadConnections(): Promise<void> {
    const wid = ws.currentId;
    if (!wid) return;
    try {
      const all = await api.get<Connection[]>(`/workspaces/${wid}/connections`);
      const next = all.filter((c) => isDbKind(c.kind));
      // If the workspace's connection set changed identity (different ids),
      // treat it as a workspace switch: drop open tabs + snapshots so we don't
      // carry stale connections from a previous workspace into this one.
      const sameSet =
        next.length === this.connections.length &&
        next.every((c) => this.connections.some((p) => p.id === c.id));
      this.connections = next;
      if (!sameSet) {
        this.openConnIds = [];
        this.snapshots.clear();
        this.selectedConnId = null;
        this.capabilities = null;
        this.schemaRoot = [];
      } else {
        // Prune any open tab/snapshot whose connection no longer exists.
        this.openConnIds = this.openConnIds.filter((id) => next.some((c) => c.id === id));
        for (const id of [...this.snapshots.keys()]) {
          if (!next.some((c) => c.id === id)) this.snapshots.delete(id);
        }
      }
      // Start fresh — do NOT auto-open a connection; the user picks one from the
      // sidebar. Only clear active state when no connections remain.
      if (this.connections.length === 0) {
        this.selectedConnId = null;
        this.capabilities = null;
        this.schemaRoot = [];
      }
    } catch (e) {
      toasts.error('Could not load connections', errMsg(e));
    }
  }

  /**
   * Capture the active connection's working set into `snapshots` so we can
   * restore it (without re-fetching) when we switch back. No-op when nothing
   * is selected.
   */
  private captureSnapshot(): void {
    const id = this.selectedConnId;
    if (id === null) return;
    this.snapshots.set(id, {
      capabilities: this.capabilities,
      testResult: this.testResult,
      schemaRoot: this.schemaRoot,
      childrenCache: this.childrenCache,
      expanded: this.expanded,
      loadingNodes: this.loadingNodes,
      schemaLoading: this.schemaLoading,
      selectedObjectPath: this.selectedObjectPath,
      objectDetail: this.objectDetail,
      builderTablesCache: this.builderTablesCache,
      tabs: this.tabs,
      activeTab: this.activeTab,
      savedQueries: this.savedQueries,
      history: this.history,
      mainTab: this.mainTab,
      sideTab: this.sideTab,
    });
  }

  /**
   * Restore a previously captured connection working set, reassigning each
   * reactive singleton so Svelte re-renders. Returns false when there's no
   * snapshot for `id` (caller should load fresh instead).
   */
  private restoreSnapshot(id: Id): boolean {
    const snap = this.snapshots.get(id);
    if (!snap) return false;
    this.capabilities = snap.capabilities;
    this.testResult = snap.testResult;
    this.schemaRoot = snap.schemaRoot;
    this.childrenCache = snap.childrenCache;
    this.expanded = snap.expanded;
    this.loadingNodes = snap.loadingNodes;
    this.schemaLoading = snap.schemaLoading;
    this.selectedObjectPath = snap.selectedObjectPath;
    this.objectDetail = snap.objectDetail;
    this.builderTablesCache = snap.builderTablesCache;
    this.tabs = snap.tabs;
    this.activeTab = snap.activeTab;
    this.savedQueries = snap.savedQueries;
    this.history = snap.history;
    this.mainTab = snap.mainTab;
    this.sideTab = snap.sideTab;
    return true;
  }

  /**
   * Open (or focus) a connection as a top-level tab. If it already has a
   * snapshot, restore it instantly with no network; otherwise load it fresh.
   * Snapshots the currently-active connection first so switching back is free.
   */
  async openConnection(id: Id): Promise<void> {
    if (id === this.selectedConnId) return;
    this.captureSnapshot();
    if (!this.openConnIds.includes(id)) {
      this.openConnIds = [...this.openConnIds, id];
    }
    this.selectedConnId = id;
    if (this.restoreSnapshot(id)) return;
    await this.loadConnectionFresh(id);
    // Capture an initial snapshot so subsequent switches restore this state.
    this.captureSnapshot();
  }

  /** Backwards-compatible alias: selecting a connection opens/focuses its tab. */
  async selectConnection(id: Id): Promise<void> {
    await this.openConnection(id);
  }

  /**
   * Close an open connection tab, discarding its snapshot. When the closed tab
   * was active, switch to a neighbor (previous index, else first remaining);
   * when none remain, clear the active session.
   */
  closeConnection(id: Id): void {
    const idx = this.openConnIds.indexOf(id);
    if (idx === -1) return;
    const wasActive = this.selectedConnId === id;
    this.openConnIds = this.openConnIds.filter((x) => x !== id);
    this.snapshots.delete(id);
    if (!wasActive) return;

    if (this.openConnIds.length === 0) {
      // Nothing left open — clear the active working set.
      this.selectedConnId = null;
      this.capabilities = null;
      this.testResult = null;
      this.schemaRoot = [];
      this.childrenCache = new Map();
      this.builderTablesCache = new Map();
      this.expanded = new Set();
      this.loadingNodes = new Set();
      this.schemaLoading = false;
      this.objectDetail = null;
      this.selectedObjectPath = null;
      this.tabs = [blankTab()];
      this.activeTab = 0;
      this.history = [];
      this.mainTab = 'query';
      return;
    }
    // Focus the previous tab (or the first if we closed index 0). The active id
    // is gone, so clear it first to let openConnection do the switch.
    const neighbor = this.openConnIds[Math.max(0, idx - 1)];
    this.selectedConnId = null;
    void this.openConnection(neighbor);
  }

  /**
   * Fresh load of a connection's active-session fields: reset the working set,
   * then fetch capabilities + schema root + history. Resets ONLY active-session
   * state — never `openConnIds`/`snapshots`.
   */
  private async loadConnectionFresh(id: Id): Promise<void> {
    this.selectedConnId = id;
    this.capabilities = null;
    this.schemaRoot = [];
    this.childrenCache = new Map();
    this.builderTablesCache = new Map();
    this.expanded = new Set();
    this.loadingNodes = new Set();
    this.objectDetail = null;
    this.selectedObjectPath = null;
    this.testResult = null;
    // Restore this connection's persisted query tabs (in-progress work from a
    // previous session); otherwise start with one blank tab. Never inherit the
    // previously active connection's tabs.
    const restored = this.restoreTabs(id);
    this.tabs = restored?.tabs ?? [blankTab()];
    this.activeTab = restored?.activeTab ?? 0;
    this.mainTab = 'query';
    this.sideTab = 'schema';
    await Promise.all([this.loadCapabilities(id), this.loadSchemaRoot(id), this.loadHistory(id)]);
  }

  private async loadCapabilities(id: Id): Promise<void> {
    try {
      this.capabilities = await api.get<DbCapabilities>(`${this.connBase(id)}/capabilities`);
      // A non-SQL engine can't use the visual JOIN builder; keep main tab valid.
      if (this.mainTab === 'builder' && !this.supportsBuilder) this.mainTab = 'query';
    } catch (e) {
      toasts.error('Could not load DB capabilities', errMsg(e));
    }
  }

  private async loadSchemaRoot(id: Id): Promise<void> {
    this.schemaLoading = true;
    try {
      this.schemaRoot = await api.get<SchemaNode[]>(`${this.connBase(id)}/schema`);
    } catch (e) {
      toasts.error('Could not load schema', errMsg(e));
    } finally {
      this.schemaLoading = false;
    }
  }

  /** Re-fetch the schema root, clearing the children cache. */
  async refreshSchema(): Promise<void> {
    if (!this.selectedConnId) return;
    this.childrenCache = new Map();
    this.builderTablesCache = new Map();
    this.expanded = new Set();
    await this.loadSchemaRoot(this.selectedConnId);
  }

  /** Test the selected connection. */
  async testConnection(): Promise<void> {
    const id = this.selectedConnId;
    if (!id) return;
    this.testing = true;
    this.testResult = null;
    try {
      this.testResult = await api.post<DbTestResult>(`${this.connBase(id)}/test`, {});
      if (this.testResult.ok) {
        toasts.success('Connection OK', this.testResult.message || `${this.testResult.latency_ms ?? '?'} ms`);
      } else {
        toasts.error('Connection failed', this.testResult.message);
      }
    } catch (e) {
      toasts.error('Test failed', errMsg(e));
    } finally {
      this.testing = false;
    }
  }

  // ── Tree expansion ──────────────────────────────────────────────────────

  isExpanded(nodeId: string): boolean {
    return this.expanded.has(nodeId);
  }
  childrenOf(nodeId: string): SchemaNode[] | undefined {
    return this.childrenCache.get(nodeId);
  }
  isLoadingNode(nodeId: string): boolean {
    return this.loadingNodes.has(nodeId);
  }

  /** Toggle/lazy-load a node's children. */
  async expand(node: SchemaNode): Promise<void> {
    const id = this.selectedConnId;
    if (!id || !node.has_children) return;

    if (this.expanded.has(node.id)) {
      this.expanded.delete(node.id);
      this.expanded = new Set(this.expanded);
      return;
    }
    this.expanded.add(node.id);
    this.expanded = new Set(this.expanded);

    if (this.childrenCache.has(node.id)) return; // already loaded
    this.loadingNodes.add(node.id);
    this.loadingNodes = new Set(this.loadingNodes);
    try {
      const children = await api.post<SchemaNode[]>(`${this.connBase(id)}/schema/children`, {
        path: node.id,
      });
      this.childrenCache.set(node.id, children);
      this.childrenCache = new Map(this.childrenCache);
    } catch (e) {
      toasts.error('Could not load children', errMsg(e));
      this.expanded.delete(node.id);
      this.expanded = new Set(this.expanded);
    } finally {
      this.loadingNodes.delete(node.id);
      this.loadingNodes = new Set(this.loadingNodes);
    }
  }

  /** Open an object (table/view/collection/key) → detail + Structure tab. */
  async openObject(node: SchemaNode): Promise<void> {
    const id = this.selectedConnId;
    if (!id) return;
    this.selectedObjectPath = node.id;
    this.objectLoading = true;
    this.objectDetail = null;
    this.mainTab = 'structure';
    try {
      this.objectDetail = await api.post<ObjectDetail>(`${this.connBase(id)}/object`, {
        path: node.id,
      });
    } catch (e) {
      toasts.error('Could not load object', errMsg(e));
    } finally {
      this.objectLoading = false;
    }
  }

  /** Fetch object detail for an arbitrary table path (used by the builder). */
  async fetchObject(path: string): Promise<ObjectDetail | null> {
    const id = this.selectedConnId;
    if (!id) return null;
    try {
      return await api.post<ObjectDetail>(`${this.connBase(id)}/object`, { path });
    } catch (e) {
      toasts.error('Could not load object', errMsg(e));
      return null;
    }
  }

  // ── Builder catalog (full enumeration for the visual JOIN canvas) ────────

  /**
   * Flat list of databases for the active connection. Sourced from the schema
   * root (kind === 'database'). When the engine exposes no database level
   * (single implicit db), returns one empty entry so the palette still works.
   */
  async listBuilderDatabases(): Promise<{ name: string; path: string }[]> {
    const dbs = this.schemaRoot
      .filter((n) => n.kind === 'database')
      .map((n) => ({ name: n.label, path: n.id }));
    return dbs.length ? dbs : [{ name: '', path: '' }];
  }

  /**
   * Flat list of tables + views in a database, resolving any intermediate
   * Folder nodes (MySQL returns `folder:tables`/`folder:views`; ClickHouse
   * returns tables directly). Cached per db path.
   */
  async listBuilderTables(dbPath: string): Promise<{ label: string; path: string; kind: string }[]> {
    const id = this.selectedConnId;
    if (!id) return [];
    const cached = this.builderTablesCache.get(dbPath);
    if (cached) return cached;
    try {
      const out: { label: string; path: string; kind: string }[] = [];
      const seen = new Set<string>();
      // For an empty implicit-db path, query the schema root's children path.
      const first = await api.post<SchemaNode[]>(`${this.connBase(id)}/schema/children`, {
        path: dbPath,
      });
      for (const node of first) {
        if (node.kind === 'folder') {
          const kids = await api.post<SchemaNode[]>(`${this.connBase(id)}/schema/children`, {
            path: node.id,
          });
          for (const k of kids) {
            if ((k.kind === 'table' || k.kind === 'view') && !seen.has(k.id)) {
              seen.add(k.id);
              out.push({ label: k.label, path: k.id, kind: k.kind });
            }
          }
        } else if ((node.kind === 'table' || node.kind === 'view') && !seen.has(node.id)) {
          seen.add(node.id);
          out.push({ label: node.label, path: node.id, kind: node.kind });
        }
      }
      out.sort((a, b) => a.label.localeCompare(b.label));
      this.builderTablesCache.set(dbPath, out);
      this.builderTablesCache = new Map(this.builderTablesCache);
      return out;
    } catch (e) {
      toasts.error('Could not load tables', errMsg(e));
      return [];
    }
  }

  // ── Query ─────────────────────────────────────────────────────────────────

  /** Run the active tab's statement (or a given one) and store the result. */
  async runQuery(statement?: string, node?: string): Promise<QueryResult | null> {
    const id = this.selectedConnId;
    const t = this.tab;
    if (!id) {
      toasts.error('No connection selected');
      return null;
    }
    const sql = (statement ?? t.statement).trim();
    if (!sql) {
      toasts.error('Statement is empty');
      return null;
    }
    if (statement !== undefined) t.statement = statement;
    // Cancel any prior in-flight run for this tab before starting a new one.
    this.runControllers.get(t.id)?.abort();
    const controller = new AbortController();
    this.runControllers.set(t.id, controller);
    t.running = true;
    t.error = null;
    try {
      // Honor an explicit LIMIT in the SQL; otherwise apply the configured
      // default row cap. The server also injects this LIMIT into the SQL so a
      // huge table isn't fully scanned — this value just sizes that cap.
      const explicit = parseExplicitLimit(sql);
      const result = await api.post<QueryResult>(
        `${this.connBase(id)}/query`,
        {
          statement: sql,
          max_rows: explicit ?? this.rowLimit,
          node: node ?? null,
        },
        controller.signal,
      );
      t.result = result;
      void this.loadHistory(id);
      return result;
    } catch (e) {
      // A user-initiated abort isn't an error — leave the prior result intact.
      if (isAbortError(e) || controller.signal.aborted) {
        toasts.info('Query stopped');
        return null;
      }
      t.error = errMsg(e);
      toasts.error('Query failed', errMsg(e));
      return null;
    } finally {
      // Only clear running/controller if this run is still the current one
      // (a newer run may have replaced it).
      if (this.runControllers.get(t.id) === controller) {
        this.runControllers.delete(t.id);
        t.running = false;
      }
    }
  }

  /** Abort the in-flight query for a tab (defaults to the active tab). */
  abortQuery(tabId?: number): void {
    const id = tabId ?? this.tab?.id;
    if (id == null) return;
    const c = this.runControllers.get(id);
    if (c) {
      c.abort();
      this.runControllers.delete(id);
      const t = this.tabs.find((x) => x.id === id);
      if (t) t.running = false;
    }
  }

  // ── Table actions (schema-tree context menu) ──────────────────────────────

  /** Backtick-quote a SQL identifier (works for MySQL + ClickHouse). */
  private quoteIdent(name: string): string {
    return '`' + name.replace(/`/g, '``') + '`';
  }

  /**
   * Build a qualified SQL table reference from a tree node id like
   * `db:configserver/table:props`. Returns the quoted `db`.`table` ref plus the
   * raw parts, or null when the node isn't a SQL table/view.
   */
  tableRefFromNode(node: SchemaNode): { ref: string; db: string | null; table: string } | null {
    const segs = node.id.split('/').map((s) => {
      const i = s.indexOf(':');
      return i < 0 ? ([s, ''] as const) : ([s.slice(0, i), s.slice(i + 1)] as const);
    });
    const find = (k: string) => segs.find(([kk]) => kk === k)?.[1];
    const table = find('table') ?? find('view');
    if (!table) return null;
    const db = find('db') ?? find('schema') ?? null;
    const ref = db ? `${this.quoteIdent(db)}.${this.quoteIdent(table)}` : this.quoteIdent(table);
    return { ref, db, table };
  }

  /** Open a statement in a new query tab; optionally run it immediately. */
  async openInNewTab(sql: string, opts?: { run?: boolean; name?: string }): Promise<void> {
    this.newTab(sql);
    if (opts?.name) this.tab.name = opts.name;
    if (opts?.run) await this.runQuery();
  }

  /** New tab: `SELECT * FROM <table>` and run it (server applies the row cap). */
  async selectRows(node: SchemaNode): Promise<void> {
    const r = this.tableRefFromNode(node);
    if (!r) return;
    await this.openInNewTab(`SELECT * FROM ${r.ref}`, { run: true, name: r.table });
  }

  /** New tab: `SELECT * FROM <table>` without running (Send to SQL Editor). */
  async sendSelectToEditor(node: SchemaNode): Promise<void> {
    const r = this.tableRefFromNode(node);
    if (!r) return;
    await this.openInNewTab(`SELECT * FROM ${r.ref}`, { name: r.table });
  }

  /** New tab pre-filled with a TRUNCATE — NOT run; the user reviews + runs it. */
  async truncateTable(node: SchemaNode): Promise<void> {
    const r = this.tableRefFromNode(node);
    if (!r) return;
    await this.openInNewTab(`TRUNCATE TABLE ${r.ref};`, { name: `TRUNCATE ${r.table}` });
    toasts.warn('Review before running', 'This will delete all rows. Press Run to apply.');
  }

  /** New tab pre-filled with a DROP — NOT run; the user reviews + runs it. */
  async dropObject(node: SchemaNode): Promise<void> {
    const r = this.tableRefFromNode(node);
    if (!r) return;
    const verb = node.kind === 'view' ? 'DROP VIEW' : 'DROP TABLE';
    await this.openInNewTab(`${verb} ${r.ref};`, { name: `DROP ${r.table}` });
    toasts.warn('Review before running', 'This will drop the object. Press Run to apply.');
  }

  /** Fetch completions for the text before the cursor. */
  async complete(prefix: string, node?: string): Promise<DbCompletionItem[]> {
    const id = this.selectedConnId;
    if (!id) return [];
    try {
      const res = await api.post<{ items: DbCompletionItem[] }>(`${this.connBase(id)}/completion`, {
        prefix,
        database: this.selectedConn?.params?.db ? String(this.selectedConn.params.db) : undefined,
        node: node ?? null,
      });
      return res.items ?? [];
    } catch {
      // Completion failures must never break typing — degrade silently.
      return [];
    }
  }

  // ── Saved queries ─────────────────────────────────────────────────────────

  async loadSavedQueries(): Promise<void> {
    const base = this.wsBase();
    if (!base) return;
    try {
      this.savedQueries = await api.get<DbSavedQuery[]>(`${base}/saved-queries`);
    } catch (e) {
      toasts.error('Could not load saved queries', errMsg(e));
    }
  }

  async saveQuery(name: string, statement: string): Promise<DbSavedQuery | null> {
    const base = this.wsBase();
    if (!base) return null;
    try {
      const saved = await api.post<DbSavedQuery>(`${base}/saved-queries`, {
        connection_id: this.selectedConnId,
        name,
        statement,
      });
      this.savedQueries = [saved, ...this.savedQueries.filter((q) => q.id !== saved.id)];
      toasts.success('Query saved', saved.name);
      return saved;
    } catch (e) {
      toasts.error('Save query failed', errMsg(e));
      return null;
    }
  }

  async deleteSavedQuery(id: Id): Promise<void> {
    try {
      await api.del(`/db/saved-queries/${id}`);
      this.savedQueries = this.savedQueries.filter((q) => q.id !== id);
    } catch (e) {
      toasts.error('Delete query failed', errMsg(e));
    }
  }

  /** Load a saved query into a fresh tab. */
  openSavedQuery(q: DbSavedQuery): void {
    this.newTab(q.statement);
    this.tab.name = q.name;
  }

  // ── History ─────────────────────────────────────────────────────────────

  async loadHistory(connId?: Id): Promise<void> {
    const id = connId ?? this.selectedConnId;
    if (!id) return;
    try {
      this.history = await api.get<DbHistoryEntry[]>(`${this.connBase(id)}/history?limit=100`);
    } catch (e) {
      toasts.error('Could not load history', errMsg(e));
    }
  }

  /** Load a history entry's statement into a fresh tab. */
  openHistory(h: DbHistoryEntry): void {
    this.newTab(h.statement);
  }

  // ── Dashboards ────────────────────────────────────────────────────────────

  async loadDashboards(): Promise<void> {
    const base = this.wsBase();
    if (!base) return;
    try {
      this.dashboards = await api.get<DbDashboard[]>(`${base}/dashboards`);
      if (this.dashboards.length > 0 && !this.dashboards.some((d) => d.id === this.selectedDashboardId)) {
        this.selectedDashboardId = this.dashboards[0].id;
      }
      await this.loadWidgets();
    } catch (e) {
      toasts.error('Could not load dashboards', errMsg(e));
    }
  }

  async loadWidgets(): Promise<void> {
    const base = this.wsBase();
    if (!base) return;
    try {
      this.widgets = await api.get<DbWidget[]>(`${base}/widgets`);
    } catch (e) {
      toasts.error('Could not load widgets', errMsg(e));
    }
  }

  async createDashboard(name: string): Promise<DbDashboard | null> {
    const base = this.wsBase();
    if (!base) return null;
    try {
      const d = await api.post<DbDashboard>(`${base}/dashboards`, { name });
      this.dashboards = [...this.dashboards, d];
      this.selectedDashboardId = d.id;
      return d;
    } catch (e) {
      toasts.error('Create dashboard failed', errMsg(e));
      return null;
    }
  }

  async renameDashboard(id: Id, name: string): Promise<void> {
    try {
      const d = await api.patch<DbDashboard>(`/db/dashboards/${id}`, { name });
      this.dashboards = this.dashboards.map((x) => (x.id === id ? d : x));
    } catch (e) {
      toasts.error('Rename dashboard failed', errMsg(e));
    }
  }

  async setDashboardRefresh(id: Id, refresh_secs: number | null): Promise<void> {
    try {
      const d = await api.patch<DbDashboard>(`/db/dashboards/${id}`, { refresh_secs });
      this.dashboards = this.dashboards.map((x) => (x.id === id ? d : x));
    } catch (e) {
      toasts.error('Update dashboard failed', errMsg(e));
    }
  }

  async deleteDashboard(id: Id): Promise<void> {
    try {
      await api.del(`/db/dashboards/${id}`);
      this.dashboards = this.dashboards.filter((d) => d.id !== id);
      this.widgets = this.widgets.filter((w) => w.dashboard_id !== id);
      if (this.selectedDashboardId === id) {
        this.selectedDashboardId = this.dashboards[0]?.id ?? null;
      }
    } catch (e) {
      toasts.error('Delete dashboard failed', errMsg(e));
    }
  }

  // ── Widgets ────────────────────────────────────────────────────────────────

  async createWidget(input: {
    title: string;
    statement: string;
    viz: DbViz;
    mapping?: DbWidgetMapping;
    options?: Record<string, unknown>;
    dashboard_id?: Id | null;
    connection_id?: Id | null;
  }): Promise<DbWidget | null> {
    const base = this.wsBase();
    const connId = input.connection_id ?? this.selectedConnId;
    if (!base || !connId) {
      toasts.error('No connection selected');
      return null;
    }
    try {
      const w = await api.post<DbWidget>(`${base}/widgets`, {
        connection_id: connId,
        title: input.title,
        statement: input.statement,
        viz: input.viz,
        dashboard_id: input.dashboard_id ?? this.selectedDashboardId,
        mapping: input.mapping ?? {},
        options: input.options ?? {},
      });
      this.widgets = [...this.widgets, w];
      toasts.success('Widget added', w.title);
      return w;
    } catch (e) {
      toasts.error('Create widget failed', errMsg(e));
      return null;
    }
  }

  async updateWidget(id: Id, patch: Partial<Pick<DbWidget, 'title' | 'statement' | 'viz' | 'mapping' | 'options' | 'dashboard_id'>>): Promise<void> {
    try {
      const w = await api.patch<DbWidget>(`/db/widgets/${id}`, patch);
      this.widgets = this.widgets.map((x) => (x.id === id ? w : x));
    } catch (e) {
      toasts.error('Update widget failed', errMsg(e));
    }
  }

  async deleteWidget(id: Id): Promise<void> {
    try {
      await api.del(`/db/widgets/${id}`);
      this.widgets = this.widgets.filter((w) => w.id !== id);
    } catch (e) {
      toasts.error('Delete widget failed', errMsg(e));
    }
  }

  async runWidget(id: Id): Promise<QueryResult | null> {
    try {
      return await api.post<QueryResult>(`/db/widgets/${id}/run`, {});
    } catch (e) {
      toasts.error('Widget query failed', errMsg(e));
      return null;
    }
  }

  /** Widgets belonging to the selected dashboard. */
  widgetsForSelectedDashboard(): DbWidget[] {
    const did = this.selectedDashboardId;
    if (!did) return [];
    return this.widgets.filter((w) => w.dashboard_id === did);
  }

  // ── Agent integration ─────────────────────────────────────────────────────

  /** Ask an agent to explain a result/object; opens the new session. */
  async explainWithAgent(content: string, question?: string, title?: string): Promise<void> {
    const id = this.selectedConnId;
    if (!id) {
      toasts.error('No connection selected');
      return;
    }
    try {
      const session = await api.post<Session>(`${this.connBase(id)}/explain-with-agent`, {
        content,
        question: question ?? null,
        title: title ?? null,
      });
      ws.addSession(session);
      toasts.success('Sent to agent', session.title);
    } catch (e) {
      toasts.error('Explain with agent failed', errMsg(e));
    }
  }
}

export const database = new DatabaseStore();
