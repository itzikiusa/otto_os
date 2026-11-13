// The Processes panel: what is running on the server right now, read-only,
// and a Kill that is a reviewed write. Pure (no stores) so node tests import it.
//
// - Listing runs through the normal query route with `read_only: true`: the
//   server classifies the statement as a read and runs it in the engine's
//   native read-only mode. The daemon does not record these refreshes in query
//   history (`service/process_list.rs` keeps the same statement text).
// - Kill is the engine's own statement, sent through the same query route as
//   any write: `KILL QUERY …` (MySQL / ClickHouse: the statement, not the
//   connection), `pg_cancel_backend` / `pg_terminate_backend` (PostgreSQL:
//   cancel the query, or close the connection), `killOp` (MongoDB). A
//   Read-only connection refuses it on the server (no confirm lifts that), a
//   production one takes the usual typed confirm, and its history row is the
//   audit. Ids come from the listing and are validated before they go in.

/** Exactly what `crates/otto-dbviewer/src/service/process_list.rs` matches. */
export const PROCESS_SQL = {
  mysql: 'SHOW FULL PROCESSLIST',
  clickhouse:
    'SELECT query_id, user, elapsed, read_rows, memory_usage, query FROM system.processes ORDER BY elapsed DESC',
  postgres:
    "SELECT pid, usename, datname, client_addr::text AS client_addr, application_name, state, wait_event_type, wait_event, backend_type, round(EXTRACT(EPOCH FROM now() - CASE WHEN state = 'active' THEN query_start ELSE state_change END)::numeric, 1) AS seconds, substr(query, 1, 4000) AS query FROM pg_stat_activity WHERE state = 'active' OR backend_type IS NULL OR backend_type = 'client backend' ORDER BY query_start NULLS LAST",
  mongodb:
    '{"collection":"$cmd.currentOp","pipeline":[{"$currentOp":{"allUsers":true,"idleConnections":false}}]}',
} as const;

export type ProcessEngine = keyof typeof PROCESS_SQL;

export function processEngine(kind: string | null | undefined): ProcessEngine | null {
  return kind === 'mysql' || kind === 'clickhouse' || kind === 'postgres' || kind === 'mongodb' ? kind : null;
}

/** What PostgreSQL shows as the query of another role's session to a login
 *  without `pg_read_all_stats` (its state and client are NULL too). */
export const PG_HIDDEN = '<insufficient privilege>';

/** One running statement, normalized across engines. */
export interface ProcessRow {
  /** MySQL connection Id / ClickHouse query_id / PostgreSQL pid / Mongo opid — what Kill targets. */
  id: string;
  user: string;
  /** PostgreSQL database / MongoDB namespace; absent for MySQL and ClickHouse. */
  database?: string;
  /** Seconds running (MySQL `Time`, ClickHouse `elapsed`, PostgreSQL time in its state, Mongo `microsecs_running`). */
  seconds: number | null;
  /** MySQL `Command` / `State`; ClickHouse read rows; PostgreSQL state + wait; Mongo op + desc. */
  state: string;
  query: string;
  /** PostgreSQL withheld this session's details from this login. */
  hidden?: boolean;
}

const str = (v: unknown): string => (v === null || v === undefined ? '' : String(v));
const num = (v: unknown): number | null => {
  const n = typeof v === 'number' ? v : Number.parseFloat(str(v));
  return Number.isFinite(n) ? n : null;
};
const join = (parts: string[], sep: string): string => parts.filter(Boolean).join(sep);

/** Rows of the listing result as `ProcessRow`s, by column name (case-insensitive). */
export function parseProcesses(
  engine: ProcessEngine,
  columns: readonly { name: string }[],
  rows: readonly unknown[][],
): ProcessRow[] {
  const at = (name: string): number => columns.findIndex((c) => c.name.toLowerCase() === name);
  const pick = (names: string[]) => names.map(at);
  if (engine === 'mysql') {
    const [id, user, host, cmd, time, state, info] = pick(['id', 'user', 'host', 'command', 'time', 'state', 'info']);
    return rows.map((r) => ({
      id: str(r[id]),
      user: join([str(r[user]), str(r[host])], '@'),
      seconds: num(r[time]),
      state: join([str(r[cmd]), str(r[state])], ' · '),
      query: str(r[info]),
    }));
  }
  if (engine === 'postgres') {
    const [pid, user, db, client, state, wtype, wevent, btype, secs, query] = pick([
      'pid', 'usename', 'datname', 'client_addr', 'state', 'wait_event_type', 'wait_event', 'backend_type', 'seconds', 'query',
    ]);
    // The listing session sees itself running the listing: drop that row.
    const own = PROCESS_SQL.postgres.slice(0, 60);
    return rows
      .filter((r) => !str(r[query]).startsWith(own))
      .map((r) => {
        const hidden = str(r[query]) === PG_HIDDEN;
        const wait = join([str(r[wtype]), str(r[wevent])], ': ');
        const kind = str(r[btype]);
        return {
          id: str(r[pid]),
          // inet text carries the host mask (`/32`, `/128`); a host needs none.
          user: join([str(r[user]), str(r[client]).replace(/\/(32|128)$/, '')], '@'),
          database: str(r[db]),
          seconds: num(r[secs]),
          state: join([str(r[state]), wait && `waiting on ${wait}`, kind !== 'client backend' ? kind : ''], ' · '),
          query: hidden ? '' : str(r[query]),
          ...(hidden ? { hidden } : {}),
        };
      });
  }
  if (engine === 'mongodb') {
    const [opid, user, client, ns, op, secs, lock, desc, cmd] = pick([
      'opid', 'user', 'client', 'ns', 'op', 'secs_running', 'waiting_for_lock', 'desc', 'command',
    ]);
    return rows.map((r) => ({
      id: str(r[opid]),
      user: join([str(r[user]), str(r[client])], ' · '),
      database: str(r[ns]),
      seconds: num(r[secs]),
      state: join([str(r[op]), str(r[desc]), r[lock] === true ? 'waiting for a lock' : ''], ' · '),
      query: str(r[cmd]),
    }));
  }
  const [qid, user, elapsed, read, , query] = pick(['query_id', 'user', 'elapsed', 'read_rows', 'memory_usage', 'query']);
  return rows.map((r) => ({
    id: str(r[qid]),
    user: str(r[user]),
    seconds: num(r[elapsed]),
    state: r[read] === undefined ? '' : `${str(r[read])} rows read`,
    query: str(r[query]),
  }));
}

/** `query` stops the statement; `connection` (PostgreSQL only) closes the session. */
export type KillAction = 'query' | 'connection';

export interface KillChoice {
  action: KillAction;
  /** Button label. */
  label: string;
  /** Button tooltip / confirm title. */
  title: string;
  /** What the confirm says will happen. */
  effect: string;
}

/** The Kill buttons a row offers, the default (least disruptive) first. */
export function killChoices(engine: ProcessEngine): KillChoice[] {
  if (engine === 'postgres') {
    return [
      { action: 'query', label: 'Cancel', title: 'Cancel query', effect: 'Cancel the statement this session is running (pg_cancel_backend). The session stays connected.' },
      { action: 'connection', label: 'Terminate', title: 'Terminate connection', effect: 'Close this session (pg_terminate_backend). Its open transaction is rolled back and the client is disconnected.' },
    ];
  }
  if (engine === 'mongodb') {
    return [{ action: 'query', label: 'Kill', title: 'Kill operation', effect: 'Stop this operation (killOp).' }];
  }
  return [{ action: 'query', label: 'Kill', title: 'Kill query', effect: 'Stop this statement. The connection stays open.' }];
}

const PG_MAX_PID = 2_147_483_647;

/** The statement that stops `p`, or null when its id is not a valid one. */
export function killSql(engine: ProcessEngine, p: Pick<ProcessRow, 'id'>, action: KillAction = 'query'): string | null {
  if (engine === 'mysql') return action === 'query' && /^\d+$/.test(p.id) ? `KILL QUERY ${p.id}` : null;
  if (engine === 'postgres') {
    // A pid is a positive int4: only digits ever reach the statement.
    if (!/^\d{1,10}$/.test(p.id) || Number(p.id) < 1 || Number(p.id) > PG_MAX_PID) return null;
    return `SELECT ${action === 'connection' ? 'pg_terminate_backend' : 'pg_cancel_backend'}(${Number(p.id)})`;
  }
  if (engine === 'mongodb') {
    if (action !== 'query') return null;
    // A mongod opid is a number; a mongos one is "<shard>:<number>" (sent as a string).
    if (/^\d+$/.test(p.id) && Number.isSafeInteger(Number(p.id))) return JSON.stringify({ killOp: 1, op: Number(p.id) });
    if (/^[A-Za-z0-9_.-]+:\d+$/.test(p.id)) return JSON.stringify({ killOp: 1, op: p.id });
    return null;
  }
  if (!p.id || action !== 'query') return null;
  return `KILL QUERY WHERE query_id = '${p.id.replace(/\\/g, '\\\\').replace(/'/g, "\\'")}' ASYNC`;
}

/** Notices above the table: why some text is hidden, or why the list is partial. */
export function processNotices(engine: ProcessEngine, rows: readonly ProcessRow[], message?: string | null): string[] {
  const out: string[] = [];
  const hidden = rows.filter((r) => r.hidden).length;
  if (engine === 'postgres' && hidden > 0) {
    out.push(
      `Sessions of other roles: ${hidden}. PostgreSQL hides their state and query from this login; it needs pg_read_all_stats (or superuser) to see them.`,
    );
  }
  if (engine === 'mongodb' && message) out.push(message);
  return out;
}

/** A kill the server ran without error but did not carry out, as a sentence (else null). */
export function killNotDone(engine: ProcessEngine, p: Pick<ProcessRow, 'id'>, rows: readonly unknown[][] | undefined): string | null {
  // pg_cancel_backend / pg_terminate_backend return false (with a warning)
  // when the pid is no longer a server process.
  if (engine === 'postgres' && rows?.[0]?.[0] === false) return `PostgreSQL did not signal pid ${p.id}: the session may have ended already.`;
  return null;
}

/** The query as the Kill confirm names it: one line, capped. */
export function queryPreview(q: string, max = 240): string {
  const one = q.replace(/\s+/g, ' ').trim();
  if (!one) return '(no statement — an idle connection)';
  return one.length > max ? `${one.slice(0, max - 1)}…` : one;
}
