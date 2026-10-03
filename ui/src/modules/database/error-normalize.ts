// Database Explorer error normaliser — turns an engine's raw error string into
// a short headline, a likely cause, a fix hint, "did you mean" suggestions and
// a statement position. The driver side (crates/otto-dbviewer/src/errors.rs)
// already cleaned the text (no ClickHouse stack trace / version banner, no Mongo
// `labels: {} … server response` dump) and appended TAGGED TRAILER LINES —
// `DETAIL:`, `HINT:`, `SQLSTATE:`, `POSITION:`, `ERRNO:`, `CODE:`,
// `STREAMED_ROWS:`, `SUGGEST:` — that this module reads. Older daemons send the
// legacy shapes (`error returned from database: 1054 (42S22): …`, `Kind: …,
// labels: {}`); those parse too. The raw text is always kept for "Show full
// error" and "Ask AI to fix". Pure (no stores) so it unit-tests under node.

export type DbErrorKind =
  | 'syntax'
  | 'unknown_column'
  | 'unknown_table'
  | 'unknown_name'
  | 'duplicate'
  | 'auth'
  | 'permission'
  | 'network'
  | 'timeout'
  | 'memory'
  | 'readonly'
  | 'busy'
  | 'script'
  | 'other';

export interface DbErrorPosition {
  /** 1-based line in the statement. */
  line: number;
  /** 1-based column, when the engine reports one. */
  col?: number;
}

export interface NormalizedDbError {
  /** One line: what failed. */
  title: string;
  /** The likely cause / server detail (may be absent). */
  cause?: string;
  /** What to do about it (may be absent). */
  hint?: string;
  /** Short chip text: `Code 60 · UNKNOWN_TABLE`, `Error 1054 · 42S22`, `42703`. */
  code?: string;
  kind: DbErrorKind;
  /** The unknown name the suggestions would replace. */
  token?: string;
  /** Nearest valid names (server's "maybe you meant", the daemon's completion
   *  cache via `SUGGEST:`, or a static operator/command list). */
  suggestions: string[];
  position?: DbErrorPosition;
  /** Rows that had already streamed when the query failed (ClickHouse). */
  streamedRows?: number | 'unknown';
  /** The untouched input. */
  raw: string;
}

const TAGS = ['DETAIL', 'HINT', 'SQLSTATE', 'POSITION', 'ERRNO', 'CODE', 'STREAMED_ROWS', 'SUGGEST'] as const;
type Tag = (typeof TAGS)[number];

/** Split the cleaned message from its tagged trailer lines. */
export function parseTrailers(text: string): { message: string; tags: Partial<Record<Tag, string>> } {
  const tags: Partial<Record<Tag, string>> = {};
  const keep: string[] = [];
  for (const line of text.split('\n')) {
    const m = line.match(/^([A-Z_]+): (.*)$/);
    if (m && (TAGS as readonly string[]).includes(m[1]) && !(m[1] in tags)) {
      tags[m[1] as Tag] = m[2].trim();
    } else {
      keep.push(line);
    }
  }
  return { message: keep.join('\n').trim(), tags };
}

const ENGINE_NAME: Record<string, string> = {
  mysql: 'MySQL',
  postgres: 'Postgres',
  clickhouse: 'ClickHouse',
  mongodb: 'MongoDB',
  redis: 'Redis',
};

function engineName(engine: string | null): string {
  return (engine && ENGINE_NAME[engine]) || 'the database';
}

/** Trim to one readable line (first sentence-ish, ≤ 160 chars). */
function oneLine(s: string): string {
  const first = s.split('\n').find((l) => l.trim())?.trim() ?? '';
  return first.length > 160 ? `${first.slice(0, 157)}…` : first;
}

function capital(s: string): string {
  return s ? s.charAt(0).toUpperCase() + s.slice(1) : s;
}

function fmtCount(n: number): string {
  return n.toLocaleString('en-US');
}

/** 1-based character offset (Postgres POSITION) → line/col in `statement`. */
export function offsetToPosition(statement: string, offset: number): DbErrorPosition | undefined {
  if (!statement || !Number.isFinite(offset) || offset < 1) return undefined;
  const chars = Array.from(statement);
  if (offset > chars.length + 1) return undefined;
  let line = 1;
  let col = 1;
  for (let i = 0; i < offset - 1; i++) {
    if (chars[i] === '\n') {
      line++;
      col = 1;
    } else {
      col++;
    }
  }
  return { line, col };
}

/** Optimal-string-alignment distance (Levenshtein + adjacent transposition). */
export function editDistance(a: string, b: string): number {
  const x = Array.from(a);
  const y = Array.from(b);
  const d: number[][] = Array.from({ length: x.length + 1 }, (_, i) => [i, ...Array(y.length).fill(0)]);
  for (let j = 0; j <= y.length; j++) d[0][j] = j;
  for (let i = 1; i <= x.length; i++) {
    for (let j = 1; j <= y.length; j++) {
      const cost = x[i - 1] === y[j - 1] ? 0 : 1;
      let v = Math.min(d[i - 1][j] + 1, d[i][j - 1] + 1, d[i - 1][j - 1] + cost);
      if (i > 1 && j > 1 && x[i - 1] === y[j - 2] && x[i - 2] === y[j - 1]) v = Math.min(v, d[i - 2][j - 2] + 1);
      d[i][j] = v;
    }
  }
  return d[x.length][y.length];
}

/** Nearest names to `token` from `candidates` (case-insensitive, distance ≤ 2
 *  and under half the token), best first, at most 3. */
export function nearest(token: string, candidates: readonly string[]): string[] {
  const t = token.toLowerCase();
  const max = Math.max(1, Math.min(2, Math.floor((Array.from(t).length - 1) / 2)));
  return candidates
    .map((c) => ({ c, d: c.toLowerCase() === t ? Infinity : editDistance(t, c.toLowerCase()) }))
    .filter((s) => s.d <= max)
    .sort((a, b) => a.d - b.d)
    .slice(0, 3)
    .map((s) => s.c);
}

// Static vocabularies for engines whose "unknown X" errors name a fixed set.
const MONGO_OPERATORS = [
  '$eq', '$ne', '$gt', '$gte', '$lt', '$lte', '$in', '$nin', '$and', '$or', '$nor', '$not', '$exists',
  '$type', '$expr', '$regex', '$options', '$text', '$where', '$all', '$elemMatch', '$size', '$mod',
  '$set', '$unset', '$inc', '$push', '$pull', '$addToSet', '$pop', '$rename', '$min', '$max', '$mul',
  '$match', '$group', '$project', '$sort', '$limit', '$skip', '$unwind', '$lookup', '$addFields',
  '$count', '$facet', '$replaceRoot', '$sum', '$avg', '$first', '$last', '$geoWithin', '$near',
];
const REDIS_COMMANDS = [
  'GET', 'SET', 'DEL', 'EXISTS', 'EXPIRE', 'TTL', 'PTTL', 'TYPE', 'KEYS', 'SCAN', 'INCR', 'DECR', 'INCRBY',
  'MGET', 'MSET', 'APPEND', 'STRLEN', 'GETSET', 'SETEX', 'SETNX', 'HGET', 'HSET', 'HDEL', 'HGETALL',
  'HKEYS', 'HVALS', 'HLEN', 'HMGET', 'HMSET', 'HEXISTS', 'HINCRBY', 'HSCAN', 'LPUSH', 'RPUSH', 'LPOP',
  'RPOP', 'LRANGE', 'LLEN', 'LINDEX', 'LSET', 'LREM', 'SADD', 'SREM', 'SMEMBERS', 'SISMEMBER', 'SCARD',
  'SSCAN', 'ZADD', 'ZREM', 'ZRANGE', 'ZREVRANGE', 'ZSCORE', 'ZCARD', 'ZRANK', 'ZSCAN', 'ZRANGEBYSCORE',
  'XADD', 'XRANGE', 'XLEN', 'XREAD', 'PING', 'INFO', 'DBSIZE', 'SELECT', 'RENAME', 'PERSIST', 'OBJECT',
  'MEMORY', 'CLIENT', 'CONFIG', 'PUBLISH', 'UNLINK', 'JSON.GET', 'JSON.SET',
];

const NET_HINT = 'Is the server up, or the SSH tunnel for this profile open? Test the connection.';
const TIMEOUT_HINT = 'Raise the ⏱ tab timeout, or narrow the query (a WHERE, a LIMIT, an index).';

/** Pull `host:port` out of a transport error (request URL, Mongo address, …). */
function endpointOf(message: string): string | undefined {
  const url = message.match(/url \((?:https?:\/\/)?([^/)\s?]+)/);
  if (url) return url[1];
  const at = message.match(/\bat ([\w.-]+:\d+)/);
  if (at) return at[1];
  const addr = message.match(/Address: ([\w.-]+:\d+)/);
  return addr?.[1];
}

/** Transport-level failures — the same headline for every engine. */
function networkRule(engine: string | null, message: string): Partial<NormalizedDbError> | null {
  const name = engineName(engine);
  const at = endpointOf(message);
  const where = at ? ` at ${at}` : '';
  if (/pool timed out while waiting for an open connection/i.test(message)) {
    return {
      kind: 'busy',
      title: 'All connections are busy.',
      hint: 'A previous query may still be running — Stop it, or retry in a moment.',
    };
  }
  if (/connection refused/i.test(message)) {
    return { kind: 'network', title: `Can't reach ${name}${where} (connection refused).`, hint: NET_HINT };
  }
  if (/failed to lookup address|dns error|nodename nor servname|name or service not known|no such host/i.test(message)) {
    return { kind: 'network', title: `Can't resolve the ${name} host${where}.`, hint: 'Check the host name in the profile, or your network/VPN.' };
  }
  if (/certificate|tls handshake|InvalidCertificate|establish a TLS connection/i.test(message)) {
    return {
      kind: 'network',
      title: `The TLS handshake with ${name}${where} failed.`,
      cause: oneLine(message),
      hint: 'Check the profile\'s TLS mode and CA — a tunnelled host needs the real host name for the certificate.',
    };
  }
  if (/connection reset|broken pipe|unexpected eof|connection closed|lost connection/i.test(message)) {
    return { kind: 'network', title: `The connection to ${name}${where} dropped.`, hint: 'Retry; if it keeps happening, check the server and the tunnel.' };
  }
  if (/operation timed out|connect timed out|deadline has elapsed|timed out/i.test(message)) {
    return { kind: 'timeout', title: `${capital(name)}${where} didn't answer in time.`, hint: NET_HINT };
  }
  return null;
}

/** Strip the API/sqlx wrappers older daemons leave on. */
function unwrap(raw: string): string {
  let s = raw.trim();
  s = s.replace(/^upstream:\s*/, '');
  s = s.replace(/^error returned from database:\s*/, '');
  return s;
}

// --- per-engine rules --------------------------------------------------------

function clickhouseRule(message: string): Partial<NormalizedDbError> | null {
  const m = message.match(/^Code:\s*(\d+)\.\s*(?:DB::Exception:\s*)*([\s\S]*)$/);
  if (!m) return null;
  const code = Number(m[1]);
  let body = m[2].trim();
  // Legacy daemons: drop the stack trace / version banner here too.
  body = body.split(/\nStack trace:|\n\s*0\.\s/)[0].replace(/\s*\(version [^\n]*\)\s*$/, '').trim();
  const nameM = body.match(/\(([A-Z][A-Z0-9_]+)\)\s*$/);
  const name = nameM?.[1];
  const text = (nameM ? body.slice(0, nameM.index) : body).trim();
  const chip = `Code ${code}${name ? ` · ${name}` : ''}`;
  const out: Partial<NormalizedDbError> = { code: chip };
  // `Maybe you meant: ['name', 'game']` / `Maybe you meant default.users?`
  const maybeList = text.match(/Maybe you meant:?\s*\[([^\]]*)\]/);
  const maybeOne = text.match(/Maybe you meant:?\s*([`'"]?)([\w.]+)\1\?/);
  const suggestions = maybeList
    ? [...maybeList[1].matchAll(/'([^']+)'/g)].map((x) => x[1])
    : maybeOne
      ? [maybeOne[2]]
      : [];
  out.suggestions = suggestions;
  const lc = text.match(/\(line (\d+), col (\d+)\)/);
  if (lc) out.position = { line: Number(lc[1]), col: Number(lc[2]) };

  switch (code) {
    case 60: {
      const t = text.match(/Table ([\w.`]+) does(?:n't| not) exist/)?.[1]?.replace(/`/g, '')
        ?? text.match(/identifier '([^']+)'/)?.[1];
      return { ...out, kind: 'unknown_table', token: t, title: t ? `Table \`${t}\` doesn't exist.` : 'That table doesn\'t exist.' };
    }
    case 81: {
      const db = text.match(/Database ([\w`]+) does(?:n't| not) exist/)?.[1]?.replace(/`/g, '');
      return { ...out, kind: 'unknown_table', title: db ? `Database \`${db}\` doesn't exist.` : 'That database doesn\'t exist.' };
    }
    case 47: {
      const c = text.match(/identifier `([^`]+)`/)?.[1] ?? text.match(/Missing columns: '([^']+)'/)?.[1];
      return { ...out, kind: 'unknown_column', token: c, title: c ? `Unknown column \`${c}\`.` : 'Unknown column.' };
    }
    case 46: {
      const f = text.match(/[Ff]unction (?:with name )?[`']?([\w.]+)[`']?/)?.[1];
      return { ...out, kind: 'unknown_name', token: f, title: f ? `Unknown function \`${f}\`.` : 'Unknown function.' };
    }
    case 62: {
      const near = text.match(/failed at position \d+ \('([^']*)'\)/)?.[1];
      const at = out.position ? ` (line ${out.position.line}, col ${out.position.col})` : '';
      const expected = text.match(/Expected one of: ([^\n]*?)\.?$/)?.[1];
      return {
        ...out,
        kind: 'syntax',
        title: near ? `Syntax error near \`${near}\`${at}.` : `Syntax error${at}.`,
        cause: expected ? `Expected one of: ${expected}` : undefined,
      };
    }
    case 241: {
      const limit = text.match(/maximum: ([\d.]+ \w+)/)?.[1];
      return {
        ...out,
        kind: 'memory',
        title: `Query ran out of memory${limit ? ` (limit ${limit})` : ''}.`,
        cause: oneLine(text),
        hint: 'Aggregate fewer keys, add a WHERE/LIMIT, or use approximate functions (uniq instead of uniqExact).',
      };
    }
    case 159: {
      const secs = text.match(/maximum: ([\d.]+)/)?.[1];
      return {
        ...out,
        kind: 'timeout',
        title: `Stopped after ${secs ? `${Number(secs)} s` : 'the time limit'} (server max_execution_time).`,
        hint: 'Narrow the query, or raise the ⏱ tab timeout.',
      };
    }
    case 516: {
      const user = text.match(/^([\w.@-]+): Authentication failed/)?.[1];
      return {
        ...out,
        kind: 'auth',
        title: `Sign-in to ClickHouse failed${user ? ` for \`${user}\`` : ''}.`,
        hint: 'Check the profile\'s user and password.',
      };
    }
    case 164:
      return { ...out, kind: 'readonly', title: 'This user is read-only on the server.', hint: 'Writes need a different user.' };
    case 497:
      return { ...out, kind: 'permission', title: 'This user isn\'t allowed to do that.', cause: oneLine(text) };
    default:
      return { ...out, kind: 'other', title: capital(oneLine(text.replace(/:\s*While executing.*$/, ''))) };
  }
}

function mysqlRule(message: string, tags: Partial<Record<Tag, string>>): Partial<NormalizedDbError> | null {
  let text = message;
  let errno = tags.ERRNO;
  let state = tags.SQLSTATE;
  // Legacy: `1054 (42S22): Unknown column …`.
  const legacy = text.match(/^(\d{4,5})(?: \(([0-9A-Z]{5})\))?: ([\s\S]*)$/);
  if (legacy) {
    errno ??= legacy[1];
    state ??= legacy[2];
    text = legacy[3];
  }
  if (!errno) return null;
  const out: Partial<NormalizedDbError> = { code: `Error ${errno}${state ? ` · ${state}` : ''}` };
  const lineM = text.match(/\bat line (\d+)\b/);
  if (lineM) out.position = { line: Number(lineM[1]) };
  switch (errno) {
    case '1054': {
      const c = text.match(/Unknown column '([^']+)'/)?.[1];
      return { ...out, kind: 'unknown_column', token: c, title: c ? `Unknown column \`${c}\`.` : 'Unknown column.' };
    }
    case '1146': {
      const t = text.match(/Table '([^']+)' doesn't exist/)?.[1];
      return { ...out, kind: 'unknown_table', token: t, title: t ? `Table \`${t}\` doesn't exist.` : 'That table doesn\'t exist.' };
    }
    case '1049': {
      const db = text.match(/Unknown database '([^']+)'/)?.[1];
      return { ...out, kind: 'unknown_table', title: db ? `Database \`${db}\` doesn't exist.` : 'That database doesn\'t exist.' };
    }
    case '1064': {
      const near = text.match(/near '([\s\S]*?)' at line \d+/)?.[1];
      const short = near && near.length > 40 ? `${near.slice(0, 40)}…` : near;
      const at = out.position ? ` (line ${out.position.line})` : '';
      return { ...out, kind: 'syntax', title: short ? `Syntax error near \`${short}\`${at}.` : `Syntax error${at}.` };
    }
    case '1045': {
      const who = text.match(/for user '([^']*)'@'([^']*)'/);
      return {
        ...out,
        kind: 'auth',
        title: who ? `MySQL rejected user \`${who[1]}\` from ${who[2]}.` : 'MySQL rejected the sign-in.',
        hint: 'Check the password, or the user\'s host grant.',
      };
    }
    case '1044':
    case '1142':
    case '1143':
      return { ...out, kind: 'permission', title: 'This user isn\'t allowed to do that.', cause: text };
    case '1062': {
      const dup = text.match(/Duplicate entry '([\s\S]*)' for key '([^']+)'/);
      return { ...out, kind: 'duplicate', title: dup ? `Duplicate value \`${dup[1]}\` for key \`${dup[2]}\`.` : 'Duplicate value.' };
    }
    case '1290':
    case '1836':
      return { ...out, kind: 'readonly', title: 'The server is read-only.', cause: text, hint: 'Writes need the primary, or a different user.' };
    case '3024':
    case '1317':
    case '1969':
      return { ...out, kind: 'timeout', title: 'Stopped by the tab timeout.', hint: TIMEOUT_HINT };
    case '2006':
    case '2013':
      return { ...out, kind: 'network', title: 'Lost the connection to MySQL mid-query.', hint: NET_HINT };
    default:
      return { ...out, kind: 'other', title: capital(oneLine(text)) };
  }
}

function postgresRule(message: string, tags: Partial<Record<Tag, string>>): Partial<NormalizedDbError> | null {
  const state = tags.SQLSTATE;
  if (!state) return null;
  const out: Partial<NormalizedDbError> = { code: state, cause: tags.DETAIL, hint: tags.HINT };
  const quoted = (re: RegExp) => message.match(re)?.[1];
  // `HINT: Perhaps you meant to reference the column "users.name".`
  const meant = tags.HINT?.match(/Perhaps you meant to reference the (?:column|table) "([^"]+)"/)?.[1];
  switch (state) {
    case '42703': {
      const c = quoted(/column "([^"]+)" does not exist/) ?? quoted(/column ([\w.]+) does not exist/);
      const s = meant ? [c && !c.includes('.') ? meant.split('.').pop()! : meant] : [];
      return {
        ...out,
        kind: 'unknown_column',
        token: c,
        suggestions: s,
        hint: meant ? undefined : tags.HINT,
        title: c ? `Unknown column \`${c}\`.` : 'Unknown column.',
      };
    }
    case '42P01': {
      const t = quoted(/relation "([^"]+)" does not exist/);
      return { ...out, kind: 'unknown_table', token: t, title: t ? `Table \`${t}\` doesn't exist.` : 'That table doesn\'t exist.' };
    }
    case '3D000': {
      const db = quoted(/database "([^"]+)" does not exist/);
      return { ...out, kind: 'unknown_table', title: db ? `Database \`${db}\` doesn't exist.` : 'That database doesn\'t exist.' };
    }
    case '42883': {
      const f = quoted(/function ([\w.]+)\(/);
      return { ...out, kind: 'unknown_name', title: f ? `Unknown function \`${f}\` (for these argument types).` : capital(oneLine(message)) };
    }
    case '42601': {
      const near = quoted(/at or near "([^"]*)"/);
      return { ...out, kind: 'syntax', title: near ? `Syntax error near \`${near}\`.` : capital(oneLine(message)) };
    }
    case '23505': {
      const k = quoted(/unique constraint "([^"]+)"/);
      return { ...out, kind: 'duplicate', title: k ? `Duplicate value for \`${k}\`.` : 'Duplicate value.' };
    }
    case '23503': {
      const k = quoted(/foreign key constraint "([^"]+)"/);
      return { ...out, kind: 'other', title: k ? `Foreign key \`${k}\` blocks this change.` : capital(oneLine(message)) };
    }
    case '23502': {
      const c = quoted(/column "([^"]+)"/);
      return { ...out, kind: 'other', title: c ? `Column \`${c}\` can't be null.` : capital(oneLine(message)) };
    }
    case '28P01':
    case '28000': {
      const u = quoted(/for user "([^"]+)"/);
      return {
        ...out,
        kind: 'auth',
        title: `Sign-in to Postgres failed${u ? ` for \`${u}\`` : ''}.`,
        hint: out.hint ?? 'Check the profile\'s user and password (and pg_hba rules for this host).',
      };
    }
    case '42501':
      return { ...out, kind: 'permission', title: capital(oneLine(message)).replace(/^Permission denied/, 'No permission'), hint: out.hint ?? 'Ask for a GRANT, or use a different user.' };
    case '25006':
      return { ...out, kind: 'readonly', title: 'This connection is read-only.', hint: 'Writes need a different connection or user.' };
    case '57014':
      return { ...out, kind: 'timeout', title: 'Stopped by the statement timeout.', hint: TIMEOUT_HINT };
    case '53300':
      return { ...out, kind: 'busy', title: 'The server has no free connections.', hint: 'Close idle sessions, or retry in a moment.' };
    default:
      return { ...out, kind: 'other', title: capital(oneLine(message)) };
  }
}

function mongoRule(message: string, tags: Partial<Record<Tag, string>>): Partial<NormalizedDbError> | null {
  let text = message;
  let code = tags.CODE;
  // Legacy daemon: `Kind: Command failed: Error code 2 (BadValue): msg, labels: {}, …`.
  text = text.replace(/^Kind:\s*/, '').replace(/, labels: \{[\s\S]*$/, '');
  const legacy = text.match(/^Command failed: Error code (\d+) \((\w+)\): ([\s\S]*)$/);
  if (legacy) {
    code ??= `${legacy[1]} ${legacy[2]}`;
    text = legacy[3];
  }
  const legacyWrite = text.match(/WriteError\(WriteError \{ code: (\d+),[^}]*?message: "((?:[^"\\]|\\.)*)"/);
  if (legacyWrite) {
    code ??= legacyWrite[1];
    text = legacyWrite[2].replace(/\\"/g, '"');
  }
  const exit = text.match(/^mongosh exited (\S+(?: by signal)?)\n?([\s\S]*)$/);
  if (exit) {
    const lines = exit[2].split('\n').map((l) => l.trim()).filter(Boolean);
    const errLine = [...lines].reverse().find((l) => /\b(Mongo\w*Error|SyntaxError|TypeError|ReferenceError|Error)\b/.test(l));
    return {
      kind: 'script',
      title: `Script failed (exit ${exit[1]}).`,
      cause: errLine ? oneLine(errLine) : undefined,
      hint: 'The full mongosh output is under "Show full error".',
    };
  }
  const out: Partial<NormalizedDbError> = { code };
  const op = text.match(/unknown (?:top level )?operator:? (\$\w+)/i)?.[1];
  if (op) {
    return { ...out, kind: 'unknown_name', token: op, suggestions: nearest(op, MONGO_OPERATORS), title: `Unknown operator \`${op}\`.` };
  }
  const dup = text.match(/E11000 duplicate key error collection: (\S+) index: (\S+) dup key: (\{[\s\S]*\})/);
  if (dup) {
    return {
      ...out,
      kind: 'duplicate',
      title: `Duplicate key on index \`${dup[2]}\`.`,
      cause: `${dup[3].trim()} already exists in ${dup[1]}.`,
    };
  }
  const reach = text.match(/^Can't reach MongoDB(?: at (\S+?))?: ([\s\S]*)$/);
  if (reach) {
    const why = /connection refused/i.test(reach[2]) ? ' (connection refused)' : '';
    return { ...out, kind: 'network', title: `Can't reach MongoDB${reach[1] ? ` at ${reach[1]}` : ''}${why}.`, cause: why ? undefined : reach[2], hint: NET_HINT };
  }
  if (/Server selection timeout/i.test(text)) {
    return { ...out, kind: 'network', title: 'Can\'t reach MongoDB.', hint: NET_HINT };
  }
  if (/auth(entication)? failed|AuthenticationFailed|SCRAM failure/i.test(text)) {
    return { ...out, kind: 'auth', title: 'Sign-in to MongoDB failed.', cause: oneLine(text), hint: 'Check the profile\'s user, password and auth database.' };
  }
  if (/not authorized on/i.test(text)) {
    return { ...out, kind: 'permission', title: 'This user isn\'t allowed to do that.', cause: oneLine(text) };
  }
  if (/\bns (does )?not (found|exist)/i.test(text)) {
    return { ...out, kind: 'unknown_table', title: 'That collection doesn\'t exist.' };
  }
  if (/operation exceeded time limit|MaxTimeMSExpired/i.test(text)) {
    return { ...out, kind: 'timeout', title: 'Stopped by the time limit.', hint: TIMEOUT_HINT };
  }
  if (!code && text === message) return null;
  return { ...out, kind: 'other', title: capital(oneLine(text)) };
}

function redisRule(message: string): Partial<NormalizedDbError> | null {
  const text = message.replace(/^An error was signalled by the server - /, '').replace(/^ResponseError: /, '');
  if (/^WRONGTYPE\b/.test(text)) {
    return { kind: 'other', code: 'WRONGTYPE', title: 'Key holds a different type.', hint: 'Run TYPE <key> first, then use that type\'s commands.' };
  }
  const cmd = text.match(/unknown command [`']([^`']+)[`']/i)?.[1];
  if (cmd) {
    return { kind: 'unknown_name', token: cmd, suggestions: nearest(cmd.toUpperCase(), REDIS_COMMANDS), title: `Unknown command \`${cmd}\`.` };
  }
  if (/^(NOAUTH|WRONGPASS)\b|invalid username-password|AuthenticationFailed/i.test(text)) {
    return { kind: 'auth', code: text.match(/^(NOAUTH|WRONGPASS)/)?.[1], title: 'Redis sign-in failed.', hint: 'Check the profile\'s password (and ACL user).' };
  }
  if (/^NOPERM\b/.test(text)) {
    return { kind: 'permission', code: 'NOPERM', title: 'This user isn\'t allowed to run that command.', cause: oneLine(text) };
  }
  if (/^READONLY\b/.test(text)) {
    return { kind: 'readonly', code: 'READONLY', title: 'This is a read-only replica.', hint: 'Writes need the primary.' };
  }
  if (/wrong number of arguments/i.test(text)) {
    return { kind: 'syntax', title: capital(oneLine(text)), hint: 'Check the command\'s arguments.' };
  }
  return null;
}

/**
 * Normalise a Database Explorer error for display. `engine` is the active
 * engine (`mysql`/`postgres`/`clickhouse`/`mongodb`/`redis`, or null for an
 * unknown one); `statement` is the statement that failed (for POSITION → caret).
 */
export function normalizeDbError(engine: string | null, raw: string, statement = ''): NormalizedDbError {
  const { message, tags } = parseTrailers(unwrap(raw));
  let rule: Partial<NormalizedDbError> | null = null;
  switch (engine) {
    case 'clickhouse':
      rule = clickhouseRule(message);
      break;
    case 'mysql':
      rule = mysqlRule(message, tags);
      break;
    case 'postgres':
      rule = postgresRule(message, tags);
      break;
    case 'mongodb':
      rule = mongoRule(message, tags);
      break;
    case 'redis':
      rule = redisRule(message);
      break;
    default:
      // Unknown engine: try the shapes that identify themselves.
      rule = clickhouseRule(message) ?? (tags.SQLSTATE && !tags.ERRNO ? postgresRule(message, tags) : mysqlRule(message, tags));
  }
  // Transport failures read the same everywhere; an engine rule with a real
  // code wins (ClickHouse's own TIMEOUT_EXCEEDED is not a network timeout).
  if (!rule || (rule.kind === 'other' && !rule.code)) rule = networkRule(engine, message) ?? rule;

  const out: NormalizedDbError = {
    title: rule?.title || capital(oneLine(message)) || 'Query failed.',
    cause: rule?.cause,
    hint: rule?.hint,
    code: rule?.code,
    kind: rule?.kind ?? 'other',
    token: rule?.token,
    suggestions: [...(rule?.suggestions ?? [])],
    position: rule?.position,
    raw,
  };
  // Postgres POSITION (1-based char offset into what ran) → line/col.
  if (!out.position && tags.POSITION) out.position = offsetToPosition(statement, Number(tags.POSITION));
  // The daemon's completion-cache suggestions (E8).
  for (const s of (tags.SUGGEST ?? '').split(',').map((x) => x.trim()).filter(Boolean)) {
    if (!out.suggestions.includes(s)) out.suggestions.push(s);
  }
  if (!out.token) out.suggestions = [];
  // A position past the statement (a rewritten/limited statement) shows no caret.
  if (out.position && statement) {
    const lines = statement.split('\n');
    if (out.position.line > lines.length) out.position = undefined;
  }
  if (tags.STREAMED_ROWS) {
    const n = Number(tags.STREAMED_ROWS.replace(/,/g, ''));
    out.streamedRows = Number.isFinite(n) ? n : 'unknown';
    const after = typeof out.streamedRows === 'number' ? `after ${fmtCount(out.streamedRows)} rows had streamed` : 'after some rows had streamed';
    const note = `The query failed ${after} — those partial rows are not shown as a result.`;
    out.cause = out.cause ? `${note} ${out.cause}` : note;
  }
  return out;
}

/** Replace the unknown `token` with `suggestion` in `statement`, whole-word.
 *  A qualified token/suggestion (`default.userz` → `default.users`) falls back
 *  to its last segment when only that appears. `null` when it isn't there. */
export function applySuggestion(statement: string, token: string, suggestion: string): string | null {
  const esc = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const tryReplace = (from: string, to: string): string | null => {
    const re = new RegExp(`(^|[^\\w$])${esc(from)}(?![\\w$])`, 'g');
    if (!re.test(statement)) return null;
    return statement.replace(re, (_m, pre: string) => `${pre}${to}`);
  };
  const direct = tryReplace(token, suggestion);
  if (direct !== null) return direct;
  const last = (s: string) => s.split('.').pop() ?? s;
  if (token.includes('.') || suggestion.includes('.')) return tryReplace(last(token), last(suggestion));
  return null;
}
