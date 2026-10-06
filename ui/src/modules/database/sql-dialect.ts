// ONE place for the per-engine SQL text rules every hand-built statement and
// every splitter pass depends on — identifier quoting, string-literal escaping,
// boolean literals, and how the statement splitter lexes strings / comments.
// The edit builders (`edit-sql.ts`), the quick-filter chips + "Query by value"
// (`filter-chips.ts`, `query-filter.ts`), query variables and the splitter
// (`sql-util.ts`) all read from here, so a dialect fix lands everywhere at once
// (they used to carry their own copies, and Postgres drifted in three of them).
//
// Dependency-free (type import only) so node unit tests import it directly.
import type { ConnectionKind, DbEngine } from '../../lib/api/types';

/** A DB engine or a connection's kind (a superset — non-SQL kinds get the
 *  MySQL-style defaults), or nothing known. */
export type EngineLike = DbEngine | ConnectionKind | null | undefined;

/** Quote an identifier: double quotes on Postgres (backticks are a syntax
 *  error there), backticks on MySQL / ClickHouse (and as the engine-less
 *  default, e.g. a chip label). The quote char is doubled inside; ClickHouse
 *  also reads `\` as an escape inside a quoted name, so it is doubled there
 *  first (a name ending in `\` would otherwise swallow the closing backtick)
 *  — the same rule as the daemon's `import.rs` `SqlFlavor::ident`. */
export function quoteIdent(engine: EngineLike, name: string): string {
  if (engine === 'postgres') return '"' + name.replace(/"/g, '""') + '"';
  const n = engine === 'clickhouse' ? name.replace(/\\/g, '\\\\') : name;
  return '`' + n.replace(/`/g, '``') + '`';
}

/** `schema.table` (or just `table`) quoted for the engine. */
export function qualifiedName(engine: EngineLike, schema: string | null | undefined, table: string): string {
  return schema ? `${quoteIdent(engine, schema)}.${quoteIdent(engine, table)}` : quoteIdent(engine, table);
}

/** MySQL (default modes) and ClickHouse treat `\` as an escape character
 *  inside a string literal; Postgres standard strings
 *  (`standard_conforming_strings`, on by default) do not. */
export function backslashEscapes(engine: EngineLike): boolean {
  return engine === 'mysql' || engine === 'clickhouse';
}

/** Escape the INNER text of a single-quoted literal: `'` doubled always, `\`
 *  doubled when the dialect reads it as an escape (a value ending in `\` would
 *  otherwise swallow the closing quote). */
export function escapeSqlText(s: string, backslash: boolean): string {
  return (backslash ? s.replace(/\\/g, '\\\\') : s).replace(/'/g, "''");
}

/** `'…'` string literal for the engine. NUL can't appear in a Postgres text
 *  value at all (dropped); the backslash dialects spell it `\0`. */
export function stringLiteral(engine: EngineLike, s: string): string {
  if (!backslashEscapes(engine)) return `'${escapeSqlText(s.replace(/\0/g, ''), false)}'`;
  return `'${escapeSqlText(s, true).replace(/\0/g, '\\0')}'`;
}

/** Boolean literal: `TRUE` / `FALSE` on Postgres (`boolean = integer` is an
 *  error there), `1` / `0` on MySQL / ClickHouse. */
export function boolLiteral(engine: EngineLike, b: boolean): string {
  return engine === 'postgres' ? (b ? 'TRUE' : 'FALSE') : b ? '1' : '0';
}

// ── Table references ─────────────────────────────────────────────────────────

/** Resolve a schema-tree node id like `db:shop/table:orders` (Postgres trees use
 *  `db:` for the schema) to its engine-quoted `schema.table` reference plus the
 *  raw parts, or null when the node isn't a SQL table / view. */
export function tableRefFromNodeId(
  engine: EngineLike,
  id: string,
): { ref: string; db: string | null; table: string } | null {
  const segs = id.split('/').map((s) => {
    const i = s.indexOf(':');
    return i < 0 ? ([s, ''] as const) : ([s.slice(0, i), s.slice(i + 1)] as const);
  });
  const find = (k: string) => segs.find(([kk]) => kk === k)?.[1];
  const table = find('table') ?? find('view');
  if (!table) return null;
  const db = find('db') ?? find('schema') ?? null;
  return { ref: qualifiedName(engine, db, table), db, table };
}

// ── Splitter lexing ──────────────────────────────────────────────────────────

/** How the editor splits / scans a buffer:
 *  - `sql`  — `;`-delimited, MySQL lexing (`#` comments, `\` escapes in every
 *             quoted span, backtick-quoted identifiers). MySQL, ClickHouse, Mongo.
 *  - `pg`   — `;`-delimited, Postgres lexing: `#` is an operator (XOR), `\` is a
 *             plain char except in `E'…'` strings, `$$…$$` / `$tag$…$tag$`
 *             dollar quotes are strings, `/* … *\/` comments nest.
 *  - `line` — one command per line (Redis). */
export type SplitMode = 'sql' | 'line' | 'pg';

/** The split mode for a connection engine. */
export function splitModeFor(engine: EngineLike): SplitMode {
  return engine === 'redis' ? 'line' : engine === 'postgres' ? 'pg' : 'sql';
}

const isIdentChar = (c: number): boolean =>
  (c >= 48 && c <= 57) || (c >= 65 && c <= 90) || (c >= 97 && c <= 122) || c === 95 || c === 36 || c >= 128;

/** Postgres: when position `i` opens a string / quoted identifier / comment /
 *  dollar quote, the index just past its end (`sql.length` when unterminated);
 *  -1 when `i` is code. */
export function pgSpanEnd(sql: string, i: number): number {
  const n = sql.length;
  const c = sql.charCodeAt(i);
  const c2 = i + 1 < n ? sql.charCodeAt(i + 1) : -1;
  // -- line comment (the newline stays code, as in the MySQL scan)
  if (c === 45 && c2 === 45) {
    const e = sql.indexOf('\n', i);
    return e < 0 ? n : e;
  }
  // /* block comment */ — Postgres block comments NEST.
  if (c === 47 && c2 === 42) {
    let depth = 1;
    let j = i + 2;
    while (j < n && depth > 0) {
      if (sql.charCodeAt(j) === 47 && sql.charCodeAt(j + 1) === 42) {
        depth++;
        j += 2;
      } else if (sql.charCodeAt(j) === 42 && sql.charCodeAt(j + 1) === 47) {
        depth--;
        j += 2;
      } else {
        j++;
      }
    }
    return j;
  }
  // 'standard string' / E'escape string' / "quoted identifier"
  if (c === 39 || c === 34) {
    const p = i > 0 ? sql.charCodeAt(i - 1) : -1;
    const pp = i > 1 ? sql.charCodeAt(i - 2) : -1;
    const escape = c === 39 && (p === 69 || p === 101) && !isIdentChar(pp);
    let j = i + 1;
    while (j < n) {
      const d = sql.charCodeAt(j);
      if (escape && d === 92) {
        j += 2;
        continue;
      }
      if (d === c) {
        if (sql.charCodeAt(j + 1) === c) {
          j += 2;
          continue;
        }
        return j + 1;
      }
      j++;
    }
    return n;
  }
  // $$ … $$ / $tag$ … $tag$ — not `$1` (a parameter) nor `a$b$` (an identifier).
  if (c === 36) {
    if (i > 0 && isIdentChar(sql.charCodeAt(i - 1))) return -1;
    let j = i + 1;
    if (j < n && sql.charCodeAt(j) >= 48 && sql.charCodeAt(j) <= 57) return -1;
    while (j < n && isIdentChar(sql.charCodeAt(j)) && sql.charCodeAt(j) !== 36) j++;
    if (sql.charCodeAt(j) !== 36) return -1;
    const tag = sql.slice(i, j + 1);
    const close = sql.indexOf(tag, j + 1);
    return close < 0 ? n : close + tag.length;
  }
  return -1;
}

// ── Write detection (UI hint only) ───────────────────────────────────────────

const WRITE_LEAD = /^(insert|update|delete|replace|merge|upsert|truncate|drop|alter|create|rename|grant|revoke|optimize|attach|detach|exchange|set|del|unlink|flushall|flushdb|hset|hdel|lpush|rpush|lset|lrem|sadd|srem|zadd|zrem|expire|persist|incr|decr|incrby|decrby|append|mset|getdel|rpop|lpop)\b/i;

/** The leading write verb of a statement (`'UPDATE'`, `'DEL'`…) when it is
 *  OBVIOUSLY a write, else null. A fast UI hint (dashboard widgets refuse
 *  these at save time) — never the gate: the daemon classifies every widget
 *  run read-only and refuses anything it deems a write. */
export function obviousWriteVerb(statement: string): string | null {
  const s = statement.replace(/^(\s|--[^\n]*\n|\/\*[\s\S]*?\*\/)+/, '');
  const m = WRITE_LEAD.exec(s);
  if (m) return m[1].toUpperCase();
  // `WITH … UPDATE / DELETE / INSERT` (Postgres data-modifying CTEs).
  const w = /^with\b[\s\S]*?\b(insert|update|delete)\b/i.exec(s);
  return w ? w[1].toUpperCase() : null;
}
