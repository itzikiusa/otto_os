// The app side of deep links (`#/agents/<id>`, `#/threads/<id>`): resolve the
// owning workspace (lib/deepLink.ts), switch the window to it when it isn't
// the current one, and say why when the person can't read the item. Kept out
// of the workspace store (it only needs the few members below).
import { api } from './api/client';
import type { Id, Session } from './api/types';
import { deepLinkErrorText, resolveDeepLink } from './deepLink';
import { router } from './router.svelte';
import { opensInThreads } from './stores/sessionBuckets';
import { SCRATCH_WORKSPACE_ID } from './stores/sessionScope';
import { toasts } from './toast.svelte';

/** The workspace-store members a deep link needs (structural: no cycle). */
export interface DeepLinkStore {
  readonly currentId: Id | null;
  sessionById: ReadonlyMap<Id, Session>;
  /** Open tabs that are structured threads (their pane is a chat). */
  readonly threadPanes: readonly Id[];
  select(id: Id): Promise<boolean>;
  openSession(id: Id, kind?: 'thread'): void;
}

const inflight = new Set<string>();

/**
 * Truthy once `id` is loadable in this window (already listed, or its workspace
 * is now selected): 'thread' when only the runtime lookup knows it (a chat
 * pane's id in `#/agents/<id>` is one). False when it can't be read (a toast
 * says why, in `what`'s words) or the switch was refused (an unsaved-changes guard).
 * `forget`: the id came from memory (router.remembered), not a link someone
 * followed — when it no longer exists, `forget` drops it instead of a toast.
 */
export async function prepareDeepLink(store: DeepLinkStore, id: Id, what: 'session' | 'thread', forget?: () => void): Promise<false | 'session' | 'thread'> {
  if (store.sessionById.has(id)) return 'session';
  if (inflight.has(id)) return false;
  inflight.add(id);
  try {
    // The session lookup first; the runtime one answers only for a thread.
    let found: 'session' | 'thread' = 'session';
    const r = await resolveDeepLink(id, store.currentId, SCRATCH_WORKSPACE_ID, {
      session: (sid) => api.get<Session>(`/sessions/${encodeURIComponent(sid)}`),
      runtime: (tid) => api.get<{ workspace_id: Id }>(`/runtime-sessions/${encodeURIComponent(tid)}`).then((t) => ((found = 'thread'), t)),
    });
    if (r.kind === 'missing' && forget) {
      forget();
      return false;
    }
    if (r.kind !== 'ok') {
      toasts.error(`Couldn’t open this ${what}`, deepLinkErrorText(r, what) ?? undefined);
      return false;
    }
    if (r.switchTo && r.switchTo !== store.currentId && !(await store.select(r.switchTo))) return false;
    return found;
  } finally {
    inflight.delete(id);
  }
}

/** The `#/agents/<id>` route → store sync: resolve, switch, then open. A
 *  thread already open as a pane here just takes focus; one the tabs have not
 *  restored yet (a relaunch, Back onto a closed chat pane) opens as a chat pane. */
export async function openSessionDeepLink(store: DeepLinkStore, id: Id, forget?: () => void): Promise<void> {
  if (store.threadPanes.includes(id)) return store.openSession(id);
  const found = store.sessionById.has(id) || id.startsWith('__') ? 'session' : await prepareDeepLink(store, id, 'session', forget);
  if (!found) return;
  if (found === 'thread') return store.openSession(id, 'thread');
  // An old or outside link (`agents/<id>` from the tray, the floating bar, a
  // notice written before the session moved) to a session that lives in
  // Threads opens there, never as a terminal tab over the same conversation
  // (a Threads terminal-dock shell has no pane there: it opens here).
  const s = store.sessionById.get(id);
  if (s && opensInThreads(s, store.sessionById)) router.replace(`threads/${id}`);
  else store.openSession(id);
}
