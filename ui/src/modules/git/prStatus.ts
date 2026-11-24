// A pull request in words (G3): the ONE status line its page leads with and
// the ONE next step its primary offers (Fix checks → Review → Merge), plus
// the list row's word. Pure (types only) — unit/gitPrStatus.test.ts.
import type { PrChecksResp, PrDetail, PrSummary } from '../../lib/api/types';
import { isCiCheck } from './pr-merge-checks.ts';

export type PrTone = 'danger' | 'warning' | 'success' | 'calm';
export type PrNext = 'fix-checks' | 'review' | 'merge' | 'none';

export interface PrStatusLine {
  words: string;
  tone: PrTone;
  next: PrNext;
  /** "Fix checks" opens this (the first failing check's page). */
  failingUrl: string | null;
  /** Why Merge is not the next step (the ⋯ menu's Merge says it); null when it is. */
  blocked: string | null;
}

/** `mine`: your current review on it ('approved' / 'changes' / none). */
export function prStatusLine(pr: PrDetail, checks: PrChecksResp | null, mine: 'approved' | 'changes' | null): PrStatusLine {
  const none = (words: string, tone: PrTone = 'calm'): PrStatusLine => ({ words, tone, next: 'none', failingUrl: null, blocked: words });
  const review = (words: string, tone: PrTone, blocked = words): PrStatusLine => ({ words, tone, next: 'review', failingUrl: null, blocked });
  if (pr.state === 'merged') return none('Merged');
  if (pr.state === 'declined') return none('Closed without merging');
  const failing = checks?.checks.filter((c) => c.state === 'failure') ?? [];
  const total = checks?.ci.total ?? 0;
  if (failing.length > 0 || (checks?.ci.failed ?? 0) > 0) {
    const n = Math.max(failing.length, checks?.ci.failed ?? 0);
    // CI checks (the /checks probe) — not the forge's merge rules, which the
    // details card lists: one count, one word, on one line.
    return { words: `${n} of ${Math.max(total, n)} checks failing`, tone: 'danger', next: 'fix-checks', failingUrl: failing.find((c) => c.url)?.url ?? checks?.ci.url ?? null, blocked: 'Checks failing' };
  }
  if (pr.mergeable === false) return review(`Conflicts with ${pr.target_branch}`, 'danger');
  const requested = pr.reviewers.filter((r) => r.changes_requested).map((r) => r.name);
  if (requested.length > 0) {
    return review(`Changes requested by ${requested.slice(0, 2).join(', ')}${requested.length > 2 ? ` +${requested.length - 2}` : ''}`, 'warning');
  }
  if (pr.draft) return review('Draft', 'calm');
  const pending = checks?.checks.filter((c) => c.state === 'pending').length ?? 0;
  if (pending > 0) return review(`${pending} of ${total} checks running`, 'calm', 'Checks running');
  // A forge rule that is not CI (CI is the line above) — "At least 2 approvals".
  const rule = (pr.merge_checks ?? []).find((c) => !c.passed && !isCiCheck(c));
  if (rule) return review(`Merge blocked · ${rule.label}`, 'warning', rule.label);
  const approvals = Math.max(pr.reviewers.filter((r) => r.approved).length, pr.approved_by.length);
  if (approvals > 0 || mine === 'approved') {
    return { words: approvals > 1 ? `Ready to merge · ${approvals} approvals` : 'Ready to merge', tone: 'success', next: 'merge', failingUrl: null, blocked: null };
  }
  return review('Waiting for review', 'calm', 'No approval yet');
}

/** Where the header's "Review ▾" (Approve · Request changes · Comment) sits:
 *  the primary while reviewing is your next step, else the one visible
 *  secondary (after you reviewed it carries your verdict); none once closed. */
export function reviewPlacement(pr: PrDetail, status: PrStatusLine, mine: 'approved' | 'changes' | null): 'primary' | 'secondary' | 'none' {
  if (pr.state !== 'open') return 'none';
  return status.next === 'review' && mine === null ? 'primary' : 'secondary';
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
