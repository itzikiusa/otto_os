// Remote live browser — wire protocol helpers (client side) for
// `WS /ws/browser/{tab_id}/live` (docs/contracts/ws.md §1b). The message
// shapes themselves are the contract types in lib/api/types.ts; this file
// holds the pure pieces around them (binary frame parsing, parsing JSON
// frames, cursor sanitising, driver/closed copy) so they can be unit-tested
// in node — type-only imports keep it import-free at runtime.

import type {
  BrowserLiveClientMsg,
  BrowserLiveController,
  BrowserLiveFrameHeader,
  BrowserLiveServerMsg,
  BrowserLiveSession,
} from '../../../lib/api/types';

export type ClientMsg = BrowserLiveClientMsg;
export type ServerMsg = BrowserLiveServerMsg;
export type FrameHeader = BrowserLiveFrameHeader;
export type PointerButton = 'left' | 'middle' | 'right' | 'back' | 'forward' | 'none';

/** Binary frame format version this client understands. */
export const FRAME_VERSION = 1;

/** What the live view reports up to the Browser page's toolbar. */
export interface LiveViewState {
  status: 'idle' | 'connecting' | 'live' | 'reconnecting' | 'ended';
  session: BrowserLiveSession | null;
}

export function encode(msg: ClientMsg): string {
  return JSON.stringify(msg);
}

/** Parse one JSON text frame; unknown/invalid input yields null (ignored). */
export function parseServer(raw: string): ServerMsg | null {
  let v: unknown;
  try {
    v = JSON.parse(raw);
  } catch {
    return null;
  }
  if (!v || typeof v !== 'object' || typeof (v as { type?: unknown }).type !== 'string') return null;
  return v as ServerMsg;
}

/**
 * Binary screencast frame:
 *   byte 0      format version (1)
 *   bytes 1..5  u32 big-endian N = JSON header length
 *   bytes 5..   N bytes of UTF-8 JSON header (`BrowserLiveFrameHeader`)
 *   then        the image bytes (JPEG)
 * A frame with an unknown version or a malformed header is dropped.
 */
export function parseBinaryFrame(buf: ArrayBuffer): { header: FrameHeader; bytes: Uint8Array } | null {
  if (buf.byteLength < 5) return null;
  const view = new DataView(buf);
  if (view.getUint8(0) !== FRAME_VERSION) return null;
  const len = view.getUint32(1);
  if (len <= 0 || 5 + len > buf.byteLength) return null;
  try {
    const header = JSON.parse(new TextDecoder().decode(new Uint8Array(buf, 5, len))) as FrameHeader;
    if (typeof header.seq !== 'number' || !(header.device_width > 0) || !(header.device_height > 0)) return null;
    return { header, bytes: new Uint8Array(buf, 5 + len) };
  } catch {
    return null;
  }
}

/** DOM `MouseEvent.button` → wire button. */
export function buttonName(button: number): PointerButton {
  switch (button) {
    case 0: return 'left';
    case 1: return 'middle';
    case 2: return 'right';
    case 3: return 'back';
    case 4: return 'forward';
    default: return 'none';
  }
}

/** CSS cursor values the daemon may report; anything else falls back to
 *  `default` so a hostile page can't put a `url(...)` cursor on Otto's own
 *  chrome. */
const CURSORS = new Set([
  'auto', 'default', 'none', 'pointer', 'text', 'vertical-text', 'crosshair', 'move', 'grab',
  'grabbing', 'wait', 'progress', 'help', 'not-allowed', 'no-drop', 'copy', 'alias',
  'context-menu', 'cell', 'col-resize', 'row-resize', 'ew-resize', 'ns-resize', 'nesw-resize',
  'nwse-resize', 'n-resize', 's-resize', 'e-resize', 'w-resize', 'ne-resize', 'nw-resize',
  'se-resize', 'sw-resize', 'all-scroll', 'zoom-in', 'zoom-out',
]);

export function safeCursor(c: string | null | undefined): string {
  return c && CURSORS.has(c) ? c : 'default';
}

/**
 * Who may send input, from this viewer's point of view.
 *   agent  — an agent drives; input is refused until the viewer takes over
 *   other  — another person drives this session right now
 *   me     — this user drives (or nobody does: the first input claims it)
 */
export function driverFor(controller: BrowserLiveController, controllerUserId: string | null, meId: string | null): 'agent' | 'other' | 'me' {
  if (controller === 'agent') return 'agent';
  if (controller === 'human' && controllerUserId && meId && controllerUserId !== meId) return 'other';
  return 'me';
}

/** Human copy for a `closed` frame's reason. */
export function closedReason(reason: string): string {
  switch (reason) {
    case 'idle': return 'It was closed after a while with no one watching.';
    case 'crashed': return 'The browser engine stopped unexpectedly.';
    case 'revoked': return 'Your access to this workspace changed.';
    case 'replaced': return 'This tab was opened in another window.';
    default: return 'The live tab was closed.';
  }
}

/** Human copy for the SSRF guard's `blocked` frame. */
export function blockedCopy(host: string): string {
  return `Otto blocked a request to ${host}. Pages in the live browser can't reach this Mac, your local network or cloud metadata addresses.`;
}
