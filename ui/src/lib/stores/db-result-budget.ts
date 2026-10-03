// A memory budget for query results held by hidden tabs and parked connections.
//
// Every query tab keeps its last result, and a parked connection snapshot keeps
// its tabs — each result can be up to the daemon's 32 MB JSON budget, which is
// 3–5× that as JS objects. Nothing capped the TOTAL, so ten tabs across five
// connections could pin gigabytes of heap. This module is the bookkeeping: it
// records an estimated size per tab result, keeps a least-recently-VIEWED order,
// and picks which results to release when the total passes the budget. The
// store (database.svelte.ts) does the releasing: the result object is swapped
// for a row-less stub (columns + stats kept) that the grid renders as "Result
// released to save memory — Re-run".
//
// Pure module (no runes) so it unit-tests under `node --test`.

/** Default total budget across every resident result (estimated bytes). */
export const DEFAULT_RESULT_BUDGET = 384 * 1024 * 1024;
/** Results smaller than this are never worth releasing (re-running costs more). */
export const MIN_RELEASE_BYTES = 1024 * 1024;
/** Per-device override (MB), for big-memory machines and the e2e specs. */
export const BUDGET_STORAGE_KEY = 'otto.db.resultBudgetMB';

/** Rows sampled to estimate a result's size. */
const SAMPLE_ROWS = 24;
/** Overhead per cell beyond its payload (array slot, boxed value / header). */
const CELL_OVERHEAD = 16;
/** Bytes per character (JS strings are UTF-16 in the worst case). */
const CHAR_BYTES = 2;

interface ResultLike {
  columns: unknown[];
  rows: unknown[][];
  more_results?: ResultLike[];
}

function cellBytes(v: unknown): number {
  if (v === null || v === undefined) return CELL_OVERHEAD;
  if (typeof v === 'string') return CELL_OVERHEAD + v.length * CHAR_BYTES;
  if (typeof v === 'number' || typeof v === 'boolean') return CELL_OVERHEAD;
  try {
    // Objects (JSON / BSON documents): the serialized length is a fair proxy
    // for the parsed object graph, and only SAMPLE_ROWS rows pay for it.
    return CELL_OVERHEAD * 4 + (JSON.stringify(v)?.length ?? 0) * CHAR_BYTES;
  } catch {
    return CELL_OVERHEAD * 4;
  }
}

/** Estimated JS heap a result holds: sampled rows, extrapolated (+ batch sets). */
export function estimateResultBytes(r: ResultLike | null | undefined): number {
  if (!r) return 0;
  let total = 0;
  const rows = r.rows ?? [];
  const n = rows.length;
  if (n > 0) {
    const step = Math.max(1, Math.floor(n / SAMPLE_ROWS));
    let sampled = 0;
    let bytes = 0;
    for (let i = 0; i < n && sampled < SAMPLE_ROWS; i += step, sampled++) {
      const row = rows[i] ?? [];
      bytes += 32; // the row array itself
      for (const v of row) bytes += cellBytes(v);
    }
    total += Math.round((bytes / sampled) * n);
  }
  for (const m of r.more_results ?? []) total += estimateResultBytes(m);
  return total;
}

interface Entry {
  bytes: number;
  /** Monotonic "last viewed" tick (higher = more recent). */
  seen: number;
}

/** LRU bookkeeping of resident results, keyed by the query tab's stable uid. */
export class ResultBudget {
  private entries = new Map<string, Entry>();
  private tick = 0;
  budget: number;
  constructor(budget: number = DEFAULT_RESULT_BUDGET) {
    this.budget = budget;
  }

  /** Record (or replace) a tab's resident result; counts as a view. */
  note(uid: string, bytes: number): void {
    this.entries.set(uid, { bytes, seen: ++this.tick });
  }
  /** Mark a tab as just viewed (tab / connection switch). */
  touch(uid: string): void {
    const e = this.entries.get(uid);
    if (e) e.seen = ++this.tick;
  }
  /** Forget a tab (closed, result cleared or released). */
  forget(uid: string): void {
    this.entries.delete(uid);
  }
  has(uid: string): boolean {
    return this.entries.has(uid);
  }
  get total(): number {
    let t = 0;
    for (const e of this.entries.values()) t += e.bytes;
    return t;
  }
  get size(): number {
    return this.entries.size;
  }

  /**
   * The uids to release, least-recently-viewed first, until the total fits the
   * budget. `pinned` uids (the tab on screen, running tabs, tabs with
   * un-applied edits) are never picked; neither are results under
   * MIN_RELEASE_BYTES. Does NOT forget them — the caller releases, then forgets.
   */
  pick(pinned: ReadonlySet<string>): string[] {
    let total = this.total;
    if (total <= this.budget) return [];
    const order = [...this.entries.entries()]
      .filter(([uid, e]) => !pinned.has(uid) && e.bytes >= MIN_RELEASE_BYTES)
      .sort((a, b) => a[1].seen - b[1].seen);
    const out: string[] = [];
    for (const [uid, e] of order) {
      if (total <= this.budget) break;
      out.push(uid);
      total -= e.bytes;
    }
    return out;
  }
}

/** The configured budget: the per-device override (MB) when set, else the default. */
export function configuredBudget(): number {
  try {
    const raw = globalThis.localStorage?.getItem(BUDGET_STORAGE_KEY);
    const mb = raw ? Number(raw) : NaN;
    if (Number.isFinite(mb) && mb > 0) return Math.round(mb * 1024 * 1024);
  } catch {
    /* storage unavailable — the default applies */
  }
  return DEFAULT_RESULT_BUDGET;
}

// Released stubs, recognised by identity (never serialised: tabs persist
// without results, so the marker can't leak into storage or the wire types).
const released = new WeakSet<object>();

/** A row-less stand-in for a released result: columns, stats and flags kept. */
export function releasedStub<T extends ResultLike>(r: T): T {
  const rowCount = r.rows?.length ?? 0;
  const stub = { ...r, rows: [], more_results: undefined, released_rows: rowCount } as T;
  released.add(stub);
  return stub;
}

/** True for a stub made by {@link releasedStub}. */
export function isReleased(r: unknown): boolean {
  return typeof r === 'object' && r !== null && released.has(r);
}

/** Row count the released result had (for the "N rows released" copy). */
export function releasedRows(r: unknown): number {
  return isReleased(r) ? Number((r as { released_rows?: number }).released_rows ?? 0) : 0;
}

/** The app-wide budget (one per window; the DB store and grid share it). */
export const resultBudget = new ResultBudget(configuredBudget());
