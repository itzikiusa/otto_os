// Per-result caches for the results grid's derived views (perf/04 R2, R3).
//
// Everything here is keyed by the `liveRows` ARRAY IDENTITY in a WeakMap, so it
// lives exactly as long as the rows it was built from: a tab whose result the
// memory budget releases (db-result-budget.ts) drops its rows array, and these
// caches go with it.
//
//   • the search text per row and the display text per filtered column — built
//     once per result, in idle slices ahead of the first key (`prebuild`), and
//     finished synchronously only for whatever rows the slices haven't reached;
//   • the last filtered view and the last sorted view per result — switching
//     back to a sorted / filtered 100k-row tab used to re-run the full filter
//     and a 100k-string collator sort (150–400 ms in JSC) on every switch.
//
// Pure (no Svelte, no DOM): `cell` — the display stringifier — is passed in.

/** Cap per row / cell so one blob can't dominate memory; text past it is not scanned. */
export const SCAN_MAX = 65536;

export interface ViewRow {
  row: unknown[];
  idx: number;
}
type CellText = (v: unknown) => string;

interface Entry {
  /** Lower-cased search text per row; rows [0, scanDone) are built. */
  scan: string[] | null;
  scanDone: number;
  /** Display text per column (null = SQL NULL); rows [0, done) built. */
  cols: Map<number, { text: (string | null)[]; done: number }>;
  filter: { key: string; view: ViewRow[] } | null;
  sort: { base: ViewRow[]; key: string; view: ViewRow[] } | null;
}

const cache = new WeakMap<unknown[][], Entry>();

function entry(rows: unknown[][]): Entry {
  let e = cache.get(rows);
  if (!e) {
    e = { scan: null, scanDone: 0, cols: new Map(), filter: null, sort: null };
    cache.set(rows, e);
  }
  return e;
}

/** One row's search text: every non-NULL cell's display text, NUL-separated,
 *  clipped to SCAN_MAX and lower-cased. */
export function rowScanText(row: unknown[], cell: CellText): string {
  let s = '';
  for (const v of row) {
    if (v === null || v === undefined) continue;
    s += cell(v) + '\u0000';
    if (s.length >= SCAN_MAX) break;
  }
  return s.slice(0, SCAN_MAX).toLowerCase();
}

function clipScan(s: string): string {
  return s.length > SCAN_MAX ? s.slice(0, SCAN_MAX) : s;
}

/** Build search text up to row `to` (exclusive); returns rows built so far. */
function buildScan(rows: unknown[][], e: Entry, to: number, cell: CellText): number {
  if (!e.scan) e.scan = new Array<string>(rows.length);
  const scan = e.scan;
  let i = e.scanDone;
  for (; i < to; i++) scan[i] = rowScanText(rows[i], cell);
  e.scanDone = i;
  return i;
}

function colSlot(rows: unknown[][], e: Entry, ci: number): { text: (string | null)[]; done: number } {
  let c = e.cols.get(ci);
  if (!c) {
    c = { text: new Array<string | null>(rows.length), done: 0 };
    e.cols.set(ci, c);
  }
  return c;
}

function buildCol(rows: unknown[][], c: { text: (string | null)[]; done: number }, ci: number, to: number, cell: CellText): void {
  const text = c.text;
  let i = c.done;
  for (; i < to; i++) {
    const v = rows[i][ci];
    text[i] = v === null || v === undefined ? null : clipScan(cell(v));
  }
  c.done = i;
}

/** The complete search text for `rows` (finishes what idle slices left). */
export function scanFor(rows: unknown[][], cell: CellText): string[] {
  const e = entry(rows);
  if (e.scanDone < rows.length || !e.scan) buildScan(rows, e, rows.length, cell);
  return e.scan as string[];
}

/** The complete display text of column `ci` for `rows`. */
export function colScanFor(rows: unknown[][], ci: number, cell: CellText): (string | null)[] {
  const c = colSlot(rows, entry(rows), ci);
  if (c.done < rows.length) buildCol(rows, c, ci, rows.length, cell);
  return c.text;
}

/** How many rows of search text (or of column `ci`) are built — for tests / probes. */
export function scanProgress(rows: unknown[][], ci?: number): number {
  const e = cache.get(rows);
  if (!e) return 0;
  return ci === undefined ? e.scanDone : (e.cols.get(ci)?.done ?? 0);
}

export interface PrebuildOptions {
  /** Main-thread budget per slice (ms). */
  sliceMs?: number;
  /** Schedules the next slice; returns a canceller. Default: idle callback,
   *  else a 16 ms timer (WebKit has no requestIdleCallback). */
  schedule?: (fn: () => void) => () => void;
  now?: () => number;
}

function defaultSchedule(fn: () => void): () => void {
  const w = globalThis as typeof globalThis & {
    requestIdleCallback?: (cb: () => void, o?: { timeout: number }) => number;
    cancelIdleCallback?: (h: number) => void;
  };
  if (w.requestIdleCallback) {
    const h = w.requestIdleCallback(fn, { timeout: 500 });
    return () => w.cancelIdleCallback?.(h);
  }
  const t = setTimeout(fn, 16);
  return () => clearTimeout(t);
}

/** Build the search text (`target: 'search'`) or column `target`'s text for
 *  `rows` in slices of at most `sliceMs` of main-thread time, so the first
 *  filtered key finds it (mostly) done. Returns a canceller. Idempotent: a
 *  finished target costs nothing; a second call resumes where the first got to. */
export function prebuild(
  rows: unknown[][],
  target: 'search' | number,
  cell: CellText,
  opts: PrebuildOptions = {},
): () => void {
  const sliceMs = opts.sliceMs ?? 8;
  const schedule = opts.schedule ?? defaultSchedule;
  const now = opts.now ?? (() => performance.now());
  const CHUNK = 256;
  let cancel: (() => void) | null = null;
  let stopped = false;
  const step = (): void => {
    cancel = null;
    if (stopped) return;
    const e = entry(rows);
    const t0 = now();
    if (target === 'search') {
      while (e.scanDone < rows.length && now() - t0 < sliceMs) {
        buildScan(rows, e, Math.min(rows.length, e.scanDone + CHUNK), cell);
      }
      if (e.scanDone >= rows.length) return;
    } else {
      const c = colSlot(rows, e, target);
      while (c.done < rows.length && now() - t0 < sliceMs) {
        buildCol(rows, c, target, Math.min(rows.length, c.done + CHUNK), cell);
      }
      if (c.done >= rows.length) return;
    }
    cancel = schedule(step);
  };
  cancel = schedule(step);
  return () => {
    stopped = true;
    cancel?.();
    cancel = null;
  };
}

/** Free the search text of `rows` (the search was cleared). */
export function dropScan(rows: unknown[][]): void {
  const e = cache.get(rows);
  if (!e) return;
  e.scan = null;
  e.scanDone = 0;
}

/** Free every column's text of `rows` except `keep` (filters cleared). */
export function dropColScans(rows: unknown[][], keep: ReadonlySet<number>): void {
  const e = cache.get(rows);
  if (!e) return;
  for (const ci of [...e.cols.keys()]) if (!keep.has(ci)) e.cols.delete(ci);
}

/** The filtered view of `rows` for `key` (search + column filters + chips):
 *  the last one is kept per result, so returning to it is O(1). */
export function memoFilter(rows: unknown[][], key: string, compute: () => ViewRow[]): ViewRow[] {
  const e = entry(rows);
  if (e.filter && e.filter.key === key) return e.filter.view;
  const view = compute();
  e.filter = { key, view };
  return view;
}

/** The sorted view of `base` (a view over `rows`) for `key` (column + dir):
 *  the last one is kept per result. */
export function memoSort(rows: unknown[][], base: ViewRow[], key: string, compute: () => ViewRow[]): ViewRow[] {
  const e = entry(rows);
  if (e.sort && e.sort.base === base && e.sort.key === key) return e.sort.view;
  const view = compute();
  e.sort = { base, key, view };
  return view;
}
