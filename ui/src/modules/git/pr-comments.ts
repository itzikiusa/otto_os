// PR comment helpers (pure — unit/prComments.test.ts): the optimistic edits
// the Conversation applies before the forge answers (edit, delete, reactions),
// the "↪ replying to" label for flattened Bitbucket chains, and the chip a
// GitHub review summary wears.
import type { PrComment, PrReaction, PrReviewState, ReactionKey } from '../../lib/api/types';

/** The eight reactions every forge with a reactions API shares, in GitHub's order. */
export const REACTIONS: { key: ReactionKey; emoji: string; label: string }[] = [
  { key: 'thumbs_up', emoji: '👍', label: 'Thumbs up' },
  { key: 'thumbs_down', emoji: '👎', label: 'Thumbs down' },
  { key: 'laugh', emoji: '😄', label: 'Laugh' },
  { key: 'hooray', emoji: '🎉', label: 'Hooray' },
  { key: 'confused', emoji: '😕', label: 'Confused' },
  { key: 'heart', emoji: '❤️', label: 'Heart' },
  { key: 'rocket', emoji: '🚀', label: 'Rocket' },
  { key: 'eyes', emoji: '👀', label: 'Eyes' },
];
export const reactionEmoji = (key: ReactionKey): string => REACTIONS.find((r) => r.key === key)?.emoji ?? key;

/** `list` with your `key` reaction flipped: added (count + 1, `me`) or removed
 *  (count − 1; a pill nobody else holds disappears). Order is kept; a new key
 *  goes last. */
export function toggleReaction(list: readonly PrReaction[] | undefined, key: ReactionKey): PrReaction[] {
  const cur = list ?? [];
  const hit = cur.find((r) => r.key === key);
  if (!hit) return [...cur, { key, count: 1, me: true }];
  if (!hit.me) return cur.map((r) => (r.key === key ? { ...r, count: r.count + 1, me: true } : r));
  return cur.flatMap((r) => (r.key !== key ? [r] : r.count > 1 ? [{ ...r, count: r.count - 1, me: false }] : []));
}

/** Whether `key` is currently yours on `list`. */
export const reactedByMe = (list: readonly PrReaction[] | undefined, key: ReactionKey): boolean =>
  (list ?? []).some((r) => r.key === key && r.me);

/** A GitHub review summary (`review-<id>`): no edit, delete or reactions. */
export const isReviewSummary = (c: Pick<PrComment, 'id' | 'review_state'>): boolean =>
  !!c.review_state || c.id.startsWith('review-');

/** `comments` with comment `id` (a thread head or a reply) replaced by
 *  `fn(it)`; `fn` returning null removes it (a removed head takes its replies
 *  along — the reload that follows shows what the forge actually kept). */
export function mapComment(comments: readonly PrComment[], id: string, fn: (c: PrComment) => PrComment | null): PrComment[] {
  const out: PrComment[] = [];
  for (const c of comments) {
    if (c.id === id) {
      const next = fn(c);
      if (next) out.push(next);
      continue;
    }
    const replies = c.replies ?? [];
    out.push(replies.some((r) => r.id === id) ? { ...c, replies: mapComment(replies, id, fn) } : c);
  }
  return out;
}

/** Find a comment (head or reply) by id. */
export function findComment(comments: readonly PrComment[], id: string): PrComment | null {
  for (const c of comments) {
    if (c.id === id) return c;
    const r = (c.replies ?? []).find((x) => x.id === id);
    if (r) return r;
  }
  return null;
}

/** "replying to <author>" for a reply that answers ANOTHER reply in its
 *  thread (Bitbucket flattens such chains under the root); null for a direct
 *  answer to the head, or when the target is unknown. */
export function replyingTo(reply: Pick<PrComment, 'id' | 'in_reply_to'>, root: Pick<PrComment, 'id' | 'thread_id' | 'replies'>): string | null {
  const to = reply.in_reply_to;
  if (!to || to === root.id || to === root.thread_id || to === reply.id) return null;
  return (root.replies ?? []).find((r) => r.id === to)?.author ?? null;
}

/** The chip on a GitHub review summary. */
export function reviewChip(state: PrReviewState): { label: string; tone: 'success' | 'warning' | 'info' | 'dim' } {
  switch (state) {
    case 'approved':
      return { label: 'Approved', tone: 'success' };
    case 'changes_requested':
      return { label: 'Changes requested', tone: 'warning' };
    case 'commented':
      return { label: 'Commented', tone: 'info' };
    default:
      return { label: 'Dismissed', tone: 'dim' };
  }
}
