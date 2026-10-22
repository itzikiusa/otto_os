// Database Explorer store — workspace-scoped DB connections + schema browsing,
// query tabs, saved queries, history, and Superset-style dashboards/widgets.
// Reads `ws.currentId` only (never mutates it), mirroring apiClient.svelte.ts.
// A façade over per-concern parts in ./database/: `model` (types, tab factory,
// view modes), `statement` (SQL text helpers), and the sub-stores it owns and
// delegates to — `dashboards.svelte` (`dash`), `assist.svelte` (`assist`, the
// DB Assistant panel), `savedQueries.svelte` (`saved`) and `history.svelte`
// (`hist`); it re-exports all of them, so importers keep using
// `lib/stores/database.svelte`.

import {
  api,
  ApiError,
  isAbortError,
  withDeadline,
  READ_DEADLINE_MS,
  dbCloseConnection,
} from '../api/client';
import { auth } from './auth.svelte';
import { resourceAccess, type ResourceAccessChange } from './resource-access.svelte';
import { confirmer } from '../confirm.svelte';
import type {
  Connection,
  DbAssistMode,
  DbCancelOutcome,
  DbCapabilities,
  DbCompletionResponse,
  DbDashboard,
  DbExportFormat,
  DbHistoryEntry,
  DbQueryPlan,
  DbSavedQuery,
  DbSchemaGraph,
  DbTestResult,
  DbQueryStatus,
  DbWidget,
  Id,
  ObjectDetail,
  ObjectHit,
  ObjectSearchResult,
  QueryResult,
  RunQueryReq,
  SchemaNode,
  Session,
} from '../api/types';
import { ws } from './workspace.svelte';
import { toasts } from '../toast.svelte';
import { router } from '../router.svelte';
import { formatMongo } from '../database/mongo-format';
import {
  defaultVarSpec,
  stripJavaStringConcat,
  maskQueryPlaceholders,
  unmaskQueryPlaceholders,
  type VarSpec,
} from '../database/sql-util';
import { condToSql, parseFilterValText, toFilterVal, type FilterCond } from '../database/filter-chips';
import { tableRefFromNodeId } from '../database/sql-dialect';
import { normalizeDbError } from '../database/error-normalize';
import { copyTextOrThrow } from '../clipboard';
import { mapLimit, pollWhileVisible, type Poller } from '../poll';
import { forgetEditorStates, hydrateEditorHistory } from '../editor-history';
import { parkedEditCount, releaseGridResult } from '../database/grid-tab-state';
import { estimateResultBytes, isReleased, releasedStub, resultBudget } from './db-result-budget';
import { clipHistory } from './clipHistory.svelte';
import { loadErrorText } from '../loadError';
import { announceModule } from '../lazyModule';
import { toastError } from '../toastError';
import { dbPrefs, loadFlag, saveFlag, type WarmMode } from './dbPrefs.svelte';

// The module-level model (types, constants, pure tab/view helpers) and the
// statement text helpers live in ./database/; re-exported so every importer of
// this store keeps its import path.
import {
  COMPLETION_BACKOFF_MS,
  COMPLETION_CTX_MAX,
  DEFAULT_ROW_LIMIT,
  KEEP_ALIVE_MS,
  PENDING_SQL_MAX,
  ROW_LIMIT_KEY,
  TAB_HISTORY_PREFIX,
  WARM_CONCURRENCY,
  blankTab,
  effectiveViewMode,
  forgetTabHistory,
  isDbKind,
  isReadOnlyRefusal,
  isViewMode,
  isWriteBlocked,
  loadRowLimit,
  newQueryId,
  normalizeVars,
  orderPinnedFirst,
  rawResult,
} from './database/model';
import type {
  ConnPhase,
  ConnSnapshot,
  ConnStatus,
  DbMainTab,
  DbSideTab,
  GuardedWriteAnswer,
  ManagedWriteCanceled,
  PendingRun,
  QueryTab,
  QueryTabAgent,
  RunOutcome,
  SshTab,
  ViewMode,
} from './database/model';
import {
  extractWhereBody,
  parseExplicitLimit,
  rewriteWhere,
} from './database/statement';
export * from './database/model';
export * from './database/statement';
import { DbDashboardsStore } from './database/dashboards.svelte';
import { DbAssistStore } from './database/assist.svelte';
import { DbSavedQueriesStore } from './database/savedQueries.svelte';
import { DbHistoryStore } from './database/history.svelte';
export { DbDashboardsStore, type DashboardsHost } from './database/dashboards.svelte';
export { DbAssistStore, type AssistHost } from './database/assist.svelte';
export { DbSavedQueriesStore, type SavedQueriesHost } from './database/savedQueries.svelte';
export { DbHistoryStore, type HistoryHost } from './database/history.svelte';

class DatabaseStore {
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
  /** True while `restoreWorkbench` is re-opening persisted tabs, so the
   *  per-open-change persistence calls don't thrash localStorage mid-restore. */
  private restoring = false;
  /**
   * Connections closed WHILE `restoreWorkbench` was running. The restore loop
   * (and its final `selected` re-focus) would otherwise re-open a tab the user
   * just closed, and the closing persist was historically swallowed by the
   * `restoring` guard — so the closed tab came back on every reload.
   */
  private restoreTombstones = new Set<Id>();
  /**
   * Per-connection open generation, bumped by `closeConnection`. Every async
   * loader captures the epoch at start and bails after each await when it no
   * longer matches (or the id left `openConnIds`) — so a close mid-load can't
   * resurrect status entries, write stale singletons, or toast for a dead tab.
   * Non-reactive bookkeeping.
   */
  private connEpoch = new Map<Id, number>();
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
   * In-flight runs, keyed by tab id. Non-reactive (plain Map): lets us cancel a
   * running query (`abortQuery`) without storing a non-serializable controller
   * inside the reactive `$state` tab objects. Each entry carries the fetch
   * `AbortController` (to drop the HTTP wait), plus the `queryId` + `connId` the
   * run was issued with — so `abortQuery` can also tell the SERVER to cancel the
   * query engine-side (`POST …/db/cancel`), not just abandon the response.
   */
  private runControllers = new Map<
    number,
    { controller: AbortController; queryId: string; connId: Id }
  >();
  /** Tab ids with an active `reattach` poll loop — dedupes concurrent kicks. */
  private reattaching = new Set<number>();

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
    persistTabsNow: (connId) => this.persistTabsNow(connId),
    persistTabs: () => this.persistTabs(),
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
      await this.runQuery(undefined, undefined, { readOnly: true, agentLabel, confirmWrite });
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
  private connBase(id: Id): string {
    return `/connections/${id}/db`;
  }
  private wsBase(): string | null {
    return ws.currentId ? `/workspaces/${ws.currentId}/db` : null;
  }

  private accessEpoch = 0;
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
    if (change.type === 'reset' && change.identity) this.flushPersistTabs();
    this.accessEpoch++;
    this.accessRevision++;
    const ids =
      change.type === 'decision'
        ? [change.id]
        : [
            ...new Set([
              ...this.openConnIds,
              ...this.snapshots.keys(),
              ...(this.selectedConnId ? [this.selectedConnId] : []),
            ]),
          ];
    const scrub = (state: ConnSnapshot | DatabaseStore) => {
      for (const tab of state.tabs) {
        const run = this.runControllers.get(tab.id);
        run?.controller.abort();
        this.runControllers.delete(tab.id);
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
      this.connEpoch.set(id, this.epochOf(id) + 1);
      const snapshot = this.snapshots.get(id);
      if (snapshot) scrub(snapshot);
      if (this.selectedConnId === id) scrub(this);
    }
    this.queryPlan = null;
    this.planOpen = false;
    this.objectSearchSeq++;
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
      this.snapshots.clear();
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

  // ── Open-generation guards (see `connEpoch`) ────────────────────────────
  private epochOf(id: Id): number {
    return this.connEpoch.get(id) ?? 0;
  }
  /** Still the same open generation started at `epoch` (i.e. the connection
   *  wasn't closed — possibly re-opened — since the caller began)? */
  private connLive(id: Id, epoch: number): boolean {
    return this.openConnIds.includes(id) && this.epochOf(id) === epoch;
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
      this.abortQuery(closing.id);
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
    this.persistTabs();
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
        this.abortQuery(t.id);
        forgetTabHistory(t);
      } else survivors.push(t);
    }
    this.tabs = survivors.length ? survivors : [blankTab()];
    // Keep the caller's tab focused when it survived; else the first survivor
    // (a pinned tab, or the fresh blank one).
    const at = active ? this.tabs.indexOf(active) : -1;
    this.activeTab = at >= 0 ? at : 0;
    this.persistTabs();
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
    this.persistTabs();
  }

  // ── Result memory budget (lib/stores/db-result-budget.ts) ─────────────────
  /**
   * Land a result on a tab and account for it: every resident result across
   * the live tabs AND every parked connection snapshot shares one budget; past
   * it the least-recently-viewed results are released (rows dropped, columns
   * and stats kept) and the grid offers a Re-run.
   */
  private installResult(t: QueryTab, result: QueryResult): void {
    t.result = rawResult(result);
    resultBudget.note(t.uid, estimateResultBytes(result));
    this.enforceResultBudget();
  }

  /** Release least-recently-viewed results until the total fits the budget. */
  enforceResultBudget(): void {
    const pinned = new Set<string>();
    const shown = this.tabs[this.activeTab];
    if (shown) pinned.add(shown.uid);
    const consider = (tabs: QueryTab[]) => {
      for (const t of tabs) {
        // Never under a running query or un-applied cell edits.
        if (t.running || t.pending || parkedEditCount(t.uid) > 0) pinned.add(t.uid);
      }
    };
    consider(this.tabs);
    for (const snap of this.snapshots.values()) consider(snap.tabs);
    for (const uid of resultBudget.pick(pinned)) {
      resultBudget.forget(uid);
      const t = this.tabByUid(uid);
      if (!t?.result || isReleased(t.result)) continue;
      t.result = rawResult(releasedStub(t.result));
      releaseGridResult(uid);
    }
  }

  /**
   * Re-run the statement that produced the active tab's RELEASED result (same
   * statement, scope, offset and page size) — the grid's "Re-run" button.
   */
  rerunReleased(): Promise<QueryResult | null> {
    const t = this.tab;
    if (!t?.result || !isReleased(t.result) || !t.ran_statement) return Promise.resolve(null);
    return this.runQuery(t.ran_statement, t.ran_node, {
      keepOffset: true,
      transient: true,
      ...(t.result.auto_limited ? { maxRows: t.result.auto_limited } : {}),
    });
  }

  /** A query tab by its stable uid, across the live set and every parked
   *  connection snapshot (the editor's history store asks per path). */
  tabByUid(uid: string): QueryTab | null {
    const live = this.tabs.find((t) => t.uid === uid);
    if (live) return live;
    for (const snap of this.snapshots.values()) {
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
      const tabs = connId === selected ? liveTabs : this.snapshots.get(connId)?.tabs ?? [];
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
    for (const [connId, snap] of this.snapshots) {
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
      await this.openConnection(at.connId);
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
    this.persistTabs();
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
    this.persistTabs();
    if (mode !== null) {
      this.connView = mode;
      this.persistView();
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
    this.persistTabs();
  }

  /** Set + persist a query-level variable value on the active tab. */
  /** Patch a query-level variable (value / type / escape) on the active tab. */
  setVar(name: string, patch: Partial<VarSpec>): void {
    const t = this.tab;
    if (!t) return;
    const cur = t.vars[name] ?? defaultVarSpec();
    t.vars = { ...t.vars, [name]: { ...cur, ...patch } };
    this.persistTabs();
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

  // ── Tab persistence (survive reload / a cut-off session) ──────────────────
  // Open query tabs (statement + name, NOT results) are saved per CONNECTION —
  // the library (and the workbench) is global, so in-progress work follows the
  // connection across workspaces. Legacy per-(workspace, connection) entries
  // are still read as a fallback so nothing is lost on upgrade.
  private tabsKey(connId: Id): string {
    return auth.isRoot ? `otto_db_tabs:${connId}` : `otto_db_tabs:user:${auth.me?.id ?? 'anonymous'}:${connId}`;
  }
  private legacyTabsKey(connId: Id): string | null {
    return auth.isRoot && ws.currentId ? `otto_db_tabs:${ws.currentId}:${connId}` : null;
  }
  /** Trailing-edge debounce handle for `persistTabs` (null = nothing queued). */
  private persistTabsTimer: ReturnType<typeof setTimeout> | null = null;
  /** Debounced tab persistence — `setStatement` fires per keystroke and
   *  stringifying every tab each time is wasted work. Flushed synchronously
   *  before a connection switch/close (`flushPersistTabs`) so nothing is lost. */
  private persistTabs(): void {
    if (this.persistTabsTimer !== null) clearTimeout(this.persistTabsTimer);
    this.persistTabsTimer = setTimeout(() => {
      this.persistTabsTimer = null;
      this.persistTabsNow();
    }, 300);
  }
  /** Write any queued (debounced) tab persistence immediately. Must run while
   *  the owning connection is still selected — the write keys off it. */
  private flushPersistTabs(): void {
    if (this.persistTabsTimer === null) return;
    clearTimeout(this.persistTabsTimer);
    this.persistTabsTimer = null;
    this.persistTabsNow();
  }
  /** Flush queued tab persistence now — the page is being hidden or torn down
   *  (reload, quit, app switch), so the last ≤300 ms of typing isn't lost. */
  flushTabDrafts(): void {
    this.flushPersistTabs();
  }
  /** The JSON last written per tabs key: an unchanged payload skips the
   *  synchronous setItem (and the `storage` event every other same-origin
   *  document receives with both old and new values). */
  private persistedTabsJson = new Map<string, string>();
  /** The quota warning is shown once per session. */
  private tabsQuotaWarned = false;
  private persistTabsNow(connId: Id | null = this.selectedConnId): void {
    if (typeof localStorage === 'undefined' || !connId) return;
    const state = connId === this.selectedConnId ? this : this.snapshots.get(connId);
    if (!state) return;
    const key = this.tabsKey(connId);
    let json: string;
    try {
      json = JSON.stringify({
          tabs: state.tabs.map((t) => ({
            uid: t.uid,
            name: t.name,
            statement: t.statement,
            vars: t.vars,
            savedQueryId: t.savedQueryId,
            // The marker keeps the issued statement when it's small enough —
            // a re-attach after reload then still knows what ran.
            pending: t.pending
              ? {
                  queryId: t.pending.queryId,
                  connId: t.pending.connId,
                  ...(t.pending.startedAt ? { startedAt: t.pending.startedAt } : {}),
                  ...(t.pending.sql !== undefined && t.pending.sql.length <= PENDING_SQL_MAX
                    ? { sql: t.pending.sql, node: t.pending.node ?? null }
                    : {}),
                }
              : undefined,
            // Data-protection toggles survive a reload with the tab.
            timeout_ms: t.timeout_ms ?? undefined,
            mask: t.mask || undefined,
            // The explicit result view + pin survive too (absent = automatic /
            // unpinned, so older payloads read back unchanged).
            viewMode: t.viewMode ?? undefined,
            pinned: t.pinned || undefined,
            // An agent-opened tab keeps its attribution chip across a reload.
            agent: t.agent ?? undefined,
          })),
          activeTab: state.activeTab,
          activeDb: state.activeDb,
      });
    } catch {
      return;
    }
    if (this.persistedTabsJson.get(key) === json) return;
    try {
      localStorage.setItem(key, json);
      this.persistedTabsJson.set(key, json);
      // A successful write supersedes any quota fallback copy.
      try {
        sessionStorage.removeItem(key);
      } catch {
        /* unavailable */
      }
    } catch (e) {
      if (!isQuotaError(e)) return; // storage unavailable — non-fatal
      // Over the origin quota (large drafts across connections): keep the
      // drafts in sessionStorage so at least a reload restores them, and say
      // so ONCE instead of silently dropping every later write.
      let kept = false;
      try {
        sessionStorage.setItem(key, json);
        this.persistedTabsJson.set(key, json);
        kept = true;
      } catch {
        /* over quota there too */
      }
      if (!this.tabsQuotaWarned) {
        this.tabsQuotaWarned = true;
        toasts.warn(
          'Query drafts are too large to save',
          kept
            ? 'Browser storage is full — open tabs survive a reload but not a restart. Save important queries.'
            : 'Browser storage is full — open query tabs will not survive a reload. Save important queries.',
        );
      }
    }
  }
  private restoreTabs(
    connId: Id,
  ): { tabs: QueryTab[]; activeTab: number; activeDb: string | null } | null {
    if (typeof localStorage === 'undefined') return null;
    // Global key first; fall back to the legacy per-workspace entry (upgrade).
    const legacy = this.legacyTabsKey(connId);
    const key = this.tabsKey(connId);
    // A quota-fallback copy (sessionStorage) is newer than the localStorage one.
    let fallback: string | null = null;
    try {
      fallback = sessionStorage.getItem(key);
    } catch {
      /* unavailable */
    }
    const raw =
      fallback ?? localStorage.getItem(key) ?? (legacy ? localStorage.getItem(legacy) : null);
    if (!raw) return null;
    try {
      const p = JSON.parse(raw) as {
        tabs?: {
          uid?: unknown;
          name?: string;
          statement?: string;
          vars?: unknown;
          savedQueryId?: string;
          pending?: {
            queryId?: string;
            connId?: string;
            sql?: unknown;
            node?: unknown;
            startedAt?: unknown;
          } | null;
          timeout_ms?: number;
          mask?: boolean;
          viewMode?: unknown;
          pinned?: unknown;
          agent?: { session_id?: unknown; label?: unknown } | null;
        }[];
        activeTab?: number;
        activeDb?: string | null;
      };
      const seenUids = new Set<string>();
      const tabs = (p.tabs ?? []).map((t) => ({
        ...blankTab(t.statement ?? ''),
        // Keep the persisted uid (its undo history is keyed on it); a missing
        // or duplicated one (older payload, copied tab) gets a fresh uid.
        ...(typeof t.uid === 'string' && t.uid && !seenUids.has(t.uid)
          ? (seenUids.add(t.uid), { uid: t.uid })
          : {}),
        name: t.name || 'Query',
        vars: normalizeVars(t.vars),
        savedQueryId: t.savedQueryId,
        timeout_ms: typeof t.timeout_ms === 'number' && t.timeout_ms > 0 ? t.timeout_ms : null,
        mask: t.mask === true,
        // Revive a pending run marker only when it's complete and belongs to
        // THIS connection — reattach polls it against the server.
        pending:
          t.pending && t.pending.queryId && t.pending.connId === connId
            ? {
                queryId: t.pending.queryId,
                connId: t.pending.connId,
                ...(typeof t.pending.startedAt === 'number' && t.pending.startedAt > 0
                  ? { startedAt: t.pending.startedAt }
                  : {}),
                ...(typeof t.pending.sql === 'string'
                  ? {
                      sql: t.pending.sql,
                      node: typeof t.pending.node === 'string' ? t.pending.node : null,
                    }
                  : {}),
              }
            : null,
        // Unknown view names (a newer build's mode) fall back to automatic.
        viewMode: isViewMode(t.viewMode) ? t.viewMode : null,
        pinned: t.pinned === true,
        agent:
          t.agent && typeof t.agent.session_id === 'string' && typeof t.agent.label === 'string'
            ? { session_id: t.agent.session_id, label: t.agent.label }
            : null,
      }));
      if (!tabs.length) return null;
      // Pinned tabs are never dropped and always lead the strip.
      const ordered = orderPinnedFirst(
        tabs,
        Math.min(Math.max(0, p.activeTab ?? 0), tabs.length - 1),
      );
      return { tabs: ordered.tabs, activeTab: ordered.activeTab, activeDb: p.activeDb ?? null };
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
    this.persistTabs();
  }

  // ── Workbench persistence (which connections are open + their view) ────────
  // Survives a reload: which connection tabs were open, which was focused, and
  // each connection's main/side view. GLOBAL (not per workspace) — the open
  // workbench follows the user across workspace switches, like the connection
  // library itself. Legacy per-workspace entries are read as an upgrade
  // fallback. Query-tab TEXT is persisted separately (persistTabs); RESULTS
  // are intentionally not persisted.
  private openKey(): string {
    return auth.isRoot ? 'otto_db_open' : `otto_db_open:user:${auth.me?.id ?? 'anonymous'}`;
  }
  private legacyOpenKey(): string | null {
    return auth.isRoot && ws.currentId ? `otto_db_open:${ws.currentId}` : null;
  }
  private viewKey(connId: Id): string {
    return auth.isRoot ? `otto_db_view:${connId}` : `otto_db_view:user:${auth.me?.id ?? 'anonymous'}:${connId}`;
  }
  private legacyViewKey(connId: Id): string | null {
    return auth.isRoot && ws.currentId ? `otto_db_view:${ws.currentId}:${connId}` : null;
  }

  /** Persist the open-connection set + selection. No-op during a restore —
   *  except with `force` (a user CLOSE during the restore must reach storage,
   *  or the restore's final persist writes the closed tab durably back). */
  private persistWorkbench(force = false): void {
    if (typeof localStorage === 'undefined' || (this.restoring && !force)) return;
    try {
      localStorage.setItem(
        this.openKey(),
        JSON.stringify({ open: this.openConnIds, selected: this.selectedConnId }),
      );
    } catch {
      /* storage full / unavailable — non-fatal */
    }
  }

  /** Persist the active connection's main/side view (which pane it's showing)
   *  plus its remembered result view (`connView`; omitted when none). */
  private persistView(): void {
    if (typeof localStorage === 'undefined' || this.restoring || !this.selectedConnId) return;
    const key = this.viewKey(this.selectedConnId);
    try {
      // 'connections' is the global picker, never a per-connection view — store
      // 'schema' instead so a restore lands on the connection's own schema.
      const side = this.sideTab === 'connections' ? 'schema' : this.sideTab;
      localStorage.setItem(
        key,
        JSON.stringify({
          main: this.mainTab,
          side,
          ...(this.connView ? { view: this.connView } : {}),
        }),
      );
    } catch {
      /* non-fatal */
    }
  }

  /** Read a connection's persisted main/side view (validated to the known tab
   *  ids) and remembered result view (validated, else null). Returns null when
   *  absent/invalid so the caller falls back to defaults. */
  private restoreView(
    connId: Id,
  ): { main: DbMainTab; side: DbSideTab; view: ViewMode | null } | null {
    if (typeof localStorage === 'undefined') return null;
    const legacy = this.legacyViewKey(connId);
    const raw =
      localStorage.getItem(this.viewKey(connId)) ?? (legacy ? localStorage.getItem(legacy) : null);
    if (!raw) return null;
    try {
      const p = JSON.parse(raw) as { main?: string; side?: string; view?: unknown };
      const mains: DbMainTab[] = ['query', 'builder', 'structure', 'diagram', 'dashboards'];
      const sides: DbSideTab[] = ['schema', 'saved', 'history'];
      const main = mains.includes(p.main as DbMainTab) ? (p.main as DbMainTab) : 'query';
      const side = sides.includes(p.side as DbSideTab) ? (p.side as DbSideTab) : 'schema';
      return { main, side, view: isViewMode(p.view) ? p.view : null };
    } catch {
      return null;
    }
  }

  /**
   * Re-open the persisted global workbench (open connections + focused tab).
   * MUST run only AFTER `loadConnections` resolves — the restore filters against
   * the loaded connection list, so running earlier would re-open nothing. No-op
   * when tabs are already open (a refresh or workspace switch mustn't re-open
   * closed tabs) or when nothing was persisted. Each connection's main/side view
   * is restored by `loadConnectionFresh` from its own `otto_db_view:` entry.
   */
  async restoreWorkbench(): Promise<void> {
    if (typeof localStorage === 'undefined') return;
    if (this.openConnIds.length > 0) return;
    const legacy = this.legacyOpenKey();
    const raw =
      localStorage.getItem(this.openKey()) ?? (legacy ? localStorage.getItem(legacy) : null);
    if (!raw) return;
    let parsed: { open?: string[]; selected?: string | null };
    try {
      parsed = JSON.parse(raw) as { open?: string[]; selected?: string | null };
    } catch {
      return;
    }
    // Only re-open connections that still exist in the library.
    const open = (parsed.open ?? []).filter((id) => this.connections.some((c) => c.id === id));
    if (open.length === 0) return;
    // Bring persisted undo histories into memory before the editors mount
    // (IndexedDB, a few ms) so ⌘Z reaches past the reload.
    await hydrateEditorHistory();
    if (this.openConnIds.length > 0) return; // something opened meanwhile

    // Every tab chip appears at once; only the ACTIVE tab connects in the
    // foreground. The rest show "Not connected yet" and warm in the background
    // (3 at a time, bg lane) unless the user chose "connect on click only".
    // The old loop awaited `openConnection` per id — serial (N × dial) and it
    // moved the selection to each id in turn, overriding a click mid-restore.
    const selected = parsed.selected && open.includes(parsed.selected) ? parsed.selected : open[0];
    this.restoring = true;
    this.restoreTombstones.clear();
    let others: Id[] = [];
    try {
      this.openConnIds = open;
      const cs = new Map(this.connStatus);
      for (const id of open) if (id !== selected) cs.set(id, { phase: 'idle' });
      this.connStatus = cs;
      others = open.filter((id) => id !== selected);
      // The active tab's dial is issued FIRST (interactive/long lane); the
      // background warm-up starts right behind it on the separate bg lane, so a
      // slow active connection doesn't hold the others back.
      const active = this.openConnection(selected);
      if (this.warmRestored === 'background') void this.warmMany(others);
      await active;
    } finally {
      this.restoring = false;
      this.restoreTombstones.clear();
    }
    this.ensureKeepAlive();
    // Persist once, now that the full set is open (converges the storage).
    this.persistWorkbench();
    // Nothing survived the restore (or nothing was open) — land the sidebar on
    // the picker so "open a connection" is one click away, not hidden.
    if (this.openConnIds.length === 0) this.sideTab = 'connections';
  }

  /** Switch the main pane view for the active connection (persisted per conn). */
  setMainTab(t: DbMainTab): void {
    this.mainTab = t;
    this.persistView();
  }
  /** Switch the sidebar view for the active connection (persisted per conn). */
  setSideTab(t: DbSideTab): void {
    this.sideTab = t;
    this.persistView();
  }

  // ── Loading ───────────────────────────────────────────────────────────────

  /** Load connections for the current workspace. DB kinds drive the workbench
   *  (`connections`); ssh/custom land in `otherConnections` so the unified
   *  sidebar tree can render EVERY profile (opens route by type). */
  /** In-flight connection-list load, keyed by workspace + access epoch: a
   *  Home box and a page mounting together share one request. */
  private connectionsInFlight: { key: string; p: Promise<void> } | null = null;
  loadConnections(): Promise<void> {
    const key = `${ws.currentId ?? ''}:${this.accessEpoch}`;
    if (this.connectionsInFlight?.key === key) return this.connectionsInFlight.p;
    const p = this.fetchConnections().finally(() => {
      if (this.connectionsInFlight?.p === p) this.connectionsInFlight = null;
    });
    this.connectionsInFlight = { key, p };
    return p;
  }

  private async fetchConnections(): Promise<void> {
    const wid = ws.currentId;
    const accessEpoch=this.accessEpoch;
    if (!wid) return;
    this.connectionsLoading = true;
    try {
      const all = await api.get<Connection[]>(`/workspaces/${wid}/connections`);
      if(accessEpoch!==this.accessEpoch)return;
      this.connectionsError = null;
      const next = all.filter((c) => isDbKind(c.kind));
      this.otherConnections = all.filter((c) => !isDbKind(c.kind));
      this.connections = next;
      // The connection library is GLOBAL (profiles are created workspace-
      // independent), so the open workbench is global too: a workspace switch
      // no longer wipes the open tabs/snapshots — open connections stay open
      // no matter which workspace is active. Every load just prunes tabs /
      // snapshots / selection whose connection no longer exists (deleted).
      this.openConnIds = this.openConnIds.filter((id) => next.some((c) => c.id === id));
      for (const id of [...this.snapshots.keys()]) {
        if (!next.some((c) => c.id === id)) this.snapshots.delete(id);
      }
      if (this.selectedConnId && !next.some((c) => c.id === this.selectedConnId)) {
        this.selectedConnId = null;
      }
      // Start fresh — do NOT auto-open a connection; the user picks one from the
      // sidebar. Only clear active state when no connections remain.
      if (this.connections.length === 0) {
        this.selectedConnId = null;
        this.capabilities = null;
        this.schemaRoot = [];
      }
    } catch (e) {
      if (accessEpoch === this.accessEpoch) this.connectionsError = loadErrorText(e);
    } finally {
      this.connectionsLoading = false;
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
    // Any debounced tab write still belongs to THIS connection — flush it while
    // the key is still right.
    this.flushPersistTabs();
    this.snapshots.set(id, {
      capabilities: this.capabilities,
      testResult: this.testResult,
      activeDb: this.activeDb,
      schemaRoot: this.schemaRoot,
      childrenCache: this.childrenCache,
      expanded: this.expanded,
      // NEVER freeze in-flight loading flags: the loads they describe resolve
      // (or are epoch-cancelled) while this snapshot is parked, so restoring
      // them would show "Loading…" spinners nothing will ever clear.
      loadingNodes: new Set<string>(),
      schemaLoading: false,
      selectedObjectPath: this.selectedObjectPath,
      objectDetail: this.objectDetail,
      objectError: this.objectError,
      objectSearchQuery: this.objectSearchQuery,
      objectSearchHits: this.objectSearchHits,
      objectSearchTruncated: this.objectSearchTruncated,
      objectSearchScanned: this.objectSearchScanned,
      builderTablesCache: this.builderTablesCache,
      tabs: this.tabs,
      activeTab: this.activeTab,
      savedQueries: this.savedQueries,
      history: this.history,
      mainTab: this.mainTab,
      // 'connections' is a global picker view, not a per-connection state —
      // collapse it to 'schema' so reopening a connection never lands back on
      // the picker.
      sideTab: this.sideTab === 'connections' ? 'schema' : this.sideTab,
      connView: this.connView,
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
    this.activeDb = snap.activeDb;
    this.schemaRoot = snap.schemaRoot;
    this.childrenCache = snap.childrenCache;
    this.expanded = snap.expanded;
    this.loadingNodes = snap.loadingNodes;
    this.schemaLoading = snap.schemaLoading;
    this.selectedObjectPath = snap.selectedObjectPath;
    this.objectDetail = snap.objectDetail;
    this.objectError = snap.objectError;
    // Object search is per connection; invalidate any in-flight search from the
    // previous one so its late result can't land over this restore.
    this.objectSearchSeq++;
    this.objectSearching = false;
    this.objectSearchQuery = snap.objectSearchQuery;
    this.objectSearchHits = snap.objectSearchHits;
    this.objectSearchTruncated = snap.objectSearchTruncated;
    this.objectSearchScanned = snap.objectSearchScanned;
    this.builderTablesCache = snap.builderTablesCache;
    this.tabs = snap.tabs;
    this.activeTab = snap.activeTab;
    // Saved queries belong to the workspace, not this connection snapshot.
    this.history = snap.history;
    this.historyError = null;
    void this.loadHistory(id);
    this.mainTab = snap.mainTab;
    this.sideTab = snap.sideTab;
    this.connView = snap.connView;
    // A snapshot captured mid-load (a switch-away made the epoch guards drop
    // the fetches) has an empty root — re-fetch instead of restoring a
    // permanently empty tree.
    if (snap.schemaRoot.length === 0) {
      void this.loadCapabilities(id);
      void this.loadSchemaRoot(id);
    }
    // A tab may have lost its HTTP wait while this connection was in the
    // background (e.g. the daemon blipped and the earlier poll loop gave up) —
    // resume recovering its detached run. No-op for live/settled tabs.
    this.reattachPendingTabs();
    return true;
  }

  /**
   * Open (or focus) a connection as a top-level tab. If it already has a
   * snapshot, restore it instantly with no network; otherwise load it fresh.
   * Snapshots the currently-active connection first so switching back is free.
   */
  async openConnection(id: Id): Promise<void> {
    // Selecting a DB tab always returns focus to the DB workbench pane, even when
    // the same connection was already selected behind a Kafka/SSH pane.
    this.activePane = null;
    if (id === this.selectedConnId) return;
    this.ensureKeepAlive();
    this.captureSnapshot();
    if (!this.openConnIds.includes(id)) {
      this.openConnIds = [...this.openConnIds, id];
    }
    this.selectedConnId = id;
    // Persist the open set + selection now (both the snapshot-restore and
    // fresh-load paths below have already updated them).
    this.persistWorkbench();
    if (this.restoreSnapshot(id)) return;
    const epoch = this.epochOf(id);
    await this.loadConnectionFresh(id);
    // Capture an initial snapshot so subsequent switches restore this state —
    // unless the tab was closed mid-load (snapshotting would resurrect it).
    if (this.connLive(id, epoch) && this.selectedConnId === id) this.captureSnapshot();
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
    // A queued (debounced) tab write for the active conn still keys off it.
    if (wasActive) this.flushPersistTabs();
    // Stop this connection's tab runs BEFORE the snapshot is discarded: its
    // tabs are the live set when active, else the parked snapshot's. Cancels
    // server-side + clears `pending` so the reattach loops stop polling.
    const closingTabs = wasActive ? this.tabs : this.snapshots.get(id)?.tabs ?? [];
    // (Undo histories are kept: the connection's tabs are persisted and come
    // back with the same uids when it is reopened.)
    for (const t of closingTabs) this.abortRunForTab(t);
    this.openConnIds = this.openConnIds.filter((x) => x !== id);
    // Invalidate every in-flight loader for this connection (schema, caps,
    // health probe, children…) — a close must be terminal, not cosmetic.
    this.connEpoch.set(id, this.epochOf(id) + 1);
    this.warming.get(id)?.ctl.abort();
    this.warming.delete(id);
    // A close during `restoreWorkbench` must survive the restore loop (which
    // would otherwise re-open it) AND reach localStorage past the guard.
    if (this.restoring) this.restoreTombstones.add(id);
    this.snapshots.delete(id);
    const cs = new Map(this.connStatus);
    cs.delete(id);
    this.connStatus = cs;
    // Tell the daemon to tear down the connection's live resources (SSH tunnel,
    // pooled conns, in-flight queries). Best-effort fire-and-forget — the tab
    // closes regardless (mirrors closeSshTerminal's DELETE /sessions/{id}).
    void dbCloseConnection(id).catch(() => {
      /* daemon down / endpoint not deployed yet — nothing to release */
    });
    // Drop this connection's persisted per-view entry (it's no longer open).
    if (typeof localStorage !== 'undefined') {
      const vk = this.viewKey(id);
      if (vk) localStorage.removeItem(vk);
    }
    if (!wasActive) {
      this.persistWorkbench(true);
      return;
    }

    if (this.openConnIds.length === 0) {
      // Nothing left open — clear the active working set.
      this.selectedConnId = null;
      this.capabilities = null;
      this.connView = null;
      this.testResult = null;
      this.schemaRoot = [];
      this.childrenCache = new Map();
      this.builderTablesCache = new Map();
      this.expanded = new Set();
      this.nodeFilters = new Map();
      this.loadingNodes = new Set();
      this.schemaLoading = false;
      this.objectDetail = null;
      this.objectError = null;
      this.selectedObjectPath = null;
      this.clearObjectSearch();
      this.tabs = [blankTab()];
      this.activeTab = 0;
      this.history = [];
      this.mainTab = 'query';
      // Back to the picker — there's no active connection to show a schema for.
      this.sideTab = 'connections';
      this.persistWorkbench(true);
      return;
    }
    // Focus the previous tab (or the first if we closed index 0). The active id
    // is gone, so clear it first to let openConnection do the switch.
    const neighbor = this.openConnIds[Math.max(0, idx - 1)];
    this.selectedConnId = null;
    void this.openConnection(neighbor);
  }

  // ── Background warm-up (restore) + keep-alive ───────────────────────────────

  /** In-flight background warm-ups, one per connection (dedupe + cancel). */
  private warming = new Map<Id, { promise: Promise<void>; ctl: AbortController }>();

  /** Un-applied cell edits in the results grid of the ACTIVE query tab
   *  (written by ResultsGrid) — a tab close asks before discarding them. */
  livePendingEdits = $state(0);

  /** Whether `id` is being connected in the background right now. */
  isWarming(id: Id): boolean {
    return this.warming.has(id);
  }

  /** Restored-tab warming mode — lives in `dbPrefs` (Settings reads it without
   *  loading this store); see `DbPrefs.warmRestored`. */
  get warmRestored(): WarmMode {
    return dbPrefs.warmRestored;
  }
  setWarmRestored(mode: WarmMode): void {
    dbPrefs.setWarmRestored(mode);
  }

  /** Connection keep-alive toggle — lives in `dbPrefs`; flipping it there (from
   *  here or Settings) starts/stops the poller via the subscription below the
   *  class. */
  get keepAlive(): boolean {
    return dbPrefs.keepAlive;
  }
  setKeepAlive(on: boolean): void {
    dbPrefs.setKeepAlive(on);
  }
  /** React to a keep-alive toggle (wired to `dbPrefs.onKeepAliveChange`). */
  applyKeepAlive(on: boolean): void {
    if (on) this.ensureKeepAlive();
    else this.stopKeepAlive();
  }

  /** Warm `ids` in the background, at most 3 at a time (the app-wide bg lane
   *  holds 3 sockets). Skips closed ids and ids that already have a snapshot. */
  async warmMany(ids: readonly Id[]): Promise<void> {
    await mapLimit(ids, WARM_CONCURRENCY, (id) => this.warm(id));
  }

  /**
   * Connect one open, non-selected connection in the background: caps + schema
   * root on the bg lane, then park the result as its snapshot so a click
   * restores it instantly. Never touches the selected connection's singletons
   * and never moves focus. A click on the tab while this runs just loads it
   * fresh — the daemon's per-key init slot makes that request wait for this
   * dial instead of dialling twice — and this result is then dropped.
   */
  warm(id: Id): Promise<void> {
    const inflight = this.warming.get(id);
    if (inflight) return inflight.promise;
    // A parked snapshot with a tree needs nothing; one emptied by a daemon
    // restart (onDaemonRestart) gets its caps + tree refilled in place.
    const parked = this.snapshots.get(id);
    if (
      !this.openConnIds.includes(id) ||
      (parked && parked.schemaRoot.length > 0) ||
      this.selectedConnId === id
    ) {
      return Promise.resolve();
    }
    const ctl = new AbortController();
    const epoch = this.epochOf(id);
    const run = async (): Promise<void> => {
      this.mergeConnStatus(id, { phase: 'connecting', error: undefined });
      const started = typeof performance !== 'undefined' ? performance.now() : Date.now();
      try {
        const [caps, root] = await Promise.all([
          api.bg.get<DbCapabilities>(`${this.connBase(id)}/capabilities`, ctl.signal),
          api.bg.get<SchemaNode[]>(`${this.connBase(id)}/schema`, ctl.signal),
        ]);
        if (!this.connLive(id, epoch) || ctl.signal.aborted) return;
        const now = typeof performance !== 'undefined' ? performance.now() : Date.now();
        if (this.selectedConnId !== id) {
          const snap = this.snapshots.get(id);
          if (!snap) this.snapshots.set(id, this.freshSnapshot(id, caps, root));
          else if (snap.schemaRoot.length === 0) {
            // Keep the parked tabs/results; only the connection-side data refills.
            this.snapshots.set(id, { ...snap, capabilities: caps, schemaRoot: root });
          }
        }
        this.mergeConnStatus(id, {
          phase: 'ready',
          error: undefined,
          latencyMs: Math.round(now - started),
        });
      } catch (e) {
        if (!this.connLive(id, epoch)) return;
        if (ctl.signal.aborted) {
          // "Stop connecting": back to not-connected, no error dot.
          this.setConnStatus(id, { phase: 'idle' });
          return;
        }
        // The tab gets a red dot; SchemaTree's Retry handles it on open. No toast.
        this.setConnStatus(id, { phase: 'error', error: loadErrorText(e) });
      }
    };
    const promise = run().finally(() => {
      if (this.warming.get(id)?.ctl === ctl) this.warming.delete(id);
    });
    this.warming.set(id, { promise, ctl });
    return promise;
  }

  /** "Stop connecting" (tab menu): abort a background warm-up. The tab stays
   *  open, "Not connected yet"; opening it connects normally. */
  stopConnecting(id: Id): void {
    const w = this.warming.get(id);
    if (!w) return;
    w.ctl.abort();
    this.warming.delete(id);
    if (this.openConnIds.includes(id)) this.setConnStatus(id, { phase: 'idle' });
  }

  /**
   * A connection's working set as `loadConnectionFresh` would build it, but as
   * a parked snapshot (no singleton writes) — persisted tabs/view/active DB plus
   * the fetched caps + schema root.
   */
  private freshSnapshot(id: Id, caps: DbCapabilities, root: SchemaNode[]): ConnSnapshot {
    const restored = this.restoreTabs(id);
    const view = this.restoreView(id);
    let activeDb = restored?.activeDb ?? null;
    // Redis: default the active keyspace to the first DB (as loadSchemaRoot).
    if (!activeDb) activeDb = root.find((n) => n.kind === 'keyspace')?.id ?? null;
    let mainTab: DbMainTab = view?.main ?? 'query';
    if (mainTab === 'builder' && !caps.joins) mainTab = 'query';
    return {
      capabilities: caps,
      testResult: null,
      activeDb,
      schemaRoot: root,
      childrenCache: new Map(),
      expanded: new Set(),
      loadingNodes: new Set(),
      schemaLoading: false,
      selectedObjectPath: null,
      objectDetail: null,
      objectError: null,
      objectSearchQuery: '',
      objectSearchHits: null,
      objectSearchTruncated: false,
      objectSearchScanned: 0,
      builderTablesCache: new Map(),
      tabs: restored?.tabs ?? [blankTab()],
      activeTab: restored?.activeTab ?? 0,
      savedQueries: this.savedQueries,
      history: [],
      mainTab,
      sideTab: view?.side ?? 'schema',
      connView: view?.view ?? null,
    };
  }

  private keepAlivePoller: Poller | null = null;

  /** Start the 4-minute keep-alive when enabled (paused while the app is
   *  hidden — pollWhileVisible runs one tick on return). Idempotent. */
  ensureKeepAlive(): void {
    if (!this.keepAlive || this.keepAlivePoller !== null || typeof window === 'undefined') return;
    this.keepAlivePoller = pollWhileVisible(() => this.keepAliveTick(), {
      ms: KEEP_ALIVE_MS,
      immediate: false,
    });
  }
  private stopKeepAlive(): void {
    this.keepAlivePoller?.stop();
    this.keepAlivePoller = null;
  }

  /** One keep-alive round: `db/test` (acquires from the cached pool and
   *  refreshes the tunnel's last-used) for each ready connection, 2 at a time.
   *  A failed ping turns the dot red before the user's next query. */
  async keepAliveTick(): Promise<void> {
    if (!this.keepAlive || this.openConnIds.length === 0) return;
    const ids = this.openConnIds.filter((id) => this.connStatus.get(id)?.phase === 'ready');
    await mapLimit(ids, 2, async (id) => {
      const epoch = this.epochOf(id);
      try {
        const res = await api.bg.post<DbTestResult>(`${this.connBase(id)}/test`, {});
        if (!this.connLive(id, epoch)) return;
        if (!res.ok) {
          this.setConnStatus(id, { phase: 'error', error: res.message || 'Connection lost' });
        } else if (res.latency_ms != null) {
          this.mergeConnStatus(id, { latencyMs: res.latency_ms });
        }
      } catch (e) {
        if (!this.connLive(id, epoch)) return;
        // Daemon unreachable is the events client's banner, not a red dot on
        // every connection; only a real API answer marks the connection.
        if (e instanceof ApiError) this.setConnStatus(id, { phase: 'error', error: loadErrorText(e) });
      }
    });
  }

  /**
   * The daemon restarted (new boot id): every pool and tunnel it held is gone,
   * so the "ready" chips are stale. Background connections drop to "Not
   * connected yet" (their parked snapshot keeps the tabs, the tree re-fetches)
   * and re-warm; the selected one re-loads its schema in place, first.
   */
  onDaemonRestart(): void {
    if (this.openConnIds.length === 0) return;
    for (const w of this.warming.values()) w.ctl.abort();
    this.warming.clear();
    const sel = this.selectedConnId;
    const cs = new Map(this.connStatus);
    const others: Id[] = [];
    for (const id of this.openConnIds) {
      if (id === sel) continue;
      cs.set(id, { phase: 'idle' });
      // Drop the stale tree but keep the parked tabs: an empty root makes
      // restoreSnapshot re-fetch caps + schema on open.
      const snap = this.snapshots.get(id);
      if (snap) {
        this.snapshots.set(id, { ...snap, schemaRoot: [], childrenCache: new Map(), expanded: new Set() });
      }
      others.push(id);
    }
    this.connStatus = cs;
    if (sel && this.openConnIds.includes(sel)) void this.retryConnection(sel);
    if (this.warmRestored === 'background') void this.warmMany(others);
  }

  // ── Non-DB workbench panes (Kafka clusters, SSH/custom terminals) ───────────

  /** Focus an already-open Kafka cluster tab (the `brokers` store owns the tab
   *  list + which cluster is selected; this just points the main area at it). */
  focusKafka(id: Id): void {
    this.activePane = { kind: 'kafka', id };
  }

  /** Focus an already-open SSH/custom terminal tab. */
  focusSsh(id: Id): void {
    if (this.sshTabs.some((t) => t.connId === id)) this.activePane = { kind: 'ssh', id };
  }

  /** Register an ssh/custom profile's freshly-spawned session as a workbench tab
   *  and focus it. Idempotent per connection (re-opening focuses the live tab). */
  addSshTab(tab: SshTab): void {
    if (!this.sshTabs.some((t) => t.connId === tab.connId)) {
      this.sshTabs = [...this.sshTabs, tab];
    }
    this.activePane = { kind: 'ssh', id: tab.connId };
  }

  /** Close an SSH/custom terminal tab. The session keeps running and stays
   *  reachable in the Agents list — closing the tab only detaches it from the
   *  workbench. When it was the focused pane, fall back to the DB workbench. */
  closeSshTab(connId: Id): void {
    this.sshTabs = this.sshTabs.filter((t) => t.connId !== connId);
    if (this.activePane?.kind === 'ssh' && this.activePane.id === connId) {
      this.activePane = null;
    }
  }

  /**
   * Fresh load of a connection's active-session fields: reset the working set,
   * then fetch capabilities + schema root + history. Resets ONLY active-session
   * state — never `openConnIds`/`snapshots`.
   */
  private async loadConnectionFresh(id: Id): Promise<void> {
    const epoch = this.epochOf(id);
    this.selectedConnId = id;
    this.capabilities = null;
    this.connView = null;
    this.activeDb = null;
    this.schemaRoot = [];
    this.childrenCache = new Map();
    this.builderTablesCache = new Map();
    this.expanded = new Set();
    this.nodeFilters = new Map();
    this.loadingNodes = new Set();
    this.objectDetail = null;
    this.objectError = null;
    this.selectedObjectPath = null;
    this.testResult = null;
    // Object search is per connection — never carry the previous one's hits.
    this.clearObjectSearch();
    // Restore this connection's persisted query tabs (in-progress work from a
    // previous session); otherwise start with one blank tab. Never inherit the
    // previously active connection's tabs.
    const restored = this.restoreTabs(id);
    this.tabs = restored?.tabs ?? [blankTab()];
    this.activeTab = restored?.activeTab ?? 0;
    // Restore the active database too, so the first query after reopening a
    // connection is still scoped (otherwise Mongo/SQL error on an unscoped run).
    this.activeDb = restored?.activeDb ?? null;
    // Restore this connection's persisted main/side view (which pane it showed);
    // fall back to the query editor + schema tree. loadCapabilities re-validates
    // 'builder' against the engine below.
    const view = this.restoreView(id);
    this.mainTab = view?.main ?? 'query';
    this.sideTab = view?.side ?? 'schema';
    // …and the result view remembered for it (null = engine default).
    this.connView = view?.view ?? null;
    // Fresh window of history for this connection.
    this.history = [];
    this.historyError = null;
    this.historyLimit = 100;
    await Promise.all([this.loadCapabilities(id), this.loadSchemaRoot(id), this.loadHistory(id)]);
    // Closed (or superseded) while the loads were in flight — stop here: no
    // reattach polling, no health probe for a tab that no longer exists.
    if (!this.connLive(id, epoch)) return;
    // A restored tab may reference a run whose HTTP wait died with the previous
    // page (reload / app restart) while the server kept executing it detached —
    // re-attach and recover the result instead of silently dropping it.
    this.reattachPendingTabs();
    // Best-effort health probe for the tab-strip chip (server version + latency).
    // Fire-and-forget so it never blocks the first render; only when connected.
    if (this.connStatus.get(id)?.phase === 'ready') void this.probeHealth(id);
  }

  private async loadCapabilities(id: Id): Promise<void> {
    const epoch = this.epochOf(id);
    try {
      const caps = await api.get<DbCapabilities>(`${this.connBase(id)}/capabilities`);
      // Stale completion: the tab was closed, or another connection took the
      // foreground — `capabilities` is a singleton view of the SELECTED conn.
      if (!this.connLive(id, epoch) || this.selectedConnId !== id) return;
      this.capabilities = caps;
      // A non-SQL engine can't use the visual JOIN builder; keep main tab valid.
      if (this.mainTab === 'builder' && !this.supportsBuilder) this.mainTab = 'query';
    } catch (e) {
      if (!this.connLive(id, epoch)) return; // no toast for a closed tab
      toastError('Couldn’t load DB capabilities', e);
    }
  }

  /** Set a connection's liveness, reassigning the Map for Svelte-5 reactivity. */
  private setConnStatus(id: Id, status: ConnStatus): void {
    this.connStatus = new Map(this.connStatus).set(id, status);
  }

  /** Merge a patch into a connection's liveness, preserving fields not set (so a
   *  refresh keeps the health chip's server version while it re-probes latency). */
  private mergeConnStatus(id: Id, patch: Partial<ConnStatus>): void {
    const prev = this.connStatus.get(id) ?? { phase: 'connecting' as ConnPhase };
    this.connStatus = new Map(this.connStatus).set(id, { ...prev, ...patch });
  }

  private async loadSchemaRoot(id: Id): Promise<void> {
    const epoch = this.epochOf(id);
    this.schemaLoading = true;
    // Preserve any prior serverVersion/latency across a refresh's connecting dip.
    this.mergeConnStatus(id, { phase: 'connecting', error: undefined });
    const started =
      typeof performance !== 'undefined' ? performance.now() : Date.now();
    try {
      const root = await api.get<SchemaNode[]>(`${this.connBase(id)}/schema`);
      // Closed mid-fetch: write NOTHING (a status merge would resurrect the
      // connStatus entry closeConnection just deleted).
      if (!this.connLive(id, epoch)) return;
      // The singleton tree belongs to the SELECTED connection only — a
      // background load (tab switched away mid-fetch) must not clobber it; its
      // snapshot re-fetches on restore (see restoreSnapshot's empty-root path).
      if (this.selectedConnId === id) {
        this.schemaRoot = root;
        // Redis: default the active keyspace to the first DB (db0) so commands have
        // a clear, visible target and the tree marks it. Won't override a restored
        // selection. (`kind === 'keyspace'` only matches Redis.)
        if (!this.activeDb) {
          const ks = root.find((n) => n.kind === 'keyspace');
          if (ks) this.activeDb = ks.id;
        }
      }
      // Ready — record the schema round-trip as an immediate latency figure for
      // the health chip (the db/test probe refines it + adds the server version).
      const now = typeof performance !== 'undefined' ? performance.now() : Date.now();
      this.mergeConnStatus(id, {
        phase: 'ready',
        error: undefined,
        latencyMs: Math.round(now - started),
      });
    } catch (e) {
      if (!this.connLive(id, epoch)) return; // closed tab: no status, no toast
      // A hard failure drops any stale health data (replace, not merge).
      // SchemaTree renders this phase inline ("Couldn’t connect" + Retry) — no
      // toast on top of it.
      this.setConnStatus(id, { phase: 'error', error: loadErrorText(e) });
    } finally {
      // The singleton loading flag tracks the selected connection's tree.
      if (this.selectedConnId === id) this.schemaLoading = false;
    }
  }

  /**
   * Best-effort health probe for the tab-strip chip: pings `db/test` for the
   * server version + latency and merges them into the connection's liveness
   * WITHOUT touching `phase` (that's owned by the schema load). Silent on
   * failure — the chip just shows what it has (the phase already reflects errors).
   */
  private async probeHealth(id: Id): Promise<void> {
    const epoch = this.epochOf(id);
    try {
      const started =
        typeof performance !== 'undefined' ? performance.now() : Date.now();
      const res = await api.post<DbTestResult>(`${this.connBase(id)}/test`, {});
      // Closed mid-probe: the merge would resurrect the deleted status entry.
      if (!res.ok || !this.connLive(id, epoch)) return;
      const now = typeof performance !== 'undefined' ? performance.now() : Date.now();
      this.mergeConnStatus(id, {
        serverVersion: res.server_version ?? undefined,
        latencyMs: res.latency_ms ?? Math.round(now - started),
      });
    } catch {
      /* health chip is best-effort */
    }
  }

  /**
   * Re-attempt a connection after a failure (or to re-probe): re-run capabilities
   * + schema only — NOT a full `loadConnectionFresh` — so the user's open query
   * tabs/editor are preserved. `loadSchemaRoot` flips connStatus connecting →
   * ready|error. Targets the active connection by default.
   *
   * Only invoked from the schema panel's error state (the Retry button), where
   * the tree caches were never populated — so, unlike `refreshSchema`, it does
   * not need to clear `childrenCache`/`expanded`/`builderTablesCache`.
   */
  async retryConnection(id: Id | null = this.selectedConnId): Promise<void> {
    if (!id) return;
    // Drop any stale completion snapshot from the previous (failed) session.
    void this.refreshCompletions(id);
    await Promise.all([this.loadCapabilities(id), this.loadSchemaRoot(id)]);
  }

  /** Re-fetch the schema root, clearing the children + completion caches. */
  async refreshSchema(): Promise<void> {
    if (!this.selectedConnId) return;
    this.childrenCache = new Map();
    this.builderTablesCache = new Map();
    this.expanded = new Set();
    // Clear the daemon's smart-completion snapshot too so suggestions reflect
    // the schema the user just refreshed (fire-and-forget; non-blocking).
    void this.refreshCompletions(this.selectedConnId);
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
        toasts.error('Couldn’t connect', this.testResult.message);
      }
    } catch (e) {
      toastError('Couldn’t run the test', e);
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
    await this.loadChildren(id, node.id);
  }

  /** Current prefix filter typed for a node (empty string when none). */
  nodeFilter(nodeId: string): string {
    return this.nodeFilters.get(nodeId) ?? '';
  }

  /**
   * Apply (or clear) a prefix filter on a node and reload its children. Used by
   * the Redis keyspace filter so huge databases load a narrowed, bounded set
   * instead of attempting to list every key.
   */
  async applyNodeFilter(node: SchemaNode, value: string): Promise<void> {
    const id = this.selectedConnId;
    if (!id) return;
    const v = value.trim();
    if (v) this.nodeFilters.set(node.id, v);
    else this.nodeFilters.delete(node.id);
    this.nodeFilters = new Map(this.nodeFilters);

    // Bust the cache and (re)load with the new filter; keep the node expanded.
    this.childrenCache.delete(node.id);
    this.childrenCache = new Map(this.childrenCache);
    this.expanded.add(node.id);
    this.expanded = new Set(this.expanded);
    await this.loadChildren(id, node.id);
  }

  /** Fetch a node's children (honouring any stored filter) into the cache. */
  /** Toggle sticky row estimates and drop the cache so the tree refetches. */
  setShowCounts(on: boolean): void {
    this.showCounts = on;
    saveFlag('db.showCounts', on);
    // Cached children were fetched without (or with) counts — drop the cache and
    // refetch what's on screen, or every expanded node would sit childless with
    // no spinner and no way to reload short of collapsing it by hand.
    this.childrenCache = new Map();
    const id = this.selectedConnId;
    if (!id) return;
    for (const nodeId of this.expanded) void this.loadChildren(id, nodeId);
  }

  toggleSidebar(): void {
    this.sidebarCollapsed = !this.sidebarCollapsed;
    saveFlag('db.sidebarCollapsed', this.sidebarCollapsed);
  }

  /**
   * Server-side object lookup. The tree's own filter only ever sees nodes that
   * are already loaded, so finding a table inside a never-opened schema has to
   * go to the catalog. A blank needle clears the hits instead of searching.
   */
  async searchObjects(q: string, schema?: string): Promise<void> {
    const connId = this.selectedConnId;
    if (!connId || !q.trim()) {
      this.objectSearchHits = null;
      this.objectSearchTruncated = false;
      this.objectSearchScanned = 0;
      return;
    }
    const seq = ++this.objectSearchSeq;
    // A newer keystroke ABORTS the superseded catalog search instead of only
    // ignoring its answer (the daemon stops scanning a big catalog early).
    this.objectSearchAbort?.abort();
    const ac = (this.objectSearchAbort = new AbortController());
    this.objectSearching = true;
    try {
      const r = await api.post<ObjectSearchResult>(
        `${this.connBase(connId)}/search-objects`,
        {
          q: q.trim(),
          schema: this.objectSearchScope === 'schema' ? schema : undefined,
          scope: this.objectSearchScope,
        },
        ac.signal,
      );
      if (seq !== this.objectSearchSeq) return; // a newer keystroke won
      this.objectSearchHits = r.hits;
      this.objectSearchTruncated = r.truncated;
      this.objectSearchScanned = r.scanned;
    } catch (e) {
      if (seq === this.objectSearchSeq && !isAbortError(e)) {
        this.objectSearchHits = [];
        toastError('Couldn’t object search', e);
      }
    } finally {
      if (seq === this.objectSearchSeq) {
        this.objectSearching = false;
        this.objectSearchAbort = null;
      }
    }
  }

  clearObjectSearch(): void {
    this.objectSearchAbort?.abort();
    this.objectSearchAbort = null;
    this.objectSearchSeq++;
    this.objectSearchQuery = '';
    this.objectSearchHits = null;
    this.objectSearching = false;
    this.objectSearchTruncated = false;
    this.objectSearchScanned = 0;
  }

  private objectSearchSeq = 0;
  private objectSearchAbort: AbortController | null = null;

  private async loadChildren(connId: string, nodeId: string): Promise<void> {
    const epoch = this.epochOf(connId);
    // The tree singletons (children/expanded/loading) always describe the
    // SELECTED connection — after any await, a completion for a connection
    // that was closed or switched away must not touch them.
    const stale = (): boolean => !this.connLive(connId, epoch) || this.selectedConnId !== connId;
    this.loadingNodes.add(nodeId);
    this.loadingNodes = new Set(this.loadingNodes);
    try {
      const filter = this.nodeFilters.get(nodeId);
      const children = await api.post<SchemaNode[]>(`${this.connBase(connId)}/schema/children`, {
        path: nodeId,
        filter: filter || undefined,
        counts: this.showCounts || undefined,
      });
      if (stale()) return;
      this.childrenCache.set(nodeId, children);
      this.childrenCache = new Map(this.childrenCache);
    } catch (e) {
      if (stale()) return; // no toast / collapse for a dead view
      toastError('Couldn’t load children', e);
      this.expanded.delete(nodeId);
      this.expanded = new Set(this.expanded);
    } finally {
      if (!stale()) {
        this.loadingNodes.delete(nodeId);
        this.loadingNodes = new Set(this.loadingNodes);
      }
    }
  }

  /** Open an object (table/view/collection/key) → detail + Structure tab. */
  async openObject(node: SchemaNode): Promise<void> {
    const id = this.selectedConnId;
    if (!id) return;
    const epoch = this.epochOf(id);
    const seq = ++this.objectOpenSeq;
    this.selectedObjectPath = node.id;
    this.objectLoading = true;
    this.objectDetail = null;
    this.objectError = null;
    this.setMainTab('structure');
    // Stale when the connection was closed/switched, or another object was
    // opened while this fetch was in flight — the singletons moved on.
    const stale = (): boolean =>
      !this.connLive(id, epoch) ||
      this.selectedConnId !== id ||
      this.selectedObjectPath !== node.id;
    try {
      const detail = await api.post<ObjectDetail>(`${this.connBase(id)}/object`, {
        path: node.id,
      });
      if (stale()) return;
      this.objectDetail = detail;
    } catch (e) {
      if (stale()) return;
      // Keep the failure visible in the Structure view (the toast fades and the
      // neutral "no object" empty state would misreport a load error).
      this.objectError = loadErrorText(e);
      toastError('Couldn’t load object', e);
    } finally {
      // A NEWER openObject owns the loading flag; anything else (including a
      // stale completion) must release it so no spinner runs forever.
      if (seq === this.objectOpenSeq) this.objectLoading = false;
    }
  }

  /** Monotonic openObject sequence — the latest call owns `objectLoading`. */
  private objectOpenSeq = 0;

  /** Fetch object detail for an arbitrary table path (used by the builder). */
  async fetchObject(path: string): Promise<ObjectDetail | null> {
    const id = this.selectedConnId;
    if (!id) return null;
    try {
      return await api.post<ObjectDetail>(`${this.connBase(id)}/object`, { path });
    } catch (e) {
      toastError('Couldn’t load object', e);
      return null;
    }
  }

  /** Copy a table/view's CREATE statement (the DDL the server already derives via
   *  `SHOW CREATE` for MySQL + ClickHouse) to the clipboard. */
  async copyCreateStatement(node: SchemaNode): Promise<void> {
    const detail = await this.fetchObject(node.id);
    const ddl = detail?.ddl?.trim();
    if (!ddl) {
      // For a routine we DID fetch (detail present) but got no body, MySQL blanked
      // the "Create …" column — that's a privilege limitation, not a bug. Say so.
      const isRoutine = detail?.kind === 'procedure' || detail?.kind === 'function';
      if (isRoutine) {
        toasts.error(
          'Definition not available',
          `MySQL returned no body for ${node.label} — the connected account likely lacks privilege to view routine definitions (needs SHOW_ROUTINE, or SELECT on the routine).`,
        );
      } else {
        toasts.error('Couldn’t build the create statement', `Otto couldn’t derive the DDL for ${node.label}.`);
      }
      return;
    }
    try {
      await copyTextOrThrow(ddl);
      toasts.success('Create statement copied', node.label);
    } catch {
      toasts.error('Couldn’t copy the create statement', 'Select the text and copy it manually.');
    }
  }

  /** Re-fetch the currently-open object's detail (e.g. after an import changes
   *  its row count). No-op when no object is open. */
  async refreshObject(): Promise<void> {
    const path = this.selectedObjectPath;
    if (!path) return;
    const detail = await this.fetchObject(path);
    if (detail) this.objectDetail = detail;
  }

  /**
   * Fetch the relationship graph (ERD) for a schema/database: tables (with
   * columns + PK/FK flags) and the FK edges between them. Read-only; backed by
   * the same introspection the tree uses. `maxTables` caps the fan-out.
   */
  /** The ERD graph for `schema`. Null ONLY with no connection selected; a
   *  failed load throws so the diagram renders it inline with Retry — a null
   *  must never read as "No schema selected" (same rule as listBuilderTables). */
  async fetchSchemaGraph(schema: string, maxTables = 60): Promise<DbSchemaGraph | null> {
    const id = this.selectedConnId;
    if (!id) return null;
    return await api.post<DbSchemaGraph>(`${this.connBase(id)}/schema-graph`, {
      schema,
      max_tables: maxTables,
    });
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
  /** The tables/views of `dbPath`, cached per path. THROWS on a failed load:
   *  the palette renders it inline with Retry and the import path names the
   *  database — an empty list must only ever mean "this database has none". */
  async listBuilderTables(dbPath: string): Promise<{ label: string; path: string; kind: string }[]> {
    const id = this.selectedConnId;
    if (!id) return [];
    const cached = this.builderTablesCache.get(dbPath);
    if (cached) return cached;
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
  }

  // ── Query ─────────────────────────────────────────────────────────────────

  /** True when `t` (a tab of connection `connId`) is the query tab on screen:
   *  the Database page is open, on that connection's DB workbench, with `t`
   *  as its active tab. */
  private isVisibleTab(connId: Id, t: QueryTab): boolean {
    return (
      (router.module === 'database' || router.module === 'connections') &&
      this.selectedConnId === connId &&
      this.activePane === null &&
      this.tab?.id === t.id
    );
  }

  /** Run the active tab's statement (or a given one) and store the result.
   *  `node` scopes it: omitted (`undefined`) → the active database; `null` →
   *  explicitly NO scope (how a result that ran unscoped is paged/refreshed —
   *  it must not pick up a database selected since). */
  async runQuery(
    statement?: string,
    node?: string | null,
    opts?: {
      transient?: boolean;
      keepOffset?: boolean;
      cursor?: unknown;
      /**
       * Run READ-ONLY (`QueryRequest.read_only`): the server refuses a write/DDL
       * before touching the driver. What an agent's UI-control run always sends —
       * the human's token must never quietly upgrade the agent to writes.
       */
      readOnly?: boolean;
      /**
       * Asked when a `readOnly` run was refused as a write on an UNGUARDED
       * connection (the attributed "‹agent› wants to write" confirm). True
       * re-runs it normally. A guarded (prod / read-only) connection skips this
       * and goes straight to the typed confirm, attributed with `agentLabel`.
       */
      confirmWrite?: () => Promise<boolean>;
      /** Who asked, for the typed guarded-write confirm ("Claude · fix-orders"). */
      agentLabel?: string;
      /** Filled with how the run ended (see {@link RunOutcome}). */
      outcome?: RunOutcome;
      /** Called right before a confirm waits on the person (an agent command
       *  extends its deadline to the human-wait cap). */
      awaitingHuman?: (note: string) => void;
      /** Row cap for THIS run instead of the persisted default (`rowLimit`) —
       *  an agent's `row_limit` must not rewrite the person's preference. */
      maxRows?: number;
    },
  ): Promise<QueryResult | null> {
    const id = this.selectedConnId;
    const origin = this.selectedConn ? { ...this.selectedConn } : null;
    const t = this.tab;
    const outcome = opts?.outcome;
    if (!id) {
      toasts.error('No connection selected');
      if (outcome) Object.assign(outcome, { status: 'invalid', error: 'no connection selected' });
      return null;
    }
    const sql = (statement ?? t.statement).trim();
    if (!sql) {
      toasts.error('Statement is empty');
      if (outcome) Object.assign(outcome, { status: 'invalid', error: 'statement is empty' });
      return null;
    }
    // A transient run (the selected / current statement, variable-substituted)
    // must NOT clobber the editor's full multi-statement buffer.
    if (statement !== undefined && !opts?.transient) t.statement = statement;
    // A fresh run starts at the first page; only the pager (runPage) keeps offset.
    if (!opts?.keepOffset) t.offset = 0;
    // Cancel any prior in-flight run for this tab before starting a new one
    // (server-side too, so a previous heavy query stops on the DB).
    this.abortQuery(t.id);
    const controller = new AbortController();
    // Per-run id the server registers the query under; the same id lets the
    // cancel endpoint issue engine-native cancellation (KILL QUERY / etc.) and
    // the query-status endpoint re-attach to a run whose HTTP wait was lost.
    const queryId = newQueryId();
    const accessEpoch=this.accessEpoch;
    // The engine this run belongs to — a failure may land after the person has
    // switched connection, and its background toast must read it as THIS engine.
    const runEngine = this.capabilities?.engine ?? null;
    this.runControllers.set(t.id, { controller, queryId, connId: id });
    // Scope to the active database (so unqualified tables resolve) unless a
    // node was passed — `null` included, which means "no scope".
    const scopeNode = node === undefined ? this.activeDb || null : node;
    t.running = true;
    t.error = null;
    t.err_statement = null;
    t.pending = { queryId, connId: id, sql, node: scopeNode, startedAt: Date.now() };
    this.persistTabs();
    // Stop clears ownership immediately; its HTTP wait stays alive until the
    // native cancel responds, so the signal alone no longer identifies a live run.
    const stopped = () => accessEpoch !== this.accessEpoch || controller.signal.aborted
      || this.runControllers.get(t.id)?.controller !== controller || t.pending?.queryId !== queryId;
    try {
      // Honor an explicit LIMIT in the SQL; otherwise apply the configured
      // default row cap. The server also injects this LIMIT into the SQL so a
      // huge table isn't fully scanned — this value just sizes that cap.
      const explicit = parseExplicitLimit(sql);
      // Per-tab timeout (opt-in; null / 0 = no limit).
      const tabTimeoutMs = this.tab?.timeout_ms ?? null;

      const tabMask = this.tab?.mask ?? false;
      // Dropped (below) once the human approves the write the read-only run
      // refused — the re-run is then an ordinary run under their own confirm.
      let readOnly = opts?.readOnly === true;
      const post = (confirmWrite: boolean): Promise<QueryResult> => {
        const body: RunQueryReq = {
          statement: sql,
          max_rows: explicit ?? opts?.maxRows ?? this.rowLimit,
          node: scopeNode,
          confirm_write: confirmWrite,
          // Per-run id so the cancel endpoint can issue engine-native
          // cancellation (KILL QUERY / etc.) for this in-flight query.
          query_id: queryId,
          // Footer pager: server appends OFFSET (Mongo: skip) when auto-limiting.
          ...(t.offset > 0 ? { offset: t.offset } : {}),
          // Keyset "Next" (Mongo): the previous page's `next_cursor`, echoed back
          // so the server pages by `_id > cursor` instead of `skip`. Sent next to
          // `offset` — the server ignores `skip` when the cursor applies and falls
          // back to it when the find isn't keyset-eligible.
          ...(opts?.cursor !== undefined ? { cursor: opts.cursor } : {}),
          // Driver-enforced timeout (engine-native, e.g. MySQL MAX_EXECUTION_TIME).
          ...(tabTimeoutMs && tabTimeoutMs > 0 ? { timeout_ms: tabTimeoutMs } : {}),
          // Server-side PII/prod masking: redacts cell values before they leave
          // the server. Only sent when the toggle is explicitly on.
          ...(tabMask ? { mask: true } : {}),
          ...(readOnly ? { read_only: true } : {}),
        };
        return api.post<QueryResult>(`${this.connBase(id)}/query`, body, controller.signal);
      };

      let result: QueryResult;
      try {
        result = await post(false);
      } catch (e) {
        if (stopped()) throw e;
        if (readOnly && isReadOnlyRefusal(e)) {
          // An agent's read-only run hit a write/DDL: the human decides. A
          // guarded connection always takes the typed confirm (and re-runs with
          // the explicit confirm flag); an unguarded one takes the caller's
          // attributed confirm, then re-runs as an ordinary run.
          const guarded = origin?.environment === 'prod' || origin?.read_only === true;
          opts?.awaitingHuman?.(guarded ? 'Waiting for the typed write confirm' : 'Waiting for you to allow the write');
          const answer: GuardedWriteAnswer = guarded
            ? await this.confirmGuardedWrite(origin, e, opts?.agentLabel)
            : (await (opts?.confirmWrite?.() ?? Promise.resolve(false)))
              ? 'confirmed'
              : 'declined';
          if (answer !== 'confirmed' || stopped()) {
            // A mistyped name was already explained by the gate.
            if (answer !== 'mismatch') toasts.info('Write canceled');
            if (t.pending?.queryId === queryId) this.clearPending(t);
            if (outcome) Object.assign(outcome, { status: 'cancelled', error: 'the user declined the write' });
            return null;
          }
          readOnly = false;
          result = await post(guarded);
        } else if (isWriteBlocked(e)) {
          // Production / read-only guardrail: the server refused a write/DDL on a
          // guarded connection. Ask for a typed confirmation and, if granted,
          // retry with the explicit confirm flag.
          opts?.awaitingHuman?.('Waiting for the typed write confirm');
          const answer = await this.confirmGuardedWrite(origin, e, opts?.agentLabel);
          if (answer !== 'confirmed' || stopped()) {
            if (answer !== 'mismatch') toasts.info('Write canceled');
            if (t.pending?.queryId === queryId) this.clearPending(t);
            if (outcome) Object.assign(outcome, { status: 'cancelled', error: 'the user declined the write' });
            return null;
          }
          result = await post(true);
        } else {
          throw e;
        }
      }
      if (stopped()) {
        if (outcome) Object.assign(outcome, { status: 'aborted', error: 'the run was stopped or superseded' });
        return null;
      }
      this.installResult(t, result);
      t.ran_statement = sql;
      t.ran_node = scopeNode;
      t.err_statement = null;
      this.clearPending(t);
      void this.loadHistory(id);
      if (outcome) outcome.status = 'ok';
      return result;
    } catch (e) {
      // A user-initiated abort isn't an error — leave the prior result intact.
      // What the Stop actually achieved is reported by `abortQuery` from the
      // server's cancel outcome — this only drops our wait, so it must not
      // claim the query stopped.
      if (isAbortError(e) || stopped()) {
        if (outcome) Object.assign(outcome, { status: 'aborted', error: 'the query was stopped' });
        return null;
      }
      // The server answered → the query itself finished with an error.
      if (e instanceof ApiError) {
        if (outcome) Object.assign(outcome, { status: 'failed', error: loadErrorText(e) });
        this.clearPending(t);
        t.error = loadErrorText(e);
        // What failed — ErrorPanel's caret / "Ask AI to fix" describe THIS, not
        // the (possibly much larger, since edited) editor buffer.
        t.err_statement = sql;
        // The tab's inline ErrorPanel already says this when it's on screen —
        // only toast when the failing tab is NOT the one the person is looking at.
        // Headline only (the normalised title) — the full error, cause and
        // hint wait in the tab's ErrorPanel.
        if (!this.isVisibleTab(id, t)) {
          toasts.error('Couldn’t query', normalizeDbError(runEngine, loadErrorText(e), sql).title);
        }
        return null;
      }
      // The HTTP wait was lost (page teardown / network blip) but the server
      // keeps executing the query detached — re-attach by query_id instead of
      // declaring failure.
      if (outcome) Object.assign(outcome, { status: 'detached', error: loadErrorText(e) });
      void this.reattach(t);
      return null;
    } finally {
      // Only clear running/controller if this run is still the current one
      // (a newer run may have replaced it).
      if (this.runControllers.get(t.id)?.controller === controller) {
        this.runControllers.delete(t.id);
        t.running = false;
      }
    }
  }

  /**
   * Page the active tab's auto-limited result by `delta` pages (±1). The page
   * size is the server's applied LIMIT (`auto_limited`); re-runs the same
   * statement with the new row offset (server appends OFFSET / Mongo skip).
   * "Next" on a keyset-eligible Mongo find also echoes the page's `next_cursor`
   * back as `cursor`, so the server pages by `_id > cursor` instead of `skip`;
   * "Prev" stays offset-based. No-op when the current result wasn't auto-paginated.
   */
  runPage(
    delta: number,
    extra?: Pick<
      NonNullable<Parameters<DatabaseStore['runQuery']>[2]>,
      'readOnly' | 'confirmWrite' | 'agentLabel' | 'outcome' | 'awaitingHuman'
    >,
  ): Promise<QueryResult | null> {
    const t = this.tab;
    const pageSize = t?.result?.auto_limited ?? 0;
    // No known producing statement (a re-attached run from an older marker):
    // never fall back to re-running the whole editor buffer.
    if (!t || pageSize <= 0 || !t.ran_statement) return Promise.resolve(null);
    const next = Math.max(0, t.offset + delta * pageSize);
    if (next === t.offset) return Promise.resolve(null);
    // The cursor belongs to the page the user is LEAVING — read it before the
    // re-run replaces `t.result`.
    const cursor = delta > 0 ? (t.result?.next_cursor ?? undefined) : undefined;
    const prev = t.offset;
    t.offset = next;
    // How the run ended — the caller's object when it passed one (an agent
    // command reads it back), else a local one.
    const outcome: RunOutcome = extra?.outcome ?? {};
    // Page the statement (and scope node) that PRODUCED the result — the editor
    // buffer / active DB may have been edited since the run. `ran_node` is
    // passed as-is: a `null` (ran unscoped) must stay unscoped, not fall back
    // to a database selected since. `transient` keeps the buffer untouched.
    return this.runQuery(t.ran_statement ?? undefined, t.ran_node, {
      ...extra,
      outcome,
      keepOffset: true,
      transient: true,
      cursor,
      // Keep the page size the pages are counted in: the result being paged
      // may have run with a different cap (an agent's `row_limit`, or a Limit
      // changed since) than the persisted default.
      maxRows: pageSize,
    }).then((r) => {
      // A stopped, declined or failed page change leaves the grid on the page
      // it was on — so must the pager (else the footer says "51–100" over rows
      // 1–50 and the next Next skips a page). A detached run keeps `next`: its
      // re-attach installs THAT page. A newer pager move owns the offset.
      if (!r && outcome.status !== 'detached' && t.offset === next) t.offset = prev;
      return r;
    });
  }

  /**
   * Typed confirmation gate for a write on a guarded (prod / read-only)
   * connection. The user must type the connection name verbatim, so a write to
   * production is a deliberate, explicit act. `'confirmed'` only on an exact,
   * case-insensitive match. A typed name that doesn't match is explained here
   * (the shared prod-gate wording) and answers `'mismatch'`, so callers don't
   * add a second, vaguer "canceled" toast. Public so other write flows (the
   * import dialog keeps an import-worded copy today) can take the same gate.
   */
  async confirmGuardedWrite(
    conn: Pick<Connection, 'name' | 'environment'> | null,
    blocked?: unknown,
    agentLabel?: string,
  ): Promise<GuardedWriteAnswer> {
    if (!conn) return 'declined';
    const prod = conn.environment === 'prod';
    const label = prod ? 'PRODUCTION' : 'read-only';
    // An agent-driven write names who is asking — the typed confirm is the
    // person's decision about SOMEONE ELSE's statement.
    const who = agentLabel ? `${agentLabel} wants to run this. ` : '';
    // Redis `KEYS` is gated too — not a write, but it blocks the whole server
    // while it walks every key; say that instead of "can modify data".
    const blockingKeys = blocked instanceof ApiError && blocked.message.includes('KEYS blocks');
    const body = blockingKeys
      ? `KEYS blocks the Redis server while it scans every key on the ${label} ` +
        `connection "${conn.name}" — prefer SCAN with MATCH/COUNT. Type the ` +
        `connection name to run it anyway.`
      : `You are about to run a WRITE / schema change on the ${label} connection ` +
        `"${conn.name}". This can modify or destroy data. Type the connection ` +
        `name to confirm.`;
    // Titled by what the connection IS — a read-only dev connection is not
    // "production". The danger Modal carries the tone; no glyph.
    const title = blockingKeys
      ? 'Confirm blocking command'
      : prod
        ? 'Confirm production write'
        : 'Confirm write on a read-only connection';
    const typed = await confirmer.promptText(who + body, {
      title,
      confirmLabel: blockingKeys ? 'Run KEYS' : 'Run write',
      placeholder: conn.name,
      danger: true,
    });
    if (typed == null) return 'declined';
    if (typed.trim().toLowerCase() !== conn.name.trim().toLowerCase()) {
      toasts.info('Nothing was changed', 'What you typed didn’t match, so the action was canceled.');
      return 'mismatch';
    }
    return 'confirmed';
  }

  /**
   * Run an arbitrary statement against the selected connection (used by inline
   * cell edits / row deletes), applying the production / read-only write-gate:
   * if the server blocks the write, ask for a typed confirmation and retry with
   * the confirm flag. Returns the result, or `{ canceled }` saying why nothing
   * ran (see {@link ManagedWriteCanceled}).
   *
   * Throws on any non-guardrail failure so the caller keeps its own error UX.
   */
  async runManagedStatement(sql: string, node?: string | null): Promise<QueryResult | ManagedWriteCanceled> {
    const id = this.selectedConnId;
    const origin = this.selectedConn ? { ...this.selectedConn } : null;
    const accessEpoch = this.accessEpoch;
    const connEpoch = id ? this.epochOf(id) : 0;
    if (!id) throw new Error('No connection selected');
    // Same scope rule as `runQuery`: only an OMITTED node means "active DB";
    // an explicit `null` (a grid edit of a result that ran unscoped) stays
    // unscoped instead of landing in whatever database is selected now.
    const scopeNode = node === undefined ? this.activeDb || null : node;
    const post = (confirmWrite: boolean): Promise<QueryResult> =>
      api.post<QueryResult>(`${this.connBase(id)}/query`, {
        statement: sql,
        node: scopeNode,
        confirm_write: confirmWrite,
      });
    try {
      return await post(false);
    } catch (e) {
      if (isWriteBlocked(e)) {
        const answer = await this.confirmGuardedWrite(origin, e);
        if (answer !== 'confirmed') return { canceled: answer };
        if (accessEpoch !== this.accessEpoch || connEpoch !== this.epochOf(id)) return { canceled: 'stale' };
        return await post(true);
      }
      throw e;
    }
  }

  /**
   * Run a real query plan for the active tab's statement: SQL engines prepend
   * `EXPLAIN`; Mongo sends the `explain` flag (server `explain` command). The
   * plan replaces the tab's result.
   */
  async runExplain(opts?: { readOnly?: boolean }): Promise<QueryResult | null> {
    const id = this.selectedConnId;
    const t = this.tab;
    if (!id) {
      toasts.error('No connection selected');
      return null;
    }
    const stmt = t.statement.trim();
    if (!stmt) {
      toasts.error('Statement is empty');
      return null;
    }
    const isSql = this.capabilities?.sql === true;
    const accessEpoch=this.accessEpoch;
    t.running = true;
    t.error = null;
    t.err_statement = null;
    try {
      const body: Record<string, unknown> = isSql
        ? { statement: `EXPLAIN ${stmt}`, max_rows: this.rowLimit, node: this.activeDb || null }
        : { statement: stmt, max_rows: this.rowLimit, node: this.activeDb || null, explain: true };
      // An agent's Explain can't smuggle a write through `EXPLAIN ANALYZE …`.
      if (opts?.readOnly) body.read_only = true;
      const result = await api.post<QueryResult>(`${this.connBase(id)}/query`, body);
      if(accessEpoch!==this.accessEpoch)return null;
      this.installResult(t, result);
      t.ran_statement = stmt;
      t.ran_node = this.activeDb || null;
      return result;
    } catch (e) {
      t.error = loadErrorText(e);
      t.err_statement = stmt;
      toastError('Couldn’t explain', e);
      return null;
    } finally {
      t.running = false;
    }
  }

  /**
   * Fetch a NORMALIZED query plan (`POST …/db/query-plan`) for the active
   * statement and open the plan panel. Falls back to the raw `EXPLAIN` → grid
   * path (`runExplain`) if the endpoint fails or can't normalize the plan, so
   * Explain always shows something. The statement is EXPLAIN-wrapped server-side
   * (never executed raw), so this is read-only even on a guarded connection.
   */
  async explainPlan(opts?: { readOnly?: boolean }): Promise<void> {
    const accessEpoch=this.accessEpoch;
    const id = this.selectedConnId;
    const stmt = this.tab.statement.trim();
    if (!id) {
      toasts.error('No connection selected');
      return;
    }
    if (!stmt) {
      toasts.error('Statement is empty');
      return;
    }
    try {
      const plan = await api.post<DbQueryPlan>(`${this.connBase(id)}/query-plan`, {
        statement: stmt,
        node: this.activeDb || null,
      });
      if(accessEpoch!==this.accessEpoch)return;
      this.queryPlan = plan;
      this.planOpen = true;
    } catch {
      // Engine can't produce a normalized plan (or the endpoint failed) — fall
      // back to the always-available raw EXPLAIN → grid path.
      if(accessEpoch!==this.accessEpoch)return;
      this.closePlan();
      await this.runExplain(opts);
    }
  }

  /** Close the query-plan panel and drop the plan. */
  closePlan(): void {
    this.planOpen = false;
    this.queryPlan = null;
  }

  /**
   * Stop the in-flight query for a tab (defaults to the active tab). Aborts the
   * fetch (drops our HTTP wait) AND tells the server to cancel the query
   * engine-side (`POST …/db/cancel` with the run's `query_id`) so the database
   * stops the heavy work and frees the cached connection — not just our client.
   * The server cancel never blocks stopping the UI. `report` (the user's Stop
   * button / Esc) toasts what the server says the Stop ACHIEVED — "stopped"
   * only when it really stopped; tab closes and superseded runs stay quiet.
   */
  abortQuery(tabId?: number, opts?: { report?: boolean }): void {
    const report = opts?.report === true;
    const id = tabId ?? this.tab?.id;
    if (id == null) return;
    const at = this.locateTab(id);
    if (at) {
      this.abortRunForTab(at.tab, report);
      // A captured tab ID can belong to a parked connection. Persist its
      // cleared marker there, so restoring/reloading cannot reattach the run.
      this.persistTabsNow(at.connId);
      return;
    }
    // No retained tab (a closed/foreign run) — still drop the controller if held.
    const entry = this.runControllers.get(id);
    if (!entry) return;
    this.runControllers.delete(id);
    void this.sendCancel(entry.connId, entry.queryId, report).finally(() => entry.controller.abort());
  }

  /** Ask the server to cancel a run (`POST …/db/cancel`). With `report`, toast
   *  its outcome: only `cancelled` means the database stopped the work. */
  private sendCancel(connId: Id, queryId: string, report: boolean): Promise<void> {
    // Writes have no default transport deadline. Bound Stop so a lost cancel
    // response still releases the original query wait. Never retry cancellation.
    const deadline = withDeadline(READ_DEADLINE_MS);
    return api
      .post<DbCancelOutcome | undefined>(`${this.connBase(connId)}/cancel`, { query_id: queryId }, deadline.signal)
      .then((out) => {
        if (!report) return;
        switch (out?.status) {
          case 'aborted':
            toasts.warn(
              'Query stopped in Otto',
              'No further statement will run, but a statement already sent may still finish on the server.',
            );
            break;
          case 'not_stoppable':
            toasts.warn(
              'Query is still running',
              'This engine cannot cancel it — it runs until it finishes or reaches its timeout.',
            );
            break;
          case 'not_running':
            toasts.info('Query had already finished');
            break;
          default:
            toasts.info('Query stopped');
        }
      })
      .catch((e) => {
        if (report) toastError('Couldn’t stop the query', e);
      })
      .finally(deadline.done);
  }

  /**
   * Cancel one tab's run — server-side engine cancel (fire-and-forget), fetch
   * abort, and run-state reset. Works on snapshot tabs of a BACKGROUND
   * connection too, which is why it never persists: the localStorage key
   * belongs to the ACTIVE connection (callers that own it persist themselves).
   * The run's identity is the live controller entry, or — when the HTTP wait
   * was already lost and the tab is in re-attach mode — its pending marker.
   */
  private abortRunForTab(t: QueryTab, report = false): void {
    const entry = this.runControllers.get(t.id);
    const target = entry ?? (t.pending ? { ...t.pending } : null);
    if (!target) return;
    this.runControllers.delete(t.id);
    // Governed queries are owned by their HTTP request. Aborting that request
    // first removes the native cancellation handle before /cancel can use it.
    // Capture this controller: a new run may start before cancellation settles.
    void this.sendCancel(target.connId, target.queryId, report).finally(() => entry?.controller.abort());
    // Clear UI ownership immediately; late results/errors cannot affect this
    // tab, and re-attach must not resurrect a run the user explicitly stopped.
    t.running = false;
    t.pending = null;
  }

  /** Forget a tab's pending run marker (persisted with the tabs). */
  private clearPending(t: QueryTab): void {
    if (!t.pending) return;
    t.pending = null;
    this.persistTabs();
  }

  /**
   * Re-attach a tab to a run whose HTTP wait was lost: poll the server's
   * `db/query-status` for the tab's `pending` run until it reports `done`
   * (install the parked result/error) or `unknown` (daemon restarted or the
   * outcome expired — stop quietly). The query itself keeps executing
   * server-side the whole time; this only recovers its outcome.
   */
  async reattach(t: QueryTab): Promise<void> {
    const pending = t.pending;
    if (!pending) return;
    // One poll loop per tab — a second kick (restore + catch both fire) joins
    // the existing loop instead of doubling the polling.
    if (this.reattaching.has(t.id)) return;
    this.reattaching.add(t.id);
    try {
      await this.reattachLoop(t, pending);
    } finally {
      this.reattaching.delete(t.id);
    }
  }

  private async reattachLoop(
    t: QueryTab,
    pending: PendingRun,
  ): Promise<void> {
    t.running = true;
    let failures = 0;
    // Re-entrancy / staleness guard: stop when the tab moved on to a different
    // run (a new runQuery replaces `pending`) or the marker was cleared.
    while (t.pending && t.pending.queryId === pending.queryId) {
      let st: DbQueryStatus;
      try {
        st = await api.post<DbQueryStatus>(`${this.connBase(pending.connId)}/query-status`, {
          query_id: pending.queryId,
        });
        failures = 0;
      } catch {
        // Daemon unreachable (sleep/restart in progress): retry a few times,
        // then give up for now — the persisted marker lets a later visit retry.
        if (++failures >= 5) {
          t.running = false;
          return;
        }
        await new Promise((r) => setTimeout(r, 2000));
        continue;
      }
      if (st.status === 'running') {
        // A run this page didn't start (agent, other window, an old marker):
        // backfill its real start so the timer doesn't count from 0.
        if (
          st.elapsed_ms != null &&
          t.pending?.queryId === pending.queryId &&
          !t.pending.startedAt
        ) {
          t.pending = { ...t.pending, startedAt: Date.now() - st.elapsed_ms };
          this.persistTabs();
        }
        await new Promise((r) => setTimeout(r, 1500));
        continue;
      }
      if (t.pending?.queryId !== pending.queryId) return; // superseded mid-poll
      if (st.status === 'done') {
        if (st.error != null) {
          t.error = st.error;
          t.err_statement = pending.sql ?? null;
        } else if (st.result) {
          this.installResult(t, st.result);
          // The statement + scope the run was ISSUED with (kept in the marker).
          // Never the editor buffer: it is usually a multi-statement script the
          // run only took one statement of, and the pager re-runs ran_statement.
          t.ran_statement = pending.sql ?? null;
          t.ran_node = pending.node ?? null;
          t.error = null;
          t.err_statement = null;
        }
        void this.loadHistory(pending.connId);
      }
      // done or unknown: either way this run is over for the client.
      t.running = false;
      this.clearPending(t);
      return;
    }
  }

  /** Kick off re-attach polling for every tab restored with a pending run. */
  private reattachPendingTabs(): void {
    for (const t of this.tabs) {
      if (t.pending && !this.runControllers.has(t.id)) void this.reattach(t);
    }
  }

  // ── Table actions (schema-tree context menu) ──────────────────────────────

  /**
   * Build a qualified SQL table reference from a tree node id like
   * `db:configserver/table:props`, quoted for the selected connection's engine
   * (`"public"."orders"` on Postgres, backticks on MySQL / ClickHouse). Returns
   * the ref plus the raw parts, or null when the node isn't a SQL table/view.
   */
  tableRefFromNode(node: SchemaNode): { ref: string; db: string | null; table: string } | null {
    return tableRefFromNodeId(this.selectedConn?.kind, node.id);
  }

  /** Open a statement in a new query tab; optionally run it immediately. `node`
   *  scopes execution (e.g. a Redis keyspace `kdb:N` so the right DB is SELECTed). */
  async openInNewTab(
    sql: string,
    opts?: { run?: boolean; name?: string; node?: string },
  ): Promise<void> {
    this.newTab(sql);
    if (opts?.name) this.tab.name = opts.name;
    if (opts?.run) await this.runQuery(undefined, opts.node);
  }

  // ── Redis key actions ─────────────────────────────────────────────────────

  /** Split a Redis key node id `kdb:<n>/key:<fullkey>` into its keyspace + key.
   *  The key may itself contain ':' / '/', so we slice at the first `/key:`. */
  redisKeyParts(node: SchemaNode): { key: string; keyspace: string } | null {
    const i = node.id.indexOf('/key:');
    if (i < 0) return null;
    return { key: node.label, keyspace: node.id.slice(0, i) };
  }

  /** The correct read command for a Redis key, based on its value TYPE (carried
   *  in the node's `detail`). GET only works on strings — hashes need HGETALL,
   *  lists LRANGE, etc. (using GET on a hash is what returned `(nil)`). */
  redisReadCommand(type: string | undefined, key: string): string {
    const k = /\s|"/.test(key) ? `"${key.replace(/\\/g, '\\\\').replace(/"/g, '\\"')}"` : key;
    switch ((type ?? '').toLowerCase()) {
      case 'hash':
        return `HGETALL ${k}`;
      case 'list':
        return `LRANGE ${k} 0 -1`;
      case 'set':
        return `SMEMBERS ${k}`;
      case 'zset':
        return `ZRANGE ${k} 0 -1 WITHSCORES`;
      case 'stream':
        return `XRANGE ${k} - +`;
      default:
        return `GET ${k}`;
    }
  }

  /** New tab with the type-correct read command for a Redis key, scoped to its
   *  keyspace; runs immediately unless `opts.run === false`. */
  async getRedisValue(node: SchemaNode, opts?: { run?: boolean }): Promise<void> {
    const r = this.redisKeyParts(node);
    if (!r) return;
    const cmd = this.redisReadCommand(node.detail, r.key);
    await this.openInNewTab(cmd, { run: opts?.run ?? true, name: r.key, node: r.keyspace });
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

  /** Open the file→table Import dialog, prefilling the target table from a
   *  schema-tree node (the raw, unquoted table name the INSERT needs). */
  openImportDialog(node?: SchemaNode): void {
    // Mongo tree nodes are collections (`coll:`), never `table:`/`view:` — the
    // SQL resolver can't match them, so the prefill was always empty there.
    this.importTable = node
      ? (this.tableRefFromNode(node)?.table ?? this.collectionRefFromNode(node)?.coll) ?? ''
      : '';
    this.importDialogOpen = true;
  }

  /** Resolve a Mongo collection node to its `{ db, coll }`. */
  collectionRefFromNode(node: SchemaNode): { db: string | null; coll: string } | null {
    const segs = node.id.split('/').map((s) => {
      const i = s.indexOf(':');
      return i < 0 ? ([s, ''] as const) : ([s.slice(0, i), s.slice(i + 1)] as const);
    });
    const find = (k: string) => segs.find(([kk]) => kk === k)?.[1];
    const coll = find('coll') ?? find('collection');
    if (!coll) return null;
    return { db: find('db') ?? null, coll };
  }

  /** New tab: `db.<coll>.find({})` scoped to the collection's database, then run. */
  async findRows(node: SchemaNode): Promise<void> {
    const r = this.collectionRefFromNode(node);
    if (!r) return;
    if (r.db) this.setActiveDb(r.db);
    await this.openInNewTab(`db.${r.coll}.find({})`, { run: true, name: r.coll });
  }

  /** New tab: `db.<coll>.find({})` without running (Send to editor). */
  async sendFindToEditor(node: SchemaNode): Promise<void> {
    const r = this.collectionRefFromNode(node);
    if (!r) return;
    if (r.db) this.setActiveDb(r.db);
    await this.openInNewTab(`db.${r.coll}.find({})`, { name: r.coll });
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

  // ── Quick filters (chips that own the active tab's WHERE clause) ───────────
  // Chips accumulate without running the query — the user adds more, then runs.
  // Repeated equals on a column collapse into IN / NOT IN; include vs exclude
  // are separate directions. Applying rewrites the statement's WHERE in place.

  /** Quick-filter chips for the active tab. */
  get filters(): FilterCond[] {
    return this.tab?.filters ?? [];
  }

  /** On the first chip, fold any hand-written WHERE into a removable raw chip
   * so chips can safely own the WHERE from then on. */
  private absorbExistingWhere(t: QueryTab): void {
    if (t.filters.length > 0) return;
    const existing = extractWhereBody(t.statement);
    if (existing && existing.trim()) t.filters.push({ kind: 'raw', text: existing.trim() });
  }

  /** Add a value-based filter from a cell (include = equals, exclude = not). */
  addQuickFilter(column: string, value: unknown, mode: 'include' | 'exclude'): void {
    const t = this.tab;
    if (!t || !column) return;
    this.absorbExistingWhere(t);
    const op = mode === 'include' ? 'in' : 'not_in';
    const fv = toFilterVal(value);
    let cond = t.filters.find(
      (c): c is Extract<FilterCond, { kind: 'col' }> =>
        c.kind === 'col' && c.column === column && c.op === op,
    );
    if (!cond) {
      cond = { kind: 'col', column, op, values: [] };
      t.filters.push(cond);
    }
    if (!cond.values.some((v) => v.raw === fv.raw && v.isNull === fv.isNull)) cond.values.push(fv);
    this.applyFilters();
  }

  /** Add an empty (value-less) filter on a column, to be filled in the bar. */
  addColumnFilter(column: string): void {
    const t = this.tab;
    if (!t || !column) return;
    this.absorbExistingWhere(t);
    if (!t.filters.some((c) => c.kind === 'col' && c.column === column)) {
      t.filters.push({ kind: 'col', column, op: 'in', values: [] });
    }
    this.applyFilters();
  }

  /** Add a typed value to an existing column chip. */
  addFilterValue(condIndex: number, text: string): void {
    const t = this.tab;
    const c = t?.filters[condIndex];
    if (!t || !c || c.kind !== 'col' || !text.trim()) return;
    const fv = parseFilterValText(text);
    if (!c.values.some((v) => v.raw === fv.raw && v.isNull === fv.isNull)) c.values.push(fv);
    this.applyFilters();
  }

  removeFilterValue(condIndex: number, valIndex: number): void {
    const t = this.tab;
    const c = t?.filters[condIndex];
    if (!t || !c || c.kind !== 'col') return;
    c.values.splice(valIndex, 1);
    this.applyFilters();
  }

  removeFilterCond(condIndex: number): void {
    const t = this.tab;
    if (!t) return;
    t.filters.splice(condIndex, 1);
    this.applyFilters();
  }

  /** Flip a column chip between include (IN) and exclude (NOT IN). */
  toggleFilterMode(condIndex: number): void {
    const t = this.tab;
    const c = t?.filters[condIndex];
    if (!t || !c || c.kind !== 'col') return;
    c.op = c.op === 'in' ? 'not_in' : 'in';
    this.applyFilters();
  }

  clearFilters(): void {
    const t = this.tab;
    if (!t) return;
    t.filters = [];
    this.applyFilters();
  }

  /** Take over the WHERE from a cell "Query by value" / "Add to query" action:
   * set the rewritten statement AND drop any quick-filter chips — otherwise the
   * chip bar would desync and the next chip op (`applyFilters`) would silently
   * rebuild the WHERE from the stale chips, discarding the spliced condition.
   * Clears the chips WITHOUT `applyFilters` (which would strip the new WHERE).
   * Does NOT run. */
  setStatementFromCellFilter(statement: string): void {
    const t = this.tab;
    if (!t) return;
    t.filters = [];
    this.setStatement(statement);
  }

  /** Rewrite the active statement's WHERE from the chips (does NOT run). */
  private applyFilters(): void {
    const t = this.tab;
    if (!t) return;
    // Identifier quoting, string escaping and boolean literals follow the
    // connection's engine (`sql-dialect.ts`) — Postgres gets `"col"`, no
    // backslash doubling and TRUE/FALSE.
    const engine = this.selectedConn?.kind ?? null;
    const body = t.filters
      .map((c) => condToSql(c, engine))
      .filter((s) => s.trim())
      .join(' AND ');
    t.statement = rewriteWhere(t.statement, body);
    this.persistTabs();
  }

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

/** A storage write refused for space (WebKit/Chromium/Firefox spellings). */
function isQuotaError(e: unknown): boolean {
  return (
    e instanceof DOMException &&
    (e.name === 'QuotaExceededError' || e.name === 'NS_ERROR_DOM_QUOTA_REACHED' || e.code === 22)
  );
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
