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
