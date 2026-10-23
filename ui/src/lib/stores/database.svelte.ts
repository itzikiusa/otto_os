// Database Explorer store — workspace-scoped DB connections + schema browsing,
// query tabs, saved queries, history, and Superset-style dashboards/widgets.
// Reads `ws.currentId` only (never mutates it), mirroring apiClient.svelte.ts.
// A façade over per-concern parts in ./database/: `model` (types, tab factory,
// view modes), `statement` (SQL text helpers), and the sub-stores it owns and
// delegates to — `connections.svelte` (`conn`: open/close/switch, warm-up,
// keep-alive, and the shared `ConnEpochs` open-generation token `epochs`),
// `schemaTree.svelte` (`tree`), `objectOps.svelte` (`objects`),
// `queryRuns.svelte` (`runs`), `workbenchPersist.svelte` (`persist`),
// `dashboards.svelte` (`dash`), `assist.svelte` (`assist`, the DB Assistant
// panel), `savedQueries.svelte` (`saved`), `history.svelte` (`hist`) and
// `filters.svelte` (`chips`). The façade keeps the reactive working set (a
// connection switch snapshots and restores it whole) and re-exports every
// part, so importers keep using `lib/stores/database.svelte`.

import {
  api,
  isAbortError,
} from '../api/client';
import { resourceAccess, type ResourceAccessChange } from './resource-access.svelte';
import type {
  Connection,
  DbAssistMode,
  DbCapabilities,
  DbCompletionResponse,
  DbDashboard,
  DbExportFormat,
  DbHistoryEntry,
  DbQueryPlan,
  DbSavedQuery,
  DbSchemaGraph,
  DbTestResult,
  DbWidget,
  Id,
  ObjectDetail,
  ObjectHit,
  QueryResult,
  SchemaNode,
  Session,
} from '../api/types';
import { ws } from './workspace.svelte';
import { toasts } from '../toast.svelte';
import { formatMongo } from '../database/mongo-format';
import {
  defaultVarSpec,
  stripJavaStringConcat,
  maskQueryPlaceholders,
  unmaskQueryPlaceholders,
  type VarSpec,
} from '../database/sql-util';
import type { FilterCond } from '../database/filter-chips';
import { forgetEditorStates } from '../editor-history';
import { resultBudget } from './db-result-budget';
import { clipHistory } from './clipHistory.svelte';
import { announceModule } from '../lazyModule';
import { toastError } from '../toastError';
import { dbPrefs, loadFlag, type WarmMode } from './dbPrefs.svelte';

// The module-level model (types, constants, pure tab/view helpers) and the
// statement text helpers live in ./database/; re-exported so every importer of
// this store keeps its import path.
import {
  COMPLETION_BACKOFF_MS,
  COMPLETION_CTX_MAX,
  DEFAULT_ROW_LIMIT,
  ROW_LIMIT_KEY,
  TAB_HISTORY_PREFIX,
  blankTab,
  effectiveViewMode,
  forgetTabHistory,
  loadRowLimit,
  orderPinnedFirst,
} from './database/model';
import type {
  ConnSnapshot,
  ConnStatus,
  DbMainTab,
  DbSideTab,
  GuardedWriteAnswer,
  ManagedWriteCanceled,
  QueryTab,
  QueryTabAgent,
  SshTab,
  ViewMode,
} from './database/model';
export * from './database/model';
export * from './database/statement';
import { DbDashboardsStore } from './database/dashboards.svelte';
import { DbAssistStore } from './database/assist.svelte';
import { DbSavedQueriesStore } from './database/savedQueries.svelte';
import { DbHistoryStore } from './database/history.svelte';
import { DbFiltersStore } from './database/filters.svelte';
import { ConnEpochs, DbConnectionsStore } from './database/connections.svelte';
import { DbSchemaTreeStore } from './database/schemaTree.svelte';
import { DbObjectOpsStore } from './database/objectOps.svelte';
import { DbQueryRunsStore } from './database/queryRuns.svelte';
import { DbWorkbenchPersistStore } from './database/workbenchPersist.svelte';
export { DbDashboardsStore, type DashboardsHost } from './database/dashboards.svelte';
export { DbAssistStore, type AssistHost } from './database/assist.svelte';
export { DbSavedQueriesStore, type SavedQueriesHost } from './database/savedQueries.svelte';
export { DbHistoryStore, type HistoryHost } from './database/history.svelte';
export { DbFiltersStore, type FiltersHost } from './database/filters.svelte';
export { ConnEpochs, DbConnectionsStore, type ConnectionsHost } from './database/connections.svelte';
export { DbSchemaTreeStore, type SchemaTreeHost } from './database/schemaTree.svelte';
export { DbObjectOpsStore, type ObjectOpsHost } from './database/objectOps.svelte';
export { DbQueryRunsStore, type QueryRunsHost } from './database/queryRuns.svelte';
export { DbWorkbenchPersistStore, type WorkbenchPersistHost } from './database/workbenchPersist.svelte';

export class DatabaseStore {
  // ── Connections ────────────────────────────────────────────────────────────
  /** All workspace connections, filtered to the DB engines (workbench-openable). */
  connections: Connection[] = $state([]);
  /** Non-DB profiles (ssh/custom) — rendered in the unified sidebar tree;
   *  opening one spawns a terminal session instead of a workbench tab. */
  otherConnections: Connection[] = $state([]);
  /** Connection-list load state: a failed load renders inline with Retry in the
   *  sidebar — never as "No connections yet". */
  connectionsLoading = $state(false);
  connectionsError: string | null = $state(null);
  selectedConnId: Id | null = $state(null);
  /** Connections currently open as top-level tabs, in display order. */
  openConnIds: Id[] = $state([]);
  /**
   * ssh/custom profiles opened as workbench tabs, each backed by a live PTY
   * session. Kafka clusters use the `brokers` store's own tab list; DB kinds use
   * `openConnIds`. The tab strip renders all three so every connection kind opens
   * seamlessly in the same workbench (see `activePane`).
   */
  sshTabs: SshTab[] = $state([]);
  /**
   * Which non-DB pane is focused, or `null` when a DB connection tab is active
   * (the DB workbench keys off `selectedConnId`). Set by the tab strip / tree so
   * the main area can render the Kafka viewer or an SSH terminal in place instead
   * of navigating away.
   */
  activePane: { kind: 'kafka' | 'ssh'; id: Id } | null = $state(null);

  // ── Workbench persistence (./database/workbenchPersist.svelte.ts) ─────────
  flushTabDrafts(): void { this.persist.flushTabDrafts(); }
  restoreWorkbench(): Promise<void> { return this.persist.restoreWorkbench(); }
  /** Per-connection open generation (see `ConnEpochs`): shared by every part,
   *  bumped by `closeConnection` and by an access change. */
  readonly epochs = new ConnEpochs(() => this.openConnIds);
  // The per-concern parts (./database/*.svelte.ts). Each reads and calls the
  // façade through its `…Host` contract; the façade keeps the reactive working
  // set (it is snapshotted per connection) and delegates the behaviour.
  readonly conn: DbConnectionsStore = new DbConnectionsStore(this);
  readonly tree: DbSchemaTreeStore = new DbSchemaTreeStore(this);
  readonly objects: DbObjectOpsStore = new DbObjectOpsStore(this);
  readonly runs: DbQueryRunsStore = new DbQueryRunsStore(this);
  readonly persist: DbWorkbenchPersistStore = new DbWorkbenchPersistStore(this);

  /**
   * Per-connection liveness, keyed by connection id. Persisted across tab
   * switches (parallel to `openConnIds`); deliberately NOT part of a
   * per-connection snapshot, so a background tab keeps its red dot. Drives the
   * tab status indicator and the schema panel's connecting/error states.
   */
  connStatus: Map<Id, ConnStatus> = $state(new Map());
  capabilities: DbCapabilities | null = $state(null);
  /**
   * The result view remembered for the selected connection — the last explicit
   * pick made on any of its tabs (`setViewMode`). Fills in for tabs without a
   * pick of their own (precedence in `effectiveViewMode`); never copied INTO a
   * tab. Per-connection (snapshotted); persisted in the connection's
   * `otto_db_view:` entry next to its main/side pane.
   */
  connView: ViewMode | null = $state(null);
  testResult: DbTestResult | null = $state(null);
  testing = $state(false);
  /** Default row cap for statements without an explicit LIMIT (persisted). */
  rowLimit = $state(loadRowLimit());
  /**
   * Active database for the selected connection (SQL engines). When set, queries
   * run scoped to it (sent as the request `node`), so unqualified table names
   * resolve without a `db.` prefix. Per-connection (snapshotted).
   */
  activeDb: string | null = $state(null);

  // ── Connections, warm-up, keep-alive (./database/connections.svelte.ts) ──────
  loadConnections(): Promise<void> { return this.conn.loadConnections(); }
  openConnection(id: Id): Promise<void> { return this.conn.openConnection(id); }
  closeConnection(id: Id): void { this.conn.closeConnection(id); }
  isWarming(id: Id): boolean { return this.conn.isWarming(id); }
  get warmRestored(): WarmMode { return this.conn.warmRestored; }
  setWarmRestored(mode: WarmMode): void { this.conn.setWarmRestored(mode); }
  get keepAlive(): boolean { return this.conn.keepAlive; }
  setKeepAlive(on: boolean): void { this.conn.setKeepAlive(on); }
  applyKeepAlive(on: boolean): void { this.conn.applyKeepAlive(on); }
  warmMany(ids: readonly Id[]): Promise<void> { return this.conn.warmMany(ids); }
  warm(id: Id): Promise<void> { return this.conn.warm(id); }
  stopConnecting(id: Id): void { this.conn.stopConnecting(id); }
  ensureKeepAlive(): void { this.conn.ensureKeepAlive(); }
  keepAliveTick(): Promise<void> { return this.conn.keepAliveTick(); }
  onDaemonRestart(): void { this.conn.onDaemonRestart(); }
  focusKafka(id: Id): void { this.conn.focusKafka(id); }
  focusSsh(id: Id): void { this.conn.focusSsh(id); }
  addSshTab(tab: SshTab): void { this.conn.addSshTab(tab); }
  closeSshTab(connId: Id): void { this.conn.closeSshTab(connId); }
  retryConnection(id: Id | null = this.selectedConnId): Promise<void> { return this.conn.retryConnection(id); }
  testConnection(): Promise<void> { return this.conn.testConnection(); }

  selectedConn: Connection | null = $derived(
    this.connections.find((c) => c.id === this.selectedConnId) ?? null,
  );

  /** Selected connection points at production. Drives the red danger rail. */
  isProd: boolean = $derived(this.selectedConn?.environment === 'prod');
  /**
   * Selected connection is write-guarded: production OR explicitly read-only.
   * Writes/DDL require a typed confirmation before they run.
   */
  isGuarded: boolean = $derived(
    this.selectedConn != null &&
      (this.selectedConn.environment === 'prod' || this.selectedConn.read_only === true),
  );

  /** Liveness of the currently-selected connection (null until its first load). */
  activeConnStatus: ConnStatus | null = $derived(
    this.selectedConnId ? this.connStatus.get(this.selectedConnId) ?? null : null,
  );

  // ── Schema tree ──────────────────────────────────────────────────────────
  schemaRoot: SchemaNode[] = $state([]);
  /** Lazy children cache keyed by node id. */
  childrenCache: Map<string, SchemaNode[]> = $state(new Map());
  /** Expanded node ids. */
  expanded: Set<string> = $state(new Set());
  /** Per-node prefix filter (Redis keyspaces with many keys). Keyed by node id. */
  nodeFilters: Map<string, string> = $state(new Map());
  /** Nodes whose children are currently loading. */
  loadingNodes: Set<string> = $state(new Set());
  schemaLoading = $state(false);
  /**
   * Show engine-native row ESTIMATES next to tree objects. Off by default and
   * persisted: gathering the statistic is what makes expanding a database slow
   * on a big server, so this stays the user's explicit choice.
   */
  showCounts = $state(loadFlag('db.showCounts', false));
  /** Collapse the schema sidebar to its rail (⌘B). Persisted. */
  sidebarCollapsed = $state(loadFlag('db.sidebarCollapsed', false));

  // ── Object search (server-side; the tree's own filter is client-side and
  // can only ever match nodes that are already loaded) ─────────────────────
  objectSearchScope: 'schema' | 'all' = $state('schema');
  /**
   * The live query. Lives HERE, not in SchemaTree: the component remounts when
   * you open an object or switch side tabs, and a component-local query would
   * reset to '' while these hits survived — leaving the tree stuck on stale
   * results with no visible way back.
   */
  objectSearchQuery = $state('');
  objectSearchHits: ObjectHit[] | null = $state(null);
  objectSearching = $state(false);
  objectSearchTruncated = $state(false);
  objectSearchScanned = $state(0);

  // ── Selected object (Structure view) ────────────────────────────────────
  selectedObjectPath: string | null = $state(null);
  objectDetail: ObjectDetail | null = $state(null);
  objectLoading = $state(false);
  /** Why the selected object failed to load (null = no failure). Rendered by the
   *  Structure view so a failed open doesn't decay into the neutral empty state
   *  once the toast fades. Cleared on every new open / connection load. */
  objectError: string | null = $state(null);

  // ── File→table import dialog (0002) ──────────────────────────────────────
  // Open state + the table to prefill, set when launched from the schema-tree
  // "Import into…" action. The dialog component (mounted in DatabasePage) reads
  // these and clears `importDialogOpen` on close.
  importDialogOpen = $state(false);
  importTable = $state('');

  // ── Builder catalog cache (palette table lists, keyed by db path) ─────────
  // The schema tree is lazy/partial; the visual builder needs the full catalog
  // on demand. Cached so re-opening the palette is instant.
  builderTablesCache: Map<string, { label: string; path: string; kind: string }[]> = $state(
    new Map(),
  );

  // ── Query tabs ────────────────────────────────────────────────────────────
  tabs: QueryTab[] = $state([blankTab()]);
  activeTab = $state(0);
  get tab(): QueryTab {
    return this.tabs[this.activeTab] ?? this.tabs[0];
  }

  // ── Query runs, plans, cancel, re-attach (./database/queryRuns.svelte.ts) ─
  enforceResultBudget(): void { this.runs.enforceResultBudget(); }
  rerunReleased(): Promise<QueryResult | null> { return this.runs.rerunReleased(); }
  runQuery(...a: Parameters<DbQueryRunsStore['runQuery']>): Promise<QueryResult | null> { return this.runs.runQuery(...a); }
  runPage(...a: Parameters<DbQueryRunsStore['runPage']>): Promise<QueryResult | null> { return this.runs.runPage(...a); }
  confirmGuardedWrite(conn: Pick<Connection, 'name' | 'environment'> | null, blocked?: unknown, agentLabel?: string): Promise<GuardedWriteAnswer> { return this.runs.confirmGuardedWrite(conn, blocked, agentLabel); }
  runManagedStatement(sql: string, node?: string | null): Promise<QueryResult | ManagedWriteCanceled> { return this.runs.runManagedStatement(sql, node); }
  runExplain(opts?: { readOnly?: boolean }): Promise<QueryResult | null> { return this.runs.runExplain(opts); }
  explainPlan(opts?: { readOnly?: boolean }): Promise<void> { return this.runs.explainPlan(opts); }
  closePlan(): void { this.runs.closePlan(); }
  abortQuery(tabId?: number, opts?: { report?: boolean }): void { this.runs.abortQuery(tabId, opts); }
  reattach(t: QueryTab): Promise<void> { return this.runs.reattach(t); }

  // ── UI tabs ────────────────────────────────────────────────────────────────
  mainTab: DbMainTab = $state('query');
  /** A statement handed to the Builder ("Open in Builder" on a query tab): the
   *  builder parses it into its model on mount and clears this. Scoped to the
   *  connection it came from so a switch can't import it into another server. */
  builderImport: { connId: Id; sql: string } | null = $state(null);
  /** Send a statement to the Builder tab (round trip from the editor). */
  openInBuilder(sql: string): void {
    if (!this.selectedConnId || !this.supportsBuilder) return;
    this.builderImport = { connId: this.selectedConnId, sql };
    this.setMainTab('builder');
  }
  // Default to the connection picker — it's the global view shown before any
  // connection is open. Opening a connection switches to 'schema' (see
  // loadConnectionFresh); snapshots never restore 'connections' (captureSnapshot).
  sideTab: DbSideTab = $state('connections');

  // ── Saved queries (./database/savedQueries.svelte.ts) ─────────────────────
  readonly saved = new DbSavedQueriesStore({
    wsBase: () => this.wsBase(),
    accessEpoch: () => this.accessEpoch,
    selectedConnId: () => this.selectedConnId,
    tabs: () => this.tabs,
    activeTab: () => this.tab ?? null,
    locateTab: (tabId) => this.locateTab(tabId),
    persistTabsNow: (connId) => this.persist.persistTabsNow(connId),
    persistTabs: () => this.persist.persistTabs(),
    focusQueryTab: (i) => {
      this.switchTab(i);
      this.setMainTab('query');
    },
    newTab: (statement) => this.newTab(statement),
  });
  get savedQueries(): DbSavedQuery[] { return this.saved.savedQueries; }
  set savedQueries(v: DbSavedQuery[]) { this.saved.savedQueries = v; }
  get savedQueriesLoading(): boolean { return this.saved.savedQueriesLoading; }
  get savedQueriesError(): string | null { return this.saved.savedQueriesError; }

  // ── History (./database/history.svelte.ts) ───────────────────────────────
  readonly hist = new DbHistoryStore({
    selectedConnId: () => this.selectedConnId,
    accessEpoch: () => this.accessEpoch,
    newTab: (statement) => this.newTab(statement),
  });
  get history(): DbHistoryEntry[] { return this.hist.history; }
  set history(v: DbHistoryEntry[]) { this.hist.history = v; }
  get historyLoading(): boolean { return this.hist.historyLoading; }
  get historyError(): string | null { return this.hist.historyError; }
  set historyError(v: string | null) { this.hist.historyError = v; }
  get historyLimit(): number { return this.hist.historyLimit; }
  set historyLimit(v: number) { this.hist.historyLimit = v; }
  get historyLoadingMore(): boolean { return this.hist.historyLoadingMore; }

  // ── Dashboards / widgets (./database/dashboards.svelte.ts) ─────────────────
  readonly dash = new DbDashboardsStore({
    wsBase: () => this.wsBase(),
    accessEpoch: () => this.accessEpoch,
    selectedConnId: () => this.selectedConnId,
  });
  get dashboards(): DbDashboard[] { return this.dash.dashboards; }
  set dashboards(v: DbDashboard[]) { this.dash.dashboards = v; }
  get widgets(): DbWidget[] { return this.dash.widgets; }
  set widgets(v: DbWidget[]) { this.dash.widgets = v; }
  get dashboardsLoading(): boolean { return this.dash.dashboardsLoading; }
  get dashboardsError(): string | null { return this.dash.dashboardsError; }
  set dashboardsError(v: string | null) { this.dash.dashboardsError = v; }
  get selectedDashboardId(): Id | null { return this.dash.selectedDashboardId; }
  set selectedDashboardId(v: Id | null) { this.dash.selectedDashboardId = v; }
  get selectedDashboard(): DbDashboard | null { return this.dash.selectedDashboard; }

  // ── Query plan (Explain) ──────────────────────────────────────────────────
  // A normalized query plan from the engine's native EXPLAIN, shown in a
  // collapsible tree panel beside the results. Null when the panel is closed.
  queryPlan: DbQueryPlan | null = $state(null);
  planOpen = $state(false);

  // ── DB Assistant (./database/assist.svelte.ts) ───────────────────────────
  readonly assist = new DbAssistStore({
    selectedConnId: () => this.selectedConnId,
    activeDb: () => this.activeDb,
    currentStatement: () => this.tab?.statement ?? '',
    insertIntoQuery: (sql) => {
      this.mainTab = 'query';
      this.setStatement(sql);
    },
    runReadOnly: async (agentLabel, confirmWrite) => {
      await this.runs.runQuery(undefined, undefined, { readOnly: true, agentLabel, confirmWrite });
    },
  });
  get assistOpen(): boolean { return this.assist.assistOpen; }
  set assistOpen(v: boolean) { this.assist.assistOpen = v; }
  get assistConnId(): Id | null { return this.assist.assistConnId; }
  get assistId(): string | null { return this.assist.assistId; }
  get assistSessionId(): string | null { return this.assist.assistSessionId; }
  get assistMode(): DbAssistMode { return this.assist.assistMode; }
  get assistProvider(): string { return this.assist.assistProvider; }
  get assistProposedSql(): string { return this.assist.assistProposedSql; }
  set assistProposedSql(v: string) { this.assist.assistProposedSql = v; }
  get assistNote(): string { return this.assist.assistNote; }
  set assistNote(v: string) { this.assist.assistNote = v; }
  get assistBusy(): boolean { return this.assist.assistBusy; }
  openAssist(mode: DbAssistMode, resultContext?: string | null): void { this.assist.openAssist(mode, resultContext); }
  setAssistProvider(provider: string): void { this.assist.setAssistProvider(provider); }
  startAssist(question: string): Promise<void> { return this.assist.startAssist(question); }
  stopAssist(): void { this.assist.stopAssist(); }
  setAssistSession(assistId: string, connId: Id, sessionId: string): void { this.assist.setAssistSession(assistId, connId, sessionId); }
  applyAssistUpdate(assistId: string, connId: Id, sql: string, note: string): void { this.assist.applyAssistUpdate(assistId, connId, sql, note); }
  insertAssistSql(): Promise<boolean> { return this.assist.insertAssistSql(); }
  runAssistSql(): Promise<void> { return this.assist.runAssistSql(); }
  summarizeAssist(): Promise<void> { return this.assist.summarizeAssist(); }
  closeAssist(): Promise<void> { return this.assist.closeAssist(); }

  // ── Path helpers ────────────────────────────────────────────────────────
  connBase(id: Id): string {
    return `/connections/${id}/db`;
  }
  private wsBase(): string | null {
    return ws.currentId ? `/workspaces/${ws.currentId}/db` : null;
  }

  accessEpoch = 0;
  accessRevision = $state(0);
  /** Discard server-derived data on revocation. Keep this user's draft SQL. */
  onAccessChange(change: ResourceAccessChange): void {
    if (change.type === 'decision' && change.kind !== 'connection') return;
    if (change.type === 'decision') {
      const relevant = ['discover', 'db_browse', 'db_query', 'db_export'];
      const lost =
        change.before &&
        relevant.some(
          (op) => change.before?.operations[op]?.allowed && !change.after?.operations[op]?.allowed,
        );
      if (!lost) return;
    }
    if (change.type === 'reset' && change.identity) this.persist.flushPersistTabs();
    this.accessEpoch++;
    this.accessRevision++;
    const ids =
      change.type === 'decision'
        ? [change.id]
        : [
            ...new Set([
              ...this.openConnIds,
              ...this.conn.snapshots.keys(),
              ...(this.selectedConnId ? [this.selectedConnId] : []),
            ]),
          ];
    const scrub = (state: ConnSnapshot | DatabaseStore) => {
      for (const tab of state.tabs) {
        const run = this.runs.runControllers.get(tab.id);
        run?.controller.abort();
        this.runs.runControllers.delete(tab.id);
        tab.pending = null;
        tab.running = false;
        tab.result = null;
        resultBudget.forget(tab.uid);
        tab.ran_statement = null;
        tab.ran_node = null;
        tab.error = null;
        tab.err_statement = null;
      }
      state.capabilities = null;
      state.connView = null;
      state.testResult = null;
      state.schemaRoot = [];
      state.childrenCache = new Map();
      state.expanded = new Set();
      state.loadingNodes = new Set();
      state.schemaLoading = false;
      state.selectedObjectPath = null;
      state.objectDetail = null;
      state.objectError = null;
      state.objectSearchQuery = '';
      state.objectSearchHits = null;
      state.objectSearchTruncated = false;
      state.objectSearchScanned = 0;
      state.savedQueries = [];
      state.history = [];
      if (state === this) this.builderTablesCache = new Map();
      else (state as ConnSnapshot).builderTablesCache = new Map();
    };
    for (const id of ids) {
      this.epochs.bump(id);
      const snapshot = this.conn.snapshots.get(id);
      if (snapshot) scrub(snapshot);
      if (this.selectedConnId === id) scrub(this);
    }
    this.queryPlan = null;
    this.planOpen = false;
    this.tree.objectSearchSeq++;
    this.objectSearching = false;
    this.assistOpen = false;
    if (change.type === 'reset' && change.identity) {
      // Another person's undo history (every string they typed) must not be
      // reachable from this identity.
      forgetEditorStates(TAB_HISTORY_PREFIX);
      this.connections = [];
      this.otherConnections = [];
      this.openConnIds = [];
      this.selectedConnId = null;
      this.conn.snapshots.clear();
      this.tabs = [blankTab()];
      this.activeTab = 0;
      this.activeDb = null;
      this.sshTabs = [];
      this.activePane = null;
      this.dashboards = [];
      this.widgets = [];
      this.dashboardsError = null;
      this.savedQueries = [];
      this.history = [];
    } else if (
      change.type === 'decision' &&
      change.child === undefined &&
      !change.after?.operations.discover?.allowed
    ) {
      this.connections = this.connections.filter((c) => c.id !== change.id);
      this.otherConnections = this.otherConnections.filter((c) => c.id !== change.id);
      this.openConnIds = this.openConnIds.filter((id) => id !== change.id);
      if (this.selectedConnId === change.id) this.selectedConnId = null;
    }
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
    this.persist.persistTabs();
  }
  switchTab(i: number): void {
    if (i >= 0 && i < this.tabs.length) {
      this.activeTab = i;
      this.persist.persistTabs();
    }
  }
  closeTab(i: number): void {
    // A closed tab must not keep querying: cancel its in-flight run (server-side
    // too) and drop its pending marker so the reattach loop stops polling it.
    const closing = this.tabs[i];
    // A pin is a promise the tab stays: every close path (×, ⌥⌘W, the strip
    // menu) refuses until it's unpinned, instead of silently discarding work.
    if (closing?.pinned) {
      toasts.info('Unpin to close');
      return;
    }
    if (closing) {
      this.runs.abortQuery(closing.id);
      forgetTabHistory(closing);
    }
    if (this.tabs.length === 1) {
      this.tabs = [blankTab()];
      this.activeTab = 0;
    } else {
      this.tabs = this.tabs.filter((_, idx) => idx !== i);
      if (this.activeTab >= this.tabs.length) this.activeTab = this.tabs.length - 1;
      else if (i < this.activeTab) this.activeTab -= 1;
    }
    this.persist.persistTabs();
  }
  /**
   * Close every tab but `i`, or (`closeAllTabs`) every tab — pinned tabs always
   * survive both. Closed tabs' runs are cancelled exactly like `closeTab`; when
   * nothing survives a fresh blank tab takes over.
   */
  closeOtherTabs(i: number): void {
    this.closeTabsWhere((t, idx) => idx !== i && !t.pinned, i);
  }
  closeAllTabs(): void {
    this.closeTabsWhere((t) => !t.pinned, null);
  }
  private closeTabsWhere(
    shouldClose: (t: QueryTab, idx: number) => boolean,
    keepActive: number | null,
  ): void {
    const active = keepActive === null ? null : this.tabs[keepActive] ?? null;
    const survivors: QueryTab[] = [];
    for (const [idx, t] of this.tabs.entries()) {
      if (shouldClose(t, idx)) {
        this.runs.abortQuery(t.id);
        forgetTabHistory(t);
      } else survivors.push(t);
    }
    this.tabs = survivors.length ? survivors : [blankTab()];
    // Keep the caller's tab focused when it survived; else the first survivor
    // (a pinned tab, or the fresh blank one).
    const at = active ? this.tabs.indexOf(active) : -1;
    this.activeTab = at >= 0 ? at : 0;
    this.persist.persistTabs();
  }
  /** Pin / unpin a tab. Pinned tabs group at the front of the strip (the same
   *  order a reload restores), so pinning moves the tab there and unpinning
   *  drops it to the head of the unpinned group; focus follows the active tab. */
  togglePinTab(i: number): void {
    const t = this.tabs[i];
    if (!t) return;
    t.pinned = !t.pinned;
    const ordered = orderPinnedFirst(this.tabs, this.activeTab);
    this.tabs = ordered.tabs;
    this.activeTab = ordered.activeTab;
    this.persist.persistTabs();
  }

  /** A query tab by its stable uid, across the live set and every parked
   *  connection snapshot (the editor's history store asks per path). */
  tabByUid(uid: string): QueryTab | null {
    const live = this.tabs.find((t) => t.uid === uid);
    if (live) return live;
    for (const snap of this.conn.snapshots.values()) {
      const t = snap.tabs.find((x) => x.uid === uid);
      if (t) return t;
    }
    return null;
  }

  // ── Agent UI control (lib/uiCommands/database.ts) ─────────────────────────
  // An agent session drives the Explorer through the SAME methods the buttons
  // call; these helpers only address a tab by its id across open connections
  // and stamp the agent's attribution on the tabs it opens.

  /** Enumerate the live tabs of actual open connections. The active singleton
   * wins over its older parked snapshot; returned tabs retain their proxies so
   * background results/running state update a comparison without row copies. */
  openQueryTabs(): Array<{ connId: Id; tab: QueryTab }> {
    void this.accessRevision;
    const selected = this.selectedConnId;
    const liveTabs = this.tabs;
    const known = new Set(this.connections.map((connection) => connection.id));
    return this.openConnIds.flatMap((connId) => {
      if (!known.has(connId)) return [];
      const tabs = connId === selected ? liveTabs : this.conn.snapshots.get(connId)?.tabs ?? [];
      return tabs.map((tab) => ({ connId, tab }));
    });
  }

  /** Find a query tab by id on ANY open connection — the active one, or a
   *  parked (snapshotted) background one. */
  locateTab(tabId: number): { connId: Id; index: number; tab: QueryTab } | null {
    const sel = this.selectedConnId;
    if (sel) {
      const i = this.tabs.findIndex((t) => t.id === tabId);
      if (i >= 0) return { connId: sel, index: i, tab: this.tabs[i] };
    }
    for (const [connId, snap] of this.conn.snapshots) {
      if (connId === sel) continue; // the live singletons above are newer
      const i = snap.tabs.findIndex((t) => t.id === tabId);
      if (i >= 0) return { connId, index: i, tab: snap.tabs[i] };
    }
    return null;
  }

  /** Bring a tab on screen: focus its connection (the DB pane, not a Kafka/SSH
   *  one), make it the active tab and show the Query view. Null when no open
   *  connection has it (closed, or ids reset by a reload). */
  async focusTab(tabId: number): Promise<QueryTab | null> {
    const at = this.locateTab(tabId);
    if (!at) return null;
    if (this.activePane !== null || this.selectedConnId !== at.connId) {
      await this.conn.openConnection(at.connId);
    }
    const i = this.tabs.findIndex((t) => t.id === tabId);
    if (i < 0) return null;
    if (i !== this.activeTab) this.switchTab(i);
    if (this.mainTab !== 'query') this.setMainTab('query');
    return this.tabs[i] ?? null;
  }

  /** Open a new tab on the SELECTED connection, attributed to an agent. */
  newAgentTab(statement: string, agent: QueryTabAgent): QueryTab {
    this.newTab(statement);
    const t = this.tab;
    t.agent = { ...agent };
    this.persist.persistTabs();
    return t;
  }

  /**
   * A pending "Export all rows…" the agent asked for (`otto.ui_db_export`):
   * DatabasePage mounts the ExportDialog prefilled from it while set, and the
   * HUMAN picks the folder/file and confirms. `done` reports what happened —
   * `{exported:false}` when they dismissed it. Raw (not deep-proxied): it
   * carries a callback.
   */
  exportRequest: {
    connId: Id;
    statement: string;
    node: string | null;
    format?: DbExportFormat;
    maxRows?: number;
    agentLabel?: string;
    done: (r: { exported: boolean; path?: string; rows?: number; bytes?: number }) => void;
  } | null = $state.raw(null);

  // ── Result view mode ──────────────────────────────────────────────────────
  /**
   * Set the active tab's explicit result view (`null` = back to automatic). An
   * explicit pick is ALSO remembered for the connection so the next tab opened
   * here starts the same way — but the memory is only ever a fallback: it never
   * writes into a tab, so clearing a tab's pick really returns it to automatic.
   */
  setViewMode(mode: ViewMode | null): void {
    const t = this.tab;
    if (!t) return;
    t.viewMode = mode;
    this.persist.persistTabs();
    if (mode !== null) {
      this.connView = mode;
      this.persist.persistView();
    }
  }
  /** ⇧⌘V: Grid → Vertical → JSON → Grid, stepping from the view actually on
   *  screen (`effective`, resolved by the caller) and stored as an explicit pick. */
  cycleViewMode(effective: ViewMode): void {
    const next: Record<ViewMode, ViewMode> = { grid: 'vertical', vertical: 'json', json: 'grid' };
    this.setViewMode(next[effective] ?? 'grid');
  }
  setStatement(value: string): void {
    const t = this.tab;
    if (t) {
      t.statement = value;
      t.offset = 0; // editing the statement resets the pager to the first page
    }
    this.persist.persistTabs();
  }

  /** Set + persist a query-level variable value on the active tab. */
  /** Patch a query-level variable (value / type / escape) on the active tab. */
  setVar(name: string, patch: Partial<VarSpec>): void {
    const t = this.tab;
    if (!t) return;
    const cur = t.vars[name] ?? defaultVarSpec();
    t.vars = { ...t.vars, [name]: { ...cur, ...patch } };
    this.persist.persistTabs();
  }

  /** Beautify the active tab's statement. Mongo uses a structural JS/JSON
   *  re-indenter (`db.coll.op({…})` — the SQL formatter chokes on it); the SQL
   *  engines use sql-formatter; redis (one-line commands) has nothing to format.
   *  For SQL we first unwrap Java/MyBatis string-concatenation (paste a
   *  `"SELECT … " + ${x} + "…"` blob and Format turns it into clean SQL) and mask
   *  query placeholders (`${x}`/`#{x}`/`:x`/`{x}`) so the formatter doesn't choke
   *  on them, restoring them after. Leaves the editor untouched on a parse error.
   *  sql-formatter (~370 KB of dialect tables) loads on the first Format: this
   *  store is in every window's boot closure (lib/events.svelte.ts imports it). */
  async formatStatement(): Promise<void> {
    const t = this.tab;
    if (!t || !t.statement.trim() || this.queryLanguage === 'redis') return;
    try {
      if (this.queryLanguage === 'mongo') {
        this.setStatement(formatMongo(t.statement));
        return;
      }
      const { format: formatSql } = await import('sql-formatter');
      // The tab switched while the formatter loaded: leave both untouched.
      if (this.tab !== t) return;
      const kind = this.selectedConn?.kind;
      const dialect: 'mysql' | 'postgresql' | 'sql' =
        kind === 'mysql' ? 'mysql' : kind === 'postgres' ? 'postgresql' : 'sql';
      const unwrapped = stripJavaStringConcat(t.statement);
      const { masked, tokens } = maskQueryPlaceholders(unwrapped);
      const formatted = formatSql(masked, { language: dialect, keywordCase: 'upper' });
      this.setStatement(unmaskQueryPlaceholders(formatted, tokens));
    } catch (e) {
      toastError('Couldn’t format', e);
    }
  }

  /** Set + persist the default row cap (used when a query has no own LIMIT). */
  setRowLimit(n: number): void {
    this.rowLimit = n > 0 ? n : DEFAULT_ROW_LIMIT;
    if (typeof localStorage !== 'undefined') {
      localStorage.setItem(ROW_LIMIT_KEY, String(this.rowLimit));
    }
  }

  /** Database names available on the active connection (from the schema root). */
  get databaseNames(): string[] {
    return this.schemaRoot.filter((n) => n.kind === 'database').map((n) => n.label);
  }

  /** Redis logical DBs (db0, db1, …) from the schema root. `id` is the keyspace
   *  path (`kdb:N`) used as the active-DB scope; `label` is what's shown. */
  get keyspaces(): { id: string; label: string }[] {
    return this.schemaRoot
      .filter((n) => n.kind === 'keyspace')
      .map((n) => ({ id: n.id, label: n.label }));
  }

  /** Whether the active connection is Redis. */
  get isRedis(): boolean {
    return this.capabilities?.engine === 'redis';
  }

  /** Set the active database (queries scope to it). Empty string clears it.
   * Persisted with the connection's tabs so it survives reopening. */
  setActiveDb(name: string | null): void {
    this.activeDb = name && name.length > 0 ? name : null;
    this.persist.persistTabs();
  }

  /** Switch the main pane view for the active connection (persisted per conn). */
  setMainTab(t: DbMainTab): void {
    this.mainTab = t;
    this.persist.persistView();
  }
  /** Switch the sidebar view for the active connection (persisted per conn). */
  setSideTab(t: DbSideTab): void {
    this.sideTab = t;
    this.persist.persistView();
  }

  /** Un-applied cell edits in the results grid of the ACTIVE query tab
   *  (written by ResultsGrid) — a tab close asks before discarding them. */
  livePendingEdits = $state(0);

  // ── Schema tree + object search + builder catalog (./database/schemaTree.svelte.ts)
  refreshSchema(): Promise<void> { return this.tree.refreshSchema(); }
  isExpanded(nodeId: string): boolean { return this.tree.isExpanded(nodeId); }
  childrenOf(nodeId: string): SchemaNode[] | undefined { return this.tree.childrenOf(nodeId); }
  isLoadingNode(nodeId: string): boolean { return this.tree.isLoadingNode(nodeId); }
  expand(node: SchemaNode): Promise<void> { return this.tree.expand(node); }
  nodeFilter(nodeId: string): string { return this.tree.nodeFilter(nodeId); }
  applyNodeFilter(node: SchemaNode, value: string): Promise<void> { return this.tree.applyNodeFilter(node, value); }
  setShowCounts(on: boolean): void { this.tree.setShowCounts(on); }
  toggleSidebar(): void { this.tree.toggleSidebar(); }
  searchObjects(q: string, schema?: string): Promise<void> { return this.tree.searchObjects(q, schema); }
  clearObjectSearch(): void { this.tree.clearObjectSearch(); }
  fetchSchemaGraph(schema: string, maxTables = 60): Promise<DbSchemaGraph | null> { return this.tree.fetchSchemaGraph(schema, maxTables); }
  listBuilderDatabases(): Promise<{ name: string; path: string }[]> { return this.tree.listBuilderDatabases(); }
  listBuilderTables(dbPath: string): Promise<{ label: string; path: string; kind: string }[]> { return this.tree.listBuilderTables(dbPath); }

  // ── Object operations (./database/objectOps.svelte.ts) ───────────────────
  openObject(node: SchemaNode): Promise<void> { return this.objects.openObject(node); }
  fetchObject(path: string): Promise<ObjectDetail | null> { return this.objects.fetchObject(path); }
  copyCreateStatement(node: SchemaNode): Promise<void> { return this.objects.copyCreateStatement(node); }
  refreshObject(): Promise<void> { return this.objects.refreshObject(); }
  tableRefFromNode(node: SchemaNode): { ref: string; db: string | null; table: string } | null { return this.objects.tableRefFromNode(node); }
  openInNewTab(sql: string, opts?: { run?: boolean; name?: string; node?: string }): Promise<void> { return this.objects.openInNewTab(sql, opts); }
  redisKeyParts(node: SchemaNode): { key: string; keyspace: string } | null { return this.objects.redisKeyParts(node); }
  redisReadCommand(type: string | undefined, key: string): string { return this.objects.redisReadCommand(type, key); }
  getRedisValue(node: SchemaNode, opts?: { run?: boolean }): Promise<void> { return this.objects.getRedisValue(node, opts); }
  selectRows(node: SchemaNode): Promise<void> { return this.objects.selectRows(node); }
  sendSelectToEditor(node: SchemaNode): Promise<void> { return this.objects.sendSelectToEditor(node); }
  openImportDialog(node?: SchemaNode): void { this.objects.openImportDialog(node); }
  collectionRefFromNode(node: SchemaNode): { db: string | null; coll: string } | null { return this.objects.collectionRefFromNode(node); }
  findRows(node: SchemaNode): Promise<void> { return this.objects.findRows(node); }
  sendFindToEditor(node: SchemaNode): Promise<void> { return this.objects.sendFindToEditor(node); }
  truncateTable(node: SchemaNode): Promise<void> { return this.objects.truncateTable(node); }
  dropObject(node: SchemaNode): Promise<void> { return this.objects.dropObject(node); }

  // ── Quick filters (chips that own the active tab's WHERE clause) ───────────
  // Delegated to lib/stores/database/filters.svelte.ts.
  readonly chips = new DbFiltersStore({
    activeTab: () => this.tab ?? null,
    engine: () => this.selectedConn?.kind ?? null,
    setStatement: (statement) => this.setStatement(statement),
    persistTabs: () => this.persist.persistTabs(),
  });
  get filters(): FilterCond[] { return this.chips.filters; }
  addQuickFilter(column: string, value: unknown, mode: 'include' | 'exclude'): void { this.chips.addQuickFilter(column, value, mode); }
  addColumnFilter(column: string): void { this.chips.addColumnFilter(column); }
  addFilterValue(condIndex: number, text: string): void { this.chips.addFilterValue(condIndex, text); }
  removeFilterValue(condIndex: number, valIndex: number): void { this.chips.removeFilterValue(condIndex, valIndex); }
  removeFilterCond(condIndex: number): void { this.chips.removeFilterCond(condIndex); }
  toggleFilterMode(condIndex: number): void { this.chips.toggleFilterMode(condIndex); }
  clearFilters(): void { this.chips.clearFilters(); }
  setStatementFromCellFilter(statement: string): void { this.chips.setStatementFromCellFilter(statement); }

  /** Until when (epoch ms) completion stays quiet after a failed request, per
   *  connection — a broken connection must not be re-asked on every word. */
  private completionBackoff = new Map<Id, number>();

  /**
   * Fetch completions for the text around the cursor. Callers scope `prefix` /
   * `suffix` to the current statement; both are capped here as a backstop so a
   * request can never carry a whole large buffer (or trip the daemon's body
   * limit). `signal` aborts a superseded request.
   */
  async complete(
    prefix: string,
    suffix = '',
    node?: string,
    signal?: AbortSignal,
  ): Promise<DbCompletionResponse> {
    const none: DbCompletionResponse = { items: [] };
    const id = this.selectedConnId;
    if (!id) return none;
    if ((this.completionBackoff.get(id) ?? 0) > Date.now()) return none;
    try {
      const res = await api.post<DbCompletionResponse>(
        `${this.connBase(id)}/completion`,
        {
          prefix: prefix.length > COMPLETION_CTX_MAX ? prefix.slice(-COMPLETION_CTX_MAX) : prefix,
          suffix: suffix.length > COMPLETION_CTX_MAX ? suffix.slice(0, COMPLETION_CTX_MAX) : suffix,
          database:
            this.activeDb ??
            (this.selectedConn?.params?.db ? String(this.selectedConn.params.db) : undefined),
          node: node ?? null,
        },
        signal,
      );
      this.completionBackoff.delete(id);
      return { items: res.items ?? [], truncated: res.truncated === true };
    } catch (e) {
      // Completion failures must never break typing — degrade silently, and
      // back off briefly (a superseded, aborted request is not a failure).
      if (!isAbortError(e) && !signal?.aborted) {
        this.completionBackoff.set(id, Date.now() + COMPLETION_BACKOFF_MS);
      }
      return none;
    }
  }

  /**
   * Clear the daemon's cached completion snapshot for the active connection so
   * the next completion re-introspects the live schema. Fire-and-forget; called
   * from `refreshSchema`/`retryConnection`. Completion stays cached otherwise.
   */
  async refreshCompletions(id: Id | null = this.selectedConnId): Promise<void> {
    if (!id) return;
    try {
      await api.post(`/connections/${id}/db/completion/refresh`, {});
    } catch {
      // Best-effort — a failed cache clear must never break the refresh flow.
    }
  }

  // ── Saved queries / history: delegated to `saved` / `hist` ─────────────────
  loadSavedQueries(): Promise<void> { return this.saved.loadSavedQueries(); }
  saveQuery(name: string, statement: string): Promise<DbSavedQuery | null> { return this.saved.saveQuery(name, statement); }
  updateSavedQuery(id: Id, patch: { name?: string; statement?: string }): Promise<DbSavedQuery | null> { return this.saved.updateSavedQuery(id, patch); }
  renameSavedQuery(id: Id, name: string): Promise<void> { return this.saved.renameSavedQuery(id, name); }
  saveActiveTab(name: string): Promise<DbSavedQuery | null> { return this.saved.saveActiveTab(name); }
  deleteSavedQuery(id: Id): Promise<void> { return this.saved.deleteSavedQuery(id); }
  openSavedQuery(q: DbSavedQuery): void { this.saved.openSavedQuery(q); }
  loadHistory(connId?: Id): Promise<void> { return this.hist.loadHistory(connId); }
  loadMoreHistory(): Promise<void> { return this.hist.loadMoreHistory(); }
  get canLoadMoreHistory(): boolean { return this.hist.canLoadMoreHistory; }
  openHistory(h: DbHistoryEntry): Promise<void> { return this.hist.openHistory(h); }

  // ── Dashboards / widgets: delegated to `dash` ──────────────────────────────
  loadDashboards(): Promise<void> { return this.dash.loadDashboards(); }
  loadWidgets(): Promise<void> { return this.dash.loadWidgets(); }
  createDashboard(name: string): Promise<DbDashboard | null> { return this.dash.createDashboard(name); }
  renameDashboard(id: Id, name: string): Promise<void> { return this.dash.renameDashboard(id, name); }
  setDashboardRefresh(id: Id, refresh_secs: number | null): Promise<void> { return this.dash.setDashboardRefresh(id, refresh_secs); }
  deleteDashboard(id: Id): Promise<void> { return this.dash.deleteDashboard(id); }
  createWidget(input: Parameters<DbDashboardsStore['createWidget']>[0]): Promise<DbWidget | null> { return this.dash.createWidget(input); }
  updateWidget(id: Id, patch: Parameters<DbDashboardsStore['updateWidget']>[1]): Promise<DbWidget | null> { return this.dash.updateWidget(id, patch); }
  deleteWidget(id: Id): Promise<void> { return this.dash.deleteWidget(id); }
  runWidget(id: Id): Promise<QueryResult | null> { return this.dash.runWidget(id); }
  /** Widgets belonging to the selected dashboard. */
  widgetsForSelectedDashboard(): DbWidget[] { return this.dash.widgetsForSelectedDashboard(); }

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
      toastError('Couldn’t explain with agent', e);
    }
  }
}


export const database = new DatabaseStore();
// Routed by `peek()` in lib/events.svelte.ts (perf G2): let it see this store
// however it was first imported.
announceModule('database', database);
// Settings → Appearance flips the keep-alive pref without importing this store.
dbPrefs.onKeepAliveChange((on) => database.applyKeepAlive(on));
// Copies made while a masked tab is active never reach the clipboard ring.
clipHistory.setGuard(() => database.tab?.mask === true);
if (typeof window !== 'undefined') {
  // The tab-draft write is debounced 300 ms — flush it when the page goes away
  // (reload / quit) or is hidden (app switch), so the last edits survive.
  window.addEventListener('pagehide', () => database.flushTabDrafts());
  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'hidden') database.flushTabDrafts();
  });
}
resourceAccess.subscribe(change=>database.onAccessChange(change));
