import type { PrComment, ReviewComment } from '../../lib/api/types';

// Re-posting an approved-but-"not posted" review comment is NOT always safe:
// the daemon never retries a forge 5xx because the forge may have created the
// comment anyway. Before a re-post the panel looks the comment up on the PR.

const norm = (s: string): string => s.replace(/\s+/g, ' ').trim();

/** Whether `c` already appears on the PR: same body text (the forge copy may
 *  carry a prefix — e.g. the general-comment fallback cites `path:line`), and,
 *  when the forge kept it inline, the same path + line. Replies count too. */
export function alreadyOnPr(c: ReviewComment, comments: PrComment[]): boolean {
  const body = norm(c.body);
  if (body === '') return false;
  const hit = (p: PrComment): boolean => {
    if (!norm(p.body).includes(body)) return false;
    // An inline copy must sit on the same anchor; a general copy (path null)
    // is the anchor-rejected fallback and matches by body alone.
    if (p.path !== null && c.path !== null && (p.path !== c.path || (c.line !== null && p.line !== c.line))) return false;
    return true;
  };
  const walk = (list: PrComment[]): boolean => list.some((p) => hit(p) || walk(p.replies ?? []));
  return walk(comments);
}
