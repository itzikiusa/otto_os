// Database Explorer store — the module-level model: connection kinds, query-tab
// shape and factory, result view-mode resolution, per-connection snapshots and
// the small constants the DatabaseStore façade (../database.svelte.ts) runs on.
// Pure (no runes): the façade owns all reactive state and re-exports this file.

import type { IconName } from '../../components/Icon.svelte';
import { ApiError } from '../../api/client';
import type {
  DbCapabilities,
  DbEngine,
  DbHistoryEntry,
  DbSavedQuery,
  DbTestResult,
  Id,
  ObjectDetail,
  ObjectHit,
  QueryResult,
  SchemaNode,
} from '../../api/types';
import type { FilterCond } from '../../database/filter-chips';
import { defaultVarSpec, type VarSpec } from '../../database/sql-util';
import { forgetEditorState } from '../../editor-history';
import { dropGridState } from '../../database/grid-tab-state';
import { resultBudget } from '../db-result-budget';

/** Connection kinds the explorer can browse (the DB engines). */
export const DB_KINDS = ['mysql', 'postgres', 'redis', 'mongodb', 'clickhouse'] as const;
export type DbKind = (typeof DB_KINDS)[number];

export function isDbKind(k: string): k is DbKind {
  return (DB_KINDS as readonly string[]).includes(k);
}

/**
 * The production / read-only write-gate rejection. The server tags the message
 * with `write_blocked: ` (a stable marker, see otto-dbviewer) so the UI can
 * recognise it and offer a typed confirmation rather than string-matching prose.
 */
export function isWriteBlocked(e: unknown): boolean {
  return e instanceof ApiError && e.message.startsWith('write_blocked:');
}

/**
 * The refusal of a `read_only` run (`QueryRequest.read_only`, what an agent's
 * UI-control run always sends): the statement classifies as a write/DDL. Tagged
 * `read_only: ` by otto-dbviewer — distinct from `write_blocked:` (a guarded
 * connection) and `mcp_read_only:` (no human in that loop).
 */
export function isReadOnlyRefusal(e: unknown): boolean {
  return e instanceof ApiError && e.message.startsWith('read_only:');
}

/** Persisted default row cap applied when a statement has no explicit LIMIT. */
export const ROW_LIMIT_KEY = 'otto_db_row_limit';
export const DEFAULT_ROW_LIMIT = 1000;
/** Sentinel "no cap" value sent as max_rows for the "All" option. */
export const ROW_LIMIT_ALL = 1_000_000;

export function loadRowLimit(): number {
  if (typeof localStorage === 'undefined') return DEFAULT_ROW_LIMIT;
  const v = Number(localStorage.getItem(ROW_LIMIT_KEY));
  return Number.isFinite(v) && v > 0 ? v : DEFAULT_ROW_LIMIT;
}

/** Restored tabs warmed at once (matches the bg lane's 3 app-wide sockets). */
export const WARM_CONCURRENCY = 3;
/** Keep-alive period: under the daemon's 5-minute pool idle timeout. */
export const KEEP_ALIVE_MS = 4 * 60_000;


/**
 * A fresh per-run id sent with a query so the server can register it and a later
 * `db/cancel` (same id) can issue engine-native cancellation. Uses
 * `crypto.randomUUID` where available, with a non-cryptographic fallback (the id
 * only needs to be unique among this client's in-flight queries).
 */
export function newQueryId(): string {
  if (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function') {
    return crypto.randomUUID();
  }
  return `q-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;
}


/** Glyph (Icon name) for a connection engine. */
export function engineGlyph(kind: string): IconName {
  switch (kind) {
    case 'redis':
      return 'key';
    case 'mongodb':
      return 'box';
    case 'clickhouse':
      return 'grid';
    case 'postgres':
      return 'layers';
    case 'ssh':
      return 'terminal';
    case 'custom':
      return 'gear';
    case 'kafka': // broker clusters in the unified tree
      return 'split';
    default:
      return 'db';
  }
}


// ── Result view mode ──────────────────────────────────────────────────────────
// How a tab's result renders: a columnar grid, one record per block (Postgres
// `\x` / ClickHouse FORMAT Vertical) or one JSON object per row. Where the mode
// comes from is layered — see `effectiveViewMode` — so an explicit pick never
// fights the automatic defaults, and nothing here resets on a tab switch.
export type ViewMode = 'grid' | 'vertical' | 'json';
export const VIEW_MODES: readonly ViewMode[] = ['grid', 'vertical', 'json'];
export function isViewMode(v: unknown): v is ViewMode {
  return typeof v === 'string' && (VIEW_MODES as readonly string[]).includes(v);
}
/** The view a result opens in when nothing else decided. Mongo documents are
 *  nested and ragged, so they read best one record at a time; the SQL engines
 *  (and Redis) keep the columnar grid. */
export const ENGINE_DEFAULT_VIEW: Record<DbEngine, ViewMode> = {
  mongodb: 'vertical',
  mysql: 'grid',
  postgres: 'grid',
  clickhouse: 'grid',
  redis: 'grid',
};
/** Everything `effectiveViewMode` / `viewModeReason` weigh. `autoVerticalCols`
 *  is the user's "switch to Vertical past N columns" setting FOR THIS ENGINE
 *  (`ui.dbAutoVerticalFor` — MongoDB 10, SQL engines 0 = never, by default). */
export interface ViewModeInputs {
  tabPick: ViewMode | null;
  connPick: ViewMode | null;
  columnCount: number;
  autoVerticalCols: number;
  engine: DbEngine | null;
}
export function wideResult(a: ViewModeInputs): boolean {
  return a.autoVerticalCols > 0 && a.columnCount > a.autoVerticalCols;
}
/**
 * The view a result renders in. Precedence, first match wins:
 *   1. an explicit pick on THIS tab (the segmented control / ⇧⌘V);
 *   2. the wide-result threshold — more columns than the user's limit → Vertical;
 *   3. the pick remembered for the connection (the last explicit pick on it);
 *   4. the engine default (`ENGINE_DEFAULT_VIEW`), Grid when the engine is unknown.
 * So the threshold never overrides a choice made on the tab, and a choice made
 * on another tab of the same connection only fills in when nothing else did.
 */
export function effectiveViewMode(a: ViewModeInputs): ViewMode {
  if (a.tabPick) return a.tabPick;
  if (wideResult(a)) return 'vertical';
  if (a.connPick) return a.connPick;
  return (a.engine ? ENGINE_DEFAULT_VIEW[a.engine] : undefined) ?? 'grid';
}
/** Why `effectiveViewMode` chose what it chose — the view switch's tooltip. */
export function viewModeReason(a: ViewModeInputs): string {
  if (a.tabPick) return 'your pick for this tab';
  if (wideResult(a)) return `auto: ${a.columnCount} columns > ${a.autoVerticalCols}`;
  if (a.connPick) return 'remembered for this connection';
  return 'engine default';
}

// A prototype Svelte's `proxy()` does not recognise as plain: an object with it
// is stored in `$state` AS IS (proxy() only wraps Object/Array prototypes).
export const RAW_RESULT_PROTO: object = Object.create(Object.prototype);

/**
 * Keep a query result OUT of the deep `tabs` proxy tree. Results are immutable
 * snapshots (edits re-query, nothing patches rows in place), but assigned into
 * a deep `$state` they became lazy proxies: a 100k-row result then cost 43–52 ms
 * to land, 0.7–0.9 s for the first grid-search key (82 ms each after) and
 * 38–54 ms to stringify complex cells — vs ~1 ms raw. Swapping the prototype
 * (still an ordinary object for JSON, spread and property access) makes Svelte
 * store it by reference; replacing `tab.result` wholesale stays reactive.
 */
export function rawResult<T extends object>(r: T): T {
  if (Object.getPrototypeOf(r) === Object.prototype) Object.setPrototypeOf(r, RAW_RESULT_PROTO);
  return r;
}

/** An open query tab: an editable statement + its last result + quick filters. */
export interface QueryTab {
  id: number;
  name: string;
  statement: string;
  result: QueryResult | null;
  /**
   * The statement that PRODUCED `result` — not what is currently in the editor.
   * The grid's editability probe, "Copy as INSERT" target, filter-chip splices
   * and "Export all rows…" all describe the rows ON SCREEN, so they must key off
   * this and not the live buffer: keying them off the editor text made the whole
   * result toolbar re-derive (and visibly flicker) on EVERY keystroke, and would
   * have exported / spliced a query the user was still typing. Transient — a
   * reload has no result to describe.
   */
  ran_statement: string | null;
  /**
   * The scope node the run that produced `result` was issued with (an explicit
   * node, else the active DB at run time). The footer pager re-runs the RAN
   * statement, so it must reuse the RAN scope too — the user may have switched
   * the active database since. Transient, like `ran_statement`.
   */
  ran_node: string | null;
  running: boolean;
  error: string | null;
  /**
   * The statement whose run produced `error` — what ErrorPanel's line/caret
   * excerpt and "Ask AI to fix" describe. Runs are transient (the statement at
   * the cursor, variable-substituted), so an engine's "at line N" is relative to
   * THIS text, not to the editor buffer. Transient, like `ran_statement`.
   */
  err_statement: string | null;
  /** Quick-filter chips that own the statement's WHERE clause. */
  filters: FilterCond[];
  /**
   * Optional per-tab statement timeout in milliseconds. When set, passed to the
   * server which forwards it to the driver (MySQL: MAX_EXECUTION_TIME hint).
   * 0 or null = no limit.
   */
  timeout_ms: number | null;
  /**
   * When true, the server redacts PII/secrets from result cells before returning
   * them. The response `masked` flag confirms it was applied. Persisted per-tab
   * so the toggle survives statement changes.
   */
  mask: boolean;
  /**
   * Query-level variables (`:name` / `{name}`) the editor substitutes into the
   * statement before running. Each holds a value + type (`string` default →
   * quoted, `number` → raw, `raw` → verbatim) + an escape flag. Per-tab (not
   * global); persisted so values survive reloads.
   */
  vars: Record<string, VarSpec>;
  /**
   * The saved-query id this tab was opened from, if any. When set, "Save" on the
   * tab updates that saved query in place (PATCH); "Save as new" always creates a
   * fresh one. Cleared when the tab's origin no longer applies.
   */
  savedQueryId?: Id;
  /**
   * Zero-based row offset for the footer pager. Only meaningful when the last
   * result was auto-limited (a single paginatable SELECT / Mongo find). Reset to
   * 0 whenever the statement changes; advanced by `runPage`. Transient (not
   * persisted).
   */
  offset: number;
  /**
   * The in-flight (or orphaned) run this tab is attached to. Set for the whole
   * life of a run; when the HTTP wait is lost without a user Stop (page
   * navigation / reload / network blip), the server keeps executing detached
   * and this is what `reattach` polls (`db/query-status`) to recover the
   * result. Persisted so a full reload can still re-attach; cleared when the
   * result lands, the user stops the query, or the server no longer knows the id.
   */
  pending?: PendingRun | null;
  /**
   * The user's explicit result view for this tab (segmented control / ⇧⌘V);
   * `null`/absent = automatic (threshold → connection memory → engine default,
   * see `effectiveViewMode`). Persisted with the tab so it survives a reload and
   * is NOT reset by switching tabs.
   */
  viewMode?: ViewMode | null;
  /**
   * A pinned tab survives "Close others" / "Close all" and can't be closed by
   * × / ⌥⌘W until unpinned. Pinned tabs sit first in the strip. Persisted.
   */
  pinned: boolean;
  /**
   * Set when an agent session opened this tab over UI control
   * (`otto.ui_db_new_tab` / `ui_db_run_query`, lib/uiCommands/database.ts): the
   * strip shows an attributed chip so the person always sees whose tab it is.
   * Persisted with the tab; absent = a tab the person opened.
   */
  agent?: QueryTabAgent | null;
  /**
   * Stable identity of this tab across reloads (`id` is renumbered on every
   * restore). Keys the editor's per-tab undo history (lib/editor-history.ts)
   * so ⌘Z survives a remount and a reload. Persisted with the tab.
   */
  uid: string;
}

/**
 * A run a tab is attached to (see {@link QueryTab.pending}). `sql` / `node` are
 * the statement and scope it was issued with, so a re-attached result (or
 * error) describes — and pages — what actually ran, never the editor buffer.
 * Absent on markers persisted by older builds or when the statement was too
 * large to persist; the re-attached result then has no pager.
 */
export interface PendingRun {
  queryId: string;
  connId: Id;
  sql?: string;
  node?: string | null;
  /**
   * When the run started (epoch ms). Drives the running timer, so it keeps
   * counting across tab / connection / view switches and reloads instead of
   * restarting at 0s. Absent on older markers until the daemon's
   * `query-status` reports `elapsed_ms` (reattach backfills it).
   */
  startedAt?: number;
}

/** Hard cap on each side of a completion request's cursor context. */
export const COMPLETION_CTX_MAX = 16_384;
/** Quiet period after a failed completion request (per connection). */
export const COMPLETION_BACKOFF_MS = 5_000;

/** Longest statement persisted inside a pending-run marker (localStorage). */
export const PENDING_SQL_MAX = 32_768;

/** Who drove a query tab (see {@link QueryTab.agent}). */
export interface QueryTabAgent {
  session_id: string;
  /** Short attribution shown in the chip ("Claude · fix-orders"). */
  label: string;
}

/**
 * What a {@link DatabaseStore.runQuery} call ended as — filled in by the store
 * when the caller passes `opts.outcome`. `runQuery` itself returns `null` for
 * every non-result ending (Stop, a declined write, an engine error, a lost wait),
 * which is right for the buttons but not for an agent command that has to report
 * WHICH one happened.
 */
/** How the typed guarded-write gate ({@link DatabaseStore.confirmGuardedWrite})
 *  was answered: a mistyped name (`'mismatch'`) is already explained to the
 *  person, a plain cancel (`'declined'`) is not. */
export type GuardedWriteAnswer = 'confirmed' | 'declined' | 'mismatch';

/** Why {@link DatabaseStore.runManagedStatement} ran nothing: the typed confirm
 *  was dismissed (`'declined'`), the name was mistyped (`'mismatch'` — already
 *  toasted by the gate, so callers add no second toast), or access / the
 *  connection changed while it waited (`'stale'`). */
export interface ManagedWriteCanceled {
  canceled: 'declined' | 'mismatch' | 'stale';
}

export interface RunOutcome {
  status?: 'ok' | 'cancelled' | 'aborted' | 'failed' | 'detached' | 'invalid';
  error?: string;
}

/** Normalize a persisted vars blob — legacy `Record<string,string>` (bare value)
 *  or the new `Record<string,VarSpec>` — into VarSpecs. */
export function normalizeVars(raw: unknown): Record<string, VarSpec> {
  const out: Record<string, VarSpec> = {};
  if (!raw || typeof raw !== 'object') return out;
  for (const [k, v] of Object.entries(raw as Record<string, unknown>)) {
    if (typeof v === 'string') {
      out[k] = defaultVarSpec(v);
    } else if (v && typeof v === 'object') {
      const o = v as Record<string, unknown>;
      out[k] = {
        value: typeof o.value === 'string' ? o.value : '',
        type: o.type === 'number' || o.type === 'raw' ? o.type : 'string',
        escape: typeof o.escape === 'boolean' ? o.escape : true,
      };
    }
  }
  return out;
}

/** Key prefix of a query tab's undo history in lib/editor-history.ts. */
export const TAB_HISTORY_PREFIX = 'dbtab:';
/** Drop a closed tab's undo history and parked grid state. */
export function forgetTabHistory(t: QueryTab): void {
  forgetEditorState(TAB_HISTORY_PREFIX + t.uid);
  dropGridState(t.uid);
  resultBudget.forget(t.uid);
}

let nextTabId = 1;
/** A fresh stable tab uid (see {@link QueryTab.uid}). */
export function newTabUid(): string {
  try {
    if (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function') {
      return crypto.randomUUID();
    }
  } catch {
    /* insecure context — fall through */
  }
  // getRandomValues works in insecure contexts too (randomUUID doesn't).
  const bytes = crypto.getRandomValues(new Uint8Array(8));
  return `t${Date.now().toString(36)}${Array.from(bytes, (b) => b.toString(16).padStart(2, '0')).join('')}`;
}
export function blankTab(statement = ''): QueryTab {
  return {
    id: nextTabId++,
    uid: newTabUid(),
    name: 'Query',
    statement,
    result: null,
    ran_statement: null,
    ran_node: null,
    running: false,
    error: null,
    err_statement: null,
    filters: [],
    timeout_ms: null,
    mask: false,
    vars: {},
    offset: 0,
    pending: null,
    viewMode: null,
    pinned: false,
    agent: null,
  };
}

/**
 * Pinned tabs first (stable within each group), the way the strip shows them —
 * shared by pin/unpin and the reload restore so the order never changes across a
 * reload. Returns the reordered list plus where `activeIdx` moved to.
 */
export function orderPinnedFirst(
  tabs: QueryTab[],
  activeIdx: number,
): { tabs: QueryTab[]; activeTab: number } {
  const active = tabs[activeIdx];
  const ordered = [...tabs.filter((t) => t.pinned), ...tabs.filter((t) => !t.pinned)];
  const activeTab = active ? Math.max(0, ordered.indexOf(active)) : 0;
  return { tabs: ordered, activeTab };
}

/** Main-pane tabs of the DB page. */
export type DbMainTab = 'query' | 'builder' | 'structure' | 'diagram' | 'dashboards';
/** Sidebar tab switch. `connections` is the global connection picker; the
 *  others are per-connection views of the active connection. */
export type DbSideTab = 'connections' | 'schema' | 'saved' | 'history';

/** An ssh/custom profile opened as a workbench tab, bound to a live PTY session
 *  so the tab can render a terminal in place (see {@link DatabaseStore.sshTabs}). */
export interface SshTab {
  connId: Id;
  sessionId: Id;
  name: string;
  /** Profile kind ('ssh' | 'custom' | a DB kind opened as a CLI client) — drives the tab glyph. */
  kind: string;
}

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
export interface ConnSnapshot {
  capabilities: DbCapabilities | null;
  testResult: DbTestResult | null;
  activeDb: string | null;
  schemaRoot: SchemaNode[];
  childrenCache: Map<string, SchemaNode[]>;
  expanded: Set<string>;
  loadingNodes: Set<string>;
  schemaLoading: boolean;
  selectedObjectPath: string | null;
  objectDetail: ObjectDetail | null;
  objectError: string | null;
  // Server-side object search is per connection — without snapshotting it,
  // conn A's hits (and query) would render over conn B's tree after a switch.
  objectSearchQuery: string;
  objectSearchHits: ObjectHit[] | null;
  objectSearchTruncated: boolean;
  objectSearchScanned: number;
  builderTablesCache: Map<string, { label: string; path: string; kind: string }[]>;
  tabs: QueryTab[];
  activeTab: number;
  savedQueries: DbSavedQuery[];
  history: DbHistoryEntry[];
  mainTab: DbMainTab;
  sideTab: DbSideTab;
  connView: ViewMode | null;
}

/**
 * `idle` = restored as a tab but not connected yet (background warm-up pending,
 * or "connect on click only"); `stale` is folded into `idle` after a daemon
 * restart (the old pool/tunnel is gone; it re-warms).
 */
export type ConnPhase = 'idle' | 'connecting' | 'ready' | 'error';
export interface ConnStatus {
  phase: ConnPhase;
  /** Failure reason (set only when phase === 'error'). */
  error?: string;
  /** Server version string (from the `db/test` probe), shown in the health chip. */
  serverVersion?: string;
  /** Connect/probe latency in ms (schema-load timing or the `db/test` ping),
   *  shown in the health chip tooltip. */
  latencyMs?: number;
}

