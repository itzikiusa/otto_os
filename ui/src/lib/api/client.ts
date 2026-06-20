// Minimal fetch wrapper for /api/v1 + WS URL helper.

import type { Problem } from './types';
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
    path.startsWith('/db')
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
