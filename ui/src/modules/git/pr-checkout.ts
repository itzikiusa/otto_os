// "Check out" for a PR: fetch, switch (local edits stashed around it), then say
// whether the local branch actually matches the PR. The switch never pulls
// (docs/contracts/api.md, `POST /repos/{id}/checkout`), so a local branch from
// an earlier checkout stays at its old tip — the fetched status's `behind`
// (against its upstream, the PR's source branch on origin) is what tells.
import type { RepoStatusResp } from '../../lib/api/types';
import { plural } from '../../lib/plural.ts';

export type CheckoutVerdict =
  | { kind: 'current' }
  /** Local tip is behind the PR's branch on origin (nothing of its own). */
  | { kind: 'behind'; behind: number; ahead: number }
  /** Both sides have commits the other lacks — the PR branch was rewritten
   *  (force-pushed) since an earlier checkout, or you committed on it too.
   *  A fast-forward pull can't fix it; "Update to the PR" can (asks first,
   *  keeps your commits on a backup branch). */
  | { kind: 'diverged'; behind: number; ahead: number }
  /** Local commits the PR doesn't have. */
  | { kind: 'ahead'; ahead: number }
  /** No upstream: nothing to compare the local tip with. */
  | { kind: 'unknown' };

export function checkoutVerdict(s: Pick<RepoStatusResp, 'upstream' | 'ahead' | 'behind'>): CheckoutVerdict {
  if (!s.upstream) return { kind: 'unknown' };
  if (s.behind > 0 && s.ahead > 0) return { kind: 'diverged', behind: s.behind, ahead: s.ahead };
  if (s.behind > 0) return { kind: 'behind', behind: s.behind, ahead: s.ahead };
  if (s.ahead > 0) return { kind: 'ahead', ahead: s.ahead };
  return { kind: 'current' };
}

const commits = (n: number): string => plural(n, 'commit');

/** Toast wording for a verdict: level, title and body. */
export function checkoutToast(branch: string, v: CheckoutVerdict): { level: 'success' | 'warn'; title: string; body: string } {
  const stash = 'Any local changes were stashed first and restored.';
  switch (v.kind) {
    case 'current':
      return { level: 'success', title: `Checked out ${branch}`, body: stash };
    case 'behind':
      return {
        level: 'warn',
        title: `${branch} is behind the pull request`,
        body: `Your local branch is ${commits(v.behind)} behind${v.ahead > 0 ? ` and ${commits(v.ahead)} ahead` : ''}. Pull to get the PR’s latest commits.`,
      };
    case 'diverged':
      return {
        level: 'warn',
        title: `${branch} differs from the pull request`,
        body: `The PR branch was rewritten (force-pushed) or you committed on it: yours has ${commits(v.ahead)} the PR doesn’t and is ${commits(v.behind)} behind. Update to the PR’s version — your commits stay on a backup branch.`,
      };
    case 'ahead':
      return { level: 'warn', title: `Checked out ${branch}`, body: `Your local branch has ${commits(v.ahead)} the pull request doesn’t.` };
    case 'unknown':
      return { level: 'warn', title: `Checked out ${branch}`, body: 'It has no upstream branch, so Otto can’t tell whether it matches the pull request.' };
  }
}

/** The confirm before moving `branch` to the PR's version (local, but it
 *  rewrites where the branch points — never done silently). */
export function updateConfirm(branch: string, v: { ahead: number; behind: number }): { title: string; message: string; confirmLabel: string } {
  return {
    title: `Update ${branch} to the pull request?`,
    confirmLabel: 'Update branch',
    message:
      `${branch} moves to the PR’s version (${commits(v.behind)} you don’t have).\n\n` +
      `Your ${commits(v.ahead)} not on the PR ${v.ahead === 1 ? 'stays' : 'stay'} on a backup branch (otto-backup/${branch}-…) and in the reflog — nothing is deleted.\n\n` +
      'Uncommitted changes are kept; if the update would overwrite one, nothing changes and you are told.',
  };
}
