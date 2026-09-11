// Sidebar session order — the PURE half (no runes, no DOM) so `node --test`
// can exercise it and the store file can re-export it. "Recent" is the list as
// the store hands it over (no re-sort); "Manual" applies a persisted id order.
import type { Id } from '../api/types';

export type OrderMode = 'recent' | 'manual';
export const LS_SESSION_ORDER = 'otto_session_order_'; // + ws key → { mode, order: Id[] }

/** Manual order: ids NOT in `order` (new sessions) go on TOP, newest `last_active_at` first (string compare — ISO);
 *  then the known ids in `order`'s sequence. Ids in `order` with no session are ignored. */
export function applyOrder<T extends { id: Id; last_active_at: string }>(sessions: T[], order: Id[]): T[] {
  const byId = new Map(sessions.map((s) => [s.id, s]));
  const known = new Set(order.filter((id) => byId.has(id)));
  const fresh = sessions
    .filter((s) => !known.has(s.id))
    .sort((a, b) => b.last_active_at.localeCompare(a.last_active_at));
  const seen = new Set<Id>();
  const rest: T[] = [];
  for (const id of order) {
    const s = byId.get(id);
    if (s && !seen.has(id)) {
      rest.push(s);
      seen.add(id);
    }
  }
  return [...fresh, ...rest];
}

/** Pull `fromId` out of `ids` and reinsert at `toId`'s position (ui.reorderSidebar semantics); unknown ids ⇒ unchanged. */
export function reorder(ids: Id[], fromId: Id, toId: Id): Id[] {
  if (fromId === toId) return ids;
  const next = [...ids];
  const from = next.indexOf(fromId);
  const to = next.indexOf(toId);
  if (from < 0 || to < 0) return ids;
  next.splice(from, 1);
  next.splice(to, 0, fromId);
  return next;
}
