import { api } from '../../lib/api/client';
import { confirmer } from '../../lib/confirm.svelte';
import type { DeclineReviewCommentReq, ReviewComment, ReviewTriage } from '../../lib/api/types';
import { UNVERIFIED_MARK } from './reviewTriage';

// The network / dialog half of reviewTriage.ts (which stays pure for unit tests).

export function fetchTriage(reviewId: string): Promise<ReviewTriage> {
  return api.get<ReviewTriage>(`/reviews/${reviewId}/triage`);
}

/** Ask once what a decline means for the comment's finding. `null` = cancelled. */
export async function askDismissal(c: ReviewComment): Promise<DeclineReviewCommentReq | null> {
  const head = c.body.replace(UNVERIFIED_MARK, '').split('\n')[0];
  const picked = await confirmer.choose(
    `Decline “${head}”? Its tracked finding is closed with it, so it stops counting toward merge readiness.`,
    {
      title: 'Decline comment',
      options: [
        { label: 'Not a problem', value: 'false_positive', kind: 'primary' },
        { label: 'Won’t fix', value: 'waived' },
      ],
    },
  );
  return picked.value === 'false_positive' || picked.value === 'waived' ? { dismiss: picked.value } : null;
}
