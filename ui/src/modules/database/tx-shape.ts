// Manual transactions across runs do not work, so the editor says so. Every Run
// executes on its own pooled session, and a session that ran BEGIN / START
// TRANSACTION (or COMMIT, SAVEPOINT …) is CLOSED when the run ends instead of
// going back to the pool (otto-dbviewer `types::sql_leaves_session_state`;
// MySQL `run_batch` and Postgres `run_write` / `run_batch` call
// `close_on_drop`). Both servers roll back a transaction left open when its
// connection closes. So `START TRANSACTION` → (next run) `UPDATE …` → (next
// run) `ROLLBACK` autocommits the UPDATE and the ROLLBACK undoes nothing.
// A single run holding BEGIN … COMMIT executes on ONE session and works.
//
// This classifies a run's text (MySQL / PostgreSQL only) over its CODE
// (`codeStatements` blanks strings and comments). Pure, so node tests import it.
import type { EngineLike } from '../../lib/database/sql-dialect.ts';
import { splitModeFor } from '../../lib/database/sql-dialect.ts';
import { codeStatements } from '../../lib/database/sql-util.ts';

export interface TxShape {
  /** Every statement is transaction control (BEGIN / COMMIT / SAVEPOINT …):
   *  on its own session it has nothing to act on. */
  controlOnly: boolean;
  /** The run opens a transaction and does not close it: it is rolled back
   *  when the run's session closes. */
  unclosed: boolean;
}

type TxOp = 'open' | 'close' | 'other-control' | null;

/** The transaction role of one statement's code (lowercased, blanked). */
function txOp(code: string, engine: 'mysql' | 'postgres'): TxOp {
  const s = code.replace(/;\s*$/, '').replace(/\s+/g, ' ').trim().toLowerCase();
  if (/^start transaction\b/.test(s)) return 'open';
  // `BEGIN` / `BEGIN WORK` / Postgres `BEGIN TRANSACTION ISOLATION …`.
  if (/^begin(?: (?:work|transaction)\b.*)?$/.test(s) || (engine === 'postgres' && /^begin\b/.test(s))) return 'open';
  // `ROLLBACK TO [SAVEPOINT] x` keeps the transaction open.
  if (/^rollback(?: work| transaction)? to\b/.test(s)) return 'other-control';
  if (/^(?:commit|rollback)\b/.test(s)) return 'close';
  if (engine === 'postgres' && /^(?:end|abort)\b/.test(s)) return 'close';
  if (/^(?:savepoint|release)\b/.test(s)) return 'other-control';
  return null;
}

/** How `sql` (one Run's text) uses transactions, or null when it does not
 *  touch them — and always null for engines other than MySQL / PostgreSQL. */
export function txShape(sql: string | null | undefined, engine: EngineLike): TxShape | null {
  if (!sql || (engine !== 'mysql' && engine !== 'postgres')) return null;
  const ops = codeStatements(sql, splitModeFor(engine)).map(({ code }) => txOp(code, engine));
  if (ops.length === 0 || ops.every((op) => op === null)) return null;
  let open = false;
  for (const op of ops) {
    if (op === 'open') open = true;
    else if (op === 'close') open = false;
  }
  return { controlOnly: ops.every((op) => op !== null), unclosed: open };
}

const ENGINE_NAME = { mysql: 'MySQL', postgres: 'PostgreSQL' } as const;

/** The notice shown under the editor after such a run. */
export function txNotice(shape: TxShape, engine: 'mysql' | 'postgres'): string {
  const lead =
    'Each run uses its own session, so a transaction cannot span runs. Put the whole transaction in one Run all (⇧⌘↵), or edit rows and use Review before applying.';
  if (shape.unclosed && shape.controlOnly) {
    return `${lead} The transaction this run opened ended with it (${ENGINE_NAME[engine]} rolled it back when the session closed) — the next run is not inside it.`;
  }
  if (shape.unclosed) {
    return `${lead} This run left a transaction open: its session was closed when the run ended, so ${ENGINE_NAME[engine]} rolled back everything the run changed after BEGIN.`;
  }
  return `${lead} A COMMIT or ROLLBACK run on its own acts on a fresh session with no open transaction, so it changed nothing.`;
}

/** The confirm shown BEFORE running a batch that leaves a transaction open. */
export function txUnclosedWarning(engine: 'mysql' | 'postgres'): string {
  return `This run opens a transaction and does not close it. Each run uses its own session, which is closed when the run ends — ${ENGINE_NAME[engine]} then rolls the open transaction back, so nothing it changes after BEGIN is kept.\n\nAdd COMMIT at the end to keep the changes, or run it anyway to try them out.`;
}
