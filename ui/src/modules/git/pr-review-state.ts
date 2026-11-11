// Your own review on a PR — what the header's Approve / Request changes pair
// shows — and the optimistic patch applied the moment the forge accepts one,
// so the reviewer list, the header and the list row change at once instead of
// on the next 2-minute refresh (the reload that follows reconciles).
import type { PrDetail, PrReviewer } from '../../lib/api/types';

export type MyReview = 'approved' | 'changes' | null;

export function myReview(pr: Pick<PrDetail, 'reviewers' | 'approved_by'>, isMe: (name: string) => boolean): MyReview {
  const r = pr.reviewers.find((x) => isMe(x.name));
  if (r?.changes_requested) return 'changes';
  if (r?.approved || pr.approved_by.some(isMe)) return 'approved';
  return null;
}

/** `pr` with your review set to `kind` (you are added as a reviewer if the
 *  list doesn't have you yet, under `fallbackName`). */
export function withMyReview(pr: PrDetail, kind: Exclude<MyReview, null>, isMe: (name: string) => boolean, fallbackName: string): PrDetail {
  const now = new Date().toISOString();
  const mark = (r: PrReviewer): PrReviewer => ({ ...r, approved: kind === 'approved', changes_requested: kind === 'changes', reviewed_at: now });
  const hit = pr.reviewers.some((r) => isMe(r.name));
  const reviewers = hit
    ? pr.reviewers.map((r) => (isMe(r.name) ? mark(r) : r))
    : [...pr.reviewers, mark({ name: fallbackName, approved: false, changes_requested: false, avatar_url: null, reviewed_at: null })];
  const others = pr.approved_by.filter((n) => !isMe(n));
  const me = reviewers.find((r) => isMe(r.name) || r.name === fallbackName)?.name ?? fallbackName;
  return { ...pr, reviewers, approved_by: kind === 'approved' ? [...others, me] : others };
}
