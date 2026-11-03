// What a running step is doing and waiting on, in words — derived from the
// reviews a review_run step launched (one per repo in a fan-out). Pure: unit
// tested in ui/unit/workflowStepProgress.test.ts.
import type { Review, ReviewAgentState } from '../../lib/api/types';

/** Agent statuses that will never change again on their own. */
const SETTLED = new Set(['done', 'error', 'skipped', 'cancelled']);

/** A step reads as "stuck" after this long with every child settled. */
export const STUCK_AFTER_MS = 10 * 60_000;

export interface RepoProgress {
  review: Review;
  /** Reviewers (summarizer excluded). */
  reviewers: ReviewAgentState[];
  summarizer: ReviewAgentState | null;
  done: number;
  running: number;
  /** Rows with no session yet: waiting for a free reviewer slot. */
  queued: number;
  finished: boolean;
}

export interface StepProgress {
  /** Short phase word: Queued / Reviewing / Summarizing / Finishing / Done. */
  phase: string;
  /** e.g. "2/2 repos · 12/13 agents done". */
  detail: string;
  /** What the step waits on right now (null once nothing is pending). */
  waitingOn: string | null;
  /** Every child (agent / review) has settled — nothing will move the step
   *  forward on its own except the engine noticing. */
  settled: boolean;
  repos: RepoProgress[];
}

export function repoProgress(review: Review): RepoProgress {
  const rows = review.agents ?? [];
  const reviewers = rows.length > 1 ? rows.slice(0, -1) : rows;
  const summarizer = rows.length > 1 ? rows[rows.length - 1] : null;
  const done = reviewers.filter((a) => SETTLED.has(a.status)).length;
  const running = reviewers.filter((a) => a.status === 'running' || a.status === 'waiting').length;
  const queued = reviewers.filter((a) => a.status === 'pending').length;
  return { review, reviewers, summarizer, done, running, queued, finished: review.status !== 'running' };
}

/** `null` when the step launched no review (nothing to say beyond its logs). */
export function stepProgress(reviews: Review[]): StepProgress | null {
  if (!reviews.length) return null;
  const repos = reviews.map(repoProgress);
  const agentRows = repos.flatMap((r) => [...r.reviewers, ...(r.summarizer ? [r.summarizer] : [])]);
  const agentsDone = agentRows.filter((a) => SETTLED.has(a.status)).length;
  const reposDone = repos.filter((r) => r.finished).length;
  const reposPart = repos.length > 1 ? `${reposDone}/${repos.length} repos · ` : '';
  const detail = `${reposPart}${agentsDone}/${agentRows.length} agents done`;
  const current = repos.find((r) => !r.finished);
  if (!current) return { phase: 'Done', detail, waitingOn: null, settled: true, repos };
  const allSettled = repos.every((r) => r.finished || [...r.reviewers, ...(r.summarizer ? [r.summarizer] : [])].every((a) => SETTLED.has(a.status)));
  let phase: string;
  let waitingOn: string | null;
  if (current.running > 0) {
    phase = 'Reviewing';
    waitingOn = current.queued > 0
      ? `${current.running} reviewing · ${current.queued} queued for a reviewer slot`
      : `${current.running} ${current.running === 1 ? 'reviewer' : 'reviewers'}`;
  } else if (current.queued > 0) {
    phase = 'Queued';
    waitingOn = 'a free reviewer slot (slots are shared by every review on this Mac)';
  } else if (current.summarizer && !SETTLED.has(current.summarizer.status)) {
    phase = 'Summarizing';
    waitingOn = 'the summarizer';
  } else {
    phase = 'Finishing';
    waitingOn = 'the review to settle';
  }
  return { phase, detail, waitingOn, settled: allSettled, repos };
}

/** Track when a step's children first all settled while the step still ran:
 *  returns the new "settled since" stamp (null when not settled). */
export function settledSince(prev: number | null, settled: boolean, now: number): number | null {
  if (!settled) return null;
  return prev ?? now;
}

export function isStuck(since: number | null, now: number): boolean {
  return since !== null && now - since >= STUCK_AFTER_MS;
}
