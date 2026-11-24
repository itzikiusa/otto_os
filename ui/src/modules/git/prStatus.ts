// A pull request in words (G3): the ONE status line its page leads with and
// the ONE next step its primary offers (Fix checks → Review → Merge), plus
// the list row's word. Pure (types only) — unit/gitPrStatus.test.ts.
import type { PrChecksResp, PrDetail, PrSummary } from '../../lib/api/types';

export type PrTone = 'danger' | 'warning' | 'success' | 'calm';
export type PrNext = 'fix-checks' | 'review' | 'merge' | 'none';

export interface PrStatusLine {
  words: string;
  tone: PrTone;
  next: PrNext;
  /** "Fix checks" opens this (the first failing check's page). */
  failingUrl: string | null;
}

/** `mine`: your current review on it ('approved' / 'changes' / none). */
export function prStatusLine(pr: PrDetail, checks: PrChecksResp | null, mine: 'approved' | 'changes' | null): PrStatusLine {
  const none = (words: string, tone: PrTone = 'calm'): PrStatusLine => ({ words, tone, next: 'none', failingUrl: null });
  if (pr.state === 'merged') return none('Merged');
  if (pr.state === 'declined') return none('Closed without merging');
  const failing = checks?.checks.filter((c) => c.state === 'failure') ?? [];
  const total = checks?.ci.total ?? 0;
  if (failing.length > 0 || (checks?.ci.failed ?? 0) > 0) {
    const n = Math.max(failing.length, checks?.ci.failed ?? 0);
    return { words: `Checks failing · ${n} of ${total}`, tone: 'danger', next: 'fix-checks', failingUrl: failing.find((c) => c.url)?.url ?? checks?.ci.url ?? null };
  }
  if (pr.mergeable === false) return { words: `Conflicts with ${pr.target_branch}`, tone: 'danger', next: 'review', failingUrl: null };
  const requested = pr.reviewers.filter((r) => r.changes_requested).map((r) => r.name);
  if (requested.length > 0) {
    return { words: `Changes requested by ${requested.slice(0, 2).join(', ')}${requested.length > 2 ? ` +${requested.length - 2}` : ''}`, tone: 'warning', next: 'review', failingUrl: null };
  }
  if (pr.draft) return { words: 'Draft', tone: 'calm', next: 'review', failingUrl: null };
  const pending = checks?.checks.filter((c) => c.state === 'pending').length ?? 0;
  if (pending > 0) return { words: `Checks running · ${pending} of ${total}`, tone: 'calm', next: 'review', failingUrl: null };
  const approvals = Math.max(pr.reviewers.filter((r) => r.approved).length, pr.approved_by.length);
  if (approvals > 0 || mine === 'approved') {
    return { words: approvals > 1 ? `Ready to merge · ${approvals} approvals` : 'Ready to merge', tone: 'success', next: 'merge', failingUrl: null };
  }
  return { words: 'Waiting for review', tone: 'calm', next: 'review', failingUrl: null };
}

export const NEXT_LABEL: Record<Exclude<PrNext, 'none'>, string> = { 'fix-checks': 'Fix checks', review: 'Review', merge: 'Merge' };

/** The list row's word: what the PR needs, in the shared vocabulary. */
export function prRowWord(pr: PrSummary): { word: string; tone: PrTone } {
  if (pr.state === 'merged') return { word: 'Merged', tone: 'calm' };
  if (pr.state === 'declined') return { word: 'Closed', tone: 'calm' };
  const ci = pr.ci_status;
  if (ci === 'failing' || ci === 'failure') return { word: 'Checks failing', tone: 'danger' };
  if (pr.review?.status === 'changes_requested') return { word: 'Changes requested', tone: 'warning' };
  if (pr.draft) return { word: 'Draft', tone: 'calm' };
  if (pr.review?.status === 'approved') return { word: 'Approved', tone: 'success' };
  if (pr.review?.status === 'waiting') return { word: 'Review requested', tone: 'warning' };
  if (ci === 'pending') return { word: 'Checks running', tone: 'calm' };
  return { word: 'Open', tone: 'calm' };
}

/** The pull request the list opens on (never "pick one"): what needs you
 *  first — your PR with failing checks or changes requested, then one
 *  waiting for a review — else the first row. */
export function firstNeedingPr(prs: readonly PrSummary[], isMine: (pr: PrSummary) => boolean): PrSummary | null {
  const open = prs.filter((p) => p.state === 'open');
  const word = (p: PrSummary): string => prRowWord(p).word;
  return (
    open.find((p) => isMine(p) && (word(p) === 'Checks failing' || word(p) === 'Changes requested')) ??
    open.find((p) => !isMine(p) && word(p) === 'Review requested') ??
    prs[0] ??
    null
  );
}
