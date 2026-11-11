// The PR Timeline tab, built from what the PR view already loads (detail +
// commits): no provider has one cheap "activity" call that works the same on
// GitHub, Bitbucket and GitLab, so the events are assembled here. Pure, so
// unit/prTimeline.test.ts runs it under node --test.
import type { PrComment, PrCommit, PrDetail } from '../../lib/api/types';

export type TimelineEvent =
  | { kind: 'opened'; at: string; who: string }
  | { kind: 'commits'; at: string; who: string; commits: PrCommit[] }
  | { kind: 'comment'; at: string; who: string; comment: PrComment; replies: number }
  | { kind: 'approved'; at: string; who: string }
  | { kind: 'changes'; at: string; who: string }
  | { kind: 'closed'; at: string; who: string; state: 'merged' | 'declined' };

const ms = (iso: string): number => {
  const t = Date.parse(iso);
  return Number.isFinite(t) ? t : 0;
};

/**
 * Chronological events (oldest first). Consecutive commits by one author
 * within `burstMs` fold into one "pushed N commits" event; comment threads
 * appear once at their first comment, with the reply count. Events without
 * a usable timestamp (a reviewer the provider gives no time for) are left
 * out rather than pinned to 1970.
 */
export function buildTimeline(pr: PrDetail, commits: PrCommit[] | null, burstMs = 30 * 60_000): TimelineEvent[] {
  const out: TimelineEvent[] = [];
  if (pr.created_at) out.push({ kind: 'opened', at: pr.created_at, who: pr.author });

  const sorted = [...(commits ?? [])].filter((c) => ms(c.date) > 0).sort((a, b) => ms(a.date) - ms(b.date));
  let burst: { at: string; who: string; commits: PrCommit[] } | null = null;
  for (const c of sorted) {
    const last = burst?.commits[burst.commits.length - 1];
    if (burst && last && burst.who === c.author && ms(c.date) - ms(last.date) <= burstMs) {
      burst.commits.push(c);
      continue;
    }
    if (burst) out.push({ kind: 'commits', ...burst });
    burst = { at: c.date, who: c.author, commits: [c] };
  }
  if (burst) out.push({ kind: 'commits', ...burst });

  for (const c of pr.comments) {
    if (ms(c.created_at) === 0) continue;
    out.push({ kind: 'comment', at: c.created_at, who: c.author, comment: c, replies: c.replies?.length ?? 0 });
  }
  for (const r of pr.reviewers) {
    if (!r.reviewed_at || ms(r.reviewed_at) === 0) continue;
    if (r.approved) out.push({ kind: 'approved', at: r.reviewed_at, who: r.name });
    else if (r.changes_requested) out.push({ kind: 'changes', at: r.reviewed_at, who: r.name });
  }
  if (pr.state === 'merged' || pr.state === 'declined') {
    out.push({ kind: 'closed', at: pr.updated_at, who: '', state: pr.state });
  }
  // Stable: equal timestamps keep insertion order (opened → commits → …).
  return out
    .map((e, i) => ({ e, i }))
    .sort((a, b) => ms(a.e.at) - ms(b.e.at) || a.i - b.i)
    .map(({ e }) => e);
}
