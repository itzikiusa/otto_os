// Pure column-shape helpers for the results grid (GridView / RowDetail /
// ResultsGrid). No Svelte, no DOM — unit-tested in unit/dbGridFormat.test.ts.

// Self-contained on purpose (node:test imports it directly, without a bundler).

/** Structured (object/array) and not a one-key Extended-JSON scalar sentinel
 *  (`{"$oid": …}`, `{"$date": …}` …) — the classification `isComplex` makes in
 *  results-format, without its toast-store import. */
function isComplex(v: unknown): boolean {
  if (v === null || typeof v !== 'object') return false;
  if (Array.isArray(v)) return true;
  const keys = Object.keys(v as object);
  return !(keys.length === 1 && keys[0].startsWith('$'));
}

/** How a column's values are laid out: numbers right-aligned in tabular
 *  figures, JSON as a link-styled summary, everything else left-aligned. */
export type ColumnKind = 'num' | 'bool' | 'json' | 'time' | 'text';

const NUM_RE =
  /^(?:nullable\()?\s*(?:u?int\d*|(?:tiny|small|medium|big)?int(?:eger)?|serial|bigserial|smallserial|dec(?:imal)?|numeric|number|float\d*|double(?: precision)?|real|money|decimal\d*)\b/i;
const BOOL_RE = /^(?:nullable\()?\s*(?:bool(?:ean)?|bit\(1\)|tinyint\(1\))/i;
const TIME_RE = /^(?:nullable\()?\s*(?:date(?:time\d*)?|timestamp(?:tz)?|time(?:tz)?|interval|year)\b/i;
const JSON_RE = /^(?:nullable\()?\s*(?:jsonb?|object|array|map|tuple)\b/i;

/**
 * Classify a column from its engine type hint, falling back to a sample of
 * its values (Mongo / Redis / computed columns carry no type). A boolean
 * `tinyint(1)` is checked before the integer rule so MySQL flags read as
 * flags; UNSIGNED / ZEROFILL suffixes don't matter (prefix match).
 */
export function columnKind(typeHint: string | null | undefined, sample: unknown[] = []): ColumnKind {
  const t = (typeHint ?? '').trim();
  if (t) {
    if (BOOL_RE.test(t)) return 'bool';
    if (NUM_RE.test(t)) return 'num';
    if (TIME_RE.test(t)) return 'time';
    if (JSON_RE.test(t)) return 'json';
    return 'text';
  }
  let seen = 0;
  let nums = 0;
  let bools = 0;
  let objs = 0;
  for (const v of sample) {
    if (v === null || v === undefined) continue;
    seen += 1;
    if (typeof v === 'number' || typeof v === 'bigint') nums += 1;
    else if (typeof v === 'boolean') bools += 1;
    else if (isComplex(v)) objs += 1;
    if (seen >= 50) break;
  }
  if (seen === 0) return 'text';
  if (nums === seen) return 'num';
  if (bools === seen) return 'bool';
  if (objs === seen) return 'json';
  return 'text';
}

/** Width (in `ch`) of the row-number column. It depends ONLY on how many rows
 *  there are — never on whether the result turned out editable — so the async
 *  editability probe can't widen it after first paint (the "table jumps"
 *  bug). The selection checkbox slot is always reserved. */
export function rowNumberWidthCh(totalRows: number): number {
  const digits = String(Math.max(1, totalRows)).length;
  return Math.max(3, digits) + 1;
}

/** Move `from` to `to` in a column order (display positions). */
export function moveColumn(order: number[], from: number, to: number): number[] {
  if (from === to || from < 0 || to < 0 || from >= order.length || to >= order.length) return order;
  const next = order.slice();
  const [c] = next.splice(from, 1);
  next.splice(to, 0, c);
  return next;
}

/** Case-insensitive "contains" match for the header filter row; `NULL` /
 *  `!NULL` / `=exact` / `>n` `<n` are the few operators worth typing. */
export function cellMatchesFilter(display: string, isNull: boolean, raw: string): boolean {
  const q = raw.trim();
  if (!q) return true;
  const lower = q.toLowerCase();
  if (lower === 'null') return isNull;
  if (lower === '!null' || lower === 'not null') return !isNull;
  if (isNull) return false;
  if (q.startsWith('=')) return display === q.slice(1).trim();
  const cmp = /^(>=|<=|>|<)\s*(-?\d+(?:\.\d+)?)$/.exec(q);
  if (cmp) {
    const n = Number(display);
    if (!Number.isFinite(n)) return false;
    const b = Number(cmp[2]);
    switch (cmp[1]) {
      case '>':
        return n > b;
      case '<':
        return n < b;
      case '>=':
        return n >= b;
      default:
        return n <= b;
    }
  }
  return display.toLowerCase().includes(lower);
}
