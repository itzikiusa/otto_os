// Remote live browser — REST calls (engine status/install/settings, starting
// a live session for a tab). PROVISIONAL shapes, written before the daemon
// contract landed; see modules/browser/live/protocol.ts.

import { api, wsConnect } from './client';

/** Which Chromium build the daemon runs. `chrome` = Chrome for Testing in
 *  new-headless mode (the default, closest to what users see); the lighter
 *  `headless_shell` = chrome-headless-shell. */
export type LiveEngineKind = 'chrome' | 'headless_shell';

export interface LiveEngineStatus {
  /** `missing` until the one-time download; `ready` once installed. */
  state: 'missing' | 'installing' | 'ready' | 'failed' | 'unsupported';
  /** The installed (or installing) build. */
  engine: LiveEngineKind | null;
  version?: string | null;
  /** Download progress while `installing`. */
  progress?: { received_bytes: number; total_bytes: number | null } | null;
  /** Why the last install failed / why it is unsupported. */
  error?: string | null;
  /** Approximate download size per build, for the enable step's copy. */
  download_bytes?: Partial<Record<LiveEngineKind, number>>;
  /** "Show the window on this Mac" — run headed (off by default). */
  headed?: boolean;
}

export interface LiveSessionResp {
  session_id: string;
  /** Path of the live WebSocket (auth via the bearer subprotocol). */
  ws_path: string;
}

export function engineStatus(): Promise<LiveEngineStatus> {
  return api.get<LiveEngineStatus>('/browser/live/engine');
}

export function installEngine(engine: LiveEngineKind): Promise<LiveEngineStatus> {
  return api.post<LiveEngineStatus>('/browser/live/engine/install', { engine });
}

export function updateEngineSettings(body: { engine?: LiveEngineKind; headed?: boolean }): Promise<LiveEngineStatus> {
  return api.patch<LiveEngineStatus>('/browser/live/engine', body);
}

export function startLive(tabId: string, viewport: { width: number; height: number; device_scale_factor: number }): Promise<LiveSessionResp> {
  return api.post<LiveSessionResp>(`/browser/tabs/${tabId}/live`, { viewport });
}

export function openLiveSocket(wsPath: string): WebSocket {
  const sock = wsConnect(wsPath);
  sock.binaryType = 'arraybuffer';
  return sock;
}

/** Fallback sizes for the enable step when the daemon doesn't report them. */
export const DEFAULT_DOWNLOAD_BYTES: Record<LiveEngineKind, number> = {
  chrome: 150 * 1024 * 1024,
  headless_shell: 78 * 1024 * 1024,
};
