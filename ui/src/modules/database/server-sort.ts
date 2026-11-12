// "Sort on server" for a capped / paged result. The grid's header sort orders
// only the LOADED rows (a 1,000-row page of a million-row table), which reads
// like sorting the table. When the statement on screen is one plain SELECT the
// "Query by value" splicer already parses (`splitStatement`), this rewrites its
// ORDER BY so the server sorts before the LIMIT. Anything else (UNION, INTO,
// several statements, a WITH, a non-identifier column) gets no rewrite — the
// chip then only says it sorted the loaded rows. Pure, so node tests import it.
import type { EngineLike } from '../../lib/database/sql-dialect.ts';
import { quoteIdent } from '../../lib/database/sql-dialect.ts';
import { scanTopLevel, splitStatement, stripSqlComments } from '../../lib/stores/database/statement.ts';

/** Clauses that follow ORDER BY: the new ORDER BY goes before the first. */
const AFTER_ORDER = new Set(['limit', 'settings', 'format']);
/** Clauses the rewrite does not attempt. */
const REFUSE = new Set(['union', 'union all', 'into', 'prewhere']);

/** `sql` with its top-level ORDER BY replaced by `ORDER BY <column> <dir>`, or
 *  null when the statement isn't a single plain SELECT this can rewrite safely. */
export function withServerOrder(
  sql: string,
  column: string,
  dir: 'asc' | 'desc',
  engine: EngineLike,
): string | null {
  if (engine !== 'mysql' && engine !== 'postgres' && engine !== 'clickhouse') return null;
  // A result column ORDER BY can name: a plain identifier (no expression text).
  if (!/^[A-Za-z_][A-Za-z0-9_$]*$/.test(column)) return null;
  const core = sql.trim().replace(/;\s*$/, '');
  if (!/^select\b/i.test(stripSqlComments(core).trim())) return null;
  if (!splitStatement(core)) return null;
  const hits = scanTopLevel(core);
  if (hits.some((h) => REFUSE.has(h.kw))) return null;
  const order = `ORDER BY ${quoteIdent(engine, column)} ${dir.toUpperCase()}`;
  const ob = hits.find((h) => h.kw === 'order by');
  // The scanner matches `order by` with one space only: any other spelling of
  // an existing ORDER BY would be left in place and doubled — refuse instead.
  if (/\border(?! by\b)\s+by\b/i.test(stripSqlComments(core))) return null;
  const after = hits.find((h) => AFTER_ORDER.has(h.kw) && (!ob || h.idx > ob.idx));
  const head = core.slice(0, ob ? ob.idx : after ? after.idx : core.length).trimEnd();
  const tail = after ? core.slice(after.idx).trim() : '';
  return tail ? `${head}\n${order}\n${tail}` : `${head}\n${order}`;
}

/** The rewrite for "Sort on server", only when the editor still holds exactly
 *  the statement on screen (so nothing else in the buffer is replaced) and the
 *  column name is unique in the result. */
export function serverSortSql(
  buffer: string | null | undefined,
  ran: string | null | undefined,
  columns: readonly { name: string }[],
  col: number,
  dir: 'asc' | 'desc',
  engine: EngineLike,
): string | null {
  const norm = (s: string | null | undefined) => (s ?? '').trim().replace(/;\s*$/, '');
  if (!ran || norm(buffer) !== norm(ran)) return null;
  const name = columns[col]?.name;
  if (!name || columns.filter((c) => c.name === name).length !== 1) return null;
  return withServerOrder(ran, name, dir, engine);
}
