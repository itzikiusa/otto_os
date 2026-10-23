// Database Explorer store — workbench persistence: each connection's query
// tabs (debounced draft writes, quota handling, restore), which connections
// are open, and each connection's main / side pane and view mode, so a reload
// or a cut-off session comes back where it was. The DatabaseStore façade owns
// one instance as `persist` and delegates `restoreWorkbench` / `flushTabDrafts`.

import { auth } from '../auth.svelte';
import type { Id } from '../../api/types';
import { ws } from '../workspace.svelte';
import { toasts } from '../../toast.svelte';
import { hydrateEditorHistory } from '../../editor-history';
import { PENDING_SQL_MAX, blankTab, isViewMode, normalizeVars, orderPinnedFirst } from './model';
import type { DbMainTab, DbSideTab, QueryTab, ViewMode } from './model';
import type { DatabaseStore } from '../database.svelte';

/** The DatabaseStore members this part reads and calls (its whole contract with the façade). */
export type WorkbenchPersistHost = Pick<
  DatabaseStore,
  | 'activeDb'
  | 'activeTab'
  | 'conn'
  | 'connStatus'
  | 'connView'
  | 'connections'
  | 'mainTab'
  | 'openConnIds'
  | 'selectedConnId'
  | 'sideTab'
  | 'tabs'
>;

export class DbWorkbenchPersistStore {
  private readonly db: WorkbenchPersistHost;
  constructor(db: WorkbenchPersistHost) {
    this.db = db;
  }

  /** True while `restoreWorkbench` is re-opening persisted tabs, so the
   *  per-open-change persistence calls don't thrash localStorage mid-restore. */
  restoring = false;
  /**
   * Connections closed WHILE `restoreWorkbench` was running. The restore loop
   * (and its final `selected` re-focus) would otherwise re-open a tab the user
   * just closed, and the closing persist was historically swallowed by the
   * `restoring` guard — so the closed tab came back on every reload.
   */
  restoreTombstones = new Set<Id>();

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
  persistTabs(): void {
    if (this.persistTabsTimer !== null) clearTimeout(this.persistTabsTimer);
    this.persistTabsTimer = setTimeout(() => {
      this.persistTabsTimer = null;
      this.persistTabsNow();
    }, 300);
  }
  /** Write any queued (debounced) tab persistence immediately. Must run while
   *  the owning connection is still selected — the write keys off it. */
  flushPersistTabs(): void {
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
  persistTabsNow(connId: Id | null = this.db.selectedConnId): void {
    if (typeof localStorage === 'undefined' || !connId) return;
    const state = connId === this.db.selectedConnId ? this.db : this.db.conn.snapshots.get(connId);
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
  restoreTabs(
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
  viewKey(connId: Id): string {
    return auth.isRoot ? `otto_db_view:${connId}` : `otto_db_view:user:${auth.me?.id ?? 'anonymous'}:${connId}`;
  }
  private legacyViewKey(connId: Id): string | null {
    return auth.isRoot && ws.currentId ? `otto_db_view:${ws.currentId}:${connId}` : null;
  }

  /** Persist the open-connection set + selection. No-op during a restore —
   *  except with `force` (a user CLOSE during the restore must reach storage,
   *  or the restore's final persist writes the closed tab durably back). */
  persistWorkbench(force = false): void {
    if (typeof localStorage === 'undefined' || (this.restoring && !force)) return;
    try {
      localStorage.setItem(
        this.openKey(),
        JSON.stringify({ open: this.db.openConnIds, selected: this.db.selectedConnId }),
      );
    } catch {
      /* storage full / unavailable — non-fatal */
    }
  }

  /** Persist the active connection's main/side view (which pane it's showing)
   *  plus its remembered result view (`connView`; omitted when none). */
  persistView(): void {
    if (typeof localStorage === 'undefined' || this.restoring || !this.db.selectedConnId) return;
    const key = this.viewKey(this.db.selectedConnId);
    try {
      // 'connections' is the global picker, never a per-connection view — store
      // 'schema' instead so a restore lands on the connection's own schema.
      const side = this.db.sideTab === 'connections' ? 'schema' : this.db.sideTab;
      localStorage.setItem(
        key,
        JSON.stringify({
          main: this.db.mainTab,
          side,
          ...(this.db.connView ? { view: this.db.connView } : {}),
        }),
      );
    } catch {
      /* non-fatal */
    }
  }

  /** Read a connection's persisted main/side view (validated to the known tab
   *  ids) and remembered result view (validated, else null). Returns null when
   *  absent/invalid so the caller falls back to defaults. */
  restoreView(
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
    if (this.db.openConnIds.length > 0) return;
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
    const open = (parsed.open ?? []).filter((id) => this.db.connections.some((c) => c.id === id));
    if (open.length === 0) return;
    // Bring persisted undo histories into memory before the editors mount
    // (IndexedDB, a few ms) so ⌘Z reaches past the reload.
    await hydrateEditorHistory();
    if (this.db.openConnIds.length > 0) return; // something opened meanwhile

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
      this.db.openConnIds = open;
      const cs = new Map(this.db.connStatus);
      for (const id of open) if (id !== selected) cs.set(id, { phase: 'idle' });
      this.db.connStatus = cs;
      others = open.filter((id) => id !== selected);
      // The active tab's dial is issued FIRST (interactive/long lane); the
      // background warm-up starts right behind it on the separate bg lane, so a
      // slow active connection doesn't hold the others back.
      const active = this.db.conn.openConnection(selected);
      if (this.db.conn.warmRestored === 'background') void this.db.conn.warmMany(others);
      await active;
    } finally {
      this.restoring = false;
      this.restoreTombstones.clear();
    }
    this.db.conn.ensureKeepAlive();
    // Persist once, now that the full set is open (converges the storage).
    this.persistWorkbench();
    // Nothing survived the restore (or nothing was open) — land the sidebar on
    // the picker so "open a connection" is one click away, not hidden.
    if (this.db.openConnIds.length === 0) this.db.sideTab = 'connections';
  }
}

/** A storage write refused for space (WebKit/Chromium/Firefox spellings). */
function isQuotaError(e: unknown): boolean {
  return (
    e instanceof DOMException &&
    (e.name === 'QuotaExceededError' || e.name === 'NS_ERROR_DOM_QUOTA_REACHED' || e.code === 22)
  );
}
