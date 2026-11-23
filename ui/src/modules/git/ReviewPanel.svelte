<script lang="ts">
  import { registry } from '../../lib/commands.svelte';
  import { plural } from '../../lib/plural';
  // AI review panel: start review agents, refresh via WS reviewBus (no poll),
  // approve/decline individual draft comments. Supports live per-agent progress
  // cards, a configure-agents modal, and a merge-readiness panel.
  import { onDestroy, tick, untrack } from 'svelte';
  import { summaryFallback, summaryFallbackTitle } from './reviewFallback';
  import { toastError } from '../../lib/toastError';
  import { api, ApiError, isAbortError } from '../../lib/api/client';
  import { prDiffFile, prDiffSummary } from '../../lib/diff/diff-load';
  import { alreadyOnPr } from './reviewPost';
  import type {
    Review, ReviewComment,
    RepoReviewConfigResp, StartReviewReq, FileDiff, DiffLine, MergeReadiness, ReviewTriage, EditReviewCommentReq, PrDetail,
  } from '../../lib/api/types';
  import { postAllPlan, triageIndex, type PostAllPlan } from './reviewTriage';
  import { askDismissal, fetchTriage } from './reviewTriageApi';
  import ReviewConfigModal from './ReviewConfigModal.svelte';
  import PostCommentsConfirm from './PostCommentsConfirm.svelte';
  import ReviewCommentTriage from './ReviewCommentTriage.svelte';
  import ReviewMergeGate from './ReviewMergeGate.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import Modal from '../../lib/components/Modal.svelte';
  import AgentChip from '../../lib/components/AgentChip.svelte';
  import { rel } from '../../lib/stores/now.svelte';
  import JiraIssuePicker from '../agents/JiraIssuePicker.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { git } from '../../lib/stores/git.svelte';
  import { confirmOutward } from '../../lib/confirmOutward';
  import ProviderIcon from '../../lib/components/ProviderIcon.svelte';
  import ReviewAgents from './ReviewAgents.svelte';
  import ReviewScope from './ReviewScope.svelte';
  import ReviewIncomplete from './ReviewIncomplete.svelte';
  import ReviewPollStale from './ReviewPollStale.svelte';
  import FixReviewButton from './FixReviewButton.svelte';
  import StatusBadge from '../../lib/components/StatusBadge.svelte';
  import { runStatus } from '../../lib/status';
  import FindingsBoard from './FindingsBoard.svelte';
  import { reviewBus } from '../../lib/events.svelte';
  import { appLive } from '../../lib/live';
  import { snippetLinesFor } from './reviewSnippetIndex';
  import { repoReviewConfigBadge, shouldCollapseReviewInputs } from './gitUiHelpers';
  import { renderPrMarkdown } from './prMarkdown';

  interface Props {
    repoId: string;
    prNumber: number;
  }
  let { repoId, prNumber }: Props = $props();

  // Server payloads, replaced wholesale (never mutated): `$state.raw`.
  let review: Review | null = $state.raw(null);
  // History: all past runs (newest first); review is always history[0] when set
  let history: Review[] = $state.raw([]);
  /** JSON of the last review adopted from a poll/bus fetch: an unchanged
   *  response is dropped instead of re-rendering every comment card. */
  let lastReviewJson = '';
  let lastAdopted: Review | null = null;
  function adoptReview(r: Review): boolean {
    const json = JSON.stringify(r);
    if (review !== null && review === lastAdopted && json === lastReviewJson) return false;
    lastReviewJson = json;
    lastAdopted = r;
    review = r;
    history = history.length > 0 ? [r, ...history.slice(1)] : [r];
    return true;
  }
  let loading = $state(true);
  /** A failed history load — shown inline with Retry instead of falling
   *  through to "No review yet" (which offered to start a duplicate run). */
  let loadError = $state<string | null>(null);
  let starting = $state(false);
  let cancelling = $state(false);
  // Reads started before a new run/cancel/retry must not restore old progress.
  let reviewGeneration = 0;
  let disposed = false;
  function cancelPendingPostConfirm(): void {
    if (!postConfirm) return;
    const pending = postConfirm;
    postConfirm = null;
    pending.done(null);
  }
  onDestroy(() => {
    disposed = true;
    reviewGeneration++;
    if (pollTimer !== null) {
      clearTimeout(pollTimer);
      pollTimer = null;
    }
    cancelPendingPostConfirm();
  });
  // Fallback poll (visibility-gated) used only while the review is running and
  // no WS event has arrived yet — keeps the panel alive if the WS drops.
  let pollTimer: ReturnType<typeof setTimeout> | null = null;
  let pollCount = $state(0);
  let pollPaused = $state(false);
  let pollStale = $state(false);
  // Merge-readiness data from the dedicated endpoint (fetched after a run completes).
  let mergeReadiness: MergeReadiness | null = $state(null);
  let mergeReadinessLoading = $state(false);
  let readinessError = $state<string | null>(null);
  let readinessRequest = 0;
  // Each comment's finding + copies already on the PR (the board loads its own findings).
  let triage: ReviewTriage | null = $state.raw(null);
  const triageIx = $derived(triageIndex(triage));

  function pollDelay(count: number): number {
    // While the event socket is up, `review_changed` (reviewBus, below)
    // drives the refetches; this chain is only a 30 s safety net then.
    if (appLive.connected()) return 30_000;
    if (count < 5) return 2000;
    if (count < 15) return 5000;
    return 10000;
  }
  let historyExpanded: Record<number, boolean> = $state({});
  // A child <ReviewAgents> retried one agent: adopt the refreshed review and
  // resume polling so we keep tracking the re-run.
  function onAgentRetried(r: Review): void {
    if (disposed) return;
    reviewGeneration++;
    review = r;
    history = history.length > 0 ? [r, ...history.slice(1)] : [r];
    pollCount = 0;
    pollStale = false;
    schedulePoll();
  }

  let actionBusy: Record<string, 'approve' | 'decline' | 'edit'> = $state({});
  /** Comment id → the body being edited (a person's edit of the agent draft). */
  let editingBody: Record<string, string> = $state({});

  let showJiraPicker = $state(false);
  // The currently attached issue (persists across re-runs within this component)
  let attachedIssue: { account_id: string; key: string; summary: string } | null = $state(null);
  let attachStoryBtnEl = $state<HTMLButtonElement | null>(null);

  // Free-text guidance for the review agents (e.g. "what to focus on"). Optional;
  // persists across re-runs within this component.
  let reviewContext = $state('');
  // On a completed review, collapse the Jira/focus inputs behind "Re-run review…".
  let rerunOptionsOpen = $state(false);

  let showConfig = $state(false);
  let repoCfg: RepoReviewConfigResp | null = $state(null);

  // Diff state for per-comment snippets: ONLY the files that carry a comment,
  // fetched per file through the PR diff cache shared with the Files tab
  // (PrDetail) — never the whole PR diff.
  let snippetFiles: Map<string, FileDiff> = $state.raw(new Map());
  let diffLoading = false;
  let diffAbort: AbortController | null = null;
  onDestroy(() => diffAbort?.abort());
  // Per-comment collapse state: true = expanded (default for draft)
  let diffExpanded: Record<string, boolean> = $state({});

  const MAX_POLLS = 600; // ~20 min at 2s — covers long multi-agent live reviews

  $effect(() => {
    rerunOptionsOpen = false;
    void load(repoId, prNumber);
    document.addEventListener('visibilitychange', handleVisibilityChange);
    return () => {
      if (pollTimer !== null) clearTimeout(pollTimer);
      document.removeEventListener('visibilitychange', handleVisibilityChange);
    };
  });

  // Subscribe to the reviewBus WS events. When a review_changed arrives for
  // the current PR's open review, immediately re-fetch the review (and, on
  // terminal status, also fetch findings + merge-readiness). This replaces the
  // fixed-interval poll for the running state as the primary progress channel.
  $effect(() => {
    // Access reviewBus.tick reactively so this re-runs on every tick.
    const _tick = reviewBus.tick;
    const evReviewId = reviewBus.reviewId;
    const evStatus = reviewBus.status;
    if (!evReviewId) return;
    // Only react when the event matches our open review or when this PR just
    // started a review (review may be null during the first tick). Read `review`
    // WITHOUT subscribing: refreshFromBus reassigns `review` (a fresh object on
    // every fetch), and if this effect tracked that read the write would
    // re-invalidate the effect and re-fire the fetch forever — a self-feeding
    // loop that hammered the endpoint and re-rendered the whole done-state panel
    // (findings + proof packs) until a hard reload. We only want to re-run on a
    // new reviewBus tick, never on our own review writes.
    const cur = untrack(() => review);
    if (cur && cur.id !== evReviewId) return;
    void refreshFromBus(evReviewId, evStatus);
  });

  /** Re-fetch the review when the WS bus fires. */
  async function refreshFromBus(evReviewId: string, evStatus: string): Promise<void> {
    const generation = reviewGeneration;
    try {
      const r = await api.get<Review>(`/repos/${repoId}/prs/${prNumber}/review`);
      if (disposed || generation !== reviewGeneration) return;
      adoptReview(r);
      pollStale = false;
      // Kill the fallback poll if the review is now terminal.
      if (r.status !== 'running') {
        if (pollTimer !== null) { clearTimeout(pollTimer); pollTimer = null; }
        pollCount = 0;
        if (r.status === 'done') {
          void loadDiff(repoId, prNumber);
          void loadFindingsAndReadiness(r.id);
        }
      }
    } catch {
      // If the fetch fails, fall through to the fallback poll.
    }
  }

  /** Load the merge readiness and the comment triage for a completed review. */
  async function loadFindingsAndReadiness(reviewId: string): Promise<void> {
    const generation = reviewGeneration;
    const request = ++readinessRequest;
    const current = () => !disposed && generation === reviewGeneration && request === readinessRequest && review?.id === reviewId;
    // Only show the loading state on the FIRST load. This function is called on
    // every `review_changed` bus event (refreshFromBus); flipping the loading
    // flag on each background refetch made the merge-readiness box flicker
    // "Loading…" ↔ value. On refetches we keep the current value visible and
    // swap it in silently when the new data arrives.
    const firstLoad = mergeReadiness === null;
    if (firstLoad) {
      mergeReadinessLoading = true;
    }
    try {
      // A failed triage read only loses the badges / Post-all skips, never readiness.
      const [tr, mr] = await Promise.all([fetchTriage(reviewId).catch(() => null), api.get<MergeReadiness>(`/reviews/${reviewId}/merge-readiness`)]);
      if (!current()) return;
      triage = tr;
      mergeReadiness = mr;
      readinessError = null;
    } catch (e) {
      if (current()) readinessError = loadErrorText(e);
    } finally {
      if (current()) {
        mergeReadinessLoading = false;
      }
    }
  }

  async function load(rid: string, num: number): Promise<void> {
    const generation = ++reviewGeneration;
    loading = true;
    loadError = null;
    pollCount = 0;
    pollStale = false;
    editingBody = {};
    diffExpanded = {};
    historyExpanded = {};
    actionBusy = {};
    cancelPendingPostConfirm();
    resetSnippets();
    mergeReadiness = null;
    readinessError = null;
    triage = null;
    try {
      const runs = await api.get<Review[]>(`/repos/${rid}/prs/${num}/reviews`);
      if (disposed || generation !== reviewGeneration) return;
      history = runs;
      review = runs.length > 0 ? runs[0] : null;
      if (review?.status === 'running') {
        // Start the fallback poll while waiting for WS events.
        schedulePoll();
      }
      lastReviewJson = review ? JSON.stringify(review) : '';
      lastAdopted = review;
      if (review?.status === 'done') {
        if (review.comments.length > 0) void loadDiff(rid, num);
        // Load persistent findings + merge-readiness for the done review.
        void loadFindingsAndReadiness(review.id);
      }
    } catch (e) {
      if (disposed || generation !== reviewGeneration) return;
      if (e instanceof ApiError && e.status === 404) {
        review = null;
        history = [];
      } else {
        loadError = loadErrorText(e);
      }
    } finally {
      if (!disposed && generation === reviewGeneration) loading = false;
    }
  }

  function resetSnippets(): void {
    diffAbort?.abort();
    diffAbort = null;
    diffLoading = false;
    snippetFiles = new Map();
  }

  /** Fetch the files the current review's comments point at (summary first
   *  for rename pairing, then one per-file request each, 4 at a time). */
  async function loadDiff(rid: string, num: number): Promise<void> {
    const r = review;
    if (!r || diffLoading) return;
    const want = new Set<string>();
    for (const c of r.comments) if (c.path !== null && c.line !== null && !snippetFiles.has(c.path)) want.add(c.path);
    if (want.size === 0) return;
    diffLoading = true;
    const ctl = new AbortController();
    diffAbort = ctl;
    const stale = () => disposed || ctl.signal.aborted || rid !== repoId || num !== prNumber;
    try {
      const sum = await prDiffSummary(rid, num, ctl.signal);
      if (stale()) return;
      const files = sum.files.filter((f) => want.has(f.path));
      const got = new Map(snippetFiles);
      for (let i = 0; i < files.length; i += 4) {
        const batch = await Promise.all(
          files.slice(i, i + 4).map((f) =>
            f.hunks.length > 0 && !f.hunks_omitted
              ? Promise.resolve(f)
              : prDiffFile(rid, num, f, false, ctl.signal).catch((e: unknown) => {
                  if (isAbortError(e)) throw e;
                  return null; // non-blocking: that snippet simply won't show
                }),
          ),
        );
        if (stale()) return;
        for (const f of batch) if (f && f.hunks.length > 0) got.set(f.path, f);
      }
      snippetFiles = got;
    } catch {
      // non-blocking: diff preview simply won't show
    } finally {
      if (diffAbort === ctl) {
        diffAbort = null;
        diffLoading = false;
      }
    }
  }

  function schedulePoll(delay?: number): void {
    if (pollTimer !== null) clearTimeout(pollTimer);
    if (disposed || cancelling || pollPaused) return;
    pollTimer = setTimeout(() => void poll(), delay ?? pollDelay(pollCount));
  }

  function handleVisibilityChange(): void {
    if (document.hidden) {
      pollPaused = true;
      if (pollTimer !== null) { clearTimeout(pollTimer); pollTimer = null; }
    } else {
      pollPaused = false;
      if (!pollStale && review?.status === 'running') schedulePoll(0);
    }
  }

  function resumePolling(): void {
    pollCount = 0;
    pollStale = false;
    void poll();
  }

  async function poll(): Promise<void> {
    const generation = reviewGeneration;
    if (pollCount >= MAX_POLLS) {
      pollStale = true;
      return;
    }
    pollCount++;
    try {
      const r = await api.get<Review>(`/repos/${repoId}/prs/${prNumber}/review`);
      if (disposed || generation !== reviewGeneration) return;
      pollStale = false;
      // Update the latest run within history (skipped when nothing changed).
      adoptReview(r);
      // Keep polling while the review runs OR any agent is still in progress
      // (covers a single agent retried after the overall review finished).
      const anyActive = r.agents.some(
        (a) => a.status === 'pending' || a.status === 'running' || a.status === 'waiting',
      );
      if (r.status === 'running' || anyActive) {
        if (pollCount >= MAX_POLLS) {
          pollStale = true;
        } else {
          schedulePoll();
        }
      } else {
        pollCount = 0;
        if (r.status === 'done') {
          if (r.comments.length > 0) void loadDiff(repoId, prNumber);
          void loadFindingsAndReadiness(r.id);
        }
      }
    } catch {
      // silently retry until MAX_POLLS
      if (!disposed && generation === reviewGeneration) {
        if (pollCount >= MAX_POLLS) {
          pollStale = true;
        } else {
          schedulePoll();
        }
      }
    }
  }

  async function startReview(): Promise<void> {
    if (starting) return;
    const generation = ++reviewGeneration;
    if (pollTimer !== null) clearTimeout(pollTimer);
    starting = true;
    rerunOptionsOpen = false;
    pollCount = 0;
    pollStale = false;
    resetSnippets();
    mergeReadiness = null;
    readinessError = null;
    triage = null;
    try {
      const body: StartReviewReq = {};
      if (attachedIssue) {
        body.issue_account_id = attachedIssue.account_id;
        body.issue_key = attachedIssue.key;
      }
      const trimmedContext = reviewContext.trim();
      if (trimmedContext) body.context = trimmedContext;
      const newRun = await api.post<Review>(`/repos/${repoId}/prs/${prNumber}/review`, body);
      if (disposed || generation !== reviewGeneration) return;
      // Prepend new run; keep old runs in history
      review = newRun;
      history = [newRun, ...history];
      // Start the fallback poll; the WS reviewBus will replace it when events arrive.
      if (review.status === 'running') schedulePoll();
    } catch (e) {
      toastError('Couldn’t start review', e);
    } finally {
      if (!disposed) starting = false;
    }
  }

  /** Cancel the in-flight review: tears down the agent sessions server-side and
   *  marks the run cancelled. The WS reviewBus then refreshes this panel. */
  async function cancelReview(): Promise<void> {
    if (cancelling || !review || review.status !== 'running') return;
    reviewGeneration++;
    if (pollTimer !== null) clearTimeout(pollTimer);
    cancelling = true;
    try {
      const updated = await api.post<Review>(`/reviews/${review.id}/cancel`, {});
      if (disposed) return;
      reviewGeneration++;
      review = updated;
      history = history.map((r) => (r.id === updated.id ? updated : r));
      toasts.info('Review stopped');
    } catch (e) {
      toastError('Couldn’t stop the review', e);
    } finally {
      if (!disposed) {
        cancelling = false;
        if (review?.status === 'running') schedulePoll();
      }
    }
  }

  function openJiraPicker(): void {
    showJiraPicker = true;
  }

  function closeJiraPicker(): void {
    showJiraPicker = false;
  }

  function removeAttachedIssue(): void {
    attachedIssue = null;
    void tick().then(() => attachStoryBtnEl?.focus());
  }

  function patchCommentInReview(r: Review, updated: ReviewComment): Review {
    return { ...r, comments: r.comments.map((x) => (x.id === updated.id ? updated : x)) };
  }

  // "Approving" a draft comment POSTS it to the PR on the provider under the
  // user's git account — outward-facing, so it confirms where / what / who.
  // The bulk "Post N drafts" path confirms ONCE for the batch.
  const currentRepo = $derived(
    git.allRepos.find((r) => r.id === repoId) ?? git.repos.find((r) => r.id === repoId) ?? null,
  );
  const repoLabel = $derived(currentRepo?.name ?? 'this repository');
  const prWhere = $derived(`${repoLabel} · PR #${prNumber}`);
  const PR_WHO = "The PR author and reviewers see it, posted under your git account.";

  function commentPreview(c: ReviewComment): string {
    const loc = c.path !== null ? `${c.path}${c.line !== null ? `:${c.line}` : ''} — ` : '';
    return `${loc}${c.body}`;
  }

  /** `quiet` (bulk post): no per-comment toasts — the caller reports one summary. */
  async function postComment(c: ReviewComment, confirmed = false, quiet = false): Promise<boolean> {
    // An open edit is unsent text: posting now would send the STORED body,
    // not what the box shows. Save (or cancel) the edit first.
    if (c.id in editingBody) {
      if (!quiet) toasts.warn('Save your edit first', 'The comment has unsaved edits — Save or Cancel them before posting.');
      return false;
    }
    const retry = c.state === 'approved' && !c.posted;
    if (retry && (await lookupOnPr(c, quiet))) return false;
    if (!confirmed) {
      const ok = await confirmOutward({
        verb: 'Post to PR',
        title: `Post comment to PR #${prNumber}?`,
        where: prWhere,
        what: retry
          ? `${commentPreview(c)}\n\nA previous attempt failed — the provider may have created it anyway; no copy was found on the PR.`
          : commentPreview(c),
        who: PR_WHO,
      });
      if (!ok) return false;
    }
    actionBusy = { ...actionBusy, [c.id]: 'approve' };
    try {
      const updated = await api.post<ReviewComment>(`/pr-review-comments/${c.id}/approve`);
      if (review) {
        review = patchCommentInReview(review, updated);
        if (history.length > 0) history = [review, ...history.slice(1)];
      }
      // The daemon answers 200 even when the forge refused the comment (auth,
      // network, 5xx — never retried, a 5xx may have created it): the comment
      // is approved in Otto but `posted` stays false. Never report that as sent.
      if (!updated.posted) {
        if (!quiet) toasts.error("Couldn’t post the comment", `${prWhere} refused it — it’s approved in Otto but not on the PR. Check the repository’s git account.`);
        return false;
      }
      if (!confirmed) toasts.success('Comment posted', prWhere);
      return true;
    } catch (e) {
      if (!quiet) toastError('Couldn’t post the comment', e);
      return false;
    } finally {
      const next = { ...actionBusy };
      delete next[c.id];
      actionBusy = next;
    }
  }

  /** Retry guard: true when a copy of `c` is already on the PR (a forge 5xx
   *  that created it). A failed lookup refuses the retry — a duplicate is the
   *  outward mistake this exists to prevent. */
  async function lookupOnPr(c: ReviewComment, quiet: boolean): Promise<boolean> {
    try {
      const pr = await api.get<PrDetail>(`/repos/${repoId}/prs/${prNumber}`);
      if (alreadyOnPr(c, pr.comments ?? [])) {
        if (!quiet) toasts.info('Already on the PR', 'The earlier attempt did post this comment — not posting it again.');
        // Record it posted (nothing is sent), or it stays "postable" forever
        // and every Post-all re-runs this lookup (S15-302).
        try {
          const updated = await api.patch<ReviewComment>(`/pr-review-comments/${c.id}`, { mark_posted: true });
          if (review) {
            review = patchCommentInReview(review, updated);
            if (history.length > 0) history = [review, ...history.slice(1)];
          }
        } catch {
          // Best effort: the guard above still blocks a duplicate next time.
        }
        return true;
      }
      return false;
    } catch (e) {
      if (!quiet) toastError('Couldn’t check the PR for this comment', e);
      return true;
    }
  }

  let postingAll = $state(false);
  /** Any draft with an open (unsaved) edit — bulk post would send stale text. */
  const editingAny = $derived(Object.keys(editingBody).length > 0);
  /** Drafts, plus approved comments the provider refused (re-post is safe:
   *  the daemon claims `posted` atomically and never re-posts a posted one). */
  function isPostable(c: ReviewComment): boolean {
    return c.state === 'draft' || (c.state === 'approved' && !c.posted);
  }

  /** "Post all" skips dismissed / already-posted comments and unverified
   *  claims unless included; the confirm shows every body in full. */
  const postPlan = $derived.by(() => postAllPlan((review?.comments ?? []).filter(isPostable), triageIx));
  let postConfirm: { plan: PostAllPlan; done: (send: ReviewComment[] | null) => void } | null = $state(null);
  const FORGE: Record<string, string> = { github: 'GitHub', gitlab: 'GitLab', bitbucket: 'Bitbucket' };
  const forgeName = $derived(FORGE[currentRepo?.provider ?? ''] ?? '');
  async function postAllDrafts(): Promise<void> {
    if (editingAny) {
      toasts.warn('Save your edits first', 'Some comments have unsaved edits — Save or Cancel them before posting.');
      return;
    }
    const plan = postPlan;
    const drafts = await new Promise<ReviewComment[] | null>((done) => (postConfirm = { plan, done }));
    postConfirm = null;
    if (!drafts || drafts.length === 0) return;
    postingAll = true;
    let posted = 0;
    try {
      // Re-read each comment's CURRENT state: one declined mid-run is skipped.
      for (const d of drafts) {
        if (typeof disposed !== 'undefined' && disposed) break;
        const c = review?.comments.find((x) => x.id === d.id);
        if (!c || !isPostable(c)) continue;
        if (await postComment(c, true, true)) posted++;
      }
    } finally {
      postingAll = false;
    }
    const failed = drafts.length - posted;
    if (failed === 0) toasts.success(`${plural(posted, 'comment')} posted`, prWhere);
    else {
      // ONE summary instead of a toast per failure; the failed comments stay
      // in the list (not posted) so they can be retried individually.
      toasts.error(
        `${failed} of ${drafts.length} comments couldn’t be posted`,
        `${posted > 0 ? `${posted} posted. ` : ''}${prWhere} refused the rest — check the repository’s git account, then post them again.`,
      );
    }
  }

  /** Decline asks once whether it is "Not a problem" or "Won't fix" — its
   *  finding is dismissed the same way (it stops counting toward readiness). */
  async function declineComment(c: ReviewComment): Promise<void> {
    const req = await askDismissal(c);
    if (!req) return;
    actionBusy = { ...actionBusy, [c.id]: 'decline' };
    try {
      const updated = await api.post<ReviewComment>(`/pr-review-comments/${c.id}/decline`, req);
      if (review) {
        review = patchCommentInReview(review, updated);
        if (history.length > 0) history = [review, ...history.slice(1)];
        void loadFindingsAndReadiness(review.id);
      }
      toasts.info(req.dismiss === 'waived' ? 'Declined — won’t fix' : 'Declined — not a problem');
    } catch (e) {
      toastError('Couldn’t decline comment', e);
    } finally {
      const next = { ...actionBusy };
      delete next[c.id];
      actionBusy = next;
    }
  }

  /** PATCH a not-yet-posted comment: a draft's edited body and/or restoring a
   *  declined comment to draft. Local only — nothing is sent to the PR. */
  async function editComment(c: ReviewComment, req: EditReviewCommentReq): Promise<boolean> {
    actionBusy = { ...actionBusy, [c.id]: 'edit' };
    try {
      const updated = await api.patch<ReviewComment>(`/pr-review-comments/${c.id}`, req);
      if (review) {
        review = patchCommentInReview(review, updated);
        if (history.length > 0) history = [review, ...history.slice(1)];
        if (req.restore_draft) void loadFindingsAndReadiness(review.id);
      }
      return true;
    } catch (e) {
      toastError(req.restore_draft ? 'Couldn’t restore the comment' : 'Couldn’t save the edit', e);
      return false;
    } finally {
      const next = { ...actionBusy };
      delete next[c.id];
      actionBusy = next;
    }
  }

  function stopEditing(id: string): void {
    const next = { ...editingBody };
    delete next[id];
    editingBody = next;
  }

  async function saveEdit(c: ReviewComment): Promise<void> {
    const body = (editingBody[c.id] ?? '').trim();
    if (body === '') {
      toasts.error('Comment can’t be empty');
      return;
    }
    if (body === c.body.trim() || (await editComment(c, { body }))) stopEditing(c.id);
  }

  // --- Config modal ---

  // Load the repo's binding once (and on repo change) so the panel can badge
  // an active override next to the Configure buttons.
  $effect(() => {
    const rid = repoId;
    let live = true;
    repoCfg = null;
    void api
      .get<RepoReviewConfigResp>(`/repos/${rid}/review-config`)
      .then((r) => { if (live && !disposed && rid === repoId) repoCfg = r; })
      .catch(() => {});
    return () => { live = false; };
  });

  /** Short badge text for an active repo override ('' = none). */
  const repoCfgBadge = $derived(repoReviewConfigBadge(repoCfg));

  function openConfig(): void {
    showConfig = true;
  }

  function closeConfig(): void {
    showConfig = false;
  }

  // --- Diff snippet helpers ---
  const getSnippetLines = (c: ReviewComment): DiffLine[] | null => snippetLinesFor(c, snippetFiles);

  function isDiffExpandedDefault(c: ReviewComment): boolean {
    return c.state === 'draft';
  }

  function toggleDiff(id: string, defaultExpanded: boolean): void {
    const current = id in diffExpanded ? diffExpanded[id] : defaultExpanded;
    diffExpanded = { ...diffExpanded, [id]: !current };
  }

  function isDiffExpanded(id: string, defaultExpanded: boolean): boolean {
    return id in diffExpanded ? diffExpanded[id] : defaultExpanded;
  }

  const approvedCount = $derived.by(() => {
    const cs = review?.comments ?? [];
    return cs.filter((c: ReviewComment) => c.state === 'approved' && c.posted).length;
  });
  const draftCount = $derived.by(() => {
    const cs = review?.comments ?? [];
    return cs.filter((c: ReviewComment) => c.state === 'draft').length;
  });
  const postableCount = $derived(postPlan.send.length + postPlan.unverified.length);
  const totalCount = $derived.by(() => review?.comments?.length ?? 0);
  /** Who drafted the comments: the summarizer (last agent row) plus the lenses
   *  it merged. Null for reviews without agent rows. */
  const draftedBy = $derived.by(() => {
    const ag = review?.agents ?? [];
    if (ag.length === 0) return null;
    const summ = ag[ag.length - 1];
    const reviewers = ag.length > 1 ? ag.slice(0, -1) : ag;
    const lenses = [...new Set(reviewers.map((a) => a.lens ?? a.name).filter((n) => n.trim() !== ''))];
    return { provider: summ.provider, model: summ.model, lenses };
  });

  /** Format an ISO timestamp as "X ago" */
  // The shared, self-ticking relative time (lib/stores/now) — no local formatter.
  const timeAgo = (iso: string): string => rel(iso);

  function toggleHistoryRun(idx: number): void {
    historyExpanded = { ...historyExpanded, [idx]: !historyExpanded[idx] };
  }
  $effect(() => registry.register(`pr-review:${repoId}:${prNumber}`, loading || loadError ? [] : [
    review?.status === 'running'
      ? { id: `review.stop.${repoId}.${prNumber}`, title: 'Stop PR review', group: 'Git', keywords: 'review findings pr', disabled: cancelling, run: cancelReview }
      : { id: `review.start.${repoId}.${prNumber}`, title: 'Send PR to review agents', group: 'Git', keywords: 'review findings pr', disabled: starting, run: startReview },
    ...(review?.status === 'done' && postableCount > 0 ? [{ id: `review.post.${repoId}.${prNumber}`, title: `Post ${postableCount} review comments to PR…`, group: 'Git', keywords: 'review findings pr', disabled: postingAll || editingAny, run: postAllDrafts }] : []),
  ]));
</script>

{#snippet reviewInputs()}
      <div class="rp-jira-row">
        {#if attachedIssue}
          <span class="chip rp-jira-chip">
            <Icon name="ticket" size={12} />
            <span class="rp-jira-text" title="{attachedIssue.key} — {attachedIssue.summary}">{attachedIssue.key} — {attachedIssue.summary}</span>
            <button class="rp-jira-remove" onclick={removeAttachedIssue} aria-label="Remove story {attachedIssue.key}" title="Remove story"><Icon name="x" size={12} /></button>
          </span>
        {:else}
          <button class="btn small ghost" bind:this={attachStoryBtnEl} onclick={openJiraPicker}><Icon name="ticket" size={12} /> Attach story…</button>
        {/if}
      </div>
      <div class="rp-context-row">
        <textarea dir="auto" aria-label="Review focus"
          class="rp-context-input"
          rows={2}
          placeholder="What should the reviewers focus on? (optional)"
          bind:value={reviewContext}
        ></textarea>
      </div>
{/snippet}
{#snippet reviewLaunchControls()}
      {@render reviewInputs()}
      <button class="btn small ghost rp-cfg-btn" onclick={openConfig}>
        <Icon name="gear" size={12} /> Configure agents…
        {#if repoCfgBadge !== ''}
          <span class="chip rp-cfg-chip" title="This repository has its own review config">{repoCfgBadge}</span>
        {/if}
      </button>
  <div class="rp-launch-action">
    <button class="btn primary" disabled={starting} onclick={startReview}>{starting ? 'Starting…' : 'Send to review agents'}</button>
  </div>
{/snippet}

<div class="rp">
  {#if loading}
    <div style="padding: 16px"><Skeleton rows={4} height={36} /></div>
  {:else if loadError}
    <LoadState what="this pull request’s reviews" error={loadError} empty onretry={() => void load(repoId, prNumber)} />
  {:else if !review}
    <div class="rp-no-review">
      <EmptyState
        icon="zap"
        title="No review yet"
        body="Run AI agents to analyze the diff and generate draft comments."
      />
      {@render reviewLaunchControls()}
    </div>
  {:else if review.status === 'running'}
    <div class="rp-running-header">
      {#if pollStale}
        <Icon name="warning" size={16} />
        <span class="rp-running-title">Status updates paused</span>
      {:else}
        <span class="spinner" style="--spinner-size: 18px" aria-hidden="true"></span>
        <span class="rp-running-title">Reviewing…</span>
      {/if}
      <span class="grow"></span>
      <button
        class="btn small ghost rp-cancel-btn"
        disabled={cancelling}
        onclick={cancelReview}
        data-testid="review-cancel"
      >
        <Icon name="stop" size={12} /> {cancelling ? 'Stopping…' : 'Stop review'}
      </button>
      <button class="btn small ghost" onclick={openConfig}><Icon name="gear" size={12} /> Configure…</button>
    </div>
    {#if pollStale}
      <ReviewPollStale message="Live status checks stopped after waiting a long time. The review may still be running." onretry={resumePolling} />
    {/if}
    <!-- Live agent cards (shared with the local review) -->
    {#if review.agents && review.agents.length > 0}
      <ReviewScope {review} />
      <ReviewAgents {review} view="running" onretried={onAgentRetried} />
    {:else}
      <p class="dim" style="font-size: var(--fs-s);padding:8px 0">Agents starting…</p>
    {/if}
  {:else if review.status === 'cancelled'}
    <div class="rp-error card" data-testid="review-cancelled">
      <Icon name="x" size={14} />
      <span class="rp-error-msg">Review canceled.</span>
    </div>
    {@render reviewLaunchControls()}
  {:else if review.status === 'error'}
    <ReviewScope {review} />
    <ReviewAgents {review} view="done" onretried={onAgentRetried} />
    <ReviewIncomplete {review} />
    {@render reviewLaunchControls()}
  {:else}
    <!-- status === 'done' -->
    <div class="rp-header">
      <span class="rp-stats">
        <span class="rp-stat">{plural(totalCount, 'comment')}</span>
        {#if review.summary_fallback}
          <span
            class="chip warn rp-fallback-badge"
            title={summaryFallbackTitle(summaryFallback(review.agents))}
          >
            deterministic fallback ({summaryFallback(review.agents).provider} summary not used)
          </span>
        {/if}
        {#if approvedCount > 0}
          <span class="chip ok">{approvedCount} posted</span>
        {/if}
        {#if draftCount > 0}
          <span class="chip">{draftCount} draft</span>
        {/if}
      </span>
      {#if postableCount > 1}
        <button
          class="btn small"
          disabled={postingAll || editingAny}
          title={editingAny ? 'Save or cancel the open edits first' : undefined}
          data-testid="rp-post-all"
          onclick={() => void postAllDrafts()}
        >
          {postingAll ? 'Posting…' : `Post ${postableCount} comments to PR…`}
        </button>
      {/if}
      <button class="btn small ghost" onclick={openConfig}>
        <Icon name="gear" size={12} /> Configure…
        {#if repoCfgBadge !== ''}
          <span class="chip rp-cfg-chip" title="This repository has its own review config">{repoCfgBadge}</span>
        {/if}
      </button>
      <FixReviewButton {review} />
      <button
        class="btn small ghost"
        aria-expanded={rerunOptionsOpen}
        disabled={starting}
        onclick={() => (rerunOptionsOpen = !rerunOptionsOpen)}
      >
        <Icon name="refresh" size={12} /> Re-run review…
      </button>
    </div>
    {#if pollStale}
      <ReviewPollStale message="Live status checks stopped after waiting a long time. A retried agent may still be running." onretry={resumePolling} />
    {/if}
    {#if !shouldCollapseReviewInputs(review.status, rerunOptionsOpen)}
      <div class="rp-rerun-panel card">
        {@render reviewInputs()}
        <div class="rp-rerun-actions">
          <button class="btn small primary" disabled={starting} onclick={startReview}>
            <Icon name="refresh" size={12} /> {starting ? 'Starting…' : 'Run review'}
          </button>
          <button class="btn small ghost" disabled={starting} onclick={() => (rerunOptionsOpen = false)}>Cancel</button>
        </div>
      </div>
    {/if}

    <ReviewMergeGate {review} readiness={mergeReadiness} loading={mergeReadinessLoading} error={readinessError}
      onretry={() => review && void loadFindingsAndReadiness(review.id)} />

    <!-- Findings workflow board: persisted Finding rows with the 6-state status,
         the 7 triage actions, and the Proof Pack. Appears once the review is done. -->
    {#if ws.currentId}
      <div class="rp-findings-section">
        <h3 class="rp-findings-title">Findings</h3>
        <FindingsBoard reviewId={review.id} workspaceId={ws.currentId} />
      </div>
    {/if}

    <!-- Per-agent breakdown: open each agent’s (archived) session + its own
         findings. Shared with the local review; excludes the summarizer. -->
    <ReviewScope {review} />
    {#if review.agents.length > 1}
      <ReviewAgents {review} view="done" onretried={onAgentRetried} />
    {/if}

    {#if review.comments.length === 0}
      <p class="dim" style="font-size: var(--fs-s); padding: 16px 0">No comments generated.</p>
    {:else}
      {#if draftedBy}
        <!-- Attribution: the comments are the summarizer’s merge of every
             lens (per-comment origin isn’t recorded), and stay drafts here
             until a person posts them. -->
        <p class="rp-attrib" data-testid="rp-attrib">
          <ProviderIcon provider={draftedBy.provider} size={12} />
          <span>
            Drafted by Otto’s review ({draftedBy.provider}{draftedBy.model ? ` · ${draftedBy.model}` : ''}){draftedBy.lenses.length > 0 ? ` from ${draftedBy.lenses.join(', ')}` : ''}. Nothing reaches the PR until you post it.
          </span>
        </p>
      {/if}
      <div class="rp-list">
        {#each review.comments as c (c.id)}
          {@const snippetLines = getSnippetLines(c)}
          {@const defaultExpanded = isDiffExpandedDefault(c)}
          {@const expanded = isDiffExpanded(c.id, defaultExpanded)}
          <div class="rp-comment card">
            <div class="rp-comment-head">
              <span class="severity-chip sev-{c.severity}">{c.severity}</span>
              <AgentChip title={draftedBy ? `Drafted by ${draftedBy.provider}${draftedBy.model ? ` · ${draftedBy.model}` : ''}` : 'Drafted by review agent'} />
              <span class="rp-comment-time dim" title={new Date(review.created_at).toLocaleString()}>{timeAgo(review.created_at)}</span>
              {#if c.path !== null}
                <span class="mono rp-loc" dir="ltr" title={c.path}>{c.path}{c.line !== null ? `:${c.line}` : ''}</span>
              {/if}
              <ReviewCommentTriage comment={c} triage={triageIx.get(c.id)} />
              <span class="grow"></span>
              {#if c.state === 'draft'}
                <button
                  class="btn small"
                  disabled={!!actionBusy[c.id] || postingAll || c.id in editingBody}
                  data-testid="rp-post-comment"
                  title={c.id in editingBody ? 'Save or cancel your edit first' : `Post this comment to ${prWhere}`}
                  onclick={() => void postComment(c)}
                >
                  {actionBusy[c.id] === 'approve' ? 'Posting…' : 'Post to PR…'}
                </button>
                {#if !(c.id in editingBody)}
                  <button
                    class="btn small ghost"
                    disabled={!!actionBusy[c.id] || postingAll}
                    data-testid="rp-edit-comment"
                    title="Edit the drafted text before posting"
                    onclick={() => (editingBody = { ...editingBody, [c.id]: c.body })}
                  >
                    Edit
                  </button>
                {/if}
                <button
                  class="btn small ghost"
                  disabled={!!actionBusy[c.id] || postingAll}
                  onclick={() => declineComment(c)}
                >
                  {actionBusy[c.id] === 'decline' ? 'Declining…' : 'Decline'}
                </button>
              {:else if c.state === 'approved' && c.posted}
                <span class="chip ok rp-badge">
                  <Icon name="check" size={12} /> posted
                </span>
              {:else if c.state === 'approved'}
                <span class="chip rp-badge dim" title="Approved in Otto, but the provider refused the post">not posted</span>
                <button
                  class="btn small"
                  disabled={!!actionBusy[c.id] || postingAll}
                  data-testid="rp-retry-post"
                  title="Post this comment to {prWhere} again"
                  onclick={() => void postComment(c)}
                >
                  {actionBusy[c.id] === 'approve' ? 'Posting…' : 'Retry post…'} <!-- ui-guards: allow — an action retry (re-post), not a failed load -->
                </button>
              {:else}
                <span class="chip rp-badge dim">declined</span>
                {#if !c.posted}
                  <button
                    class="btn small ghost"
                    disabled={!!actionBusy[c.id]}
                    data-testid="rp-restore-comment"
                    title="Move this comment back to draft"
                    onclick={() => void editComment(c, { restore_draft: true })}
                  >
                    {actionBusy[c.id] === 'edit' ? 'Restoring…' : 'Restore to draft'}
                  </button>
                {/if}
              {/if}
            </div>
            {#if c.state === 'draft' && c.id in editingBody}
              <textarea dir="auto"
                class="rp-edit-body"
                rows={5}
                aria-label="Comment text"
                data-testid="rp-edit-body"
                bind:value={editingBody[c.id]}
              ></textarea>
              <div class="rp-edit-actions">
                <button class="btn small" disabled={!!actionBusy[c.id]} onclick={() => void saveEdit(c)}>
                  {actionBusy[c.id] === 'edit' ? 'Saving…' : 'Save'}
                </button>
                <button class="btn small ghost" disabled={!!actionBusy[c.id]} onclick={() => stopEditing(c.id)}>
                  Cancel
                </button>
              </div>
            {:else}
              <div class="rp-comment-body md-body">
                <!-- renderPrMarkdown (lib/md renderMarkdownGfm) sanitizes output via allowlist -->
                {@html renderPrMarkdown(c.body)}
              </div>
            {/if}

            <!-- Diff snippet -->
            {#if snippetLines !== null}
              <div class="rp-diff-toggle-row">
                <button
                  class="rp-diff-toggle"
                  onclick={() => toggleDiff(c.id, defaultExpanded)}
                  aria-expanded={expanded}
                >
                  <Icon name={expanded ? 'chevronDown' : 'chevronRight'} size={12} /> {expanded ? 'Hide diff' : 'Show diff'}
                </button>
              </div>
              {#if expanded}
                <div class="rp-diff-snippet" dir="ltr">
                  {#each snippetLines as dl}
                    {@const isTarget = (dl.new_line !== null && dl.new_line === c.line) || (dl.new_line === null && dl.old_line !== null && dl.old_line === c.line)}
                    <div
                      class="rp-diff-line rp-diff-{dl.origin}{isTarget ? ' rp-diff-target' : ''}"
                    >
                      <span class="rp-diff-gutter">
                        {dl.origin === 'add' ? '+' : dl.origin === 'del' ? '-' : ' '}
                      </span>
                      <span class="rp-diff-linenum">{dl.new_line ?? dl.old_line ?? ''}</span>
                      <span class="rp-diff-content">{dl.content}</span>
                    </div>
                  {/each}
                </div>
              {/if}
            {/if}
          </div>
        {/each}
      </div>
    {/if}
  {/if}

  <!-- Past reviews history (runs index 1+) -->
  {#if history.length > 1}
    {@const headerOpen = !!historyExpanded['_header' as unknown as number]}
    <div class="rp-history">
      <button
        class="rp-history-toggle"
        onclick={() => { historyExpanded = { ...historyExpanded, ['_header' as unknown as number]: !headerOpen }; }}
        aria-expanded={headerOpen}
      >
        <Icon name={headerOpen ? 'chevronDown' : 'chevronRight'} size={12} /> Past reviews ({history.length - 1})
      </button>
      {#if headerOpen}
        <div class="rp-history-list">
          {#each history.slice(1) as run, i (run.id)}
            {@const isOpen = !!historyExpanded[i]}
            <div class="rp-history-run card">
              <button
                class="rp-history-run-header"
                onclick={() => toggleHistoryRun(i)}
                aria-expanded={isOpen}
              >
                <span class="dim" style="font-size: var(--fs-xs)">{timeAgo(run.created_at)}</span>
                <StatusBadge status={runStatus(run.status)} />
                {#if run.agents && run.agents.length > 0}
                  <span class="dim" style="font-size:var(--fs-xs)">{run.agents.filter(a => a.status === 'done').length}/{run.agents.length} agents</span>
                {/if}
                <span class="dim" style="font-size:var(--fs-xs)">{plural(run.comments.length, 'comment')}</span>
                <span class="grow"></span>
                <span class="dim" aria-hidden="true"><Icon name={isOpen ? 'chevronDown' : 'chevronRight'} size={12} /></span>
              </button>
              {#if isOpen}
                <div class="rp-history-run-body">
                  {#if run.comments.length === 0}
                    <p class="dim" style="font-size: var(--fs-xs);padding:4px 0">No comments for this run.</p>
                  {:else}
                    {#each run.comments as c (c.id)}
                      <div class="rp-comment card rp-history-comment">
                        <div class="rp-comment-head">
                          <span class="severity-chip sev-{c.severity}">{c.severity}</span>
                          <AgentChip title="Drafted by review agent" />
                          <span class="rp-comment-time dim" title={new Date(run.created_at).toLocaleString()}>{timeAgo(run.created_at)}</span>
                          {#if c.path !== null}
                            <span class="mono rp-loc" dir="ltr" title={c.path}>{c.path}{c.line !== null ? `:${c.line}` : ''}</span>
                          {/if}
                          <span class="grow"></span>
                          {#if c.state === 'approved' && c.posted}
                            <span class="chip ok rp-badge"><Icon name="check" size={12} /> posted</span>
                          {:else if c.state === 'approved'}
                            <span class="chip rp-badge dim">not posted</span>
                          {:else if c.state === 'declined'}
                            <span class="chip rp-badge dim">declined</span>
                          {:else}
                            <span class="chip rp-badge dim">draft</span>
                          {/if}
                        </div>
                        <div class="rp-comment-body md-body">
                          <!-- renderPrMarkdown (lib/md renderMarkdownGfm) sanitizes output via allowlist -->
                          {@html renderPrMarkdown(c.body)}
                        </div>
                      </div>
                    {/each}
                  {/if}
                </div>
              {/if}
            </div>
          {/each}
        </div>
      {/if}
    </div>
  {/if}
</div>

<!-- Configure agents modal -->
{#if showConfig}
  <ReviewConfigModal {repoId} onclose={closeConfig} onupdated={(rc) => (repoCfg = rc)} />
{/if}

{#if postConfirm}
  <PostCommentsConfirm plan={postConfirm.plan} where={`${prWhere}${forgeName ? ` on ${forgeName}` : ''}`} who={PR_WHO} onclose={postConfirm.done} />
{/if}

<!-- Jira story picker modal -->
{#if showJiraPicker}
  <Modal title="Attach story" width={520} onclose={closeJiraPicker}>
    {#snippet children()}
      <JiraIssuePicker onpick={(iss) => { attachedIssue = iss; closeJiraPicker(); }} />
    {/snippet}
    {#snippet footer()}
      <button class="btn ghost" onclick={closeJiraPicker}>Cancel</button>
    {/snippet}
  </Modal>
{/if}

<style>
  .rp {
    padding: 4px 0 32px;
  }

  /* Findings workflow board section */
  .rp-findings-section {
    margin: 12px 0;
    padding-top: 10px;
    border-top: 1px solid var(--border);
  }
  .rp-findings-title {
    font-size: var(--fs-s);
    font-weight: 600;
    margin: 0 0 4px;
  }

  /* No-review state with configure button */
  .rp-no-review {
    position: relative;
  }
  .rp-cfg-btn {
    margin-block-start: 8px;
    font-size: var(--fs-xs);
  }

  /* Running state */
  .rp-running-header {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 12px 0 10px;
    flex-wrap: wrap;
  }
  .rp-running-title {
    font-size: var(--fs-m);
    font-weight: 600;
  }

  /* Error state */
  .rp-error {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 12px 14px;
    color: var(--danger);
    margin-top: 8px;
  }
  .rp-error-msg {
    flex: 1;
    min-width: 0;
    overflow-wrap: anywhere;
    font-size: var(--fs-s);
  }

  /* Done: header row */
  .rp-header {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-bottom: 12px;
    flex-wrap: wrap;
  }
  .rp-stats {
    display: flex;
    align-items: center;
    gap: 8px;
    flex: 1;
    flex-wrap: wrap;
    min-width: 0;
  }
  .rp-stat {
    font-size: var(--fs-s);
    font-weight: 600;
  }

  /* Comment list */
  .rp-attrib {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 4px 0 8px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .rp-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .rp-rerun-panel {
    padding: 8px 12px;
    margin-block-end: 12px;
  }
  .rp-rerun-actions {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-block-start: 8px;
  }
  .rp-comment {
    padding: 10px 14px;
    border-inline-start: 2px solid var(--border-strong);
  }
  .rp-comment-time {
    font-size: var(--fs-xs);
    white-space: nowrap;
  }
  .rp-comment-head {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 6px;
    flex-wrap: wrap;
  }
  .rp-edit-body {
    inline-size: 100%;
    box-sizing: border-box;
    font: inherit;
    font-size: var(--fs-s);
    line-height: 1.55;
    resize: vertical;
  }
  .rp-edit-actions {
    display: flex;
    gap: 6px;
    margin-block-start: 6px;
  }
  .rp-comment-body {
    margin: 0;
    font-size: var(--fs-s);
    line-height: 1.55;
    overflow-wrap: anywhere;
  }
  .rp-comment-body :global(> :first-child) { margin-block-start: 0; }
  .rp-comment-body :global(> :last-child) { margin-block-end: 0; }
  .rp-comment-body :global(p) { margin-block: 4px; }
  .rp-comment-body :global(pre) { margin-block: 6px; padding: 8px 10px; font-size: var(--fs-xs); }
  .rp-comment-body :global(code) { font-size: var(--fs-xs); }
  .rp-comment-body :global(pre code) { font-size: inherit; }
  .rp-comment-body :global(ul),
  .rp-comment-body :global(ol) { margin-block: 4px; padding-inline-start: 18px; }
  .rp-comment-body :global(img) { max-width: 100%; height: auto; }
  .rp-loc {
    overflow-wrap: anywhere;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: normal;
    max-width: 100%;
  }
  .rp-badge {
    font-size: var(--fs-xs);
    display: inline-flex;
    align-items: center;
    gap: 2px;
  }

  /* Severity chips */
  .severity-chip {
    display: inline-block;
    padding: 2px 6px;
    border-radius: var(--radius-s);
    font-size: var(--fs-xs);
    font-weight: 600;
    letter-spacing: .06em;
    text-transform: uppercase;
  }
  .sev-info {
    background: var(--info-soft);
    color: var(--info);
  }
  .sev-warn {
    background: var(--warning-soft);
    color: var(--warning);
  }
  .sev-bug {
    background: var(--danger-soft);
    color: var(--danger);
  }

  /* Diff snippet */
  .rp-diff-toggle-row {
    margin-top: 6px;
  }
  .rp-diff-toggle {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    background: none;
    border: none;
    cursor: pointer;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    padding: 0;
    line-height: 1.4;
  }
  .rp-diff-toggle:hover {
    color: var(--text);
  }
  .rp-diff-snippet {
    margin-top: 4px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    overflow-x: auto;
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    line-height: 1.45;
  }
  .rp-diff-line {
    display: flex;
    align-items: baseline;
    gap: 0;
    padding: 0 6px;
    white-space: pre;
  }
  .rp-diff-add {
    background: var(--success-soft);
    color: var(--text);
  }
  .rp-diff-del {
    background: var(--danger-soft);
    color: var(--text);
  }
  .rp-diff-context {
    color: var(--text-dim);
  }
  .rp-diff-target {
    outline: 1px solid color-mix(in srgb, var(--accent-text) 50%, transparent);
    outline-offset: -1px;
    font-weight: 600;
  }
  .rp-diff-gutter {
    width: 12px;
    flex-shrink: 0;
    user-select: none;
    opacity: 0.6;
  }
  .rp-diff-linenum {
    width: 32px;
    flex-shrink: 0;
    text-align: end;
    padding-inline-end: 8px;
    opacity: 0.45;
    user-select: none;
  }
  .rp-diff-content {
    flex: 1;
    min-width: 0;
  }

  /* Override badge on the Configure buttons */
  .rp-cfg-chip {
    margin-inline-start: 6px;
    max-width: 140px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .grow {
    flex: 1;
  }

  /* Jira attachment row */
  .rp-jira-row {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 8px;
    margin-bottom: 4px;
  }
  .rp-jira-chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-xs);
    max-width: 100%;
    min-width: 0;
  }
  .rp-jira-chip > :global(svg) {
    flex-shrink: 0;
  }
  .rp-jira-text {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .rp-jira-remove {
    display: inline-flex;
    flex-shrink: 0;
    background: none;
    border: none;
    cursor: pointer;
    padding: 0 2px;
    color: var(--text-dim);
  }
  .rp-jira-remove:hover {
    color: var(--text);
  }

  /* Free-text reviewer-guidance input */
  .rp-context-row {
    margin-top: 6px;
    margin-bottom: 4px;
  }
  .rp-context-input {
    width: 100%;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    color: var(--text);
    font-size: var(--fs-s);
    line-height: 1.5;
    padding: 6px 8px;
    box-sizing: border-box;
    resize: vertical;
  }
  .rp-context-input::placeholder {
    color: var(--text-dim);
  }

  /* ── Mobile + tablet (≤1024px) ──────────────────────────────────────────────
     ReviewPanel is a single vertical flow inside the PR detail's own scroll
     container, so the work here is: (1) the per-comment diff snippet must WRAP
     rather than scroll a fixed-width code block off the page, (2) dense button
     rows (header, comment actions) wrap and get real touch height, and (3) the
     "Configure agents" button stops overlapping the empty-state on a phone.
     Desktop (≥1025) keeps the compact 22px buttons + horizontally-scrolled
     diff snippet untouched. */
  @media (max-width: 1024px) {
    /* Diff snippet: wrap long code lines instead of an inner side-scroll, so a
       wide finding never overflows the viewport (HARD INVARIANT). */
    .rp-diff-snippet { overflow-x: visible; }
    .rp-diff-line { white-space: normal; }
    .rp-diff-content {
      white-space: pre-wrap;
      overflow-wrap: anywhere;
      word-break: break-word;
    }
    /* Action .btn hit areas come from the global coarse-pointer rule (app.css). */
    .rp-loc { max-width: 100%; }
    /* Tiny ✕ icon buttons (dismiss / remove-Jira / preset add+delete) are ~12px
       on desktop — far below a tappable size; grow them to a real touch target
       without changing their glyph. */
    .rp-jira-remove {
      min-width: var(--hit-min);
      min-height: var(--hit-min);
      display: inline-flex;
      align-items: center;
      justify-content: center;
    }
    /* The "Past reviews" disclosure is a zero-padding text button — give it real
       tap height. */
    .rp-history-toggle { padding: 6px 0; min-height: var(--hit-min); }
    /* 16px input text prevents iOS Safari from auto-zooming on focus. */
    .rp-context-input { font-size: 16px; } /* ui-guards: allow — 16px stops iOS zoom-on-focus */
  }
  @media (max-width: 640px) {
    /* The absolutely-positioned Configure button collides with the empty-state
       title on a narrow phone — drop it back into normal flow, full-width. */
    .rp-cfg-btn {
      position: static;
      width: 100%;
      margin-top: 8px;
      min-height: var(--hit-min);
    }
    /* Comment action buttons (Approve / Decline) span the row so they're easy
       to tap once the head has wrapped them to their own line. */
    .rp-comment-head .btn { flex: 1 1 auto; }
  }

  /* History section */
  .rp-history {
    margin-top: 20px;
    border-top: 1px solid var(--border);
    padding-top: 10px;
  }
  .rp-history-toggle {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    background: none;
    border: none;
    cursor: pointer;
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--text-dim);
    padding: 0;
    line-height: 1.4;
  }
  .rp-history-toggle:hover {
    color: var(--text);
  }
  .rp-history-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-top: 8px;
  }
  .rp-history-run {
    padding: 0;
    overflow: hidden;
  }
  .rp-history-run-header {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    background: none;
    border: none;
    cursor: pointer;
    padding: 8px 12px;
    text-align: start;
    flex-wrap: wrap;
  }
  .rp-history-run-header:hover {
    background: var(--hover);
  }
  .rp-history-run-body {
    padding: 4px 8px 8px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .rp-history-comment {
    padding: 8px 12px;
    opacity: 0.85;
  }

  .rp-launch-action { margin-block-start: 12px; }
  @media (pointer: coarse), (max-width: 1024px) {
    .rp-diff-toggle { min-block-size: var(--hit-min); }
  }
</style>
