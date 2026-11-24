// What opening a repository reads, in ONE request (`GET /repos/{id}/overview`:
// status + refs + stashes / worktrees / submodules) — shared by everyone
// opening the repo (the git store's arrival and primary status, RepoView's
// first status, the graph's first refs and side lists) for a few seconds.
import { api } from './client';
import type { RepoOverview } from './types';

/** Callers within this window share one read. */
const SHARE_MS = 3000;
const reads = new Map<string, { p: Promise<RepoOverview>; at: number }>();

export function readOverview(repoId: string): Promise<RepoOverview> {
  return recentOverview(repoId) ?? fresh(repoId);
}

/** The read still in its share window, else null (never starts one). */
export function recentOverview(repoId: string): Promise<RepoOverview> | null {
  const hit = reads.get(repoId);
  return hit && Date.now() - hit.at < SHARE_MS ? hit.p : null;
}

function fresh(repoId: string): Promise<RepoOverview> {
  const p = api.get<RepoOverview>(`/repos/${encodeURIComponent(repoId)}/overview`);
  p.catch(() => reads.delete(repoId));
  reads.set(repoId, { p, at: Date.now() });
  return p;
}
