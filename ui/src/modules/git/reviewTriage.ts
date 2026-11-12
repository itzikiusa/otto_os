import type { CommentTriage, ReviewComment, ReviewTriage } from '../../lib/api/types';

// How the review panel decides what "Post all" sends. A draft comment and its
// tracked finding are triaged together (GET /reviews/{id}/triage links them):
// a comment whose finding was dismissed, one another review already posted,
// and an UNVERIFIED reviewer claim (the final verifier never checked it) are
// left out — the last only until the person explicitly includes them.

export const UNVERIFIED_MARK = '[unverified] ';

/** comment id → its triage row (empty while the triage is not loaded). */
export function triageIndex(t: ReviewTriage | null): Map<string, CommentTriage> {
  return new Map((t?.comments ?? []).map((c) => [c.comment_id, c]));
}

const DISMISSED = new Set(['false_positive', 'waived']);

export function isUnverified(c: ReviewComment, t?: CommentTriage): boolean {
  return t?.unverified ?? c.body.startsWith(UNVERIFIED_MARK);
}

/** Why a postable comment is NOT in "Post all" ('' = it is). */
export function postAllSkip(c: ReviewComment, t: CommentTriage | undefined): '' | 'dismissed' | 'on_pr' | 'unverified' {
  if (t?.finding_status && DISMISSED.has(t.finding_status)) return 'dismissed';
  if (t?.on_pr) return 'on_pr';
  if (isUnverified(c, t)) return 'unverified';
  return '';
}

export interface PostAllPlan {
  /** Sent by "Post all". */
  send: ReviewComment[];
  /** Unverified claims — sent only when the person includes them. */
  unverified: ReviewComment[];
  /** Left out: finding dismissed, or already on the PR. */
  dismissed: number;
  onPr: number;
}

export function postAllPlan(postable: ReviewComment[], index: Map<string, CommentTriage>): PostAllPlan {
  const plan: PostAllPlan = { send: [], unverified: [], dismissed: 0, onPr: 0 };
  for (const c of postable) {
    const why = postAllSkip(c, index.get(c.id));
    if (why === '') plan.send.push(c);
    else if (why === 'unverified') plan.unverified.push(c);
    else if (why === 'dismissed') plan.dismissed++;
    else plan.onPr++;
  }
  return plan;
}

const SEV_RANK: Record<string, number> = { bug: 0, warn: 1, info: 2 };

/** Comments grouped by file (general comments last), each file sorted by
 *  severity then line — the order the confirm shows them in. */
export function groupByFile(cs: ReviewComment[]): { path: string | null; comments: ReviewComment[] }[] {
  const groups = new Map<string | null, ReviewComment[]>();
  for (const c of cs) groups.set(c.path, [...(groups.get(c.path) ?? []), c]);
  return [...groups.entries()]
    .sort(([a], [b]) => (a === null ? 1 : b === null ? -1 : a.localeCompare(b)))
    .map(([path, list]) => ({
      path,
      comments: [...list].sort((x, y) => (SEV_RANK[x.severity] ?? 3) - (SEV_RANK[y.severity] ?? 3) || (x.line ?? 0) - (y.line ?? 0)),
    }));
}
