// Switch the debugger to another thread (I2-05). The request names the thread
// ONLY: the daemon picks that thread's top frame with source. The shared
// `sendDebugAction` fills a missing `frame_id` with the current frame — a frame
// of the OLD thread, which the daemon then can't find (404) — so a host posts
// this command itself. A collab guest goes through the shared path (the guest
// route proxies with its own credentials).

import { api } from '../../../lib/api/client';
import { toastError } from '../../../lib/toastError';
import { ide, type IdeDebugSessionSnapshot } from '../ideStore.svelte';

export async function selectDebugThread(threadId: number): Promise<void> {
  const cur = ide.activeDebug;
  if (!cur || cur.active_thread_id === threadId) return;
  if (ide.collabIsGuest) {
    await ide.debugCommand('select_frame', { thread_id: threadId });
    return;
  }
  try {
    const resp = await api.post<{ session: IdeDebugSessionSnapshot }>(
      `/ide/debug/sessions/${encodeURIComponent(cur.id)}/command`,
      { action: 'select_frame', thread_id: threadId },
    );
    ide.applyDebugSnapshot(resp.session);
  } catch (e) {
    toastError('Couldn’t switch to that thread', e);
  }
}
