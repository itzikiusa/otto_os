// Pure repository-to-Multi-Agent-Review route resolution for the IDE Git panel (`E-10b`).
// Zero store imports so unit tests can verify exact path matching & fallback routing directly.

export interface GitReviewRepoCandidate {
  id: string;
  path: string;
  name?: string;
  workspace_id?: string | null;
}

export interface MultiAgentReviewRouteResult {
  route: string;
  repoId: string | null;
  registered: boolean;
}

export function normalizeGitRepoPath(raw: string | null | undefined): string {
  const clean = (raw ?? '').trim().replace(/\\/g, '/').replace(/\/+$/, '');
  return clean.replace(/^\/private\/(var|tmp)(?=\/|$)/, '/$1');
}

/** Find the registered Git repository matching `targetPath` (exact root or deepest enclosing root). */
export function findRegisteredGitRepo(
  allRepos: readonly GitReviewRepoCandidate[],
  targetPath: string | null | undefined,
): GitReviewRepoCandidate | null {
  const norm = normalizeGitRepoPath(targetPath);
  if (!norm) return null;

  for (let i = 0; i < allRepos.length; i++) {
    if (normalizeGitRepoPath(allRepos[i].path) === norm) return allRepos[i];
  }

  let best: GitReviewRepoCandidate | null = null;
  let bestLen = -1;
  for (let i = 0; i < allRepos.length; i++) {
    const repo = allRepos[i];
    const repoPath = normalizeGitRepoPath(repo.path);
    if (!repoPath) continue;
    if (norm.startsWith(`${repoPath}/`) && repoPath.length > bestLen) {
      best = repo;
      bestLen = repoPath.length;
    }
  }
  return best;
}

/** Build the router destination for 1-click Multi-Agent Review in the Git module. */
export function buildMultiAgentReviewRoute(
  allRepos: readonly GitReviewRepoCandidate[],
  targetPath: string | null | undefined,
): MultiAgentReviewRouteResult {
  const match = findRegisteredGitRepo(allRepos, targetPath);
  if (!match) return { route: 'git', repoId: null, registered: false };
  return {
    route: `git/${encodeURIComponent(match.id)}/review`,
    repoId: match.id,
    registered: true,
  };
}
