import type { Review } from '../../lib/api/types';

export function reviewFixError(review: Pick<Review, 'status' | 'pr_number' | 'scope_plan'>, target: number): string | null {
  if (review.status !== 'done' || review.pr_number === 0) return 'A completed PR review is required.';
  if (!review.scope_plan?.head_sha) return 'Re-run this review to assess its scope before fixing.';
  if (!Number.isFinite(target) || target < 1 || target > 10) return 'Choose a target from 1 to 10.';
  return null;
}
