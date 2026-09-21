import type { RefBranch, RepoStatusResp } from '../../lib/api/types';

/** HEAD status is freshest after local mutations; other branches come from the
 *  bulk refs response. Older daemons omit the per-branch tracking counts. */
export function branchTracking(branch: RefBranch | undefined, status: RepoStatusResp): { ahead: number; behind: number } {
  if (branch?.name === status.branch) return { ahead: status.ahead, behind: status.behind };
  return { ahead: branch?.ahead ?? 0, behind: branch?.behind ?? 0 };
}
