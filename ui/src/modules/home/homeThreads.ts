// Threads on Home. The workspace store splits sessions into Agents and Threads
// (lib/stores/sessionBuckets.ts) and its Agents lists (`foregroundActive`,
// `plainAgentSessions`) leave every Threads-scope session out, so Home lists
// those itself: once each, labelled as a thread, opening `#/threads/<id>`.
// Structured runtime threads are not in the session list at all — only their
// "needs you" flags are counted (`ws.threadsNeedsYouCount`) — so they show as
// one row that opens Threads. Pure: `node --test` loads it directly.

import type { Session } from '../../lib/api/types';
import { inThreadsScope, isForeground } from '../../lib/stores/sessionBuckets';

/** Live Threads-scope sessions, in input order — the same rule the store's
 *  threads bucket uses (non-archived, foreground, not a connection). */
export function threadSessions(sessions: readonly Session[] | undefined, pool: ReadonlyMap<string, Session> | undefined): Session[] {
  if (!sessions) return [];
  const byId = pool ?? new Map(sessions.map((s) => [s.id, s] as const));
  return sessions.filter((s) => !s.archived && s.kind !== 'connection' && isForeground(s) && inThreadsScope(s, byId));
}

/** Route a thread opens on. */
export function threadRoute(id: string): string {
  return `threads/${encodeURIComponent(id)}`;
}

/** Threads waiting on a person that Home cannot list by name (runtime
 *  threads): the store's total minus the listed ones, never negative. */
export function unlistedWaiting(total: number | undefined, listed: number): number {
  return Math.max(0, (Number.isFinite(total) ? (total as number) : 0) - listed);
}
