// Client-side "are you sure?" classifier for the query editor's Run / Run all.
// The daemon's write gate only stops writes on guarded (Prod / read-only)
// connections; everywhere else a `DELETE FROM orders` with a forgotten WHERE
// ran on the first ⌘↵. This names the statements that remove data or schema
// wholesale so the editor can ask once before sending them.
//
// Deliberately simple keyword rules over the statement's CODE (strings,
// quoted names and comments blanked by `codeStatements`): a WHERE anywhere in
// the statement counts as a WHERE — `WHERE 1=1` included — because guessing
// at predicates would make the prompt unpredictable. Pure (no store imports),
// so node unit tests import it directly.
import type { EngineLike } from '../../lib/database/sql-dialect.ts';
import { splitModeFor } from '../../lib/database/sql-dialect.ts';
import { codeStatements } from '../../lib/database/sql-util.ts';

export type RiskKind = 'delete-all' | 'update-all' | 'drop' | 'truncate' | 'alter-drop' | 'flush';

export interface RiskyStatement {
  kind: RiskKind;
  /** The statement's first line of code, as written (≤ 120 chars). */
  preview: string;
}

/** Engines whose text is SQL this classifier understands. */
const SQL_ENGINES = new Set(['mysql', 'postgres', 'clickhouse']);

/** Why each kind is risky — the suffix after the statement in the prompt. */
const WHY: Record<RiskKind, string> = {
  'delete-all': 'no WHERE — deletes every row',
  'update-all': 'no WHERE — updates every row',
  drop: 'drops the object',
  truncate: 'empties the table',
  'alter-drop': 'drops part of the table',
  flush: 'empties the database',
};

/** Advance past a balanced `(...)` starting at `openIdx`; returns the index
 *  just after the matching `)`, or -1 if unclosed. */
function skipBalancedParens(s: string, openIdx: number): number {
  if (s[openIdx] !== '(') return -1;
  let depth = 0;
  for (let i = openIdx; i < s.length; i++) {
    if (s[i] === '(') depth++;
    else if (s[i] === ')') {
      depth--;
      if (depth === 0) return i + 1;
    }
  }
  return -1;
}

/** Split a leading `WITH [RECURSIVE] …` CTE chain into `{ bodies, main }` so
 *  CTE predicates never mask a missing `WHERE` on the main `DELETE`/`UPDATE`,
 *  and Postgres data-modifying CTEs (`WITH d AS (DELETE FROM t) SELECT …`) are
 *  checked too. Assumes strings/comments are already blanked. */
function splitCtes(s: string): { bodies: string[]; main: string } {
  if (!/^with\b/.test(s)) return { bodies: [], main: s };
  let i = 4; // past 'with'
  const skipWs = (): void => {
    while (i < s.length && /\s/.test(s[i])) i++;
  };
  skipWs();
  if (/^recursive\b/.test(s.slice(i))) {
    i += 9;
    skipWs();
  }
  const bodies: string[] = [];
  while (i < s.length) {
    // CTE name (already blanked if quoted, or plain identifier)
    const nameMatch = /^[a-z0-9_$]+/.exec(s.slice(i));
    if (nameMatch) i += nameMatch[0].length;
    skipWs();
    // Optional column list `(c1, c2)`
    if (s[i] === '(') {
      const end = skipBalancedParens(s, i);
      if (end < 0) return { bodies, main: s };
      i = end;
      skipWs();
    }
    if (!/^as\b/.test(s.slice(i))) return { bodies, main: s };
    i += 2;
    skipWs();
    // Postgres optional `[NOT] MATERIALIZED`
    const matMatch = /^(?:not\s+)?materialized\b/.exec(s.slice(i));
    if (matMatch) {
      i += matMatch[0].length;
      skipWs();
    }
    if (s[i] !== '(') return { bodies, main: s };
    const end = skipBalancedParens(s, i);
    if (end < 0) return { bodies, main: s };
    bodies.push(s.slice(i + 1, end - 1).trim());
    i = end;
    skipWs();
    if (s[i] === ',') {
      i++;
      skipWs();
      continue;
    }
    return { bodies, main: s.slice(i).trim() };
  }
  return { bodies, main: s };
}

/** Classify a single SQL statement body (after CTE stripping). */
function classifySqlCore(s: string): RiskKind | null {
  const where = /\bwhere\b/.test(s);
  if (/^delete\b/.test(s)) return where ? null : 'delete-all';
  if (/^update\b/.test(s)) return where ? null : 'update-all';
  if (/^drop\b/.test(s)) return 'drop';
  if (/^truncate\b/.test(s)) return 'truncate';
  if (/^alter table\b/.test(s) && /\bdrop\b/.test(s)) return 'alter-drop';
  return null;
}

/** The kind of wholesale removal a SQL statement (`code`, blanked) performs. */
function classifySql(code: string): RiskKind | null {
  const s = code.replace(/\s+/g, ' ').trim().toLowerCase();
  const { bodies, main } = splitCtes(s);
  const direct = classifySqlCore(main);
  if (direct) return direct;
  for (const body of bodies) {
    const inner = classifySqlCore(body);
    if (inner) return inner;
  }
  return null;
}

/** Wholesale removals in MongoDB shell statements (blanked code). */
function classifyMongo(code: string): RiskKind | null {
  // Blank JS `//` line comments in addition to SQL-style comments already blanked.
  const s = code.replace(/\/\/[^\n]*/g, ' ').replace(/\s+/g, ' ').trim();
  if (!s) return null;
  if (/\.\s*dropDatabase\s*\(/i.test(s) || /\.\s*drop\s*\(/i.test(s)) return 'drop';
  if (/\.\s*(?:deleteMany|remove)\s*\(\s*\{\s*\}\s*(?:,|\))/i.test(s)) return 'delete-all';
  if (/\.\s*updateMany\s*\(\s*\{\s*\}\s*,/i.test(s)) return 'update-all';
  return null;
}

/** Wholesale removals in Redis commands (one command per line). */
function classifyRedis(code: string): RiskKind | null {
  const s = code.replace(/\s+/g, ' ').trim().toLowerCase();
  if (!s) return null;
  if (/^(?:flushall|flushdb)\b/.test(s)) return 'flush';
  return null;
}

/** First line of the statement's code, cut from the text as written. */
function previewOf(text: string, code: string): string {
  const start = code.search(/\S/);
  const end = code.indexOf('\n', start);
  const line = text.slice(start, end < 0 ? undefined : end).trim().replace(/;\s*$/, '');
  return line.length > 120 ? `${line.slice(0, 119)}…` : line;
}

/** The statements of `sql` that delete / update every row, drop, truncate or
 *  flush, in order. Covers SQL engines (including CTEs), MongoDB and Redis. */
export function destructiveStatements(sql: string, engine: EngineLike): RiskyStatement[] {
  if (!engine) return [];
  if (engine === 'redis') {
    const out: RiskyStatement[] = [];
    for (const rawLine of sql.split(/\r?\n/)) {
      const trimmed = rawLine.trim();
      if (!trimmed || trimmed.startsWith('#') || trimmed.startsWith('--')) continue;
      const kind = classifyRedis(trimmed);
      if (kind) out.push({ kind, preview: trimmed.length > 120 ? `${trimmed.slice(0, 119)}…` : trimmed });
    }
    return out;
  }
  const classify = SQL_ENGINES.has(engine)
    ? classifySql
    : engine === 'mongodb'
      ? classifyMongo
      : null;
  if (!classify) return [];
  const out: RiskyStatement[] = [];
  for (const { text, code } of codeStatements(sql, splitModeFor(engine))) {
    const kind = classify(code);
    if (kind) out.push({ kind, preview: previewOf(text, code) });
  }
  return out;
}

/** The confirm text: the connection, then each risky statement (the first
 *  five, then a count). */
export function destructiveMessage(risky: readonly RiskyStatement[], connection: string): string {
  const shown = risky.slice(0, 5).map((r) => `• ${r.preview}  (${WHY[r.kind]})`);
  if (risky.length > 5) shown.push(`…and ${risky.length - 5} more`);
  const lead = risky.length === 1
    ? `This statement removes data on “${connection}”, and it can’t be undone from here:`
    : `${risky.length} statements remove data on “${connection}”, and they can’t be undone from here:`;
  return `${lead}\n\n${shown.join('\n')}`;
}
