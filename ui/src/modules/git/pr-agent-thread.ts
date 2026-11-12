// The one Threads thread a pull request gets (PrAgentPanel): created on the
// first ask with `source: 'pr-agent'` and `origin: {kind:'pr', id:'<repo>#<n>'}`
// so the Threads sidebar files it under "Pull requests" (not the main list),
// the daemon closes it after `embedded_thread_auto_close_hours` idle, and the
// PR finds it again (`GET …/runtime-sessions?source=pr-agent&origin_id=`).
//
// An older daemon ignores `source` / `origin` (a plain thread, listed in
// Threads) and the list filter: the thread id is then also remembered in this
// browser, so the PR still finds its thread. The modules/threads store is
// another feature's, so this talks to the runtime routes directly.
import { api, ApiError } from '../../lib/api/client';
import type {
  CreateRuntimeSessionReq,
  RuntimeAccessMode,
  RuntimePromptResp,
  RuntimeProvider,
  RuntimeSession,
} from '../../lib/api/types';

export const PR_AGENT_SOURCE = 'pr-agent';

/** The PR a thread is about, as the daemon stores it (`meta.origin.id`). */
export const prAgentOriginId = (repoId: string, n: number): string => `${repoId}#${n}`;

/** `meta.origin.label`: "otto #51 Add the VIP program" (≤ 200 characters). */
export function prAgentOriginLabel(repoName: string, n: number, title: string): string {
  const s = `${repoName} #${n} ${title}`.replace(/\s+/g, ' ').trim();
  return s.length > 200 ? `${s.slice(0, 199)}…` : s;
}

/** Providers a thread can run on (the structured runtime's drivers; the
 *  rest are terminal-only). Mirrors isRuntimeProvider in the Threads module. */
const RUNTIME_PROVIDERS: readonly string[] = ['claude', 'codex', 'gemini', 'grok', 'opencode', 'pi'];

/** The provider a new PR thread runs on: the person's default when it can
 *  run a thread and is installed, else the first that can — null when none
 *  can (the panel says what to set up instead of failing a request). */
export function pickRuntimeProvider(
  preferred: string,
  registered: readonly string[],
  ready: (p: string) => boolean,
): RuntimeProvider | null {
  const ok = (p: string): boolean => RUNTIME_PROVIDERS.includes(p) && registered.includes(p) && ready(p);
  if (ok(preferred)) return preferred as RuntimeProvider;
  return (registered.find(ok) as RuntimeProvider | undefined) ?? null;
}

function originIdOf(s: Pick<RuntimeSession, 'meta'>): string | null {
  const o = s.meta?.origin as { id?: unknown } | undefined;
  return typeof o?.id === 'string' ? o.id : null;
}

/** The PR's thread among `rows`: tagged with this origin (or the id this
 *  browser remembered), never a linked thread its agent opened (those
 *  inherit the origin), newest activity first. */
export function pickPrThread(
  rows: readonly RuntimeSession[],
  originId: string,
  rememberedId: string | null,
): RuntimeSession | null {
  const mine = rows.filter(
    (r) => !r.parent_id && (r.id === rememberedId || (r.meta?.source === PR_AGENT_SOURCE && originIdOf(r) === originId)),
  );
  mine.sort((a, b) => b.last_active_at.localeCompare(a.last_active_at));
  return mine[0] ?? null;
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
function remember(originId: string, id: string): void {
  try {
    const all = remembered();
    all[originId] = id;
    // Bounded: the newest 200 PRs (insertion order).
    const keys = Object.keys(all);
    for (const k of keys.slice(0, Math.max(0, keys.length - 200))) delete all[k];
    localStorage.setItem(REMEMBER_KEY, JSON.stringify(all));
  } catch {
    /* not remembered: a current daemon finds it by origin anyway */
  }
}

/** The PR's thread in `workspaceId` (closed ones too), or null. */
export async function findPrThread(workspaceId: string, originId: string): Promise<RuntimeSession | null> {
  const q = `source=${PR_AGENT_SOURCE}&origin_id=${encodeURIComponent(originId)}`;
  const rows = await api.get<RuntimeSession[]>(`/workspaces/${workspaceId}/runtime-sessions?${q}`);
  const hit = pickPrThread(rows, originId, remembered()[originId] ?? null);
  if (hit) return hit;
  // Remembered but not in the list (another workspace's list, or past an old
  // daemon's 500-row cap): read it directly.
  const id = remembered()[originId];
  if (!id) return null;
  try {
    return await api.get<RuntimeSession>(`/runtime-sessions/${id}`);
  } catch {
    return null;
  }
}

/** `POST /workspaces/{id}/runtime-sessions` with the embed fields. */
export type PrThreadCreateReq = CreateRuntimeSessionReq & {
  source: typeof PR_AGENT_SOURCE;
  origin: { kind: 'pr'; id: string; label?: string };
};

export async function createPrThread(workspaceId: string, req: PrThreadCreateReq): Promise<RuntimeSession> {
  const path = `/workspaces/${workspaceId}/runtime-sessions`;
  let created: RuntimeSession;
  try {
    created = await api.post<RuntimeSession>(path, req);
  } catch (e) {
    // A daemon that refuses the embed fields (400 / 422): a plain thread.
    if (!(e instanceof ApiError) || (e.status !== 400 && e.status !== 422) || !/source|origin|unknown field/i.test(e.message)) throw e;
    const plain: CreateRuntimeSessionReq = { provider: req.provider, cwd: req.cwd, title: req.title, model: req.model, access_mode: req.access_mode };
    created = await api.post<RuntimeSession>(path, plain);
  }
  remember(req.origin.id, created.id);
  return created;
}

/** Send `text` to the thread in `access` mode (sticky from this turn on). A
 *  closed thread is reopened first; a running turn gets it queued behind. */
export async function sendToPrThread(
  thread: Pick<RuntimeSession, 'id' | 'status'>,
  text: string,
  access: RuntimeAccessMode,
): Promise<RuntimePromptResp> {
  if (thread.status === 'closed') await api.post<RuntimeSession>(`/runtime-sessions/${thread.id}/reopen`);
  return api.post<RuntimePromptResp>(`/runtime-sessions/${thread.id}/prompt`, { text, access_mode: access, delivery: 'queue' });
}
