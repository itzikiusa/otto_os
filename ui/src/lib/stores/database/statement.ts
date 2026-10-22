// Database Explorer store — statement text helpers: comment stripping, the
// trailing-LIMIT probe and the top-level WHERE splicer the quick-filter chips
// and the cell "Query by value" actions share. Pure, import-free; re-exported
// by the DatabaseStore façade (../database.svelte.ts).

// Drop line (`-- …`) and block (`/* … */`) comments, string-aware, so the
// trailing-LIMIT probe below only ever sees real SQL — a `LIMIT` inside a
// trailing comment must neither satisfy nor hide the match.
export function stripSqlComments(sql: string): string {
  let out = '';
  const n = sql.length;
  let i = 0;
  while (i < n) {
    const ch = sql[i];
    if (ch === "'" || ch === '"' || ch === '`') {
      const q = ch;
      out += sql[i++];
      while (i < n) {
        out += sql[i];
        if (sql[i] === q) {
          if (sql[i + 1] === q) { out += sql[i + 1]; i += 2; continue; }
          i++;
          break;
        }
        i++;
      }
      continue;
    }
    if (ch === '-' && sql[i + 1] === '-') { while (i < n && sql[i] !== '\n') i++; continue; }
    if (ch === '/' && sql[i + 1] === '*') { i += 2; while (i < n && !(sql[i] === '*' && sql[i + 1] === '/')) i++; i += 2; continue; }
    out += sql[i++];
  }
  return out;
}

/**
 * Extract a trailing explicit `LIMIT` from a SQL statement so we honor what the
 * user wrote instead of clipping it. Handles `LIMIT n`, `LIMIT offset, count`,
 * and `LIMIT n OFFSET m`. Comments are stripped first (a `-- LIMIT 10` remark
 * is not a LIMIT). Returns the row count, or null when there's no trailing LIMIT.
 */
export function parseExplicitLimit(sql: string): number | null {
  const m = stripSqlComments(sql).match(
    /\blimit\s+(\d+)(?:\s*,\s*(\d+))?(?:\s+offset\s+\d+)?\s*;?\s*$/i,
  );
  if (!m) return null;
  return m[2] !== undefined ? Number(m[2]) : Number(m[1]);
}

// ── Quick-filter helpers (module-level) ──────────────────────────────────────

// `toFilterVal` / `parseFilterValText` / `condToSql` / `condLabel` (and the
// `FilterVal` / `FilterCond` types) live in `lib/database/filter-chips.ts`
// — dialect-aware via `sql-dialect.ts` — and are re-exported at the top.

// Top-level clause keywords that terminate a WHERE / mark where one is inserted.
export const BOUNDARY_KW = [
  'group by', 'order by', 'having', 'limit', 'window', 'qualify',
  'union all', 'union', 'into', 'settings', 'format',
];
export const SCAN_KW = ['from', 'where', 'prewhere', ...BOUNDARY_KW];

/** Find top-level (depth-0, not in string/comment) clause-keyword hits. */
export function scanTopLevel(sql: string): { kw: string; idx: number; end: number }[] {
  const hits: { kw: string; idx: number; end: number }[] = [];
  const lower = sql.toLowerCase();
  const n = sql.length;
  let depth = 0;
  let i = 0;
  while (i < n) {
    const ch = sql[i];
    if (ch === "'" || ch === '"' || ch === '`') {
      const q = ch;
      i++;
      while (i < n) {
        if (sql[i] === q) {
          if (sql[i + 1] === q) { i += 2; continue; }
          i++;
          break;
        }
        i++;
      }
      continue;
    }
    if (ch === '-' && sql[i + 1] === '-') { while (i < n && sql[i] !== '\n') i++; continue; }
    if (ch === '/' && sql[i + 1] === '*') { i += 2; while (i < n && !(sql[i] === '*' && sql[i + 1] === '/')) i++; i += 2; continue; }
    if (ch === '(') { depth++; i++; continue; }
    if (ch === ')') { depth = Math.max(0, depth - 1); i++; continue; }
    if (depth === 0 && (i === 0 || /\s/.test(sql[i - 1]))) {
      const matched = SCAN_KW.find((kw) => {
        if (!lower.startsWith(kw, i)) return false;
        const after = sql[i + kw.length];
        return after === undefined || /\s/.test(after) || after === '(';
      });
      if (matched) { hits.push({ kw: matched, idx: i, end: i + matched.length }); i += matched.length; continue; }
    }
    i++;
  }
  return hits;
}

/** Split a single SELECT into head / WHERE-body / tail. Returns null when it
 * can't safely parse (no top-level FROM, a PREWHERE, or multiple statements).
 * Exported so the cell "Query by value" / "Add to query" actions reuse the exact
 * same top-level parser as the quick-filter chips (no second, drifting splicer). */
export function splitStatement(
  sql: string,
): { head: string; whereBody: string; tail: string } | null {
  if (/;\s*\S/.test(sql)) return null; // a second statement after a semicolon
  const hits = scanTopLevel(sql);
  if (!hits.some((h) => h.kw === 'from')) return null;
  if (hits.some((h) => h.kw === 'prewhere')) return null;
  const from = hits.find((h) => h.kw === 'from')!;
  const whereHit = hits.find((h) => h.kw === 'where');
  const isBoundary = (kw: string) => BOUNDARY_KW.includes(kw);
  if (whereHit) {
    const tailHit = hits.find((h) => isBoundary(h.kw) && h.idx > whereHit.idx);
    return {
      head: sql.slice(0, whereHit.idx),
      whereBody: sql.slice(whereHit.end, tailHit ? tailHit.idx : undefined).trim(),
      tail: (tailHit ? sql.slice(tailHit.idx) : '').trim(),
    };
  }
  const tailHit = hits.find((h) => isBoundary(h.kw) && h.idx > from.idx);
  return {
    head: tailHit ? sql.slice(0, tailHit.idx) : sql,
    whereBody: '',
    tail: (tailHit ? sql.slice(tailHit.idx) : '').trim(),
  };
}

/** Replace the statement's WHERE with `newWhereBody` (removing WHERE when empty).
 * Returns the original unchanged when it can't safely parse. Exported for the
 * cell filter actions (see `splitStatement`). */
export function rewriteWhere(sql: string, newWhereBody: string): string {
  const trimmed = sql.trimEnd();
  const hadSemi = trimmed.endsWith(';');
  const core = hadSemi ? trimmed.slice(0, -1).trimEnd() : trimmed;
  const parts = splitStatement(core);
  if (!parts) return sql;
  let out = parts.head.trimEnd();
  if (newWhereBody.trim()) out += `\nWHERE ${newWhereBody.trim()}`;
  if (parts.tail) out += `\n${parts.tail}`;
  return hadSemi ? `${out};` : out;
}

/** Extract a statement's existing WHERE body (to preserve it as a raw chip). */
export function extractWhereBody(sql: string): string | null {
  const core = sql.trim().replace(/;\s*$/, '');
  const parts = splitStatement(core);
  return parts && parts.whereBody ? parts.whereBody : null;
}

