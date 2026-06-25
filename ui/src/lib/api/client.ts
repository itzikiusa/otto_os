// Minimal fetch wrapper for /api/v1 + WS URL helper.

import type {
  Problem,
  ImportReq,
  ImportResult,
  NlToSqlReq,
  NlToSqlOutcome,
  ImportSource,
  SourceStatus,
  ImportScanResult,
  ImportCreateReq,
  ImportCreateResult,
} from './types';
import { serviceHealth } from '../stores/serviceHealth.svelte';

export class ApiError extends Error {
  code: string;
  status: number;

  constructor(status: number, problem: Problem) {
    super(problem.message);
    this.code = problem.code;
    this.status = status;
  }
}

/** Infra endpoints whose 5xx means the connected target (Kafka/DB/SSH) is
 *  unreachable, not a git-provider outage — excluded from the outage banner. */
function isInfraPath(path: string): boolean {
  return (
    path.includes('/brokers') ||
    path.includes('/connections') ||
    path.startsWith('/db') ||
    // Swarm planner/recruiter run an LLM agent and return 502 (Error::Upstream)
    // when that agent times out — that is NOT a git-provider outage, so it must
    // not trigger the "remote git provider returned 502" banner.
    (path.includes('/swarm/') && (path.endsWith('/plan') || path.endsWith('/recruit')))
  );
}

export function baseUrl(): string {
  // Native app + remote browser both talk to the daemon. When the SPA is
  // served BY the daemon, same-origin works; the localStorage override is for
  // dev mode (vite on :5173, daemon on :7700).
  return localStorage.getItem('otto_base') ?? defaultBase();
}

function defaultBase(): string {
  if (location.port === '5173' || location.protocol === 'tauri:' || location.protocol === 'file:') {
    return 'http://127.0.0.1:7700';
  }
  return location.origin;
}

export function getToken(): string | null {
  return localStorage.getItem('otto_token');
}

export function setToken(token: string | null): void {
  if (token === null) localStorage.removeItem('otto_token');
  else localStorage.setItem('otto_token', token);
}

async function request<T>(
  method: string,
  path: string,
  body?: unknown,
  signal?: AbortSignal,
): Promise<T> {
  const headers: Record<string, string> = {};
  const token = getToken();
  if (token) headers['Authorization'] = `Bearer ${token}`;
  if (body !== undefined) headers['Content-Type'] = 'application/json';

  const resp = await fetch(`${baseUrl()}/api/v1${path}`, {
    method,
    headers,
    body: body === undefined ? undefined : JSON.stringify(body),
    signal,
  });

  // Surface git-provider/upstream outages (daemon maps provider failures to a
  // 502). Skip infra endpoints (Kafka brokers, DB connections, DB Explorer):
  // a 5xx there means *that* target is unreachable — shown as a real error
  // toast by the caller — not a Bitbucket/GitHub outage, so the global banner
  // would be misleading.
  if (!isInfraPath(path)) serviceHealth.report(resp.status);

  // No-content responses: 204, async-accepted 202, or any explicitly empty body.
  // (Avoids resp.json() throwing on bodyless endpoints like rewrite/testcase-generate.)
  if (resp.status === 204 || resp.status === 202 || resp.headers.get('content-length') === '0') {
    return undefined as T;
  }

  if (!resp.ok) {
    let problem: Problem = { code: 'internal', message: resp.statusText };
    try {
      problem = await resp.json();
    } catch {
      // non-JSON error body — keep statusText
    }
    throw new ApiError(resp.status, problem);
  }

  return (await resp.json()) as T;
}

export const api = {
  get: <T>(path: string, signal?: AbortSignal) => request<T>('GET', path, undefined, signal),
  post: <T>(path: string, body?: unknown, signal?: AbortSignal) =>
    request<T>('POST', path, body, signal),
  patch: <T>(path: string, body?: unknown) => request<T>('PATCH', path, body),
  put: <T>(path: string, body?: unknown) => request<T>('PUT', path, body),
  del: <T>(path: string) => request<T>('DELETE', path),
};

/** True when an error is a fetch abort (caller cancelled via AbortSignal). */
export function isAbortError(e: unknown): boolean {
  return e instanceof DOMException && e.name === 'AbortError';
}

/**
 * Fetch a binary resource from /api/v1<path> with the stored Bearer token,
 * then return a revocable object URL. The caller is responsible for calling
 * URL.revokeObjectURL() when done (e.g. on component unmount).
 */
export async function authedBlobUrl(path: string): Promise<string> {
  const token = getToken();
  const headers: Record<string, string> = token ? { Authorization: `Bearer ${token}` } : {};
  const resp = await fetch(`${baseUrl()}/api/v1${path}`, { headers });
  if (!resp.ok) {
    let problem: Problem = { code: 'internal', message: resp.statusText };
    try { problem = await resp.json(); } catch { /* non-JSON error body */ }
    throw new ApiError(resp.status, problem);
  }
  return URL.createObjectURL(await resp.blob());
}

/**
 * POST a JSON body to /api/v1<path> with the bearer token and return the RAW
 * response body as text. For download/export endpoints that reply with a
 * non-JSON body (e.g. `text/csv`) — which the JSON-parsing `request()` helper
 * cannot read (it would `resp.json()` the CSV and throw a SyntaxError).
 * Mirrors `authedBlobUrl`'s error handling; skips `serviceHealth.report` for the
 * same reason `request()` does on infra paths.
 */
export async function postForText(
  path: string,
  body: unknown,
  signal?: AbortSignal,
): Promise<string> {
  const headers: Record<string, string> = { 'Content-Type': 'application/json' };
  const token = getToken();
  if (token) headers['Authorization'] = `Bearer ${token}`;
  const resp = await fetch(`${baseUrl()}/api/v1${path}`, {
    method: 'POST',
    headers,
    body: JSON.stringify(body),
    signal,
  });
  if (!resp.ok) {
    let problem: Problem = { code: 'internal', message: resp.statusText };
    try {
      problem = await resp.json();
    } catch {
      // non-JSON error body — keep statusText
    }
    throw new ApiError(resp.status, problem);
  }
  return resp.text();
}

/**
 * POST a JSON body to /api/v1<path> and read a streamed NDJSON response,
 * invoking `onLine` for each parsed JSON line as it arrives. For long-running
 * endpoints that emit incremental progress (e.g. the streaming DB export) so the
 * caller can drive a progress bar and the connection never idles out. Mirrors
 * `postForText`'s auth + error handling.
 */
export async function postNdjsonStream(
  path: string,
  body: unknown,
  onLine: (obj: unknown) => void,
  signal?: AbortSignal,
): Promise<void> {
  const headers: Record<string, string> = { 'Content-Type': 'application/json' };
  const token = getToken();
  if (token) headers['Authorization'] = `Bearer ${token}`;
  const resp = await fetch(`${baseUrl()}/api/v1${path}`, {
    method: 'POST',
    headers,
    body: JSON.stringify(body),
    signal,
  });
  if (!resp.ok || !resp.body) {
    let problem: Problem = { code: 'internal', message: resp.statusText };
    try {
      problem = await resp.json();
    } catch {
      // non-JSON error body — keep statusText
    }
    throw new ApiError(resp.status, problem);
  }
  const reader = resp.body.getReader();
  const decoder = new TextDecoder();
  let buf = '';
  const drain = (chunk: string): void => {
    buf += chunk;
    let nl: number;
    while ((nl = buf.indexOf('\n')) >= 0) {
      const line = buf.slice(0, nl).trim();
      buf = buf.slice(nl + 1);
      if (line) onLine(JSON.parse(line) as unknown);
    }
  };
  for (;;) {
    const { value, done } = await reader.read();
    if (done) break;
    drain(decoder.decode(value, { stream: true }));
  }
  const tail = buf.trim();
  if (tail) onLine(JSON.parse(tail) as unknown);
}

/**
 * Import a local file into a SQL table (`POST …/db/import`). Reads the same
 * streamed NDJSON the export uses; `onLine` fires for each line and the promise
 * resolves with the final `{done…}` / `{error}` line so the caller can drive a
 * progress UI and handle the guarded-write retry. The server emits one final
 * line in v1 (run-to-completion), but the streaming reader is future-proof.
 */
export async function dbImport(
  connId: string,
  body: ImportReq,
  onLine?: (line: ImportResult) => void,
): Promise<ImportResult> {
  let last: ImportResult = {};
  await postNdjsonStream(`/connections/${connId}/db/import`, body, (msg) => {
    const line = msg as ImportResult;
    last = line;
    onLine?.(line);
  });
  return last;
}

/**
 * Draft a verified read query from natural language (`POST …/db/nl-to-sql`).
 * Plain JSON in/out; the server returns only an `EXPLAIN`-validated read. A 400
 * Problem surfaces as an {@link ApiError} the caller inspects (`.message` starts
 * with "NL-to-SQL is not configured" when no drafter is wired, or "could not
 * produce a valid read query" when the loop was exhausted).
 */
export function dbNlToSql(connId: string, body: NlToSqlReq): Promise<NlToSqlOutcome> {
  return api.post<NlToSqlOutcome>(`/connections/${connId}/db/nl-to-sql`, body);
}

// --- Import connections from other DB tools ---------------------------------
//
// The daemon runs locally and reads each tool's config from its default macOS
// location — the user picks a tool, never a file. Editor-gated; created
// connections always use `secret:null` (passwords are unrecoverable from the
// source tools — the user adds them later via edit). `wsId` authorizes the
// caller; created connections are global.

/** Detect which DB tools have importable configs (GET …/import/sources). */
export function importSources(wsId: string): Promise<SourceStatus[]> {
  return api.get<SourceStatus[]>(`/workspaces/${wsId}/connections/import/sources`);
}

/** Read + parse one tool's default config into ParsedConnections (POST …/import/scan). */
export function importScan(wsId: string, source: ImportSource): Promise<ImportScanResult> {
  return api.post<ImportScanResult>(`/workspaces/${wsId}/connections/import/scan`, { source });
}

/** Best-effort batch-create the chosen connections (POST …/import/create). */
export function importCreate(wsId: string, body: ImportCreateReq): Promise<ImportCreateResult> {
  return api.post<ImportCreateResult>(`/workspaces/${wsId}/connections/import/create`, body);
}

/** Build a WS URL with the auth token, e.g. wsUrl('/ws/term/SESSION_ID'). */
export function wsUrl(path: string): string {
  const base = new URL(baseUrl());
  const proto = base.protocol === 'https:' ? 'wss:' : 'ws:';
  const token = getToken() ?? '';
  return `${proto}//${base.host}${path}?token=${encodeURIComponent(token)}`;
}

/** Fixed first subprotocol paired with the bearer token on auth-by-subprotocol
 *  WebSockets (keeps the token out of the URL/query string). The server reads
 *  the token from `Sec-WebSocket-Protocol` and echoes this marker back. */
export const WS_BEARER_SUBPROTOCOL = 'otto-bearer';

/**
 * Open a WebSocket whose bearer token travels in the `Sec-WebSocket-Protocol`
 * header instead of the `?token=` query string — the URL (and the token) then
 * never lands in access logs. The browser offers `[WS_BEARER_SUBPROTOCOL, token]`;
 * the daemon validates the token and echoes `WS_BEARER_SUBPROTOCOL` back.
 *
 * Tokens are not valid `Sec-WebSocket-Protocol` values if they contain spaces or
 * other separators; Otto tokens are URL-safe so this is fine. When no token is
 * stored we open without a subprotocol (the server will 401).
 */
export function wsConnect(path: string): WebSocket {
  const base = new URL(baseUrl());
  const proto = base.protocol === 'https:' ? 'wss:' : 'ws:';
  const url = `${proto}//${base.host}${path}`;
  const token = getToken();
  return token
    ? new WebSocket(url, [WS_BEARER_SUBPROTOCOL, token])
    : new WebSocket(url);
}
