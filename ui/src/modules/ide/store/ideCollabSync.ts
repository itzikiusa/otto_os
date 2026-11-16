// Guest-side recovery for refused collab writes (the daemon's round-4 collab rules):
// - 409 "not keeping up": the room's event ring is full of unread edits. Show "Reconnecting
//   to the room…", re-read the room state (fresh versions), then resend the guest's OWN
//   current text: nothing typed is dropped.
// - 403 on a save: the host refused a protected file; its reason goes inline on that file.

import { laneFetch } from '../../../lib/api/client';
import { ideCollabNotices } from './ideCollabNotices.svelte';
import { IdeCollabHttpError, type IdeCollabRoom, type IdeDebugSessionSnapshot } from './ideTypes';

/** Backoff for resending an edit after a "not keeping up" refusal. */
export const COLLAB_RESYNC_DELAYS_MS: readonly number[] = [1500, 3000, 6000];

export function collabStatus(e: unknown): number {
  return e instanceof IdeCollabHttpError ? e.status : 0;
}

export function isNotKeepingUp(e: unknown): boolean {
  return collabStatus(e) === 409 && /not keeping up/i.test((e as Error).message);
}

export interface CollabResyncHost {
  collabRoom: IdeCollabRoom | null;
  collabGuestToken: string | null;
  collabIsGuest: boolean;
  sharedBuffers: Record<string, { content: string; version: number }>;
  applyDebugSnapshot?(snap: IdeDebugSessionSnapshot): void;
}

/** Re-read the room (`GET …/state`): the room, its buffers' current versions and content.
 *  Open tabs are NOT touched, so the guest's unsent text survives. */
export async function resyncCollabRoom(host: CollabResyncHost): Promise<boolean> {
  const room = host.collabRoom;
  if (!room) return false;
  try {
    const headers: Record<string, string> = {};
    if (host.collabGuestToken) headers['X-Ide-Collab-Token'] = host.collabGuestToken;
    const resp = await laneFetch('int', `/ide/collab/guest/${encodeURIComponent(room.id)}/state`, { headers });
    if (!resp.ok) return false;
    const st = (await resp.json()) as {
      room?: IdeCollabRoom;
      shared_buffers?: Record<string, { content: string; version: number }>;
      active_debug?: IdeDebugSessionSnapshot | null;
    };
    if (host.collabRoom?.id !== room.id) return false; // left / switched meanwhile
    if (st.room && host.collabIsGuest) host.collabRoom = st.room;
    host.sharedBuffers = { ...host.sharedBuffers, ...(st.shared_buffers ?? {}) };
    if (st.active_debug) host.applyDebugSnapshot?.(st.active_debug);
    return true;
  } catch {
    return false;
  }
}

/**
 * After a "not keeping up" refusal: resync, wait, resend `currentText()` (read at send time,
 * so later keystrokes are included). Bounded; ends in a calm "failed" state with the guest's
 * text still in the editor. Returns whether the edit finally went through.
 */
export async function recoverRefusedEdit(
  host: CollabResyncHost,
  resend: () => Promise<void>,
  wait: (ms: number) => Promise<void> = (ms) => new Promise((r) => setTimeout(r, ms)),
): Promise<boolean> {
  ideCollabNotices.roomSync = 'reconnecting';
  for (let i = 0; i < COLLAB_RESYNC_DELAYS_MS.length; i++) {
    await wait(COLLAB_RESYNC_DELAYS_MS[i]);
    if (!host.collabRoom) { ideCollabNotices.roomSync = null; return false; }
    await resyncCollabRoom(host);
    try {
      await resend();
      ideCollabNotices.roomSync = null;
      return true;
    } catch (e) {
      if (!isNotKeepingUp(e)) break;
    }
  }
  ideCollabNotices.roomSync = 'failed';
  return false;
}
