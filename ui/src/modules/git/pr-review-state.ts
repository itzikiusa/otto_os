// Your own review on a PR — what the header's Approve / Request changes pair
// shows — and the optimistic patch applied the moment the forge accepts one,
// so the reviewer list, the header and the list row change at once instead of
// on the next 2-minute refresh (the reload that follows reconciles).
//
// "You" is the reviewer the daemon flags `is_me` (matched by the forge's
// stable user id). Only when no reviewer carries the flag — an older daemon,
// an identity the daemon couldn't look up, or you aren't a reviewer yet — do we
// fall back to matching names against your account handles, which misses on
// Bitbucket (a reviewer is a display name, the username an email).
import type { PrDetail, PrMergeCheck, PrReviewSummary, PrReviewer } from '../../lib/api/types';

export type MyReview = 'approved' | 'changes' | null;

/** Your reviewer row: the `is_me` one, else the first whose name matches a handle. */
export function myReviewer(reviewers: readonly PrReviewer[], isMe: (name: string) => boolean): PrReviewer | undefined {
  return reviewers.find((r) => r.is_me) ?? reviewers.find((r) => isMe(r.name));
}

export function myReview(pr: Pick<PrDetail, 'reviewers' | 'approved_by'>, isMe: (name: string) => boolean): MyReview {
  const r = myReviewer(pr.reviewers, isMe);
  if (r?.changes_requested) return 'changes';
  if (r?.approved) return 'approved';
  // GitLab lists approvers only in approved_by on some reads; the flag lives
  // on reviewers, so this one stays a name match.
  if (!r && pr.approved_by.some(isMe)) return 'approved';
  return null;
}

/** Mirror `PrReviewSummary::from_counts` so optimistic patches update list chips immediately. */
export function reviewSummaryFrom(reviewers: readonly PrReviewer[], approved_by: readonly string[]): PrReviewSummary | null {
  const approvals = Math.max(reviewers.filter((r) => r.approved).length, approved_by.length);
  if (reviewers.some((r) => r.changes_requested)) return { status: 'changes_requested', approvals };
  if (approvals > 0) return { status: 'approved', approvals };
  if (reviewers.length > 0) return { status: 'waiting', approvals: 0 };
  return null;
}

function patchMergeChecks(
  checks: readonly PrMergeCheck[],
  reviewers: readonly PrReviewer[],
  approvals: number,
): PrMergeCheck[] {
  const changesAsked = reviewers.filter((r) => r.changes_requested).length;
  return checks.map((c) => {
    if (c.key === 'approvals') {
      const m = c.detail?.match(/of\s+(\d+)/) ?? c.label.match(/(\d+)/);
      if (m) {
        const req = Number(m[1]);
        if (Number.isFinite(req) && req > 0) {
          return { ...c, passed: approvals >= req, detail: `${approvals} of ${req}` };
        }
      }
    }
    if (c.key === 'no_changes_requested') {
      return {
        ...c,
        passed: changesAsked === 0,
        detail: changesAsked > 0 ? `${changesAsked} ${changesAsked === 1 ? 'reviewer' : 'reviewers'} requested changes` : null,
      };
    }
    return c;
  });
}

function applyDerivedReviewState(
  pr: PrDetail,
  reviewers: PrReviewer[],
  approved_by: string[],
  serverUpdatedAt?: string,
): PrDetail {
  const review = reviewSummaryFrom(reviewers, approved_by);
  const approvals = review?.approvals ?? 0;
  const nextUpdatedAt = serverUpdatedAt !== undefined ? serverUpdatedAt : pr.updated_at;
  return {
    ...pr,
    reviewers,
    approved_by,
    review,
    ...(nextUpdatedAt !== undefined ? { updated_at: nextUpdatedAt } : {}),
    ...(pr.merge_checks != null ? { merge_checks: patchMergeChecks(pr.merge_checks, reviewers, approvals) } : {}),
  };
}

/** `pr` with your review set to `kind`. Your reviewer row is patched in place;
 *  when the list doesn't have you yet you are added under `fallbackName`,
 *  marked `is_me` so the next read of this object still finds you. */
export function withMyReview(pr: PrDetail, kind: Exclude<MyReview, null>, isMe: (name: string) => boolean, fallbackName: string): PrDetail {
  const now = new Date().toISOString();
  const mark = (r: PrReviewer): PrReviewer => ({ ...r, is_me: true, approved: kind === 'approved', changes_requested: kind === 'changes', reviewed_at: now });
  const hit = myReviewer(pr.reviewers, isMe);
  const reviewers = hit
    ? pr.reviewers.map((r) => (r === hit ? mark(r) : r))
    : [...pr.reviewers, mark({ name: fallbackName, approved: false, changes_requested: false, avatar_url: null, reviewed_at: null })];
  const me = hit?.name ?? fallbackName;
  const others = pr.approved_by.filter((n) => n !== me && !isMe(n));
  const approved_by = kind === 'approved' ? [...others, me] : others;
  return applyDerivedReviewState(pr, reviewers, approved_by, pr.updated_at);
}

/** `pr` with your `kind` review withdrawn: your reviewer row stays (you are
 *  still a reviewer) but no longer approves / requests changes, and you leave
 *  `approved_by`. No-op when that isn't your current review. */
export function withoutMyReview(pr: PrDetail, kind: Exclude<MyReview, null>, isMe: (name: string) => boolean): PrDetail {
  const hit = myReviewer(pr.reviewers, isMe);
  const reviewers = hit
    ? pr.reviewers.map((r) => (r === hit ? { ...r, approved: kind === 'approved' ? false : r.approved, changes_requested: kind === 'changes' ? false : r.changes_requested } : r))
    : pr.reviewers;
  const approved_by = kind === 'approved' ? pr.approved_by.filter((n) => n !== hit?.name && !isMe(n)) : pr.approved_by;
  if (reviewers === pr.reviewers && approved_by === pr.approved_by) return pr;
  return applyDerivedReviewState(pr, reviewers, approved_by, pr.updated_at);
}

/** Guard against eventually-consistent forge read replicas (e.g. Bitbucket Cloud
 *  answering `approved: false` for 0.5–2 s right after `POST …/approve` returned
 *  204): keeps the confirmed write verdict applied until the replica catches up. */
export function reconcileConfirmedReview(
  fetched: PrDetail,
  expected: MyReview,
  isMe: (name: string) => boolean,
  fallbackName: string,
): { detail: PrDetail; caughtUp: boolean; serverUpdatedAt: string | undefined } {
  const serverUpdatedAt = fetched.updated_at;
  if (myReview(fetched, isMe) === expected) {
    return { detail: fetched, caughtUp: true, serverUpdatedAt };
  }
  if (expected === 'approved' || expected === 'changes') {
    const patched = withMyReview(fetched, expected, isMe, fallbackName);
    return {
      detail: serverUpdatedAt !== undefined ? { ...patched, updated_at: serverUpdatedAt } : patched,
      caughtUp: false,
      serverUpdatedAt,
    };
  }
  const cleared = withoutMyReview(withoutMyReview(fetched, 'approved', isMe), 'changes', isMe);
  return {
    detail: serverUpdatedAt !== undefined ? { ...cleared, updated_at: serverUpdatedAt } : cleared,
    caughtUp: false,
    serverUpdatedAt,
  };
}

export const REVIEW_RECONCILE_WINDOW_MS = 12_000;

export interface ReviewReconcilerEnv {
  now?: () => number;
  setTimer?: (fn: () => void, ms: number) => ReturnType<typeof setTimeout>;
  clearTimer?: (id: ReturnType<typeof setTimeout>) => void;
}

export interface ReviewReconcileMeta {
  detail: PrDetail;
  ownReviewSettled: boolean;
}

export interface ReviewReconciler {
  note(rid: string, num: number, kind: MyReview): void;
  resetFor(rid: string, num: number): void;
  clear(): void;
  reconcile(
    rid: string,
    num: number,
    fetched: PrDetail,
    isMe: (name: string) => boolean,
    fallbackName: string,
    onRetry?: () => void,
  ): PrDetail;
  reconcileWithMeta(
    rid: string,
    num: number,
    fetched: PrDetail,
    isMe: (name: string) => boolean,
    fallbackName: string,
    onRetry?: () => void,
  ): ReviewReconcileMeta;
}

/** Per-PR write-verdict reconciler with bounded background retry timers (1.5s, 3s, 4.5s). */
export function createReviewReconciler(env: ReviewReconcilerEnv = {}): ReviewReconciler {
  const nowFn = env.now ?? (() => Date.now());
  const setTimer = env.setTimer ?? ((fn, ms) => setTimeout(fn, ms));
  const clearTimer = env.clearTimer ?? ((id) => clearTimeout(id));
  let state: { rid: string; num: number; kind: MyReview; at: number; retries: number; settledClock: boolean } | null = null;
  let timer: ReturnType<typeof setTimeout> | null = null;
  const clearTimerOnly = (): void => {
    if (timer !== null) {
      clearTimer(timer);
      timer = null;
    }
  };
  const clear = (): void => {
    state = null;
    clearTimerOnly();
  };
  const reconcileWithMeta = (
    rid: string,
    num: number,
    fetched: PrDetail,
    isMe: (name: string) => boolean,
    fallbackName: string,
    onRetry?: () => void,
  ): ReviewReconcileMeta => {
    if (!state || state.rid !== rid || state.num !== num) {
      return { detail: fetched, ownReviewSettled: false };
    }
    const rec = reconcileConfirmedReview(fetched, state.kind, isMe, fallbackName);
    if (rec.caughtUp) {
      const ownReviewSettled = !state.settledClock;
      clear();
      return { detail: fetched, ownReviewSettled };
    }
    if (nowFn() - state.at > REVIEW_RECONCILE_WINDOW_MS) {
      clear();
      return { detail: fetched, ownReviewSettled: false };
    }
    if (onRetry && state.retries < 3 && timer === null) {
      state.retries += 1;
      timer = setTimer(() => {
        timer = null;
        onRetry();
      }, 1500 * state.retries);
    }
    return { detail: rec.detail, ownReviewSettled: false };
  };
  return {
    clear,
    note(rid, num, kind) {
      clear();
      state = { rid, num, kind, at: nowFn(), retries: 0, settledClock: false };
    },
    resetFor(rid, num) {
      if (state && (state.rid !== rid || state.num !== num)) clear();
    },
    reconcile(rid, num, fetched, isMe, fallbackName, onRetry) {
      return reconcileWithMeta(rid, num, fetched, isMe, fallbackName, onRetry).detail;
    },
    reconcileWithMeta,
  };
}
