// Database Explorer store — connections: the connection list load, open /
// close / switch (per-connection working-set snapshots), the per-connection
// load (capabilities, tabs, view, schema root), liveness status, the
// background warm-up of restored tabs and the keep-alive poll. The
// DatabaseStore façade (../database.svelte.ts) owns one instance as `conn` and
// delegates its public methods here; the reactive working set (connections,
// open tabs, status map, …) stays on the façade, which snapshots it whole.
//
// `ConnEpochs` is the shared open-generation token: `closeConnection` bumps a
// connection's epoch, and every async loader in every part captures it at
// start and drops its response when the connection closed (or re-opened)
// meanwhile.

import { api, ApiError, dbCloseConnection } from '../../api/client';
import type { Connection, DbCapabilities, DbTestResult, Id, SchemaNode } from '../../api/types';
import { ws } from '../workspace.svelte';
import { toasts } from '../../toast.svelte';
import { mapLimit, pollWhileVisible, type Poller } from '../../poll';
import { loadErrorText } from '../../loadError';
import { toastError } from '../../toastError';
import { dbPrefs, type WarmMode } from '../dbPrefs.svelte';
import { KEEP_ALIVE_MS, WARM_CONCURRENCY, blankTab, isDbKind } from './model';
import type { ConnPhase, ConnSnapshot, ConnStatus, DbMainTab, SshTab } from './model';
import type { DatabaseStore } from '../database.svelte';

/**
 * Per-connection open generation — the one stale-response guard every part
 * shares. `closeConnection` (and an access revocation) `bump`s a connection;
 * every async loader captures `of(id)` at start and, after each await, drops
 * its result unless `live(id, epoch)` — so a close mid-load can't resurrect
 * status entries, write stale singletons, or toast for a dead tab, and a
 * close → re-open can't let the first open's late response land. Non-reactive
 * bookkeeping.
 */
export class ConnEpochs {
  private readonly epochs = new Map<Id, number>();
  private readonly openIds: () => readonly Id[];
  constructor(openIds: () => readonly Id[]) {
    this.openIds = openIds;
  }
  of(id: Id): number {
    return this.epochs.get(id) ?? 0;
  }
  bump(id: Id): void {
    this.epochs.set(id, this.of(id) + 1);
  }
  /** Still the same open generation started at `epoch` (i.e. the connection
   *  wasn't closed — possibly re-opened — since the caller began)? */
  live(id: Id, epoch: number): boolean {
    return this.openIds().includes(id) && this.of(id) === epoch;
  }
}

/** The DatabaseStore members this part reads and calls (its whole contract with the façade). */
export type ConnectionsHost = Pick<
  DatabaseStore,
  | 'accessEpoch'
  | 'activeDb'
  | 'activePane'
  | 'activeTab'
  | 'builderTablesCache'
  | 'capabilities'
  | 'childrenCache'
  | 'connBase'
  | 'connStatus'
  | 'connView'
  | 'connections'
  | 'connectionsError'
  | 'connectionsLoading'
  | 'epochs'
  | 'expanded'
  | 'history'
  | 'historyError'
  | 'historyLimit'
  | 'loadHistory'
  | 'loadingNodes'
  | 'mainTab'
  | 'nodeFilters'
  | 'objectDetail'
  | 'objectError'
  | 'objectSearchHits'
  | 'objectSearchQuery'
  | 'objectSearchScanned'
  | 'objectSearchTruncated'
  | 'objectSearching'
  | 'openConnIds'
  | 'otherConnections'
  | 'persist'
  | 'refreshCompletions'
  | 'runs'
  | 'savedQueries'
  | 'schemaLoading'
  | 'schemaRoot'
  | 'selectedConnId'
  | 'selectedObjectPath'
  | 'sideTab'
  | 'sshTabs'
  | 'supportsBuilder'
  | 'tabs'
  | 'testResult'
  | 'testing'
  | 'tree'
>;

export class DbConnectionsStore {
  private readonly db: ConnectionsHost;
  constructor(db: ConnectionsHost) {
    this.db = db;
  }

  /**
   * Per-connection working-set snapshots, keyed by connection id. Deliberately
   * NON-reactive (plain Map, not `$state`): it's an internal cache read only
   * via capture/restore, and the singleton fields it feeds ARE reactive, so
   * reassigning them on restore is what drives the UI.
   */
  snapshots = new Map<Id, ConnSnapshot>();

  // ── Loading ───────────────────────────────────────────────────────────────

  /** Load connections for the current workspace. DB kinds drive the workbench
   *  (`connections`); ssh/custom land in `otherConnections` so the unified
   *  sidebar tree can render EVERY profile (opens route by type). */
  /** In-flight connection-list load, keyed by workspace + access epoch: a
   *  Home box and a page mounting together share one request. */
  private connectionsInFlight: { key: string; p: Promise<void> } | null = null;
  loadConnections(): Promise<void> {
    const key = `${ws.currentId ?? ''}:${this.db.accessEpoch}`;
    if (this.connectionsInFlight?.key === key) return this.connectionsInFlight.p;
    const p = this.fetchConnections().finally(() => {
      if (this.connectionsInFlight?.p === p) this.connectionsInFlight = null;
    });
    this.connectionsInFlight = { key, p };
    return p;
  }

  private async fetchConnections(): Promise<void> {
    const wid = ws.currentId;
    const accessEpoch=this.db.accessEpoch;
    if (!wid) return;
    this.db.connectionsLoading = true;
    try {
      const all = await api.get<Connection[]>(`/workspaces/${wid}/connections`);
      if(accessEpoch!==this.db.accessEpoch)return;
      this.db.connectionsError = null;
      const next = all.filter((c) => isDbKind(c.kind));
      this.db.otherConnections = all.filter((c) => !isDbKind(c.kind));
      this.db.connections = next;
      // The connection library is GLOBAL (profiles are created workspace-
      // independent), so the open workbench is global too: a workspace switch
      // no longer wipes the open tabs/snapshots — open connections stay open
      // no matter which workspace is active. Every load just prunes tabs /
      // snapshots / selection whose connection no longer exists (deleted).
      this.db.openConnIds = this.db.openConnIds.filter((id) => next.some((c) => c.id === id));
      for (const id of [...this.snapshots.keys()]) {
        if (!next.some((c) => c.id === id)) this.snapshots.delete(id);
      }
      if (this.db.selectedConnId && !next.some((c) => c.id === this.db.selectedConnId)) {
        this.db.selectedConnId = null;
      }
      // Start fresh — do NOT auto-open a connection; the user picks one from the
      // sidebar. Only clear active state when no connections remain.
      if (this.db.connections.length === 0) {
        this.db.selectedConnId = null;
        this.db.capabilities = null;
        this.db.schemaRoot = [];
      }
    } catch (e) {
      if (accessEpoch === this.db.accessEpoch) this.db.connectionsError = loadErrorText(e);
    } finally {
      this.db.connectionsLoading = false;
    }
  }

  /**
   * Capture the active connection's working set into `snapshots` so we can
   * restore it (without re-fetching) when we switch back. No-op when nothing
   * is selected.
   */
  private captureSnapshot(): void {
    const id = this.db.selectedConnId;
    if (id === null) return;
    // Any debounced tab write still belongs to THIS connection — flush it while
    // the key is still right.
    this.db.persist.flushPersistTabs();
    this.snapshots.set(id, {
      capabilities: this.db.capabilities,
      testResult: this.db.testResult,
      activeDb: this.db.activeDb,
      schemaRoot: this.db.schemaRoot,
      childrenCache: this.db.childrenCache,
      expanded: this.db.expanded,
      // NEVER freeze in-flight loading flags: the loads they describe resolve
      // (or are epoch-cancelled) while this snapshot is parked, so restoring
      // them would show "Loading…" spinners nothing will ever clear.
      loadingNodes: new Set<string>(),
      schemaLoading: false,
      selectedObjectPath: this.db.selectedObjectPath,
      objectDetail: this.db.objectDetail,
      objectError: this.db.objectError,
      objectSearchQuery: this.db.objectSearchQuery,
      objectSearchHits: this.db.objectSearchHits,
      objectSearchTruncated: this.db.objectSearchTruncated,
      objectSearchScanned: this.db.objectSearchScanned,
      builderTablesCache: this.db.builderTablesCache,
      tabs: this.db.tabs,
      activeTab: this.db.activeTab,
      savedQueries: this.db.savedQueries,
      history: this.db.history,
      mainTab: this.db.mainTab,
      // 'connections' is a global picker view, not a per-connection state —
      // collapse it to 'schema' so reopening a connection never lands back on
      // the picker.
      sideTab: this.db.sideTab === 'connections' ? 'schema' : this.db.sideTab,
      connView: this.db.connView,
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
    this.db.capabilities = snap.capabilities;
    this.db.testResult = snap.testResult;
    this.db.activeDb = snap.activeDb;
    this.db.schemaRoot = snap.schemaRoot;
    this.db.childrenCache = snap.childrenCache;
    this.db.expanded = snap.expanded;
    this.db.loadingNodes = snap.loadingNodes;
    this.db.schemaLoading = snap.schemaLoading;
    this.db.selectedObjectPath = snap.selectedObjectPath;
    this.db.objectDetail = snap.objectDetail;
    this.db.objectError = snap.objectError;
    // Object search is per connection; invalidate any in-flight search from the
    // previous one so its late result can't land over this restore.
    this.db.tree.objectSearchSeq++;
    this.db.objectSearching = false;
    this.db.objectSearchQuery = snap.objectSearchQuery;
    this.db.objectSearchHits = snap.objectSearchHits;
    this.db.objectSearchTruncated = snap.objectSearchTruncated;
    this.db.objectSearchScanned = snap.objectSearchScanned;
    this.db.builderTablesCache = snap.builderTablesCache;
    this.db.tabs = snap.tabs;
    this.db.activeTab = snap.activeTab;
    // Saved queries belong to the workspace, not this connection snapshot.
    this.db.history = snap.history;
    this.db.historyError = null;
    void this.db.loadHistory(id);
    this.db.mainTab = snap.mainTab;
    this.db.sideTab = snap.sideTab;
    this.db.connView = snap.connView;
    // A snapshot captured mid-load (a switch-away made the epoch guards drop
    // the fetches) has an empty root — re-fetch instead of restoring a
    // permanently empty tree.
    if (snap.schemaRoot.length === 0) {
      void this.loadCapabilities(id);
      void this.db.tree.loadSchemaRoot(id);
    }
    // A tab may have lost its HTTP wait while this connection was in the
    // background (e.g. the daemon blipped and the earlier poll loop gave up) —
    // resume recovering its detached run. No-op for live/settled tabs.
    this.db.runs.reattachPendingTabs();
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
    this.db.activePane = null;
    if (id === this.db.selectedConnId) return;
    this.ensureKeepAlive();
    this.captureSnapshot();
    if (!this.db.openConnIds.includes(id)) {
      this.db.openConnIds = [...this.db.openConnIds, id];
    }
    this.db.selectedConnId = id;
    // Persist the open set + selection now (both the snapshot-restore and
    // fresh-load paths below have already updated them).
    this.db.persist.persistWorkbench();
    if (this.restoreSnapshot(id)) return;
    const epoch = this.db.epochs.of(id);
    await this.loadConnectionFresh(id);
    // Capture an initial snapshot so subsequent switches restore this state —
    // unless the tab was closed mid-load (snapshotting would resurrect it).
    if (this.db.epochs.live(id, epoch) && this.db.selectedConnId === id) this.captureSnapshot();
  }

  /**
   * Close an open connection tab, discarding its snapshot. When the closed tab
   * was active, switch to a neighbor (previous index, else first remaining);
   * when none remain, clear the active session.
   */
  closeConnection(id: Id): void {
    const idx = this.db.openConnIds.indexOf(id);
    if (idx === -1) return;
    const wasActive = this.db.selectedConnId === id;
    // A queued (debounced) tab write for the active conn still keys off it.
    if (wasActive) this.db.persist.flushPersistTabs();
    // Stop this connection's tab runs BEFORE the snapshot is discarded: its
    // tabs are the live set when active, else the parked snapshot's. Cancels
    // server-side + clears `pending` so the reattach loops stop polling.
    const closingTabs = wasActive ? this.db.tabs : this.snapshots.get(id)?.tabs ?? [];
    // (Undo histories are kept: the connection's tabs are persisted and come
    // back with the same uids when it is reopened.)
    for (const t of closingTabs) this.db.runs.abortRunForTab(t);
    this.db.openConnIds = this.db.openConnIds.filter((x) => x !== id);
    // Invalidate every in-flight loader for this connection (schema, caps,
    // health probe, children…) — a close must be terminal, not cosmetic.
    this.db.epochs.bump(id);
    this.warming.get(id)?.ctl.abort();
    this.warming.delete(id);
    // A close during `restoreWorkbench` must survive the restore loop (which
    // would otherwise re-open it) AND reach localStorage past the guard.
    if (this.db.persist.restoring) this.db.persist.restoreTombstones.add(id);
    this.snapshots.delete(id);
    const cs = new Map(this.db.connStatus);
    cs.delete(id);
    this.db.connStatus = cs;
    // Tell the daemon to tear down the connection's live resources (SSH tunnel,
    // pooled conns, in-flight queries). Best-effort fire-and-forget — the tab
    // closes regardless (mirrors closeSshTerminal's DELETE /sessions/{id}).
    void dbCloseConnection(id).catch(() => {
      /* daemon down / endpoint not deployed yet — nothing to release */
    });
    // Drop this connection's persisted per-view entry (it's no longer open).
    if (typeof localStorage !== 'undefined') {
      const vk = this.db.persist.viewKey(id);
      if (vk) localStorage.removeItem(vk);
    }
    if (!wasActive) {
      this.db.persist.persistWorkbench(true);
      return;
    }

    if (this.db.openConnIds.length === 0) {
      // Nothing left open — clear the active working set.
      this.db.selectedConnId = null;
      this.db.capabilities = null;
      this.db.connView = null;
      this.db.testResult = null;
      this.db.schemaRoot = [];
      this.db.childrenCache = new Map();
      this.db.builderTablesCache = new Map();
      this.db.expanded = new Set();
      this.db.nodeFilters = new Map();
      this.db.loadingNodes = new Set();
      this.db.schemaLoading = false;
      this.db.objectDetail = null;
      this.db.objectError = null;
      this.db.selectedObjectPath = null;
      this.db.tree.clearObjectSearch();
      this.db.tabs = [blankTab()];
      this.db.activeTab = 0;
      this.db.history = [];
      this.db.mainTab = 'query';
      // Back to the picker — there's no active connection to show a schema for.
      this.db.sideTab = 'connections';
      this.db.persist.persistWorkbench(true);
      return;
    }
    // Focus the previous tab (or the first if we closed index 0). The active id
    // is gone, so clear it first to let openConnection do the switch.
    const neighbor = this.db.openConnIds[Math.max(0, idx - 1)];
    this.db.selectedConnId = null;
    void this.openConnection(neighbor);
  }

  // ── Background warm-up (restore) + keep-alive ───────────────────────────────

  /** In-flight background warm-ups, one per connection (dedupe + cancel). */
  private warming = new Map<Id, { promise: Promise<void>; ctl: AbortController }>();

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
      !this.db.openConnIds.includes(id) ||
      (parked && parked.schemaRoot.length > 0) ||
      this.db.selectedConnId === id
    ) {
      return Promise.resolve();
    }
    const ctl = new AbortController();
    const epoch = this.db.epochs.of(id);
    const run = async (): Promise<void> => {
      this.mergeConnStatus(id, { phase: 'connecting', error: undefined });
      const started = typeof performance !== 'undefined' ? performance.now() : Date.now();
      try {
        const [caps, root] = await Promise.all([
          api.bg.get<DbCapabilities>(`${this.db.connBase(id)}/capabilities`, ctl.signal),
          api.bg.get<SchemaNode[]>(`${this.db.connBase(id)}/schema`, ctl.signal),
        ]);
        if (!this.db.epochs.live(id, epoch) || ctl.signal.aborted) return;
        const now = typeof performance !== 'undefined' ? performance.now() : Date.now();
        if (this.db.selectedConnId !== id) {
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
        if (!this.db.epochs.live(id, epoch)) return;
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
    if (this.db.openConnIds.includes(id)) this.setConnStatus(id, { phase: 'idle' });
  }

  /**
   * A connection's working set as `loadConnectionFresh` would build it, but as
   * a parked snapshot (no singleton writes) — persisted tabs/view/active DB plus
   * the fetched caps + schema root.
   */
  private freshSnapshot(id: Id, caps: DbCapabilities, root: SchemaNode[]): ConnSnapshot {
    const restored = this.db.persist.restoreTabs(id);
    const view = this.db.persist.restoreView(id);
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
      savedQueries: this.db.savedQueries,
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
    if (!this.keepAlive || this.db.openConnIds.length === 0) return;
    const ids = this.db.openConnIds.filter((id) => this.db.connStatus.get(id)?.phase === 'ready');
    await mapLimit(ids, 2, async (id) => {
      const epoch = this.db.epochs.of(id);
      try {
        const res = await api.bg.post<DbTestResult>(`${this.db.connBase(id)}/test`, {});
        if (!this.db.epochs.live(id, epoch)) return;
        if (!res.ok) {
          this.setConnStatus(id, { phase: 'error', error: res.message || 'Connection lost' });
        } else if (res.latency_ms != null) {
          this.mergeConnStatus(id, { latencyMs: res.latency_ms });
        }
      } catch (e) {
        if (!this.db.epochs.live(id, epoch)) return;
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
    if (this.db.openConnIds.length === 0) return;
    for (const w of this.warming.values()) w.ctl.abort();
    this.warming.clear();
    const sel = this.db.selectedConnId;
    const cs = new Map(this.db.connStatus);
    const others: Id[] = [];
    for (const id of this.db.openConnIds) {
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
    this.db.connStatus = cs;
    if (sel && this.db.openConnIds.includes(sel)) void this.retryConnection(sel);
    if (this.warmRestored === 'background') void this.warmMany(others);
  }

  // ── Non-DB workbench panes (Kafka clusters, SSH/custom terminals) ───────────

  /** Focus an already-open Kafka cluster tab (the `brokers` store owns the tab
   *  list + which cluster is selected; this just points the main area at it). */
  focusKafka(id: Id): void {
    this.db.activePane = { kind: 'kafka', id };
  }

  /** Focus an already-open SSH/custom terminal tab. */
  focusSsh(id: Id): void {
    if (this.db.sshTabs.some((t) => t.connId === id)) this.db.activePane = { kind: 'ssh', id };
  }

  /** Register an ssh/custom profile's freshly-spawned session as a workbench tab
   *  and focus it. Idempotent per connection (re-opening focuses the live tab). */
  addSshTab(tab: SshTab): void {
    if (!this.db.sshTabs.some((t) => t.connId === tab.connId)) {
      this.db.sshTabs = [...this.db.sshTabs, tab];
    }
    this.db.activePane = { kind: 'ssh', id: tab.connId };
  }

  /** Close an SSH/custom terminal tab. The session keeps running and stays
   *  reachable in the Agents list — closing the tab only detaches it from the
   *  workbench. When it was the focused pane, fall back to the DB workbench. */
  closeSshTab(connId: Id): void {
    this.db.sshTabs = this.db.sshTabs.filter((t) => t.connId !== connId);
    if (this.db.activePane?.kind === 'ssh' && this.db.activePane.id === connId) {
      this.db.activePane = null;
    }
  }

  /**
   * Fresh load of a connection's active-session fields: reset the working set,
   * then fetch capabilities + schema root + history. Resets ONLY active-session
   * state — never `openConnIds`/`snapshots`.
   */
  private async loadConnectionFresh(id: Id): Promise<void> {
    const epoch = this.db.epochs.of(id);
    this.db.selectedConnId = id;
    this.db.capabilities = null;
    this.db.connView = null;
    this.db.activeDb = null;
    this.db.schemaRoot = [];
    this.db.childrenCache = new Map();
    this.db.builderTablesCache = new Map();
    this.db.expanded = new Set();
    this.db.nodeFilters = new Map();
    this.db.loadingNodes = new Set();
    this.db.objectDetail = null;
    this.db.objectError = null;
    this.db.selectedObjectPath = null;
    this.db.testResult = null;
    // Object search is per connection — never carry the previous one's hits.
    this.db.tree.clearObjectSearch();
    // Restore this connection's persisted query tabs (in-progress work from a
    // previous session); otherwise start with one blank tab. Never inherit the
    // previously active connection's tabs.
    const restored = this.db.persist.restoreTabs(id);
    this.db.tabs = restored?.tabs ?? [blankTab()];
    this.db.activeTab = restored?.activeTab ?? 0;
    // Restore the active database too, so the first query after reopening a
    // connection is still scoped (otherwise Mongo/SQL error on an unscoped run).
    this.db.activeDb = restored?.activeDb ?? null;
    // Restore this connection's persisted main/side view (which pane it showed);
    // fall back to the query editor + schema tree. loadCapabilities re-validates
    // 'builder' against the engine below.
    const view = this.db.persist.restoreView(id);
    this.db.mainTab = view?.main ?? 'query';
    this.db.sideTab = view?.side ?? 'schema';
    // …and the result view remembered for it (null = engine default).
    this.db.connView = view?.view ?? null;
    // Fresh window of history for this connection.
    this.db.history = [];
    this.db.historyError = null;
    this.db.historyLimit = 100;
    await Promise.all([this.loadCapabilities(id), this.db.tree.loadSchemaRoot(id), this.db.loadHistory(id)]);
    // Closed (or superseded) while the loads were in flight — stop here: no
    // reattach polling, no health probe for a tab that no longer exists.
    if (!this.db.epochs.live(id, epoch)) return;
    // A restored tab may reference a run whose HTTP wait died with the previous
    // page (reload / app restart) while the server kept executing it detached —
    // re-attach and recover the result instead of silently dropping it.
    this.db.runs.reattachPendingTabs();
    // Best-effort health probe for the tab-strip chip (server version + latency).
    // Fire-and-forget so it never blocks the first render; only when connected.
    if (this.db.connStatus.get(id)?.phase === 'ready') void this.probeHealth(id);
  }

  private async loadCapabilities(id: Id): Promise<void> {
    const epoch = this.db.epochs.of(id);
    try {
      const caps = await api.get<DbCapabilities>(`${this.db.connBase(id)}/capabilities`);
      // Stale completion: the tab was closed, or another connection took the
      // foreground — `capabilities` is a singleton view of the SELECTED conn.
      if (!this.db.epochs.live(id, epoch) || this.db.selectedConnId !== id) return;
      this.db.capabilities = caps;
      // A non-SQL engine can't use the visual JOIN builder; keep main tab valid.
      if (this.db.mainTab === 'builder' && !this.db.supportsBuilder) this.db.mainTab = 'query';
    } catch (e) {
      if (!this.db.epochs.live(id, epoch)) return; // no toast for a closed tab
      toastError('Couldn’t load DB capabilities', e);
    }
  }

  /** Set a connection's liveness, reassigning the Map for Svelte-5 reactivity. */
  setConnStatus(id: Id, status: ConnStatus): void {
    this.db.connStatus = new Map(this.db.connStatus).set(id, status);
  }

  /** Merge a patch into a connection's liveness, preserving fields not set (so a
   *  refresh keeps the health chip's server version while it re-probes latency). */
  mergeConnStatus(id: Id, patch: Partial<ConnStatus>): void {
    const prev = this.db.connStatus.get(id) ?? { phase: 'connecting' as ConnPhase };
    this.db.connStatus = new Map(this.db.connStatus).set(id, { ...prev, ...patch });
  }

  /**
   * Best-effort health probe for the tab-strip chip: pings `db/test` for the
   * server version + latency and merges them into the connection's liveness
   * WITHOUT touching `phase` (that's owned by the schema load). Silent on
   * failure — the chip just shows what it has (the phase already reflects errors).
   */
  private async probeHealth(id: Id): Promise<void> {
    const epoch = this.db.epochs.of(id);
    try {
      const started =
        typeof performance !== 'undefined' ? performance.now() : Date.now();
      const res = await api.post<DbTestResult>(`${this.db.connBase(id)}/test`, {});
      // Closed mid-probe: the merge would resurrect the deleted status entry.
      if (!res.ok || !this.db.epochs.live(id, epoch)) return;
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
  async retryConnection(id: Id | null = this.db.selectedConnId): Promise<void> {
    if (!id) return;
    // Drop any stale completion snapshot from the previous (failed) session.
    void this.db.refreshCompletions(id);
    await Promise.all([this.loadCapabilities(id), this.db.tree.loadSchemaRoot(id)]);
  }

  /** Test the selected connection. */
  async testConnection(): Promise<void> {
    const id = this.db.selectedConnId;
    if (!id) return;
    this.db.testing = true;
    this.db.testResult = null;
    try {
      this.db.testResult = await api.post<DbTestResult>(`${this.db.connBase(id)}/test`, {});
      if (this.db.testResult.ok) {
        toasts.success('Connection OK', this.db.testResult.message || `${this.db.testResult.latency_ms ?? '?'} ms`);
      } else {
        toasts.error('Couldn’t connect', this.db.testResult.message);
      }
    } catch (e) {
      toastError('Couldn’t run the test', e);
    } finally {
      this.db.testing = false;
    }
  }
}
