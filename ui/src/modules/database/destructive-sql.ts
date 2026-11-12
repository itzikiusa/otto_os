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

export type RiskKind = 'delete-all' | 'update-all' | 'drop' | 'truncate' | 'alter-drop';

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
};

/** The kind of wholesale removal `code` (one statement, blanked) performs. */
function classify(code: string): RiskKind | null {
  const s = code.replace(/\s+/g, ' ').trim().toLowerCase();
  const where = /\bwhere\b/.test(s);
  if (/^delete\b/.test(s)) return where ? null : 'delete-all';
  if (/^update\b/.test(s)) return where ? null : 'update-all';
  if (/^drop\b/.test(s)) return 'drop';
  if (/^truncate\b/.test(s)) return 'truncate';
  if (/^alter table\b/.test(s) && /\bdrop\b/.test(s)) return 'alter-drop';
  return null;
}

/** First line of the statement's code, cut from the text as written. */
function previewOf(text: string, code: string): string {
  const start = code.search(/\S/);
  const end = code.indexOf('\n', start);
  const line = text.slice(start, end < 0 ? undefined : end).trim().replace(/;\s*$/, '');
  return line.length > 120 ? `${line.slice(0, 119)}…` : line;
}

/** The statements of `sql` that delete / update every row, drop or truncate,
 *  in order. Empty for a non-SQL engine (Mongo, Redis). */
export function destructiveStatements(sql: string, engine: EngineLike): RiskyStatement[] {
  if (!engine || !SQL_ENGINES.has(engine)) return [];
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
