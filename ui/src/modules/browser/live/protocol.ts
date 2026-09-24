// Remote live browser — wire protocol (client side). The ONE place that knows
// message shapes, so the rest of the view talks in intents (move, click,
// type, resize, take over…) and a contract change lands here only.
//
// PROVISIONAL: written against local fixtures before the daemon contract
// (docs/contracts/ws.md "Browser live") landed. Keep it in lockstep with
// that section and ui/src/lib/api/types.ts once it exists.

import type { FrameGeometry } from './geometry';
import type { KeyPayload } from './keys';

export type PointerButton = 'none' | 'left' | 'middle' | 'right';

/** Who is steering the remote page. `user` = this viewer (or another human
 *  viewer), `agent` = an agent session holding the input lock. */
export interface LiveLock {
  holder: 'none' | 'user' | 'agent';
  /** The agent session id when an agent holds it. */
  session_id?: string | null;
  /** Display name of the agent (session title / persona). */
  agent_name?: string | null;
  /** Provider id for `ProviderIcon` (claude, codex…). */
  provider?: string | null;
  /** What the agent says it is doing, if anything. */
  activity?: string | null;
  /** True when THIS connection holds the lock. */
  mine?: boolean;
}

/** An outward action waiting for a person (plan §4.6 approval card). */
export interface LiveApproval {
  id: string;
  /** Short title: "Submit a form on this site?" */
  title: string;
  /** Agent's stated intent: "Personal Assistant wants to click Confirm reschedule." */
  summary: string;
  /** Why Otto classed it outward ("it submits a POST form"). */
  reason?: string | null;
  action_class: 'outward' | 'credential' | 'download' | 'new_origin' | 'eval' | string;
  where: string;
  what: string;
  who_sees: string;
  as_profile?: string | null;
  /** Blob URL of the pre-action screenshot with the target boxed. */
  screenshot_url?: string | null;
  agent_name?: string | null;
  provider?: string | null;
  requested_at: string;
  /** Page-space box of the target, for the on-frame highlight. */
  target?: { x: number; y: number; width: number; height: number } | null;
}

export type ApprovalDecision = 'approve_once' | 'always_site' | 'deny' | 'take_over';

export interface NavState {
  url: string;
  title: string;
  loading: boolean;
  can_go_back: boolean;
  can_go_forward: boolean;
}

export interface FrameMeta extends FrameGeometry {
  seq: number;
  /** Page timestamp of the frame (seconds, CDP), for latency. */
  timestamp?: number;
}

/** Server → client. JSON text messages; frames may instead arrive as binary
 *  messages (see `parseBinaryFrame`). */
export type ServerMsg =
  | { type: 'hello'; session_id: string; engine: string; engine_version?: string; nav: NavState; lock: LiveLock; viewport?: { width: number; height: number; device_scale_factor: number } }
  | { type: 'frame'; seq: number; data: string; mime?: string; meta: FrameGeometry & { timestamp?: number } }
  | { type: 'nav'; nav: NavState }
  | { type: 'cursor'; cursor: string }
  | { type: 'lock'; lock: LiveLock }
  | { type: 'approval'; approval: LiveApproval }
  | { type: 'approval_resolved'; id: string; decision: string }
  | { type: 'agent_pointer'; x: number; y: number; label?: string | null }
  | { type: 'pick_result'; selector: string; outer_html: string; text: string; url: string }
  | { type: 'new_tab'; url: string }
  | { type: 'pong'; t: number }
  | { type: 'error'; code: string; message: string }
  | { type: 'ended'; reason: string };

/** Client → server. */
export type ClientMsg =
  | { type: 'mouse'; action: 'move' | 'down' | 'up'; x: number; y: number; button: PointerButton; buttons: number; click_count: number; modifiers: number; pointer?: 'mouse' | 'touch' | 'pen' }
  | { type: 'wheel'; x: number; y: number; dx: number; dy: number; modifiers: number }
  | ({ type: 'key'; action: 'down' | 'up' } & KeyPayload)
  | { type: 'text'; text: string }
  | { type: 'resize'; width: number; height: number; device_scale_factor: number }
  | { type: 'navigate'; url: string }
  | { type: 'history'; action: 'back' | 'forward' | 'reload' | 'stop' }
  | { type: 'frame_ack'; seq: number }
  | { type: 'ping'; t: number }
  | { type: 'take_over' }
  | { type: 'hand_back' }
  | { type: 'approval'; id: string; decision: ApprovalDecision }
  | { type: 'pick'; x: number; y: number }
  | { type: 'focus'; focused: boolean };

export function encode(msg: ClientMsg): string {
  return JSON.stringify(msg);
}

/** Parse one text message; unknown/invalid input yields null (ignored). */
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
 * Binary frame layout: a 4-byte big-endian header length, a UTF-8 JSON
 * header (`{seq, meta, mime}`), then the image bytes. Lets the daemon skip
 * base64 (+33%) on the hot path.
 */
export function parseBinaryFrame(buf: ArrayBuffer): { seq: number; meta: FrameGeometry & { timestamp?: number }; mime: string; bytes: Uint8Array } | null {
  if (buf.byteLength < 4) return null;
  const view = new DataView(buf);
  const len = view.getUint32(0);
  if (len <= 0 || 4 + len > buf.byteLength) return null;
  try {
    const head = JSON.parse(new TextDecoder().decode(new Uint8Array(buf, 4, len)));
    return {
      seq: Number(head.seq) || 0,
      meta: head.meta,
      mime: typeof head.mime === 'string' ? head.mime : 'image/jpeg',
      bytes: new Uint8Array(buf, 4 + len),
    };
  } catch {
    return null;
  }
}

/** DOM `MouseEvent.button` → wire button. */
export function buttonName(button: number): PointerButton {
  return button === 0 ? 'left' : button === 1 ? 'middle' : button === 2 ? 'right' : 'none';
}

/** CSS cursor values the server may report (CDP/`getComputedStyle` names);
 *  anything else falls back to `default` so a hostile page can't set a
 *  `url(...)` cursor on Otto's own chrome. */
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
