// Browser — remote live view (daemon-owned Chromium). Mirrors
// docs/contracts/api.md "Browser — remote live view" + ws.md §1b.
//
// Engine status/settings/install are daemon-wide (`/browser/live/*`); the
// live session is per tab (`/browser/tabs/{id}/live`), private to its owner.

import { api, authedResponse, wsConnect } from './client';
import type {
  BrowserChromeBuild,
  BrowserEngineInstallJob,
  BrowserLiveCreateReq,
  BrowserLiveEmulateReq,
  BrowserLiveSession,
  BrowserLiveSettings,
  BrowserLiveStatus,
  BrowserScreenshotReq,
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

export function closeLive(tabId: string) {
  return api.del<void>(`/browser/tabs/${tabId}/live`);
}

/** Apply device emulation (preset, mobile/touch, UA, color scheme, orientation)
 *  and optionally resize the remote Chromium viewport. */
export function emulateLive(tabId: string, body: BrowserLiveEmulateReq) {
  return api.post<BrowserLiveSession>(`/browser/tabs/${tabId}/live/emulate`, body);
}

/** Capture a PNG or JPEG screenshot of the live tab (viewport, full_page, or element). */
export async function screenshotLive(tabId: string, body: BrowserScreenshotReq = {}): Promise<Blob> {
  const resp = await authedResponse(`/browser/tabs/${tabId}/live/screenshot`, {
    method: 'POST',
    contentType: 'application/json',
    body: JSON.stringify(body),
  });
  return resp.blob();
}

/** The screencast + input socket (bearer via the subprotocol, never the URL). */
export function openLiveSocket(tabId: string): WebSocket {
  const sock = wsConnect(`/ws/browser/${tabId}/live`);
  sock.binaryType = 'arraybuffer';
  return sock;
}
