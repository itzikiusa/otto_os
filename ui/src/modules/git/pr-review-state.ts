// Your own review on a PR — what the header's Approve / Request changes pair
// shows — and the optimistic patch applied the moment the forge accepts one,
// so the reviewer list, the header and the list row change at once instead of
// on the next 2-minute refresh (the reload that follows reconciles).
//
// "You" is the reviewer the daemon flags `is_me` (matched by the forge's
// stable user id). Only when no reviewer carries the flag — an older daemon,
// an identity the daemon couldn't look up, or you aren't a reviewer yet — do we
// fall back to matching names against your account handles, which misses on
// Bitbucket (a reviewer is a display name, the username an email).
import type { PrDetail, PrReviewer } from '../../lib/api/types';

export type MyReview = 'approved' | 'changes' | null;

/** Your reviewer row: the `is_me` one, else the first whose name matches a handle. */
export function myReviewer(reviewers: readonly PrReviewer[], isMe: (name: string) => boolean): PrReviewer | undefined {
  return reviewers.find((r) => r.is_me) ?? reviewers.find((r) => isMe(r.name));
}

export function myReview(pr: Pick<PrDetail, 'reviewers' | 'approved_by'>, isMe: (name: string) => boolean): MyReview {
  const r = myReviewer(pr.reviewers, isMe);
  if (r?.changes_requested) return 'changes';
  if (r?.approved) return 'approved';
  // GitLab lists approvers only in approved_by on some reads; the flag lives
  // on reviewers, so this one stays a name match.
  if (!r && pr.approved_by.some(isMe)) return 'approved';
  return null;
}

/** `pr` with your review set to `kind`. Your reviewer row is patched in place;
 *  when the list doesn't have you yet you are added under `fallbackName`,
 *  marked `is_me` so the next read of this object still finds you. */
export function withMyReview(pr: PrDetail, kind: Exclude<MyReview, null>, isMe: (name: string) => boolean, fallbackName: string): PrDetail {
  const now = new Date().toISOString();
  const mark = (r: PrReviewer): PrReviewer => ({ ...r, is_me: true, approved: kind === 'approved', changes_requested: kind === 'changes', reviewed_at: now });
  const hit = myReviewer(pr.reviewers, isMe);
  const reviewers = hit
    ? pr.reviewers.map((r) => (r === hit ? mark(r) : r))
    : [...pr.reviewers, mark({ name: fallbackName, approved: false, changes_requested: false, avatar_url: null, reviewed_at: null })];
  const me = hit?.name ?? fallbackName;
  const others = pr.approved_by.filter((n) => n !== me && !isMe(n));
  return { ...pr, reviewers, approved_by: kind === 'approved' ? [...others, me] : others };
}
