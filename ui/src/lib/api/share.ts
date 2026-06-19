// Scoped API helpers for the share-link guest view (Phase 3).
//
// IMPORTANT: these helpers accept an explicit `token` parameter and use it
// directly instead of the stored owner login token (`localStorage.otto_token`).
// This keeps the guest's scoped capability strictly isolated from any owner
// session on the same device.

import type { Session } from './types';
import { baseUrl, ApiError, WS_BEARER_SUBPROTOCOL } from './client';
import type { Problem } from './types';

/**
 * Fetch the session metadata using a scoped share token.
 * Uses `Authorization: Bearer <token>` — NOT the stored login token.
 */
export async function getSharedSession(id: string, token: string): Promise<Session> {
  const resp = await fetch(`${baseUrl()}/api/v1/sessions/${encodeURIComponent(id)}`, {
    headers: { Authorization: `Bearer ${token}` },
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
  return (await resp.json()) as Session;
}

/**
 * Open a WebSocket to `/ws/term/{id}` authenticated with the share token via
 * the `otto-bearer` Sec-WebSocket-Protocol subprotocol (keeps the token off
 * the URL / query string and out of access logs).
 *
 * Falls back to `?token=` is only needed for environments that strip custom
 * subprotocols — the server accepts both. We always prefer the subprotocol.
 */
export function openShareTerminalWs(sessionId: string, token: string): WebSocket {
  const base = new URL(baseUrl());
  const proto = base.protocol === 'https:' ? 'wss:' : 'ws:';
  const url = `${proto}//${base.host}/ws/term/${encodeURIComponent(sessionId)}`;
  return new WebSocket(url, [WS_BEARER_SUBPROTOCOL, token]);
}
