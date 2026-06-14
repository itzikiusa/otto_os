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

async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
  const headers: Record<string, string> = {};
  const token = getToken();
  if (token) headers['Authorization'] = `Bearer ${token}`;
  if (body !== undefined) headers['Content-Type'] = 'application/json';

  const resp = await fetch(`${baseUrl()}/api/v1${path}`, {
    method,
    headers,
    body: body === undefined ? undefined : JSON.stringify(body),
  });

  // Surface provider/upstream outages (daemon maps provider failures to 502).
  serviceHealth.report(resp.status);

  if (resp.status === 204) return undefined as T;

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
  get: <T>(path: string) => request<T>('GET', path),
  post: <T>(path: string, body?: unknown) => request<T>('POST', path, body),
  patch: <T>(path: string, body?: unknown) => request<T>('PATCH', path, body),
  put: <T>(path: string, body?: unknown) => request<T>('PUT', path, body),
  del: <T>(path: string) => request<T>('DELETE', path),
};

/** Build a WS URL with the auth token, e.g. wsUrl('/ws/term/SESSION_ID'). */
export function wsUrl(path: string): string {
  const base = new URL(baseUrl());
  const proto = base.protocol === 'https:' ? 'wss:' : 'ws:';
  const token = getToken() ?? '';
  return `${proto}//${base.host}${path}?token=${encodeURIComponent(token)}`;
}
