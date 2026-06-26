// Turn a clicked result cell (column + value) into a filtered query by splicing a
// condition into the CURRENT query text — powering the "Query by value" (replace
// the WHERE / find-filter) and "Add to query" (AND it on) cell actions.
//
// SQL (MySQL + ClickHouse): delegates to the SAME top-level parser the
// quick-filter chips use (`splitStatement` / `rewriteWhere` / `condToSql` /
// `toFilterVal` from the database store) so there is ONE WHERE-splicer, one
// keyword list, and one set of quoting rules — no second implementation to drift.
// The only genuinely new SQL logic here is ANDing a condition onto an existing
// freeform WHERE (with OR-precedence parenthesization).
//
// Mongo: merge/replace the first argument (the filter object) of
// `db.<coll>.find(<filter>[, …])` — there is no existing Mongo splicer to reuse.
//
// Nothing here runs a query; the caller writes the result into the editor +
// clipboard and lets the user press Run.

import { splitStatement, rewriteWhere, condToSql, toFilterVal } from '../../lib/stores/database.svelte';

export type FilterMode = 'set' | 'and';

// ── SQL ──────────────────────────────────────────────────────────────────────

/** `\`col\` = <literal>` (or `\`col\` IS NULL`) — reuses the chip quoting so a
 *  string is `'escaped'`, a number is bare, NULL becomes `IS NULL`, identically
 *  to a quick-filter chip. */
function sqlEquals(column: string, value: unknown): string {
  return condToSql({ kind: 'col', column, op: 'in', values: [toFilterVal(value)] });
}

/** True when `body` has a top-level (depth-0, outside strings/comments) ` OR ` —
 *  used to parenthesize an existing WHERE before ANDing a new term so operator
 *  precedence is preserved. */
function hasTopLevelOr(body: string): boolean {
  const n = body.length;
  let depth = 0;
  let i = 0;
  while (i < n) {
    const c = body[i];
    if (c === "'" || c === '"' || c === '`') {
      const q = c;
      i++;
      while (i < n) {
        if (body[i] === q) {
          if (body[i + 1] === q) {
            i += 2;
            continue;
          }
          i++;
          break;
        }
        i++;
      }
      continue;
    }
    if (c === '-' && body[i + 1] === '-') {
      while (i < n && body[i] !== '\n') i++;
      continue;
    }
    if (c === '/' && body[i + 1] === '*') {
      i += 2;
      while (i < n && !(body[i] === '*' && body[i + 1] === '/')) i++;
      i += 2;
      continue;
    }
    if (c === '(') {
      depth++;
      i++;
      continue;
    }
    if (c === ')') {
      depth = Math.max(0, depth - 1);
      i++;
      continue;
    }
    if (
      depth === 0 &&
      (c === 'o' || c === 'O') &&
      (body[i + 1] === 'r' || body[i + 1] === 'R') &&
      // Word boundary on BOTH sides (any non-identifier char, like the store's
      // scanTopLevel) — so `OR(`/`)OR` count, but `color`/`editor`/`for` don't.
      (i === 0 || !/[A-Za-z0-9_]/.test(body[i - 1])) &&
      (i + 2 >= n || !/[A-Za-z0-9_]/.test(body[i + 2]))
    ) {
      return true;
    }
    i++;
  }
  return false;
}

/** Splice `\`column\` = value` into a SQL query's WHERE, reusing the chip parser.
 *  'set' replaces the WHERE body; 'and' ANDs onto the existing one (or creates
 *  it). Returns null when the statement can't be safely parsed (no top-level
 *  FROM, a PREWHERE, a non-SELECT, or multiple statements) — same contract as the
 *  quick-filter chips — so the caller can hide the menu items. */
export function applySqlFilter(
  sql: string,
  column: string,
  value: unknown,
  mode: FilterMode,
): string | null {
  const core = sql.trim().replace(/;\s*$/, '');
  const parts = splitStatement(core);
  if (!parts) return null;
  const cond = sqlEquals(column, value);
  if (!cond) return null;
  let newBody: string;
  if (mode === 'and' && parts.whereBody) {
    const existing = hasTopLevelOr(parts.whereBody) ? `(${parts.whereBody})` : parts.whereBody;
    newBody = `${existing} AND ${cond}`;
  } else {
    newBody = cond;
  }
  return rewriteWhere(sql, newBody);
}

// ── Mongo find-filter splicing ───────────────────────────────────────────────

/** JSON for a Mongo value. `_id` + 24-hex string → `{"$oid": …}` (matches the
 *  grid's `mongoIdFilter`); everything else → strict JSON. */
export function mongoValueJson(col: string, v: unknown): string {
  if (v === undefined) return 'null';
  if (typeof v === 'bigint') return String(v); // JSON.stringify throws on bigint
  if (col === '_id' && typeof v === 'string' && /^[a-f0-9]{24}$/i.test(v)) {
    return `{"$oid": ${JSON.stringify(v)}}`;
  }
  try {
    return JSON.stringify(v);
  } catch {
    return JSON.stringify(String(v));
  }
}

/** `"col": <json>` — a single field for a Mongo find filter (keys JSON-quoted;
 *  the daemon's tolerant Mongo parser accepts that). */
export function mongoCondition(col: string, v: unknown): string {
  return `${JSON.stringify(col)}: ${mongoValueJson(col, v)}`;
}

/** Advance past a JS string literal starting at `i` (a quote). */
function skipJsString(s: string, i: number): number {
  const q = s[i];
  let j = i + 1;
  while (j < s.length) {
    if (s[j] === '\\') {
      j += 2;
      continue;
    }
    if (s[j] === q) {
      j += 1;
      break;
    }
    j += 1;
  }
  return j;
}

/** Advance past a `/regex/flags` literal starting at `i` (a `/`). Handles
 *  escapes and `[…]` char classes (where `/` is literal). */
function skipRegex(s: string, i: number): number {
  let j = i + 1;
  let inClass = false;
  while (j < s.length) {
    const c = s[j];
    if (c === '\\') {
      j += 2;
      continue;
    }
    if (c === '[') inClass = true;
    else if (c === ']') inClass = false;
    else if (c === '/' && !inClass) {
      j++;
      break;
    }
    j++;
  }
  while (j < s.length && /[a-z]/i.test(s[j])) j++; // flags
  return j;
}

/** Is the `/` at `i` the start of a regex literal (vs. division)? Regex follows
 *  an opener / operator / start, not a value. */
function regexStartsAt(s: string, i: number): boolean {
  let k = i - 1;
  while (k >= 0 && /\s/.test(s[k])) k--;
  if (k < 0) return true;
  return '([{,:=!&|?+-*%<>~^'.includes(s[k]);
}

/** Index of the bracket matching the one opened at `openIdx`, aware of strings,
 *  comments, and `/regex/` literals. Returns -1 when unbalanced. */
function matchBracket(s: string, openIdx: number): number {
  const open = s[openIdx];
  const close = open === '(' ? ')' : open === '{' ? '}' : ']';
  let depth = 0;
  let i = openIdx;
  while (i < s.length) {
    const c = s[i];
    if (c === "'" || c === '"' || c === '`') {
      i = skipJsString(s, i);
      continue;
    }
    if (c === '/' && s[i + 1] === '/') {
      while (i < s.length && s[i] !== '\n') i++;
      continue;
    }
    if (c === '/' && s[i + 1] === '*') {
      i += 2;
      while (i < s.length && !(s[i] === '*' && s[i + 1] === '/')) i++;
      i += 2;
      continue;
    }
    if (c === '/' && regexStartsAt(s, i)) {
      i = skipRegex(s, i);
      continue;
    }
    if (c === open) depth++;
    else if (c === close) {
      depth--;
      if (depth === 0) return i;
    }
    i++;
  }
  return -1;
}

/** Merge/replace the first argument (the filter object) of `db.<coll>.find(…)`.
 *  'set' → the filter becomes `{ <cond> }`; 'and' → `<cond>` is merged into the
 *  existing object. Returns null when there is no recognizable `find(...)` whose
 *  first argument is empty or an object literal. */
export function applyMongoFilter(
  sql: string,
  col: string,
  v: unknown,
  mode: FilterMode,
): string | null {
  const cond = mongoCondition(col, v);
  const re = /\.find\b/gi;
  let m: RegExpExecArray | null;
  while ((m = re.exec(sql)) !== null) {
    let i = m.index + m[0].length;
    while (i < sql.length && /\s/.test(sql[i])) i++;
    if (sql[i] !== '(') continue;
    const closeParen = matchBracket(sql, i);
    if (closeParen < 0) return null;
    const argStart = i + 1;
    const argEnd = closeParen; // index of ')'
    if (sql.slice(argStart, argEnd).trim().length === 0) {
      return sql.slice(0, argStart) + `{ ${cond} }` + sql.slice(argEnd);
    }
    let k = argStart;
    while (k < argEnd && /\s/.test(sql[k])) k++;
    if (sql[k] !== '{') return null; // first arg isn't an object literal
    const objEnd = matchBracket(sql, k); // index of '}'
    if (objEnd < 0) return null;
    if (mode === 'set') {
      return sql.slice(0, k) + `{ ${cond} }` + sql.slice(objEnd + 1);
    }
    const inner = sql
      .slice(k + 1, objEnd)
      .trim()
      .replace(/,\s*$/, '');
    const merged = inner ? `{ ${inner}, ${cond} }` : `{ ${cond} }`;
    return sql.slice(0, k) + merged + sql.slice(objEnd + 1);
  }
  return null;
}

// ── Dispatcher ───────────────────────────────────────────────────────────────

/** Build the filtered query for the given engine, or null when nothing sensible
 *  can be produced (empty base, a non-SELECT SQL statement, or a Mongo statement
 *  that isn't a `find`). The caller hides the menu items when this returns null. */
export function buildFilteredQuery(
  engine: 'mysql' | 'clickhouse' | 'mongodb',
  currentSql: string,
  col: string,
  v: unknown,
  mode: FilterMode,
): string | null {
  const sql = currentSql ?? '';
  if (!sql.trim()) return null;
  if (engine === 'mongodb') return applyMongoFilter(sql, col, v, mode);
  return applySqlFilter(sql, col, v, mode);
}
