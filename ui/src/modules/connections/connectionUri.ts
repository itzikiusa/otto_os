// Paste-a-URI import for the New connection form (ConnectionForm.svelte):
// a standard connection URI (mysql://, postgres://, redis://, mongodb://,
// clickhouse://, ssh://) read into the form's fields. Pure — unit-tested in
// ui/unit/connectionUri.test.ts.

import type { ConnectionKind } from '../../lib/api/types';

/** `decodeURIComponent` that never throws: a malformed escape (`%zz`) keeps
 *  the raw text instead of aborting the import with an uncaught error. */
function safeDecode(v: string): string {
  try {
    return decodeURIComponent(v);
  } catch {
    return v;
  }
}

/** Split the password out of a URI's RAW text (scheme://user:PASS@hosts/…):
 *  the template keeps every other byte as pasted. `URL.password` can't be
 *  used to find it — WHATWG re-encodes characters like `^ { } | " < > \` and
 *  space, so a replace on its form missed and left the cleartext password in
 *  the visible connection-string field. */
export function splitUriPassword(text: string): { template: string; password: string } | null {
  const start = text.indexOf('://');
  if (start < 0) return null;
  const authStart = start + 3;
  const rest = text.slice(authStart);
  const end = rest.search(/[/?#]/);
  const authority = end < 0 ? rest : rest.slice(0, end);
  let at = authority.lastIndexOf('@');
  if (at < 0 && end >= 0) {
    // An unencoded `/` in the password (`app:12/ss@h/db` — WHATWG parses
    // it as host `app`, port `12`) ends the authority early. The real
    // separator is then the first `@` past that cut which comes before any
    // query: an `@` inside `?appName=a@b` is an option value, not userinfo.
    const query = rest.search(/[?#]/);
    const later = rest.indexOf('@', end);
    if (later >= 0 && (query < 0 || later < query)) at = later;
  }
  if (at < 0) return null;
  const userinfo = rest.slice(0, at);
  const colon = userinfo.indexOf(':');
  if (colon < 0) return null;
  return {
    template: text.slice(0, authStart) + userinfo.slice(0, colon) + ':{secret}' + text.slice(authStart + at),
    password: safeDecode(userinfo.slice(colon + 1)),
  };
}

const KIND_OF_SCHEME: Record<string, ConnectionKind> = {
  mysql: 'mysql',
  'mysql+tcp': 'mysql',
  postgres: 'postgres',
  postgresql: 'postgres',
  'postgres+tcp': 'postgres',
  redis: 'redis',
  rediss: 'redis',
  mongodb: 'mongodb',
  'mongodb+srv': 'mongodb',
  clickhouse: 'clickhouse',
  'clickhouse+https': 'clickhouse',
  'clickhouse+http': 'clickhouse',
  ssh: 'ssh',
};

/** What a URI fills in. MongoDB keeps its whole string (`connString`, the
 *  password moved to `secret`); the others fill host / port / user / db. */
export interface ParsedUri {
  kind: ConnectionKind;
  host?: string;
  port?: string;
  user?: string;
  secret?: string;
  db?: string;
  connString?: string;
  tlsMode?: 'required' | 'disabled';
  tlsVerify?: boolean;
  /** Query options the form has no field for (the person reviews them). */
  ignored: string[];
  /** A suggested name: "host/db" or "host". */
  name?: string;
}

/** Read a pasted URI; `{ error }` says why it couldn't (shown inline). */
export function parseConnectionUri(raw: string): ParsedUri | { error: string } | null {
  const s = raw.trim();
  if (!s) return null;
  let url: URL;
  try {
    url = new URL(s);
  } catch {
    // The usual cause with credentials: an unencoded `/ ? # @` in the
    // password cuts the authority short (`app:pa/ss@h` → port `pa`).
    return {
      error: s.includes('@')
        ? 'Couldn’t read this connection URL. Percent-encode / ? # @ in the password (for example %2F for /).'
        : 'Couldn’t read this connection URL. Start it with a scheme, for example mysql://host:3306/db.',
    };
  }
  const scheme = url.protocol.replace(':', '').toLowerCase();
  const kind = KIND_OF_SCHEME[scheme];
  if (!kind) {
    return { error: `Couldn’t read this connection URL: “${scheme}://” isn’t a scheme Otto knows. Use mysql, postgres, redis, mongodb, clickhouse or ssh.` };
  }
  const out: ParsedUri = { kind, ignored: [] };
  const dbPart = url.pathname.replace(/^\//, '').split('/')[0];
  if (kind === 'mongodb') {
    // Keep topology/options intact but move credentials to the secret field.
    const split = splitUriPassword(s);
    out.secret = split?.password ?? '';
    out.connString = split ? split.template : s;
  } else {
    if (url.hostname) out.host = url.hostname;
    if (url.port) out.port = url.port;
    if (url.username) out.user = safeDecode(url.username);
    if (url.password) out.secret = safeDecode(url.password);
    // The first path segment is the database/db-index.
    if (dbPart) out.db = safeDecode(dbPart);
  }
  if (scheme === 'rediss' || scheme === 'clickhouse+https') out.tlsMode = 'required';
  const ssl = url.searchParams.get('sslmode') ?? url.searchParams.get('tls') ?? url.searchParams.get('ssl');
  if (ssl && ['true', '1', 'require', 'verify-ca', 'verify-full'].includes(ssl)) out.tlsMode = 'required';
  if (ssl && ['false', '0', 'disable'].includes(ssl)) out.tlsMode = 'disabled';
  if (url.searchParams.get('tlsAllowInvalidCertificates') === 'true') out.tlsVerify = false;
  if (kind !== 'mongodb') {
    const supported = new Set(['sslmode', 'tls', 'ssl', 'tlsAllowInvalidCertificates']);
    out.ignored = [...url.searchParams.keys()].filter((key) => !supported.has(key));
  }
  if (url.hostname) out.name = dbPart ? `${url.hostname}/${dbPart}` : url.hostname;
  return out;
}
