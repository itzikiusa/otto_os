// "Tell the author on Slack…" (PR ⋯ menu): the prefilled Slack-as-me message
// after a review — sent only from the Slack dialog, after you pick the person.
import type { PrDetail } from '../../lib/api/types';
import { slackSend, type SlackSendRequest } from '../../lib/slackSend.svelte';

export function reviewDoneSlack(pr: Pick<PrDetail, 'number' | 'title' | 'url' | 'author' | 'comments'>, repoSlug: string): SlackSendRequest {
  const n = pr.comments.length;
  const comments = n === 0 ? 'no comments' : n === 1 ? '1 comment' : `${n} comments`;
  return {
    title: 'Tell the author on Slack',
    context: `${repoSlug} #${pr.number} · ${pr.title}`,
    to: pr.author,
    text: `Hi! I finished reviewing #${pr.number} “${pr.title}” — ${comments} on the PR${pr.url ? `: ${pr.url}` : '.'}`,
  };
}

/** Open the Slack dialog prefilled for `pr` (nothing is sent until Send). */
export function openReviewDoneSlack(pr: Parameters<typeof reviewDoneSlack>[0], repoSlug: string): void {
  slackSend.open(reviewDoneSlack(pr, repoSlug));
}
