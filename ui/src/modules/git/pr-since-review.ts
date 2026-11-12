// "Changes since your review": the Code tab's commit picker offers the diff
// from the head you last reviewed at (approved, requested changes or
// commented on, through Otto) to the PR's current head — what a re-review
// actually needs to read after the author pushed fixes.
//
// The forges don't agree on where a review's commit lives (GitHub reviews
// carry `commit_id`; a Bitbucket approval names no commit at all), so the
// head is remembered here, per PR, the moment your review lands. The diff is
// the local clone's `range:<reviewed>..<head>` (fetched once when the clone
// lacks a commit — see PrDetail's pickCommit), so it also works after a
// force-push as long as the old head is still in the clone.
//
// A commit-picker selection is a commit sha, or a `range:a..b` token for this
// view; the helpers below turn either into a diff target and a label.
import type { PrCommit } from '../../lib/api/types';
import type { MenuItem } from '../../lib/contextmenu.svelte';

const KEY = (repoId: string, n: number): string => `otto_pr_reviewed_head:${repoId}#${n}`;

/** Remember `head` as the one you reviewed PR `n` at ('' / null: nothing). */
export function rememberReviewedHead(repoId: string, n: number, head: string | null | undefined): void {
  const h = (head ?? '').trim();
  if (!h) return;
  try {
    localStorage.setItem(KEY(repoId, n), h);
  } catch {
    /* blocked storage: the offer just isn't made */
  }
}

/** The head you last reviewed PR `n` at, if Otto saw it. */
export function reviewedHead(repoId: string, n: number): string | null {
  try {
    return localStorage.getItem(KEY(repoId, n)) || null;
  } catch {
    return null;
  }
}

/** Same commit, allowing one side to be abbreviated (Bitbucket's 12 chars). */
export function sameSha(a: string, b: string): boolean {
  const x = a.trim().toLowerCase();
  const y = b.trim().toLowerCase();
  return x.length >= 7 && y.length >= 7 && (x.startsWith(y) || y.startsWith(x));
}

/** The picker token for "changes since your review", or null when there is
 *  nothing to offer (never reviewed here, head unknown, or nothing new). */
export function sinceReviewPick(reviewed: string | null, head: string | null | undefined): string | null {
  const h = (head ?? '').trim();
  if (!reviewed || !h || sameSha(reviewed, h)) return null;
  return `range:${reviewed}..${h}`;
}

export const isRangePick = (sel: string): boolean => sel.startsWith('range:');

/** The `/repos/{id}/diff?target=` of a picker selection. */
export const pickTarget = (sel: string): string => (isRangePick(sel) ? sel : `commit:${sel}`);

/** The picker button's label for a selection (`commit`: its row, if known). */
export function pickLabel(sel: string | null, commit: PrCommit | null): string {
  if (!sel) return 'All commits';
  if (isRangePick(sel)) return 'Changes since your review';
  return commit?.subject ?? sel.slice(0, 7);
}

/** The picker's "since your review" entry (none when there's nothing new). */
export function sinceReviewItems(pick: string | null, current: string | null, choose: (sel: string) => void): MenuItem[] {
  if (!pick) return [];
  const from = pick.slice('range:'.length).split('..')[0].slice(0, 7);
  return [{ label: 'Changes since your review', icon: 'clock', hint: from, checked: current === pick, pinned: true, title: `What changed after you reviewed at ${from}`, action: () => choose(pick) }];
}
