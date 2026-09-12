// Pure cell/value formatting helpers shared by the results components
// (ResultsGrid, GridView, VerticalView, JsonView, CellViewer). No Svelte
// components or runes here — `copyText` is the one impure helper, and it only
// touches the clipboard + the toast store.
import { toasts } from '../../lib/toast.svelte';
import { bsonScalar } from './bson';

/** Non-grid views draw ALT_BATCH records at a time and grow on demand (see
 *  ResultsGrid's `altShown`); the "Show N more" button reads it too. */
export const ALT_BATCH = 25;

// Explicit NULL / empty-string pending markers (context-menu "Set NULL" /
// "Set empty string"). Typed text can't express the difference — an empty
// draft parks as NULL — so the two actions park these sentinels instead;
// sqlLiteral / mongoLiteral render them, the dirty cell displays them.
export const SET_NULL = '\u0000<null>';
export const SET_EMPTY = '\u0000<empty>';

export function isComplex(v: unknown): boolean {
  // A BSON sentinel ({"$oid":…}/{"$date":…}/{"$numberDecimal":…}) is a SCALAR
  // for display purposes — it renders as ObjectId("…")/ISODate("…"), not JSON.
  return v !== null && typeof v === 'object' && bsonScalar(v) === null;
}
/** Stringify a cell for display/search/copy: a BSON sentinel → its typed form
 *  (ObjectId("…")/ISODate("…")), a complex value → compact JSON, else String. */
export function cellStr(v: unknown): string {
  const b = bsonScalar(v);
  if (b !== null) return b;
  return isComplex(v) ? compactJson(v) : String(v);
}
/** Heuristic: does this string look like a SQL statement (DDL/DML/EXPLAIN)? */
export function looksLikeSql(s: string): boolean {
  return /^\s*(create|select|insert|update|alter|with|explain|show|drop|attach|grant)\b/i.test(s);
}
export function compactJson(v: unknown): string {
  try {
    return JSON.stringify(v);
  } catch {
    return String(v);
  }
}
export function prettyJson(v: unknown): string {
  try {
    return JSON.stringify(v, null, 2);
  } catch {
    return String(v);
  }
}
export function cellText(v: unknown): string {
  if (v === null || v === undefined) return '';
  return cellStr(v);
}
/** Hard cap on the text ONE grid cell puts in the DOM. Cells are width-clamped
 *  anyway and the full value is one click away in the cell viewer, so pushing a
 *  ~90KB blob into a 60ch box buys nothing and costs layout time on every
 *  scroll. Copy / edit / export deliberately keep using the UNCLIPPED value. */
export const CELL_MAX = 512;
export function clip(s: string): string {
  return s.length > CELL_MAX ? s.slice(0, CELL_MAX) + '…' : s;
}
export function cellDisplay(v: unknown): string {
  return clip(cellText(v));
}
/** Vertical view: render as a collapsible tree rather than raw text when the
 *  value is structured, or a scalar too long to sit inline. */
export function vvTree(v: unknown): boolean {
  if (isComplex(v)) return true;
  return typeof v === 'string' && v.length > 400;
}

/** Lightweight SQL pretty-printer: newlines before major clauses and one
 * column/arg per line inside the first paren group. String/backtick/comment
 * spans are preserved verbatim. Best-effort and never throws. */
export function formatSql(sql: string): string {
  try {
    const KW = [
      'SELECT', 'FROM', 'LEFT JOIN', 'RIGHT JOIN', 'INNER JOIN', 'OUTER JOIN', 'JOIN',
      'WHERE', 'GROUP BY', 'ORDER BY', 'HAVING', 'LIMIT', 'UNION ALL', 'UNION',
      'SETTINGS', 'PARTITION BY', 'PRIMARY KEY', 'ORDER BY', 'ENGINE', 'AS SELECT',
    ];
    let out = '';
    let depth = 0;
    let i = 0;
    let line = '';
    const flush = () => {
      if (line.trim().length) out += (out ? '\n' : '') + line.replace(/\s+$/, '');
      line = '';
    };
    while (i < sql.length) {
      const ch = sql[i];
      // Preserve quoted / backticked spans verbatim.
      if (ch === "'" || ch === '"' || ch === '`') {
        const q = ch;
        let j = i + 1;
        while (j < sql.length && sql[j] !== q) j++;
        line += sql.slice(i, j + 1);
        i = j + 1;
        continue;
      }
      if (ch === '(') {
        depth++;
        line += ch;
        // Break the column/arg list onto its own indented lines (depth 1 only).
        if (depth === 1) {
          flush();
          line = '  ';
        }
        i++;
        continue;
      }
      if (ch === ')') {
        if (depth === 1) {
          flush();
          line = '';
        }
        depth = Math.max(0, depth - 1);
        line += ch;
        i++;
        continue;
      }
      if (ch === ',' && depth === 1) {
        line += ',';
        flush();
        line = '  ';
        i++;
        continue;
      }
      // Major keyword at depth 0 → start a new line.
      if (depth === 0 && (i === 0 || /\s/.test(sql[i - 1]))) {
        const rest = sql.slice(i).toUpperCase();
        const kw = KW.find((k) => rest.startsWith(k + ' ') || rest === k || rest.startsWith(k + '\n'));
        if (kw) {
          flush();
          line = sql.slice(i, i + kw.length);
          i += kw.length;
          continue;
        }
      }
      line += ch;
      i++;
    }
    flush();
    return out || sql;
  } catch {
    return sql;
  }
}

export function fmtBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

/** Clipboard write with a failure toast; pass `successToast` to also confirm
 *  (the cell viewer's Copy does, the silent "copy value" menu items don't). */
export async function copyText(s: string, successToast?: [title: string, body?: string]): Promise<void> {
  try {
    await navigator.clipboard.writeText(s);
    if (successToast) toasts.success(successToast[0], successToast[1]);
  } catch {
    toasts.error('Copy failed');
  }
}
