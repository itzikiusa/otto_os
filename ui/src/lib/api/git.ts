// Git API client — thin typed wrappers over the generic `api` helper for the
// `/repos/{id}/…` routes that more than one view calls (status, refs, push,
// checkout, stage, pull, pull requests). Mirrors docs/contracts/api.md (the
// "Git" section).
//
// One place builds these URLs so a route change is one edit. This is the RAW
// transport: what happens AFTER a mutation (store the fresh status, bump
// `refsRev` so an open graph re-syncs) is the git store's job
// (`git.push` / `git.checkout` / `git.pull` in lib/stores/git.svelte.ts) — call
// the store for a mutation, never this, unless you own that follow-up.

import { api } from './client';
import { plural } from '../plural';
import type {
  CheckoutReq,
  PrDetail,
  PrListResp,
  PrState,
  PrSummary,
  PullReq,
  PullResp,
  PushReq,
  PushTarget,
  RefsResp,
  RepoStatusResp,
} from './types';

const repo = (repoId: string): string => `/repos/${encodeURIComponent(repoId)}`;

/** `readRefs`: the refs, or why they couldn't be read. */
export type RefsRead = { refs: RefsResp; error: null } | { refs: null; error: unknown };

/** Cheap structural equality for a repo status — scalars first, then a light
 *  per-file signature. Lets `setStatus` skip no-op writes so a fetch that found
 *  nothing new doesn't recompute the `$derived` status / re-render the toolbar +
 *  tab chips every round. NOTE: this is load-bearing for ALL status propagation
 *  (manual fetch/pull/push/checkout/merge route through `setStatus` too), so keep
 *  it in lockstep with the `RepoStatusResp` shape — any newly-rendered field must
 *  be compared here or a real change could be silently suppressed. */
export function statusEq(a: RepoStatusResp, b: RepoStatusResp): boolean {
  if (
    a.branch !== b.branch ||
    a.upstream !== b.upstream ||
    a.ahead !== b.ahead ||
    a.behind !== b.behind ||
    (a.op_in_progress ?? null) !== (b.op_in_progress ?? null) ||
    a.changes.length !== b.changes.length
  ) {
    return false;
  }
  for (let i = 0; i < a.changes.length; i++) {
    const x = a.changes[i];
    const y = b.changes[i];
    if (
      x.path !== y.path ||
      x.orig_path !== y.orig_path ||
      x.kind !== y.kind ||
      x.staged !== y.staged ||
      x.unstaged !== y.unstaged
    ) {
      return false;
    }
  }
  return true;
}

/** What a finished pull says (one wording for the toolbar, the graph menus and
 *  the agent command). A pull whose merge conflicted comes back 200 with
 *  unmerged paths — not a failure: it needs the resolver. */
export function pullReport(r: PullResp): { tone: 'warn' | 'success'; title: string; body?: string } {
  const conflicts = r.status.changes.filter((c) => c.kind === 'conflicted').length;
  if (conflicts > 0) {
    return { tone: 'warn', title: 'Pulled with conflicts', body: r.note ?? `${plural(conflicts, 'file')} need resolution — open "Resolve conflicts"` };
  }
  return { tone: 'success', title: 'Pulled', body: r.note ?? undefined };
}

export const gitApi = {
  status: (repoId: string): Promise<RepoStatusResp> => api.get<RepoStatusResp>(`${repo(repoId)}/status`),
  refs: (repoId: string): Promise<RefsResp> => api.get<RefsResp>(`${repo(repoId)}/refs`),
  /** The one `/refs` loader with one error policy: never throws, a failure
   *  comes back as `error` (the raw cause — show it via `loadErrorText`), and
   *  each caller decides only what a miss means on its own screen. */
  readRefs: (repoId: string): Promise<RefsRead> =>
    api.get<RefsResp>(`${repo(repoId)}/refs`).then(
      (refs): RefsRead => ({ refs, error: null }),
      (error: unknown): RefsRead => ({ refs: null, error }),
    ),
  push: (repoId: string, body: PushReq = {}): Promise<RepoStatusResp> =>
    api.post<RepoStatusResp>(`${repo(repoId)}/push`, body),
  /** The source/destination a force-with-lease retry must name (local-only). */
  pushTarget: (repoId: string): Promise<PushTarget> => api.get<PushTarget>(`${repo(repoId)}/push-target`),
  checkout: (repoId: string, body: CheckoutReq): Promise<RepoStatusResp> =>
    api.post<RepoStatusResp>(`${repo(repoId)}/checkout`, body),
  stage: (repoId: string, paths: string[], stage = true): Promise<RepoStatusResp> =>
    api.post<RepoStatusResp>(`${repo(repoId)}/${stage ? 'stage' : 'unstage'}`, { paths }),
  pull: (repoId: string, body?: PullReq): Promise<PullResp> => api.post<PullResp>(`${repo(repoId)}/pull`, body),
  prs: {
    list: (repoId: string, state: PrState | 'all', page: number, perPage: number): Promise<PrListResp> =>
      api.get<PrListResp>(`${repo(repoId)}/prs?state=${encodeURIComponent(state)}&page=${page}&per_page=${perPage}`),
    get: (repoId: string, number: number): Promise<PrDetail> => api.get<PrDetail>(`${repo(repoId)}/prs/${number}`),
    create: (repoId: string, body: Record<string, unknown>): Promise<PrSummary> =>
      api.post<PrSummary>(`${repo(repoId)}/prs`, body),
  },
};
