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

/** Where the commit an Amend would rewrite is already published, or null when
 *  it isn't (S15-28). `headRemotes` is the daemon's `git branch -r --contains
 *  HEAD` answer: it also catches a HEAD pushed under another name and a branch
 *  with no upstream. Until it lands (or if it failed) the upstream heuristic
 *  stands in: on the upstream with nothing ahead ⇒ HEAD is pushed. The
 *  branch's own upstream is named first when it is among the refs. */
export function amendPublishedAt(
  upstream: string | null | undefined,
  ahead: number,
  headRemotes: readonly string[] | null,
): string | null {
  if (headRemotes === null) return upstream != null && ahead === 0 ? upstream : null;
  if (headRemotes.length === 0) return null;
  if (upstream != null && headRemotes.includes(upstream)) {
    return headRemotes.length === 1 ? upstream : `${upstream} (+${headRemotes.length - 1} more)`;
  }
  return headRemotes.length === 1 ? headRemotes[0] : `${headRemotes[0]} (+${headRemotes.length - 1} more)`;
}
