// Whether a profile's terminal client gets its password on the command line
// (visible to every local process via `ps` while connected). Mirrors the
// daemon's rule (otto-connections builders.rs, its `warn_argv`): ClickHouse
// whenever a password is stored (clickhouse-client has no other channel),
// MongoDB when the URI carries one (mongosh takes the URI as an argument).
// The connection Test reports the daemon's own verdict (`warn_argv`); this is
// the form's answer before (or without) a Test. Import-free for unit tests.

export type ArgvExposure = 'clickhouse' | 'mongodb' | 'other' | null;

export function argvExposure(kind: string, connString: string, hasSecret: boolean): ArgvExposure {
  if (kind === 'clickhouse') return hasSecret ? 'clickhouse' : null;
  if (kind === 'mongodb') {
    // `{secret}` is the stored password spliced in; `user:pw@` an inline one.
    return /\{secret\}|^mongodb(?:\+srv)?:\/\/[^/@?]*:[^/@?]+@/i.test(connString.trim()) ? 'mongodb' : null;
  }
  return null;
}

/** The form's warning. A Test response that carries `warn_argv` is the
 *  daemon's verdict for exactly the tested values and wins, true OR false;
 *  without one (the driver-backed DB test has none) the local rule decides. */
export function argvWarningFor(kind: string, verdict: boolean | undefined, connString: string, hasSecret: boolean): string | null {
  if (verdict === undefined) return argvWarning(argvExposure(kind, connString, hasSecret));
  if (!verdict) return null;
  return argvWarning(kind === 'clickhouse' || kind === 'mongodb' ? kind : 'other');
}

/** The warning line for an exposure (null when there is none). */
export function argvWarning(e: ArgvExposure): string | null {
  if (e === 'clickhouse') return 'clickhouse-client only accepts the password on its command line — other processes on the machine running it can see it (ps) while connected.';
  if (e === 'other') return 'The client gets the password on its command line — other processes on the machine running it can see it (ps) while connected.';
  if (e === 'mongodb') return 'mongosh takes the connection string, password included, on its command line — other processes on the machine running it can see it (ps) while the terminal is open. The Database Explorer is not affected.';
  return null;
}
