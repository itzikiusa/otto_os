// Browser — remote live view (daemon-owned Chromium). Mirrors
// docs/contracts/api.md "Browser — remote live view" + ws.md §1b.
//
// Engine status/settings/install are daemon-wide (`/browser/live/*`); the
// live session is per tab (`/browser/tabs/{id}/live`), private to its owner.

import { api, wsConnect } from './client';
import type {
  BrowserChromeBuild,
  BrowserEngineInstallJob,
  BrowserLiveControlReq,
  BrowserLiveCreateReq,
  BrowserLiveNavReq,
  BrowserLiveSession,
  BrowserLiveSettings,
  BrowserLiveStatus,
} from './types';

export function liveStatus() {
  return api.get<BrowserLiveStatus>('/browser/live/status');
}

/** Browser Admin. Partial update; returns the full settings. */
export function updateLiveSettings(body: Partial<BrowserLiveSettings>) {
  return api.put<BrowserLiveSettings>('/browser/live/settings', body);
}

/** Browser Admin. Starts the ONE-TIME Chromium download — only ever called
 *  from an explicit click. 202 with the job; 200 `installed` when present. */
export function installEngine(build?: BrowserChromeBuild) {
  return api.post<BrowserEngineInstallJob>('/browser/live/install', build ? { build } : {});
}

/** Open (or re-attach to the caller's existing) remote session for a tab. */
export function openLive(tabId: string, body: BrowserLiveCreateReq) {
  return api.post<BrowserLiveSession>(`/browser/tabs/${tabId}/live`, { engine: 'remote', ...body });
}

export function getLive(tabId: string) {
  return api.get<BrowserLiveSession>(`/browser/tabs/${tabId}/live`);
}

export function closeLive(tabId: string) {
  return api.del<void>(`/browser/tabs/${tabId}/live`);
}

export function liveNav(tabId: string, body: BrowserLiveNavReq) {
  return api.post<BrowserLiveSession>(`/browser/tabs/${tabId}/live/nav`, body);
}

export function liveControl(tabId: string, body: BrowserLiveControlReq) {
  return api.post<BrowserLiveSession>(`/browser/tabs/${tabId}/live/control`, body);
}

/** The screencast + input socket (bearer via the subprotocol, never the URL). */
export function openLiveSocket(tabId: string): WebSocket {
  const sock = wsConnect(`/ws/browser/${tabId}/live`);
  sock.binaryType = 'arraybuffer';
  return sock;
}
