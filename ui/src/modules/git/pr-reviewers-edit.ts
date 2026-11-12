// Adding / removing reviewers on an open PR (PrReviewersEdit): the edit the
// sheet sends (`PUT /repos/{id}/prs/{number}/reviewers`) and the words the
// outward confirm shows — who is added (the forge notifies them) and who is
// removed. Pure (unit/gitPrRound2.test.ts).
import type { EditPrReviewersReq, PrDetail } from '../../lib/api/types';

const same = (a: string, b: string): boolean => a.trim().toLowerCase() === b.trim().toLowerCase();

/** The request for the sheet's state: `added` names not already on the PR,
 *  `removed` names that are. Empty sides are left out; null = no change. */
export function reviewerEdit(current: readonly string[], added: readonly string[], removed: readonly string[]): EditPrReviewersReq | null {
  const add: string[] = [];
  for (const raw of added) {
    const n = raw.trim().replace(/^@/, '');
    if (n && !current.some((c) => same(c, n)) && !add.some((a) => same(a, n))) add.push(n);
  }
  const remove = current.filter((c) => removed.some((r) => same(r, c)));
  if (add.length === 0 && remove.length === 0) return null;
  return { ...(add.length ? { add } : {}), ...(remove.length ? { remove } : {}) };
}

/** The confirm's "What" line. */
export function reviewerEditWhat(edit: EditPrReviewersReq): string {
  const lines: string[] = [];
  if (edit.add?.length) lines.push(`Add: ${edit.add.join(', ')}`);
  if (edit.remove?.length) lines.push(`Remove: ${edit.remove.join(', ')}`);
  return lines.join('\n');
}

/** The confirm's "Who sees it" line. */
export function reviewerEditWho(edit: EditPrReviewersReq, providerName: string): string {
  const added = edit.add?.length ? `${edit.add.join(', ')} ${edit.add.length === 1 ? 'is' : 'are'} asked for a review and notified by ${providerName}. ` : '';
  return `${added}The author and the PR’s reviewers see the change.`;
}

/** The reviewer names a PR shows (the structured list, else approvers). */
export function currentReviewers(pr: Pick<PrDetail, 'reviewers' | 'approved_by'>): string[] {
  return pr.reviewers.length > 0 ? pr.reviewers.map((r) => r.name) : pr.approved_by.slice();
}
