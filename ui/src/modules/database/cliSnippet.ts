// Pure CLI snippet generator for Database Explorer / Connections ("Copy as CLI").
// Emits copy-pasteable `psql`, `mysql`, `mongosh`, `redis-cli`, `clickhouse-client`,
// or `ssh` commands using POSIX single-quoting and NEVER embeds real passwords or
// Keychain secrets (credentials use safe env-var or prompt placeholders).
import type { ConnectionKind } from '../../lib/api/types';

export interface CliConnLike {
  name?: string;
  kind: ConnectionKind;
  params?: Record<string, unknown> | null;
}

/** Minimal POSIX single-quote escaping: leaves simple safe tokens unquoted. */
export function shQuote(s: string): string {
  if (/^[A-Za-z0-9_./:@%+=,-]+$/.test(s)) return s;
  return `'${s.replace(/'/g, `'\\''`)}'`;
}

/** Shell-quote a filesystem path while preserving leading `~/` tilde expansion. */
function shQuotePath(s: string): string {
  if (s.startsWith('~/')) return `~/${shQuote(s.slice(2))}`;
  return shQuote(s);
}

/** Strip inline `user:password@` credentials from a connection URI (e.g. MongoDB)
 *  so a copied CLI snippet can never leak an embedded password. */
export function redactUriPassword(uri: string): string {
  return uri.replace(/^([a-z][a-z0-9+.-]*:\/\/)([^/\s:@]+):([^/\s@]+)@/i, '$1$2:<password>@');
}

function strParam(p: Record<string, unknown>, ...keys: string[]): string {
  for (const k of keys) {
    const v = p[k];
    if (typeof v === 'string' && v.trim()) return v.trim();
  }
  return '';
}

function portParam(p: Record<string, unknown>): number | null {
  const raw = p.port;
  const n = typeof raw === 'number' ? raw : typeof raw === 'string' && raw.trim() ? Number(raw) : NaN;
  return Number.isInteger(n) && n > 0 && n <= 65535 ? n : null;
}

function tlsRequired(p: Record<string, unknown>): boolean {
  if (p.secure === true) return true;
  const tls = p.tls as Record<string, unknown> | undefined;
  return !!tls && (tls.mode === 'required' || tls.mode === 'preferred');
}

/** Human-friendly label for the CLI binary (for menu items & toasts). */
export function cliLabelForKind(kind: ConnectionKind): string | null {
  switch (kind) {
    case 'postgres': return 'psql';
    case 'mysql': return 'mysql';
    case 'mongodb': return 'mongosh';
    case 'redis': return 'redis-cli';
    case 'clickhouse': return 'clickhouse-client';
    case 'ssh': return 'ssh';
    default: return null;
  }
}

/** Build a secret-free CLI invocation for a saved connection (and optional
 *  active database override), or `null` when the connection lacks required host/URI
 *  fields or uses a custom command template. */
export function buildDbCliSnippet(
  conn: CliConnLike,
  opts: { activeDb?: string | null } = {},
): string | null {
  const p = conn.params ?? {};
  const host = strParam(p, 'host');
  const port = portParam(p);
  const user = strParam(p, 'user', 'username');
  const db = (opts.activeDb ?? '').trim() || strParam(p, 'db', 'database');
  const tls = tlsRequired(p);

  switch (conn.kind) {
    case 'postgres': {
      if (!host) return null;
      const parts = ['psql', '-h', shQuote(host)];
      if (port && port !== 5432) parts.push('-p', String(port));
      if (user) parts.push('-U', shQuote(user));
      if (db) parts.push('-d', shQuote(db));
      const prefix = tls ? 'PGSSLMODE=require ' : '';
      return `${prefix}${parts.join(' ')}`;
    }
    case 'mysql': {
      if (!host) return null;
      const parts = ['mysql', '-h', shQuote(host)];
      if (port && port !== 3306) parts.push('-P', String(port));
      if (user) parts.push('-u', shQuote(user));
      parts.push('-p');
      if (tls) parts.push('--ssl-mode=REQUIRED');
      if (db) parts.push(shQuote(db));
      return parts.join(' ');
    }
    case 'mongodb': {
      const uri = strParam(p, 'conn_string', 'uri');
      if (uri) return `mongosh ${shQuote(redactUriPassword(uri))}`;
      if (!host) return null;
      const parts = ['mongosh', '--host', shQuote(host)];
      if (port && port !== 27017) parts.push('--port', String(port));
      if (user) parts.push('--username', shQuote(user), '--password');
      if (tls) parts.push('--tls');
      if (db) parts.push(shQuote(db));
      return parts.join(' ');
    }
    case 'redis': {
      if (!host) return null;
      const parts = ['redis-cli', '-h', shQuote(host)];
      if (port && port !== 6379) parts.push('-p', String(port));
      if (user) parts.push('--user', shQuote(user));
      if (db && /^\d+$/.test(db)) parts.push('-n', db);
      if (tls) parts.push('--tls');
      return parts.join(' ');
    }
    case 'clickhouse': {
      if (!host) return null;
      const parts = ['clickhouse-client', '--host', shQuote(host)];
      if (port && port !== 9000) parts.push('--port', String(port));
      if (user) parts.push('--user', shQuote(user), '--ask-password');
      if (db) parts.push('--database', shQuote(db));
      if (tls) parts.push('--secure');
      return parts.join(' ');
    }
    case 'ssh': {
      if (!host) return null;
      const parts = ['ssh'];
      const jump = strParam(p, 'jump');
      const identity = strParam(p, 'identity_file');
      if (port && port !== 22) parts.push('-p', String(port));
      if (jump) parts.push('-J', shQuote(jump));
      if (identity) parts.push('-i', shQuotePath(identity));
      parts.push(shQuote(user ? `${user}@${host}` : host));
      return parts.join(' ');
    }
    default:
      return null;
  }
}
