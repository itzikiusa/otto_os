// Opt-in SPA outward mutation guard (Fetch/XHR POST/PUT/PATCH/DELETE) helpers
// for daemon-owned Chromium live sessions (E-04a/b). Controlled live via the
// session WebSocket (`{ type: "set_guard", guard_spa_mutations: boolean }`),
// with server confirmation reflected on `ServerMsg::State.session.guard_spa_mutations`.
import type { BrowserLiveSession } from '../../../lib/api/types';
import type { SetGuardClientMsg } from './protocol';

/** Read the server-reported `guard_spa_mutations` flag from a live session
 *  snapshot (null when absent on older daemons). */
export function readSpaGuardFlag(session: BrowserLiveSession | null | undefined): boolean | null {
  if (!session) return null;
  const v = (session as BrowserLiveSession & { guard_spa_mutations?: unknown }).guard_spa_mutations;
  return typeof v === 'boolean' ? v : null;
}

/** Build the WS control message that toggles per-session SPA Fetch/XHR guarding. */
export function makeSetGuardMsg(enabled: boolean): SetGuardClientMsg {
  return { type: 'set_guard', guard_spa_mutations: enabled };
}
