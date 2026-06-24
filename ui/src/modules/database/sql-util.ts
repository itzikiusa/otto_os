// Pure SQL text helpers for the query editor: split a multi-statement buffer on
// top-level `;`, find the statement under the cursor, and detect/substitute
// query variables (`:name` / `{name}`) — all QUOTE- and COMMENT-aware so a `;`
// or a `:token` inside a string/comment is never mistaken for code.
//
// These power "run only the selected/current statement" and "query-level
// variables" without touching the server. Kept dependency-free + pure so they're
// trivially testable and reusable.

/**
 * Mark every character position as "code" (true) or "inside a string/comment"
 * (false). Handles `'…'`, `"…"`, `` `…` `` strings (with doubled-quote AND
 * backslash escapes — covers MySQL's default mode and standard SQL), `--`/`#`
 * line comments, and `/* … *​/` block comments.
 */
function codeMask(sql: string, mode: SplitMode = 'sql'): boolean[] {
  const n = sql.length;
  const mask = new Array<boolean>(n).fill(true);
  let i = 0;
  const off = (a: number, b: number) => {
    for (let k = a; k < b && k < n; k++) mask[k] = false;
  };
  while (i < n) {
    const c = sql[i];
    const c2 = i + 1 < n ? sql[i + 1] : '';
    // line comments: -- … or # … (to end of line)
    if ((c === '-' && c2 === '-') || c === '#') {
      let j = i;
      while (j < n && sql[j] !== '\n') j++;
      off(i, j);
      i = j;
      continue;
    }
    // block comment: /* … */
    if (c === '/' && c2 === '*') {
      let j = i + 2;
      while (j < n && !(sql[j] === '*' && sql[j + 1] === '/')) j++;
      j = Math.min(n, j + 2);
      off(i, j);
      i = j;
      continue;
    }
    // string literal: ' " ` (backtick is NOT a string delimiter in redis/line mode)
    if (c === "'" || c === '"' || (mode !== 'line' && c === '`')) {
      const q = c;
      let j = i + 1;
      while (j < n) {
        if (sql[j] === '\\') {
          j += 2;
          continue;
        } // backslash escape (MySQL)
        if (sql[j] === q) {
          if (sql[j + 1] === q) {
            j += 2;
            continue;
          } // doubled-quote escape ('' "")
          j += 1;
          break;
        }
        j += 1;
      }
      off(i, j);
      i = j;
      continue;
    }
    i += 1;
  }
  return mask;
}

/** How statements are delimited: `;` (SQL/Mongo/ClickHouse) vs one-per-line (Redis). */
export type SplitMode = 'sql' | 'line';

/** Statement segments (offsets INCLUDE the trailing delimiter), covering [0, len]. */
function segments(sql: string, mode: SplitMode = 'sql'): { from: number; to: number }[] {
  const segs: { from: number; to: number }[] = [];
  let start = 0;
  if (mode === 'line') {
    // Redis: each line is a command. Newlines aren't valid inside a command, so
    // a plain line split is correct (and quotes can't span lines here).
    for (let i = 0; i < sql.length; i++) {
      if (sql[i] === '\n') {
        segs.push({ from: start, to: i + 1 });
        start = i + 1;
      }
    }
  } else {
    const mask = codeMask(sql);
    for (let i = 0; i < sql.length; i++) {
      if (sql[i] === ';' && mask[i]) {
        segs.push({ from: start, to: i + 1 });
        start = i + 1;
      }
    }
  }
  segs.push({ from: start, to: sql.length });
  return segs;
}

const stripTrailingSemi = (s: string) => s.replace(/;\s*$/, '').trim();

/**
 * The statement containing `cursor` (trimmed, no trailing `;`). With a single
 * statement, returns the whole thing. When the cursor sits in trailing
 * whitespace after the last delimiter, returns the nearest preceding statement.
 */
export function statementAtCursor(sql: string, cursor: number, mode: SplitMode = 'sql'): string {
  const segs = segments(sql, mode).map((s) => ({
    ...s,
    text: stripTrailingSemi(sql.slice(s.from, s.to)),
  }));
  const nonEmpty = segs.filter((s) => s.text);
  if (nonEmpty.length <= 1) return sql.trim().length ? nonEmpty[0]?.text ?? sql.trim() : '';
  const c = Math.max(0, Math.min(cursor, sql.length));
  let hit = segs.find((s) => c >= s.from && c < s.to) ?? segs[segs.length - 1];
  if (!hit.text) {
    const before = nonEmpty.filter((s) => s.from <= c);
    hit = before.length ? before[before.length - 1] : nonEmpty[0];
  }
  return hit.text;
}

// `:name` and `{name}` at code positions. `:name` skips `::` casts via a
// preceding-char check (no lookbehind, for older Safari).
const VAR_COLON = /:([A-Za-z_]\w*)/g;
const VAR_BRACE = /\{([A-Za-z_]\w*)\}/g;

interface VarMatch {
  name: string;
  start: number;
  end: number;
}

function matchVars(sql: string, mode: SplitMode = 'sql'): VarMatch[] {
  const mask = codeMask(sql, mode);
  const out: VarMatch[] = [];
  let m: RegExpExecArray | null;
  VAR_COLON.lastIndex = 0;
  while ((m = VAR_COLON.exec(sql))) {
    // Only a STANDALONE `:name` is a variable. Skip when preceded by a word char,
    // ':' or '}', so Postgres `::casts` and redis keys (`user:123`,
    // `user:profile`, `{tag}:field`) are never mistaken for variables.
    const prev = m.index > 0 ? sql[m.index - 1] : '';
    if (/[A-Za-z0-9_:}]/.test(prev)) continue;
    if (mask[m.index]) out.push({ name: m[1], start: m.index, end: m.index + m[0].length });
  }
  // `{name}` variables don't apply to redis (line mode): `{...}` there is a
  // Cluster hash-tag (`{user}:1`), NOT a variable — never treat it as one.
  if (mode !== 'line') {
    VAR_BRACE.lastIndex = 0;
    while ((m = VAR_BRACE.exec(sql))) {
      if (mask[m.index]) out.push({ name: m[1], start: m.index, end: m.index + m[0].length });
    }
  }
  return out;
}

/** Unique variable names referenced in `sql` (`:name` / `{name}`), in first-seen order. */
export function extractVars(sql: string, mode: SplitMode = 'sql'): string[] {
  const seen = new Set<string>();
  for (const v of matchVars(sql, mode).sort((a, b) => a.start - b.start)) seen.add(v.name);
  return [...seen];
}

/**
 * Replace `:name` / `{name}` tokens (code positions only) with the supplied
 * values (raw textual substitution — the caller controls quoting). Tokens with
 * no entry in `values` are left as-is. Replaces right-to-left to keep offsets valid.
 */
export function substituteVars(
  sql: string,
  values: Record<string, string>,
  mode: SplitMode = 'sql',
): string {
  const reps = matchVars(sql, mode)
    .filter((v) => v.name in values)
    .sort((a, b) => b.start - a.start);
  let out = sql;
  for (const r of reps) out = out.slice(0, r.start) + values[r.name] + out.slice(r.end);
  return out;
}
