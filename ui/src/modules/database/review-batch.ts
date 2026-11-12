// What a "Review & apply" batch did. A multi-statement review (one UPDATE /
// updateOne per edited row, one Redis command per cell) runs as a batch that
// STOPS at the first failing statement and still answers 200: the completed
// results first, then one `errored` entry (otto-dbviewer `errored_batch_entry`).
// The grid must read that flag — a partial write is not "Applied".
//
// MySQL and PostgreSQL batches run wrapped in `BEGIN; … COMMIT;`: the driver
// stops at the failing statement, `BEGIN` marks the pooled session
// close-on-drop (`types::sql_leaves_session_state`), so the open transaction
// is rolled back when the session closes and NOTHING from the batch lands.
// Mongo and Redis have no such batch transaction: the statements before the
// failure stay written, so only the drafts they covered are cleared.

import type { DbEngine, QueryResult } from '../../lib/api/types';
import { countStatements } from '../../lib/database/sql-util';
import { splitModeFor } from '../../lib/database/sql-dialect.ts';

/** Engines whose edit batch runs as one transaction. */
const TRANSACTIONAL: ReadonlySet<DbEngine> = new Set(['mysql', 'postgres']);

/** A statement that opens / ends / marks a transaction itself. */
const TX_CONTROL = /(^|;)\s*(begin|start\s+transaction|commit|rollback|end|savepoint|release)\b/i;

/**
 * The text to send for a review: on a transactional SQL engine a
 * multi-statement review is wrapped in `BEGIN; … COMMIT;` so it applies all or
 * nothing. Single statements are already atomic, and a review the person
 * edited to manage its own transaction is sent as written. `atomic` says which
 * one ran, so a failure can tell "nothing was written" from "the first k were".
 */
export function reviewBatchSql(engine: DbEngine | null, sql: string): { sql: string; atomic: boolean } {
  if (!engine || !TRANSACTIONAL.has(engine) || TX_CONTROL.test(sql)) return { sql, atomic: false };
  if (countStatements(sql, splitModeFor(engine)) < 2) return { sql, atomic: false };
  const body = /;\s*$/.test(sql) ? sql : `${sql};`;
  return { sql: `BEGIN;\n${body}\nCOMMIT;`, atomic: true };
}

/** A batch that stopped on an error: how many statements completed before it
 *  (wrapper statements excluded), which one failed and why. */
export interface BatchFailure {
  completed: number;
  statement: string | null;
  message: string;
}

/** The failure a 200 batch response carries, or null when every statement ran.
 *  `atomic` drops the leading `BEGIN` result from the completed count. */
export function batchFailure(res: QueryResult, atomic = false): BatchFailure | null {
  const entries = [res, ...(res.more_results ?? [])];
  const at = entries.findIndex((r) => r.errored);
  if (at < 0) return null;
  const failed = entries[at];
  return {
    completed: Math.max(0, at - (atomic ? 1 : 0)),
    statement: failed.statement ?? null,
    message: failed.message?.trim() || 'The database reported an error.',
  };
}

/** A row edit / delete whose `rows_affected` says whether it hit its row:
 *  SQL `UPDATE` / `DELETE`, and the mongosh calls the Mongo adapter emits. */
const ROW_WRITE = /^\s*(update|delete)\b/i;
const MONGO_ROW_WRITE = /\.(?:update|replace|delete)(?:One|Many)\s*\(/;
/** Engines whose `rows_affected` for those statements counts MATCHED rows. */
const MATCH_COUNTED: ReadonlySet<DbEngine> = new Set(['mysql', 'postgres', 'mongodb']);

/** Which row statements of a SUCCESSFUL review matched no row / several rows,
 *  as indices into the review's own statements (the `BEGIN` / `COMMIT`
 *  wrapper excluded). A single-row UPDATE whose row was deleted or whose key
 *  changed is not an error — it reports 0 rows, and "Applied" would be a lie.
 *  MySQL / Postgres / Mongo: their count is MATCHED rows (sqlx sets
 *  CLIENT_FOUND_ROWS; the Mongo driver reports `matched_count` for updates,
 *  `deleted_count` for deletes). Null when there is nothing to warn about (or
 *  no counts came back). */
export function affectedCheck(
  engine: DbEngine | null,
  res: QueryResult,
  atomic: boolean,
  ranSql: string,
): { unmatched: number[]; multi: number[] } | null {
  if (!engine || !MATCH_COUNTED.has(engine)) return null;
  const rowWrite = engine === 'mongodb' ? MONGO_ROW_WRITE : ROW_WRITE;
  let entries = [res, ...(res.more_results ?? [])];
  if (atomic) entries = entries.slice(1, -1);
  const unmatched: number[] = [];
  const multi: number[] = [];
  entries.forEach((r, i) => {
    // A single statement's result carries no preview: the review text is it.
    const text = r.statement ?? (atomic || res.more_results?.length ? '' : ranSql);
    if (!rowWrite.test(text) || typeof r.rows_affected !== 'number') return;
    if (r.rows_affected === 0) unmatched.push(i);
    else if (r.rows_affected > 1 && /^\s*update\b/i.test(text)) multi.push(i);
  });
  return unmatched.length || multi.length ? { unmatched, multi } : null;
}

/** What to tell the person when row statements matched nothing, and which
 *  drafts were written anyway (every statement of the row matched) — null
 *  when the review was edited and statements no longer map to rows. */
export function unmatchedReport(
  unmatched: readonly number[],
  stmtRows: readonly number[] | null,
): { written: Set<number> | null; message: string } {
  const n = unmatched.length;
  const lead = `${n === 1 ? 'A statement' : `${n} statements`} did not match any row — the row was deleted or its key changed since this result loaded.`;
  if (!stmtRows) return { written: null, message: `${lead} Nothing else was rolled back; every draft is kept. Re-run the query to see the current rows.` };
  const missed = new Set(unmatched.map((i) => stmtRows[i]));
  const written = new Set(stmtRows.filter((row) => !missed.has(row)));
  return {
    written,
    message: `${lead} ${n === 1 ? 'Its draft is' : 'Their drafts are'} kept and still pending${written.size ? '; the other edits were written' : ''}. Re-running the query shows the current rows and discards the kept drafts.`,
  };
}

/** The rows whose EVERY statement completed, given the row each statement of
 *  the batch writes (`stmtRows`, in order) and how many completed. A row split
 *  over several statements (a Redis rename) counts only once all of them ran. */
export function completedRows(stmtRows: readonly number[], completed: number): Set<number> {
  const done = new Set<number>();
  const open = new Set<number>();
  stmtRows.forEach((row, i) => (i < completed ? done : open).add(row));
  for (const row of open) done.delete(row);
  return done;
}
