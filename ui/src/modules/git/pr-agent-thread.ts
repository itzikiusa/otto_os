// The Threads thread a pull request gets (PrAgentPanel): started from the
// embed's start composer with `source: 'pr-agent'` and `origin: {kind:'pr',
// id:'<repo>#<n>'}` so the Threads sidebar files it under "Pull requests"
// (not the main list), the daemon closes it after
// `embedded_thread_auto_close_hours` idle, and the PR finds it again through
// the Threads store (`findByOrigin`: an open thread before a closed one).
//
// An older daemon ignores `source` / `origin` (a plain thread, listed in
// Threads) and the list filter: the thread id is then also remembered in this
// browser, so the PR still finds its thread. Every read and send goes through
// the Threads session port (lib/runtimeThreadPort.ts), never the runtime
// routes directly, so the embed's access chip and status stay true.
import type { RuntimeSession } from '../../lib/api/types';
import type { RuntimeEmbedStatus, RuntimeSessionPort } from '../../lib/runtimeThreadPort';

export const PR_AGENT_SOURCE = 'pr-agent';

/** The PR a thread is about, as the daemon stores it (`meta.origin.id`). */
export const prAgentOriginId = (repoId: string, n: number): string => `${repoId}#${n}`;

/** `meta.origin.label`: "otto #51 Add the VIP program" (≤ 200 characters). */
export function prAgentOriginLabel(repoName: string, n: number, title: string): string {
  const s = `${repoName} #${n} ${title}`.replace(/\s+/g, ' ').trim();
  return s.length > 200 ? `${s.slice(0, 199)}…` : s;
}

const REMEMBER_KEY = 'otto_pr_agent_threads';
function remembered(): Record<string, string> {
  try {
    const v = JSON.parse(localStorage.getItem(REMEMBER_KEY) ?? '{}');
    return v && typeof v === 'object' ? (v as Record<string, string>) : {};
  } catch {
    return {};
  }
}

/** Keep `id` as the PR's thread in this browser (an older daemon's only link). */
export function rememberPrThread(originId: string, id: string): void {
  try {
    const all = remembered();
    delete all[originId];
    all[originId] = id;
    // Bounded: the newest 200 PRs (insertion order).
    const keys = Object.keys(all);
    for (const k of keys.slice(0, Math.max(0, keys.length - 200))) delete all[k];
    localStorage.setItem(REMEMBER_KEY, JSON.stringify(all));
  } catch {
    /* not remembered: a current daemon finds it by origin anyway */
  }
}

type FindPort = Pick<RuntimeSessionPort['runtimeThreads'], 'findByOrigin' | 'loadOne'>;

/** The PR's thread in `workspaceId` (an open one first; closed ones too), or null. */
export async function findPrThread(port: FindPort, workspaceId: string, originId: string): Promise<RuntimeSession | null> {
  const id = remembered()[originId] ?? null;
  const hit = await port.findByOrigin(workspaceId, PR_AGENT_SOURCE, originId, id);
  if (hit || !id) return hit;
  // Remembered but not in the list (another workspace's list, or past an old
  // daemon's 500-row cap): read it directly.
  try {
    return await port.loadOne(id);
  } catch {
    return null;
  }
}

/** A folded / closed-sheet PR's dot, from its thread's row alone (the store
 *  keeps it live): no conversation view is mounted just for this. An
 *  approval or question waiting shows as `awaiting_permission`. */
export function prThreadStatus(
  row: Pick<RuntimeSession, 'status' | 'background_tasks'> | null | undefined,
): RuntimeEmbedStatus | null {
  if (!row) return null;
  if (row.status === 'closed') return 'closed';
  if (row.status === 'awaiting_permission') return 'needs-you';
  if (row.status === 'working' || row.status === 'starting') return 'working';
  if ((row.background_tasks?.length ?? 0) > 0) return 'background';
  return 'idle';
}
