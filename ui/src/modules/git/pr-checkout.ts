// "Check out" for a PR: fetch, switch (local edits stashed around it), then say
// whether the local branch actually matches the PR. The switch never pulls
// (docs/contracts/api.md, `POST /repos/{id}/checkout`), so a local branch from
// an earlier checkout stays at its old tip — the fetched status's `behind`
// (against its upstream, the PR's source branch on origin) is what tells.
import type { RepoStatusResp } from '../../lib/api/types';
import { plural } from '../../lib/plural.ts';

export type CheckoutVerdict =
  | { kind: 'current' }
  /** Local tip is behind the PR's branch on origin (maybe also ahead). */
  | { kind: 'behind'; behind: number; ahead: number }
  /** Local commits the PR doesn't have. */
  | { kind: 'ahead'; ahead: number }
  /** No upstream: nothing to compare the local tip with. */
  | { kind: 'unknown' };

export function checkoutVerdict(s: Pick<RepoStatusResp, 'upstream' | 'ahead' | 'behind'>): CheckoutVerdict {
  if (!s.upstream) return { kind: 'unknown' };
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
    case 'ahead':
      return { level: 'warn', title: `Checked out ${branch}`, body: `Your local branch has ${commits(v.ahead)} the pull request doesn’t.` };
    case 'unknown':
      return { level: 'warn', title: `Checked out ${branch}`, body: 'It has no upstream branch, so Otto can’t tell whether it matches the pull request.' };
  }
}
