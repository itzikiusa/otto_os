// The Processes panel: what is running on the server right now, read-only,
// and a Kill that is a reviewed write. Pure (no stores) so node tests import it.
//
// - Listing runs through the normal query route with `read_only: true`: the
//   server classifies the statement as a read and runs it in the engine's
//   native read-only mode. The daemon does not record these refreshes in query
//   history (`service/process_list.rs` keeps the same statement text).
// - Kill builds `KILL QUERY …` (it stops the statement, not the connection).
//   It is a write: a Read-only connection refuses it on the server (no
//   confirm lifts that), and a production one takes the usual typed confirm.
//
// MongoDB (`$currentOp`) is not offered yet: the driver has no native op for
// it (see docs/features/database-explorer.md → Processes).

/** Exactly what `crates/otto-dbviewer/src/service/process_list.rs` matches. */
export const PROCESS_SQL = {
  mysql: 'SHOW FULL PROCESSLIST',
  clickhouse:
    'SELECT query_id, user, elapsed, read_rows, memory_usage, query FROM system.processes ORDER BY elapsed DESC',
} as const;

export type ProcessEngine = keyof typeof PROCESS_SQL;

export function processEngine(kind: string | null | undefined): ProcessEngine | null {
  return kind === 'mysql' || kind === 'clickhouse' ? kind : null;
}

/** One running statement, normalized across engines. */
export interface ProcessRow {
  /** MySQL connection Id / ClickHouse query_id — what Kill targets. */
  id: string;
  user: string;
  /** Seconds running (MySQL `Time`, ClickHouse `elapsed`). */
  seconds: number | null;
  /** MySQL `Command` / `State`; ClickHouse read rows. */
  state: string;
  query: string;
}

const str = (v: unknown): string => (v === null || v === undefined ? '' : String(v));
const num = (v: unknown): number | null => {
  const n = typeof v === 'number' ? v : Number.parseFloat(str(v));
  return Number.isFinite(n) ? n : null;
};

/** Rows of the listing result as `ProcessRow`s, by column name (case-insensitive). */
export function parseProcesses(
  engine: ProcessEngine,
  columns: readonly { name: string }[],
  rows: readonly unknown[][],
): ProcessRow[] {
  const at = (name: string): number => columns.findIndex((c) => c.name.toLowerCase() === name);
  if (engine === 'mysql') {
    const [id, user, host, cmd, time, state, info] = ['id', 'user', 'host', 'command', 'time', 'state', 'info'].map(at);
    return rows.map((r) => ({
      id: str(r[id]),
      user: [str(r[user]), str(r[host])].filter(Boolean).join('@'),
      seconds: num(r[time]),
      state: [str(r[cmd]), str(r[state])].filter(Boolean).join(' · '),
      query: str(r[info]),
    }));
  }
  const [qid, user, elapsed, read, , query] = ['query_id', 'user', 'elapsed', 'read_rows', 'memory_usage', 'query'].map(at);
  return rows.map((r) => ({
    id: str(r[qid]),
    user: str(r[user]),
    seconds: num(r[elapsed]),
    state: r[read] === undefined ? '' : `${str(r[read])} rows read`,
    query: str(r[query]),
  }));
}

/** The statement that stops `p`, or null when its id cannot be quoted safely. */
export function killSql(engine: ProcessEngine, p: Pick<ProcessRow, 'id'>): string | null {
  if (engine === 'mysql') return /^\d+$/.test(p.id) ? `KILL QUERY ${p.id}` : null;
  if (!p.id) return null;
  return `KILL QUERY WHERE query_id = '${p.id.replace(/\\/g, '\\\\').replace(/'/g, "\\'")}' ASYNC`;
}

/** The query as the Kill confirm names it: one line, capped. */
export function queryPreview(q: string, max = 240): string {
  const one = q.replace(/\s+/g, ' ').trim();
  if (!one) return '(no statement — an idle connection)';
  return one.length > max ? `${one.slice(0, max - 1)}…` : one;
}
