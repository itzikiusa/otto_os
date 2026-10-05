import type { RefBranch, RepoStatusResp } from '../../lib/api/types';

/** HEAD status is freshest after local mutations; other branches come from the
 *  bulk refs response. Older daemons omit the per-branch tracking counts. */
export function branchTracking(branch: RefBranch | undefined, status: RepoStatusResp): { ahead: number; behind: number } {
  if (branch?.name === status.branch) return { ahead: status.ahead, behind: status.behind };
  return { ahead: branch?.ahead ?? 0, behind: branch?.behind ?? 0 };
}

/** Split a remote-tracking ref (`upstream/feature/x`) into its remote and the
 *  branch name ON that remote. The first segment is the remote — a remote name
 *  can't contain `/` in practice, a branch name can. A ref with no `/` is taken
 *  as a branch on `origin`. Destructive remote actions MUST target `remote`:
 *  stripping the prefix and pushing to origin deletes the wrong branch. */
export function splitRemoteRef(ref: string): { remote: string; branch: string } {
  const i = ref.indexOf('/');
  if (i <= 0 || i === ref.length - 1) return { remote: 'origin', branch: ref };
  return { remote: ref.slice(0, i), branch: ref.slice(i + 1) };
}
