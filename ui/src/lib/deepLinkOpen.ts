// The app side of deep links (`#/agents/<id>`, `#/threads/<id>`): resolve the
// owning workspace (lib/deepLink.ts), switch the window to it when it isn't
// the current one, and say why when the person can't read the item. Kept out
// of the workspace store (it only needs the few members below).
import { api } from './api/client';
import type { Id, Session } from './api/types';
import { deepLinkErrorText, resolveDeepLink } from './deepLink';
import { SCRATCH_WORKSPACE_ID } from './stores/sessionScope';
import { toasts } from './toast.svelte';

/** The workspace-store members a deep link needs (structural: no cycle). */
export interface DeepLinkStore {
  readonly currentId: Id | null;
  sessionById: { has(id: Id): boolean };
  select(id: Id): Promise<boolean>;
  openSession(id: Id): void;
}

const inflight = new Set<string>();

/**
 * True once `id` is loadable in this window (already listed, or its workspace
 * is now selected); false when it can't be read (a toast says why) or the
 * switch was refused (an unsaved-changes guard). `thread` also looks the id
 * up as a structured runtime thread.
 */
export async function prepareDeepLink(store: DeepLinkStore, id: Id, what: 'session' | 'thread'): Promise<boolean> {
  if (store.sessionById.has(id)) return true;
  if (inflight.has(id)) return false;
  inflight.add(id);
  try {
    const r = await resolveDeepLink(id, store.currentId, SCRATCH_WORKSPACE_ID, {
      session: (sid) => api.get<Session>(`/sessions/${encodeURIComponent(sid)}`),
      runtime:
        what === 'thread'
          ? (tid) => api.get<{ workspace_id: Id }>(`/runtime-sessions/${encodeURIComponent(tid)}`)
          : undefined,
    });
    if (r.kind !== 'ok') {
      toasts.error(`Couldn’t open this ${what}`, deepLinkErrorText(r, what) ?? undefined);
      return false;
    }
    if (r.switchTo && r.switchTo !== store.currentId) return await store.select(r.switchTo);
    return true;
  } finally {
    inflight.delete(id);
  }
}

/** The `#/agents/<id>` route → store sync: resolve, switch, then open. */
export async function openSessionDeepLink(store: DeepLinkStore, id: Id): Promise<void> {
  if (store.sessionById.has(id) || id.startsWith('__')) {
    store.openSession(id);
    return;
  }
  if (await prepareDeepLink(store, id, 'session')) store.openSession(id);
}
