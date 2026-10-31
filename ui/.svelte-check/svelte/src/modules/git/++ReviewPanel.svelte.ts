///<reference types="svelte" />
;
import { announceShown } from '../../lib/announce.svelte';
import { registry } from '../../lib/commands.svelte';
import { plural } from '../../lib/plural';
// AI review panel: start review agents, refresh via WS reviewBus (no poll),
// approve/decline individual draft comments. Supports live per-agent progress
// cards, a configure-agents modal, and a merge-readiness panel.
import { onDestroy, untrack } from 'svelte';
import { summaryFallback, summaryFallbackTitle } from './reviewFallback';
import { toastError } from '../../lib/toastError';
import { verdictLabel } from '../../lib/labels';
import { api, ApiError, isAbortError } from '../../lib/api/client';
import { prDiffFile, prDiffSummary } from './diff-load';
import { alreadyOnPr } from './reviewPost';
import type {
    Review, ReviewComment, ReviewConfig, ReviewAgentCfg, ReviewConfigPreset, RepoReviewBinding,
    RepoReviewConfigResp, StartReviewReq, FileDiff, DiffLine, MergeReadiness, ReviewFindingRow, EditReviewCommentReq, PrDetail,
  } from '../../lib/api/types';
import { toasts } from '../../lib/toast.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
import Icon from '../../lib/components/Icon.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../lib/loadError';
import Modal from '../../lib/components/Modal.svelte';
import { rel } from '../../lib/stores/now.svelte';
import JiraIssuePicker from '../agents/JiraIssuePicker.svelte';
import { ctxMenu } from '../../lib/contextmenu.svelte';
import { router } from '../../lib/router.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { git } from '../../lib/stores/git.svelte';
import { confirmOutward } from '../../lib/confirmOutward';
import ProviderIcon from '../../lib/components/ProviderIcon.svelte';
import ReviewAgents from './ReviewAgents.svelte';
import StatusBadge from '../../lib/components/StatusBadge.svelte';
import { runStatus } from '../../lib/status';
import FindingsBoard from './FindingsBoard.svelte';
// Subscribe to the WS review_changed bus (populated by events.svelte.ts) to
// re-fetch when the running review for this PR completes/errors, replacing the
// fixed-interval visibility-gated poll for the running state.
import { reviewBus } from '../../lib/events.svelte';
import { appLive } from '../../lib/live';
import { agentProviders, defaultAgentProvider } from '../../lib/providers';
;

  // --- Installed-skill metadata (library API; category + version are new) ----
  // Defined locally so this panel can read the extended fields without touching
  // the shared LibrarySkill type.
  interface LibrarySkillMeta {
    name: string;
    description: string;
    category?: string | null;
    version?: number | null;
    body?: string;
  };;
  type BundledState = 'not_installed' | 'up_to_date' | 'update_available' | 'ahead';;;
  interface BundledSkill {
    name: string;
    category?: string | null;
    version?: number | null;
    description?: string;
    installed_version?: number | null;
    state: BundledState;
  };;

  interface Props {
    repoId: string;
    prNumber: number;
  };;

  // Flattened lines + line-number indexes, built once per loaded file (the
  // old code re-flattened the whole file and ran two linear scans for EVERY
  // comment on every re-render, i.e. every 2 s poll while a review ran).
  interface SnippetIndex {
    lines: DiffLine[];
    byNew: Map<number, number>;
    byOld: Map<number, number>;
  };function $$render() {
  const reviewLaunchControls/*Ωignore_positionΩ*/ = ()/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("div", { "class":`rp-jira-row`,});
        if(attachedIssue){
           { svelteHTML.createElement("span", { "class":`chip rp-jira-chip`,});
             { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`ticket`,"size":12,}});}
             { svelteHTML.createElement("span", {   "class":`rp-jira-text`,"title":`${attachedIssue.key} — ${attachedIssue.summary}`,});attachedIssue.key;  attachedIssue.summary; }
             { svelteHTML.createElement("button", {       "class":`rp-jira-remove`,"onclick":removeAttachedIssue,"aria-label":`Remove story ${attachedIssue.key}`,"title":`Remove story`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});} }
           }
        }else{
           { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":openJiraPicker,}); { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`ticket`,"size":12,}});}   }
        }
       }
       { svelteHTML.createElement("div", { "class":`rp-context-row`,});
         { svelteHTML.createElement("textarea", {            "dir":`auto`,"aria-label":`Review focus`,"class":`rp-context-input`,"rows":2,"placeholder":`What should the reviewers focus on? (optional)`,"bind:value":reviewContext,});/*Ωignore_startΩ*/() => reviewContext = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
       }
       { svelteHTML.createElement("button", {   "class":`btn small ghost rp-cfg-btn`,"onclick":openConfig,});
         { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`gear`,"size":12,}});}  
        if(repoCfgBadge !== ''){
           { svelteHTML.createElement("span", {   "class":`chip rp-cfg-chip`,"title":`This repository has its own review config`,});repoCfgBadge; }
        }
       }
   { svelteHTML.createElement("div", { "class":`rp-launch-action`,});
     { svelteHTML.createElement("button", {     "class":`btn primary`,"disabled":starting,"onclick":startReview,});starting ? 'Starting…' : 'Send to review agents'; }
   }
};return __sveltets_2_any(0)};
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  




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
  onDestroy(() => { disposed = true; reviewGeneration++; });
  // Fallback poll (visibility-gated) used only while the review is running and
  // no WS event has arrived yet — keeps the panel alive if the WS drops.
  let pollTimer: ReturnType<typeof setTimeout> | null = null;
  let pollCount = $state(0);
  let pollPaused = $state(false);
  // Merge-readiness data from the dedicated endpoint (fetched after a run completes).
  let mergeReadiness: MergeReadiness | null = $state(null);
  let mergeReadinessLoading = $state(false);
  let readinessError = $state<string | null>(null);
  let readinessRequest = 0;
  // Persistent findings (keyed by fingerprint) from the findings endpoint.
  let findings: ReviewFindingRow[] = $state([]);

  function pollDelay(count: number): number {
    // While the event socket is up, `review_changed` (reviewBus, below)
    // drives the refetches; this chain is only a 30 s safety net then.
    if (appLive.connected()) return 30_000;
    if (count < 5) return 2000;
    if (count < 15) return 5000;
    return 10000;
  }
  // Which older run indices are expanded (history[1+])
  let historyExpanded: Record<number, boolean> = $state({});
  // A child <ReviewAgents> retried one agent: adopt the refreshed review and
  // resume polling so we keep tracking the re-run.
  function onAgentRetried(r: Review): void {
    if (disposed) return;
    reviewGeneration++;
    review = r;
    history = history.length > 0 ? [r, ...history.slice(1)] : [r];
    pollCount = 0;
    schedulePoll();
  }

  // Per-comment action busy state keyed by comment id
  let actionBusy: Record<string, 'approve' | 'decline' | 'edit'> = $state({});
  /** Comment id → the body being edited (a person's edit of the agent draft). */
  let editingBody: Record<string, string> = $state({});

  // Jira story attachment state
  let showJiraPicker = $state(false);
  // The currently attached issue (persists across re-runs within this component)
  let attachedIssue: { account_id: string; key: string; summary: string } | null = $state(null);

  // Free-text guidance for the review agents (e.g. "what to focus on"). Optional;
  // persists across re-runs within this component.
  let reviewContext = $state('');

  // Config modal state
  let showConfig = $state(false);
  let configLoading = $state(false);
  let configSaving = $state(false);
  let editAgents: ReviewAgentCfg[] = $state([]);
  let editSummarizer: ReviewAgentCfg = $state({
    name: 'Summarizer',
    provider: defaultAgentProvider(),
    providers: [],
    model: '',
    prompt: '',
  });
  // Custom presets (persisted in ReviewConfig.custom_presets)
  let editPresets: ReviewAgentCfg[] = $state([]);
  // ── Per-repo scope ──────────────────────────────────────────────────────
  // The modal edits either the GLOBAL config (all repos without an override)
  // or THIS repository's config (a named preset or a repo-only custom config).
  let configScope: 'global' | 'repo' = $state('global');
  let globalCfg: ReviewConfig | null = $state(null);
  let repoCfg: RepoReviewConfigResp | null = $state(null);
  // Named full-config presets ("conf A" / "conf Y") shared across repos.
  let namedPresets: ReviewConfigPreset[] = $state([]);
  // '' = repo-only custom config (no preset backing it).
  let selectedPresetId = $state('');
  // Inline "save as preset" name row.
  let presetSaveOpen = $state(false);
  let presetNameDraft = $state('');

  /** Ensure an agent always has a `providers` array (migration from old format). */
  function normalizeAgent(a: ReviewAgentCfg): ReviewAgentCfg {
    if (!a.providers || a.providers.length === 0) {
      return { ...a, providers: [a.provider || defaultAgentProvider()] };
    }
    return a;
  }

  /** Toggle a provider in/out of the agent's providers list. Keeps at least one. */
  function toggleProvider(agentIdx: number, p: string): void {
    const current = editAgents[agentIdx].providers ?? [defaultAgentProvider()];
    const has = current.includes(p);
    let next: string[];
    if (has) {
      next = current.filter((x) => x !== p);
      if (next.length === 0) next = [defaultAgentProvider()]; // never allow empty
    } else {
      next = [...current, p];
    }
    editAgents = editAgents.map((a, i) =>
      i === agentIdx ? { ...a, providers: next } : a
    );
  }

  // Diff state for per-comment snippets: ONLY the files that carry a comment,
  // fetched per file through the PR diff cache shared with the Files tab
  // (PrDetail) — never the whole PR diff.
  let snippetFiles: Map<string, FileDiff> = $state.raw(new Map());
  let diffLoading = false;
  let diffAbort: AbortController | null = null;
  onDestroy(() => diffAbort?.abort());
  // Per-comment collapse state: true = expanded (default for draft)
  let diffExpanded: Record<string, boolean> = $state({});

  // The Run-on / summarizer provider choices come from the LIVE registry
  // (built-ins + the user's custom providers, e.g. grok) — never a hardcoded
  // subset. Shell can't review, so agent providers only.
  const PROVIDER_OPTIONS = $derived(agentProviders());
  const MAX_POLLS = 600; // ~20 min at 2s — covers long multi-agent live reviews

  // --- Data-driven review-skill presets ------------------------------------
  // One-click lenses derived from the installed `review`-category skills. Each
  // becomes a review agent that runs as a session with the skill materialized,
  // so "Apply the `<skill>` skill" makes it follow the skill's full method.
  let skillPresets: { name: string; focus: string }[] = $state([]);
  // Pre-check: review skills that are missing or have a newer bundled version.
  let missingReviewSkills = $state(0);
  let outdatedReviewSkills = $state(0);
  let precheckDismissed = $state(false);

  const showPrecheck = $derived(
    !precheckDismissed && (missingReviewSkills > 0 || outdatedReviewSkills > 0),
  );

  /** Prettify a skill name for display: "security-review" -> "Security review". */
  function prettifySkillName(name: string): string {
    const spaced = name.replace(/[-_]+/g, ' ').trim();
    if (spaced === '') return name;
    return spaced.charAt(0).toUpperCase() + spaced.slice(1);
  }

  /** Build a preset whose lens applies the named skill (materialized in-session). */
  function presetFromSkill(s: LibrarySkillMeta): { name: string; focus: string } {
    const desc = (s.description ?? '').trim();
    return {
      name: prettifySkillName(s.name),
      focus: `Apply the \`${s.name}\` skill (it is available to you): ${desc} ${JSON_INSTR}`,
    };
  }

  $effect(() => {
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

  /** Load the merge-readiness and persistent findings for a completed review. */
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
      const [fp, mr] = await Promise.all([
        api.get<ReviewFindingRow[]>(`/reviews/${reviewId}/findings`),
        api.get<MergeReadiness>(`/reviews/${reviewId}/merge-readiness`),
      ]);
      if (!current()) return;
      findings = fp;
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

  // Load installed review skills (primary one-click lenses) + the missing/
  // outdated pre-check. Non-blocking: failures just leave the fallback presets.
  $effect(() => {
    void loadReviewSkills();
  });

  async function loadReviewSkills(): Promise<void> {
    try {
      const skills = await api.get<LibrarySkillMeta[]>('/library/skills');
      skillPresets = skills
        .filter((s) => s.category === 'review')
        .map((s) => presetFromSkill(s));
    } catch {
      // Library unreachable: keep skillPresets empty -> hardcoded fallback shows.
    }
    try {
      const bundled = await api.get<BundledSkill[]>('/library/bundled');
      const review = bundled.filter((b) => b.category === 'review');
      missingReviewSkills = review.filter((b) => b.state === 'not_installed').length;
      outdatedReviewSkills = review.filter((b) => b.state === 'update_available').length;
    } catch {
      // Pre-check is best-effort; absence simply hides the banner.
    }
  }

  // Open Settings → Skills (the bundled-skill install/update panel).
  function openSkillSettings(): void {
    router.go('settings/skills');
  }

  async function load(rid: string, num: number): Promise<void> {
    const generation = ++reviewGeneration;
    loading = true;
    loadError = null;
    resetSnippets();
    mergeReadiness = null;
    readinessError = null;
    findings = [];
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
      if (review?.status === 'running') schedulePoll(0);
    }
  }

  async function poll(): Promise<void> {
    const generation = reviewGeneration;
    pollCount++;
    try {
      const r = await api.get<Review>(`/repos/${repoId}/prs/${prNumber}/review`);
      if (disposed || generation !== reviewGeneration) return;
      // Update the latest run within history (skipped when nothing changed).
      adoptReview(r);
      // Keep polling while the review runs OR any agent is still in progress
      // (covers a single agent retried after the overall review finished).
      const anyActive = r.agents.some(
        (a) => a.status === 'pending' || a.status === 'running' || a.status === 'waiting',
      );
      if (r.status === 'running' || anyActive) {
        schedulePoll();
      } else {
        pollCount = 0;
        if (r.status === 'done' && r.comments.length > 0) {
          void loadDiff(repoId, prNumber);
        }
      }
    } catch {
      // silently retry
      if (!disposed && generation === reviewGeneration) schedulePoll();
    }
  }

  async function startReview(): Promise<void> {
    if (starting) return;
    const generation = ++reviewGeneration;
    if (pollTimer !== null) clearTimeout(pollTimer);
    starting = true;
    pollCount = 0;
    resetSnippets();
    mergeReadiness = null;
    readinessError = null;
    findings = [];
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

  // --- Jira picker ---

  function openJiraPicker(): void {
    showJiraPicker = true;
  }

  function closeJiraPicker(): void {
    showJiraPicker = false;
  }

  function removeAttachedIssue(): void {
    attachedIssue = null;
  }

  function patchCommentInReview(r: Review, updated: ReviewComment): Review {
    return { ...r, comments: r.comments.map((x) => (x.id === updated.id ? updated : x)) };
  }

  // "Approving" a draft comment POSTS it to the PR on the provider under the
  // user's git account — outward-facing, so it confirms where / what / who.
  // The bulk "Post N drafts" path confirms ONCE for the batch.
  const repoLabel = $derived(git.repos.find((r) => r.id === repoId)?.name ?? 'this repository');
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

  async function postAllDrafts(): Promise<void> {
    if (editingAny) {
      toasts.warn('Save your edits first', 'Some comments have unsaved edits — Save or Cancel them before posting.');
      return;
    }
    const drafts = (review?.comments ?? []).filter(isPostable);
    if (drafts.length === 0) return;
    const ok = await confirmOutward({
      verb: `Post ${drafts.length} comments`,
      title: `Post ${drafts.length} comments to PR #${prNumber}?`,
      where: prWhere,
      what: drafts.map((c) => `• ${commentPreview(c).split('\n')[0]}`).join('\n'),
      who: PR_WHO,
    });
    if (!ok) return;
    postingAll = true;
    let posted = 0;
    try {
      // Re-read each comment's CURRENT state: one declined mid-run is skipped.
      for (const d of drafts) {
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

  async function declineComment(c: ReviewComment): Promise<void> {
    actionBusy = { ...actionBusy, [c.id]: 'decline' };
    try {
      const updated = await api.post<ReviewComment>(`/pr-review-comments/${c.id}/decline`);
      if (review) {
        review = patchCommentInReview(review, updated);
        if (history.length > 0) history = [review, ...history.slice(1)];
      }
      toasts.info('Comment declined');
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
    void api
      .get<RepoReviewConfigResp>(`/repos/${rid}/review-config`)
      .then((r) => (repoCfg = r))
      .catch(() => {});
  });

  /** Short badge text for an active repo override ('' = none). */
  const repoCfgBadge = $derived.by(() => {
    const rc = repoCfg;
    if (rc === null || rc.scope === 'global') return '';
    return rc.scope === 'preset' ? (rc.preset_name ?? 'preset') : 'repo config';
  });

  /** Seed the edit fields from a config (normalizing `providers` arrays). */
  function seedEditFields(cfg: ReviewConfig): void {
    editAgents = cfg.agents.map((a) => normalizeAgent({ ...a }));
    editSummarizer = { ...cfg.summarizer };
  }

  async function openConfig(): Promise<void> {
    showConfig = true;
    configLoading = true;
    try {
      const [cfg, presets, binding] = await Promise.all([
        api.get<ReviewConfig>('/settings/pr-review'),
        api.get<ReviewConfigPreset[]>('/settings/pr-review/presets'),
        api.get<RepoReviewConfigResp>(`/repos/${repoId}/review-config`),
      ]);
      globalCfg = cfg;
      namedPresets = presets;
      repoCfg = binding;
      // Per-agent presets live on the GLOBAL config regardless of scope.
      editPresets = (cfg.custom_presets ?? []).map((p) => normalizeAgent({ ...p }));
      // Open on the scope the repo actually reviews with.
      configScope = binding.scope === 'global' ? 'global' : 'repo';
      selectedPresetId = binding.preset_id ?? '';
      seedEditFields(configScope === 'global' ? cfg : binding.config);
    } catch (e) {
      toastError('Couldn’t load config', e);
      showConfig = false;
    } finally {
      configLoading = false;
    }
  }

  /** Switch the modal between the global config and this repo's config. */
  function setConfigScope(scope: 'global' | 'repo'): void {
    if (scope === configScope) return;
    configScope = scope;
    if (scope === 'global') {
      if (globalCfg) seedEditFields(globalCfg);
    } else if (repoCfg) {
      selectedPresetId = repoCfg.preset_id ?? '';
      seedEditFields(repoCfg.config);
    }
  }

  /** Load a named preset's config into the edit fields. '' keeps the current
   *  fields as a repo-only custom config. */
  function pickNamedPreset(id: string): void {
    selectedPresetId = id;
    const p = namedPresets.find((x) => x.id === id);
    if (p) seedEditFields(p.config);
  }

  /** The config currently in the edit fields, provider-synced. Repo-scoped
   *  configs never carry the global per-agent presets. */
  function currentEditConfig(includePresets: boolean): ReviewConfig {
    const base = buildConfigFromEditState(includePresets ? editPresets : []);
    return includePresets ? base : { ...base, custom_presets: [] };
  }

  /** Compare two configs ignoring per-agent presets (preset "drift" check). */
  function sameConfig(a: ReviewConfig, b: ReviewConfig): boolean {
    const strip = (c: ReviewConfig): unknown => ({
      agents: c.agents.map((x) => ({ ...normalizeAgent(x), provider: undefined })),
      summarizer: { ...c.summarizer },
    });
    return JSON.stringify(strip(a)) === JSON.stringify(strip(b));
  }

  /** Revert this repo to the global config (drops its binding). */
  async function useGlobalConfig(): Promise<void> {
    configSaving = true;
    try {
      repoCfg = await api.del<RepoReviewConfigResp>(`/repos/${repoId}/review-config`);
      configScope = 'global';
      selectedPresetId = '';
      if (globalCfg) seedEditFields(globalCfg);
      toasts.success('Repo now uses the global review config');
    } catch (e) {
      toastError('Couldn’t revert to global', e);
    } finally {
      configSaving = false;
    }
  }

  /** Save the current edit fields as a NEW named preset (or update the one
   *  with the same name) and bind this repo to it. */
  async function saveAsNamedPreset(): Promise<void> {
    const name = presetNameDraft.trim();
    if (name === '') return;
    configSaving = true;
    try {
      const cfg = currentEditConfig(false);
      const existing = namedPresets.find((p) => p.name === name);
      const id = existing?.id ?? crypto.randomUUID();
      const next = existing
        ? namedPresets.map((p) => (p.id === id ? { ...p, config: cfg } : p))
        : [...namedPresets, { id, name, config: cfg }];
      namedPresets = await api.put<ReviewConfigPreset[]>('/settings/pr-review/presets', next);
      const body: RepoReviewBinding = { preset_id: id };
      repoCfg = await api.put<RepoReviewConfigResp>(`/repos/${repoId}/review-config`, body);
      selectedPresetId = id;
      presetSaveOpen = false;
      presetNameDraft = '';
      toasts.success(existing ? `Preset “${name}” updated` : `Preset “${name}” created`, 'This repo now uses it');
    } catch (e) {
      toastError('Couldn’t save preset', e);
    } finally {
      configSaving = false;
    }
  }

  function closeConfig(): void {
    showConfig = false;
  }

  function addAgent(): void {
    editAgents = [
      ...editAgents,
      { name: 'New agent', provider: defaultAgentProvider(), providers: [defaultAgentProvider()], model: '', prompt: '' },
    ];
  }

  function removeAgent(i: number): void {
    editAgents = editAgents.filter((_, idx) => idx !== i);
  }

  /** Build the ReviewConfig body from current edit state (same shape as saveConfig). */
  function buildConfigFromEditState(presets: ReviewAgentCfg[]): ReviewConfig {
    const syncedAgents = editAgents.map((a) => {
      const ps = a.providers && a.providers.length > 0 ? a.providers : [a.provider || defaultAgentProvider()];
      return { ...a, provider: ps[0], providers: ps };
    });
    return {
      agents: syncedAgents,
      summarizer: editSummarizer,
      custom_presets: presets.map((a) => {
        const ps = a.providers && a.providers.length > 0 ? a.providers : [a.provider || defaultAgentProvider()];
        return { ...a, provider: ps[0], providers: ps };
      }),
    };
  }

  /** Persist per-agent presets immediately without closing the modal. They
   *  live on the GLOBAL config — when the modal is editing a repo-scoped
   *  config, keep the stored global agents/summarizer instead of overwriting
   *  them with this repo's edit fields. */
  async function persistPresets(presets: ReviewAgentCfg[]): Promise<void> {
    try {
      const synced = presets.map((a) => {
        const ps = a.providers && a.providers.length > 0 ? a.providers : [a.provider || defaultAgentProvider()];
        return { ...a, provider: ps[0], providers: ps };
      });
      const cfg: ReviewConfig =
        configScope === 'repo' && globalCfg
          ? { ...globalCfg, custom_presets: synced }
          : buildConfigFromEditState(presets);
      await api.put('/settings/pr-review', cfg);
      globalCfg = cfg;
    } catch (e) {
      toastError('Couldn’t persist presets', e);
    }
  }

  // Save the agent at index i as a custom preset (dedupe by name) and persist immediately
  async function saveAsPreset(i: number): Promise<void> {
    const agent = editAgents[i];
    const effectiveProviders = (agent.providers && agent.providers.length > 0)
      ? agent.providers
      : [agent.provider || defaultAgentProvider()];
    const copy: ReviewAgentCfg = {
      name: agent.name,
      provider: effectiveProviders[0],
      providers: effectiveProviders,
      model: agent.model,
      prompt: agent.prompt,
      // Keep the inlined Library skill (the default correctness/security agents carry one):
      // dropping it would make the preset run the bare prompt without the skill's method.
      ...(agent.skill ? { skill: agent.skill } : {}),
    };
    const existing = editPresets.findIndex((p) => p.name === copy.name);
    let next: ReviewAgentCfg[];
    if (existing >= 0) {
      next = editPresets.map((p, idx) => (idx === existing ? copy : p));
    } else {
      next = [...editPresets, copy];
    }
    editPresets = next;
    await persistPresets(next);
    toasts.success('Saved to presets');
  }

  // Add a custom preset to editAgents
  function addPresetToAgents(preset: ReviewAgentCfg): void {
    editAgents = [...editAgents, { ...preset }];
  }

  // Remove a custom preset by index and persist immediately
  async function removePreset(i: number): Promise<void> {
    const next = editPresets.filter((_, idx) => idx !== i);
    editPresets = next;
    await persistPresets(next);
  }

  // Preset reviewers — curated lenses the user can one-click add alongside
  // their own. Each produces a JSON-array of {path,line,severity,body}.
  const JSON_INSTR =
    'Output ONLY a JSON array (no prose, no markdown fence) of objects {"path":string,"line":number,"severity":"info"|"warn"|"bug","body":string} for issues you find in the diff. Empty array if none.';
  const REVIEWER_PRESETS: { name: string; focus: string }[] = [
    { name: 'Security & vulnerabilities', focus: 'Focus on security: injection, broken authn/authz, secret/credential handling, unsafe deserialization, SSRF, path traversal, and unvalidated input.' },
    { name: 'Performance & efficiency', focus: 'Focus on performance: N+1 queries, unnecessary allocations, O(n^2) hot paths, blocking I/O, missing caching/indexes, and oversized payloads.' },
    { name: 'Correctness & bugs', focus: 'Focus on correctness: logic errors, off-by-one, null/undefined handling, wrong edge cases, and broken invariants.' },
    { name: 'Tests & coverage', focus: 'Focus on testing: missing tests for new/changed behavior, untested edge cases, brittle or flaky tests, and missing assertions.' },
    { name: 'Error handling & resilience', focus: 'Focus on error handling: swallowed errors, missing propagation, unhandled failures, retries/timeouts, and resource cleanup.' },
    { name: 'Concurrency & races', focus: 'Focus on concurrency: data races, deadlocks, unsynchronized shared state, and incorrect async/await or locking.' },
    { name: 'API & interface design', focus: 'Focus on API design: breaking changes, inconsistent naming, unclear contracts, leaky abstractions, and backward compatibility.' },
    { name: 'Readability & style', focus: 'Focus on readability: unclear naming, dead code, overly complex functions, and inconsistent style.' },
    { name: 'Documentation', focus: 'Focus on documentation: missing or outdated doc comments, unclear public APIs, and undocumented behavior changes.' },
    { name: 'Dependencies & licensing', focus: 'Focus on dependencies: risky new deps, loose version pinning, known-vulnerable packages, and license concerns.' },
  ];

  function addPreset(p: { name: string; focus: string }): void {
    editAgents = [
      ...editAgents,
      { name: p.name, provider: defaultAgentProvider(), providers: [defaultAgentProvider()], model: '', prompt: `${p.focus} ${JSON_INSTR}` },
    ];
  }

  function openPresetMenu(e: MouseEvent): void {
    const taken = new Set(editAgents.map((a) => a.name));
    // Primary lenses: installed `review`-category skills. Fall back to the
    // hardcoded presets only when no review skills are installed, so the menu
    // is never empty.
    const lensPresets = skillPresets.length > 0 ? skillPresets : REVIEWER_PRESETS;
    const builtinItems = lensPresets.map((p) => ({
      label: p.name,
      disabled: taken.has(p.name),
      action: () => addPreset(p),
    }));
    const customItems = editPresets.map((p) => ({
      label: p.name,
      disabled: taken.has(p.name),
      action: () => addPresetToAgents(p),
    }));
    const items =
      customItems.length > 0
        ? [...builtinItems, { label: '', separator: true, action: () => {} }, ...customItems]
        : builtinItems;
    ctxMenu.show(e, items, { filter: items.length > 12, maxVisible: 12 });
  }

  async function saveConfig(): Promise<void> {
    configSaving = true;
    try {
      if (configScope === 'global') {
        const cfg = currentEditConfig(true);
        await api.put('/settings/pr-review', cfg);
        globalCfg = cfg;
        toasts.success('Review config saved', 'Applies to every repo without an override');
      } else {
        // Repo scope: an untouched preset saves as the preset reference (so the
        // repo tracks later preset edits); any drift saves as a repo-only
        // custom config instead of silently mutating the shared preset.
        const cfg = currentEditConfig(false);
        const chosen = namedPresets.find((p) => p.id === selectedPresetId);
        const body: RepoReviewBinding =
          chosen && sameConfig(cfg, chosen.config) ? { preset_id: chosen.id } : { config: cfg };
        repoCfg = await api.put<RepoReviewConfigResp>(`/repos/${repoId}/review-config`, body);
        selectedPresetId = repoCfg.preset_id ?? '';
        toasts.success(
          'Repo review config saved',
          repoCfg.scope === 'preset'
            ? `Uses preset “${repoCfg.preset_name}”`
            : 'Custom config for this repo only',
        );
      }
      showConfig = false;
    } catch (e) {
      toastError('Couldn’t save config', e);
    } finally {
      configSaving = false;
    }
  }

  // --- Diff snippet helpers ---

  const CONTEXT_LINES = 3;

  const snippetIndex = new WeakMap<FileDiff, SnippetIndex>();
  function indexOf(f: FileDiff): SnippetIndex {
    let ix = snippetIndex.get(f);
    if (ix) return ix;
    ix = { lines: [], byNew: new Map(), byOld: new Map() };
    for (const hunk of f.hunks) {
      for (const line of hunk.lines) {
        const i = ix.lines.push(line) - 1;
        if (line.new_line !== null && !ix.byNew.has(line.new_line)) ix.byNew.set(line.new_line, i);
        if (line.old_line !== null && !ix.byOld.has(line.old_line)) ix.byOld.set(line.old_line, i);
      }
    }
    snippetIndex.set(f, ix);
    return ix;
  }

  function getSnippetLines(c: ReviewComment): DiffLine[] | null {
    if (c.path === null || c.line === null) return null;
    const fileDiff = snippetFiles.get(c.path);
    if (!fileDiff) return null;
    const ix = indexOf(fileDiff);
    // Find the target line index — match new_line first, fallback old_line
    const targetIdx = ix.byNew.get(c.line) ?? ix.byOld.get(c.line);
    if (targetIdx === undefined) return null;
    const start = Math.max(0, targetIdx - CONTEXT_LINES);
    const end = Math.min(ix.lines.length - 1, targetIdx + CONTEXT_LINES);
    return ix.lines.slice(start, end + 1);
  }

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
  const postableCount = $derived.by(() => (review?.comments ?? []).filter(isPostable).length);
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
  const blockerCount = $derived.by(() => review?.blocker_count ?? 0);
  const mergeReady = $derived.by(() => {
    const r = review;
    return r !== null && r.status === 'done' && blockerCount === 0;
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
;
async () => {



 { svelteHTML.createElement("div", { "class":`rp`,});
  
  if(showPrecheck){
     {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('div'),(`${missingReviewSkills > 0 ? `${missingReviewSkills} review skill${missingReviewSkills === 1 ? ' isn’t' : 's aren’t'} installed` : ''}${missingReviewSkills > 0 && outdatedReviewSkills > 0 ? ' · ' : ''}${outdatedReviewSkills > 0 ? `${outdatedReviewSkills} ${outdatedReviewSkills === 1 ? 'has an update' : 'have updates'}` : ''}`)));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {   "class":`rp-precheck`,});
       { svelteHTML.createElement("span", { "class":`rp-precheck-icon`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":14,}});} }
       { svelteHTML.createElement("span", { "class":`rp-precheck-msg`,});
        if(missingReviewSkills > 0){missingReviewSkills;  missingReviewSkills === 1 ? " isn’t" : "s aren’t"; }if(missingReviewSkills > 0 && outdatedReviewSkills > 0){' · ';}if(outdatedReviewSkills > 0){outdatedReviewSkills; outdatedReviewSkills === 1 ? 'has an update' : 'have updates';}
       }
       { svelteHTML.createElement("button", {   "class":`btn small ghost rp-precheck-btn`,"onclick":openSkillSettings,});
          
       }
       { svelteHTML.createElement("button", {         "class":`icon-btn rp-precheck-dismiss`,"onclick":() => (precheckDismissed = true),"aria-label":`Dismiss skill notice`,"title":`Dismiss skill notice`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});} }
     }}
  }
  if(loading){
     { svelteHTML.createElement("div", { "style":`padding: 16px`,}); { const $$_notelekS2C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS2C({ target: __sveltets_2_any(), props: {    "rows":4,"height":36,}});} }
  } else if (loadError){
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {      "what":`this pull request’s reviews`,"error":loadError,"empty":true,"onretry":() => void load(repoId, prNumber),}});}
  } else if (!review){
     { svelteHTML.createElement("div", { "class":`rp-no-review`,});
       { const $$_etatSytpmE2C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE2C({ target: __sveltets_2_any(), props: {       "icon":`zap`,"title":`No review yet`,"body":`Run AI agents to analyze the diff and generate draft comments.`,}});}
      ;__sveltets_2_ensureSnippet(reviewLaunchControls());
     }
  } else if (review.status === 'running'){
     { svelteHTML.createElement("div", { "class":`rp-running-header`,});
       { svelteHTML.createElement("span", {     "class":`spinner`,"style":`--spinner-size: 18px`,"aria-hidden":`true`,}); }
       { svelteHTML.createElement("span", { "class":`rp-running-title`,});  }
       { svelteHTML.createElement("span", { "class":`grow`,}); }
       { svelteHTML.createElement("button", {         "class":`btn small ghost rp-cancel-btn`,"disabled":cancelling,"onclick":cancelReview,...__sveltets_2_empty({"data-testid":`review-cancel`}),});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`stop`,"size":12,}});} cancelling ? 'Stopping…' : 'Stop review';
       }
       { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":openConfig,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`gear`,"size":12,}});}  }
     }
    
    if(review.agents && review.agents.length > 0){
       { const $$_stnegAweiveR1C = __sveltets_2_ensureComponent(ReviewAgents); new $$_stnegAweiveR1C({ target: __sveltets_2_any(), props: {     review,"view":`running`,"onretried":onAgentRetried,}});}
    }else{
       { svelteHTML.createElement("p", {   "class":`dim`,"style":`font-size: var(--fs-s);padding:8px 0`,});  }
    }
  } else if (review.status === 'cancelled'){
     { svelteHTML.createElement("div", {   "class":`rp-error card`,...__sveltets_2_empty({"data-testid":`review-cancelled`}),});
       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":14,}});}
       { svelteHTML.createElement("span", { "class":`rp-error-msg`,});  }
     }
    ;__sveltets_2_ensureSnippet(reviewLaunchControls());
  } else if (review.status === 'error'){
     { svelteHTML.createElement("div", {   "class":`rp-error card`,"role":`alert`,});
       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":14,}});}
       { svelteHTML.createElement("span", { "class":`rp-error-msg`,});    { svelteHTML.createElement("span", { "class":`dim`,});review.error ?? 'No reason was reported — check Settings → Logs.'; } }
     }
    ;__sveltets_2_ensureSnippet(reviewLaunchControls());
  }else{
    
     { svelteHTML.createElement("div", { "class":`rp-header`,});
       { svelteHTML.createElement("span", { "class":`rp-stats`,});
         { svelteHTML.createElement("span", { "class":`rp-stat`,});plural(totalCount, 'comment'); }
        if(review.summary_fallback){
           { svelteHTML.createElement("span", {     "class":`chip warn rp-fallback-badge`,"title":summaryFallbackTitle(summaryFallback(review.agents)),});
              summaryFallback(review.agents).provider;   
           }
        }
        if(approvedCount > 0){
           { svelteHTML.createElement("span", { "class":`chip ok`,});approvedCount;  }
        }
        if(draftCount > 0){
           { svelteHTML.createElement("span", { "class":`chip`,});draftCount;  }
        }
       }
      if(postableCount > 1){
         { svelteHTML.createElement("button", {           "class":`btn small`,"disabled":postingAll || editingAny,"title":editingAny ? 'Save or cancel the open edits first' : undefined,...__sveltets_2_empty({"data-testid":`rp-post-all`}),"onclick":() => void postAllDrafts(),});
          postingAll ? 'Posting…' : `Post ${postableCount} comments to PR…`;
         }
      }
       { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":openConfig,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`gear`,"size":12,}});} 
        if(repoCfgBadge !== ''){
           { svelteHTML.createElement("span", {   "class":`chip rp-cfg-chip`,"title":`This repository has its own review config`,});repoCfgBadge; }
        }
       }
       { svelteHTML.createElement("button", {     "class":`btn small ghost`,"disabled":starting,"onclick":startReview,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":12,}});} starting ? 'Starting…' : 'Re-run review';
       }
     }
     { svelteHTML.createElement("div", { "class":`rp-jira-row`,});
      if(attachedIssue){
         { svelteHTML.createElement("span", { "class":`chip rp-jira-chip`,});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`ticket`,"size":12,}});}
           { svelteHTML.createElement("span", {   "class":`rp-jira-text`,"title":`${attachedIssue.key} — ${attachedIssue.summary}`,});attachedIssue.key;  attachedIssue.summary; }
           { svelteHTML.createElement("button", {       "class":`rp-jira-remove`,"onclick":removeAttachedIssue,"aria-label":`Remove story ${attachedIssue.key}`,"title":`Remove story`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});} }
         }
      }else{
         { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":openJiraPicker,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`ticket`,"size":12,}});}   }
      }
     }
     { svelteHTML.createElement("div", { "class":`rp-context-row`,});
       { svelteHTML.createElement("textarea", {            "dir":`auto`,"aria-label":`Review focus`,"class":`rp-context-input`,"rows":2,"placeholder":`What should the reviewers focus on? (optional)`,"bind:value":reviewContext,});/*Ωignore_startΩ*/() => reviewContext = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
     }

    
    if(review.blocker_count != null || review.verdict != null){
       { svelteHTML.createElement("div", {   "class":`rp-merge-gate`,});mergeReady;!mergeReady;
        if(mergeReady){
           { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":13,}});}     }else{
           { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`zap`,"size":13,}});} plural(blockerCount, 'blocker');  }
        if(review.verdict){
           { svelteHTML.createElement("span", { "class":`rp-verdict`,}); verdictLabel(review.verdict); }
        }
       }
    }

    
    if(readinessError){
       { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {          "what":`merge readiness`,"error":readinessError,"empty":mergeReadiness === null,"variant":`compact`,"onretry":() => review && void loadFindingsAndReadiness(review.id),}});}
    }
    if(mergeReadiness !== null || mergeReadinessLoading){
       { svelteHTML.createElement("div", { "class":`rp-readiness card`,});
         { svelteHTML.createElement("div", { "class":`rp-readiness-row`,});
           { svelteHTML.createElement("span", { "class":`rp-readiness-label`,});  }
          if(mergeReadinessLoading){
             { svelteHTML.createElement("span", {   "class":`dim`,"style":`font-size: var(--fs-xs)`,});   }
          } else if (mergeReadiness !== null){
            
            const ciState = (mergeReadiness as any).ci_status ?? 'none';
             { svelteHTML.createElement("span", {     "class":`chip rp-ci-pill rp-ci-${ciState}`,"title":`CI: ${ciState}`,});
              ciState === 'none' ? 'No CI checks' : ciState === 'pending' ? 'CI pending' : ciState === 'success' ? 'CI passed' : ciState === 'failure' ? 'CI failed' : `CI ${ciState}`;
             }
            
            const approvals = (mergeReadiness as any).approvals ?? 0;
            if(approvals > 0){
               { svelteHTML.createElement("span", { "class":`chip ok rp-readiness-chip`,});plural(approvals, 'approval'); }
            }else{
               { svelteHTML.createElement("span", { "class":`chip rp-readiness-chip dim`,});  }
            }
            
            const unresolved = mergeReadiness.unresolved_total;
            if(unresolved > 0){
               { svelteHTML.createElement("span", {   "class":`chip rp-readiness-chip`,"style":`background:var(--danger-soft);color:var(--danger)`,});plural(unresolved, 'open finding'); }
            }else{
               { svelteHTML.createElement("span", { "class":`chip ok rp-readiness-chip`,});   }
            }
            
            const mergeable = (mergeReadiness as any).mergeable;
            if(mergeable === true){
               { svelteHTML.createElement("span", { "class":`chip ok rp-readiness-chip`,});  }
            } else if (mergeable === false){
               { svelteHTML.createElement("span", {   "class":`chip rp-readiness-chip`,"style":`color:var(--danger)`,});  }
            }
          }
         }
       }
    }

    
    if(ws.currentId){
       { svelteHTML.createElement("div", { "class":`rp-findings-section`,});
         { svelteHTML.createElement("h3", { "class":`rp-findings-title`,});  }
         { const $$_draoBsgnidniF2C = __sveltets_2_ensureComponent(FindingsBoard); new $$_draoBsgnidniF2C({ target: __sveltets_2_any(), props: {    "reviewId":review.id,"workspaceId":ws.currentId,}});}
       }
    }

    
    if(review.agents.length > 1){
       { const $$_stnegAweiveR1C = __sveltets_2_ensureComponent(ReviewAgents); new $$_stnegAweiveR1C({ target: __sveltets_2_any(), props: {     review,"view":`done`,"onretried":onAgentRetried,}});}
    }

    if(review.comments.length === 0){
       { svelteHTML.createElement("p", {   "class":`dim`,"style":`font-size: var(--fs-s); padding: 16px 0`,});   }
    }else{
      if(draftedBy){
        
         { svelteHTML.createElement("p", {   "class":`rp-attrib`,...__sveltets_2_empty({"data-testid":`rp-attrib`}),});
           { const $$_nocIredivorP2C = __sveltets_2_ensureComponent(ProviderIcon); new $$_nocIredivorP2C({ target: __sveltets_2_any(), props: {    "provider":draftedBy.provider,"size":12,}});}
           { svelteHTML.createElement("span", {});
                draftedBy.provider;draftedBy.model ? ` · ${draftedBy.model}` : ''; draftedBy.lenses.length > 0 ? ` from ${draftedBy.lenses.join(', ')}` : '';        
           }
         }
      }
       { svelteHTML.createElement("div", { "class":`rp-list`,});
           for(let c of __sveltets_2_ensureArray(review.comments)){c.id;
          const snippetLines = getSnippetLines(c);
          const defaultExpanded = isDiffExpandedDefault(c);
          const expanded = isDiffExpanded(c.id, defaultExpanded);
           { svelteHTML.createElement("div", { "class":`rp-comment card`,});
             { svelteHTML.createElement("div", { "class":`rp-comment-head`,});
               { svelteHTML.createElement("span", { "class":`severity-chip sev-${c.severity}`,});c.severity; }
              if(c.path !== null){
                 { svelteHTML.createElement("span", {     "class":`mono rp-loc`,"dir":`ltr`,"title":c.path,});c.path;c.line !== null ? `:${c.line}` : ''; }
              }
               { svelteHTML.createElement("span", { "class":`grow`,}); }
              if(c.state === 'draft'){
                 { svelteHTML.createElement("button", {           "class":`btn small`,"disabled":!!actionBusy[c.id] || postingAll || c.id in editingBody,...__sveltets_2_empty({"data-testid":`rp-post-comment`}),"title":c.id in editingBody ? 'Save or cancel your edit first' : `Post this comment to ${prWhere}`,"onclick":() => void postComment(c),});
                  actionBusy[c.id] === 'approve' ? 'Posting…' : 'Post to PR…';
                 }
                if(!(c.id in editingBody)){
                   { svelteHTML.createElement("button", {           "class":`btn small ghost`,"disabled":!!actionBusy[c.id] || postingAll,...__sveltets_2_empty({"data-testid":`rp-edit-comment`}),"title":`Edit the drafted text before posting`,"onclick":() => (editingBody = { ...editingBody, [c.id]: c.body }),});
                    
                   }
                }
                 { svelteHTML.createElement("button", {       "class":`btn small ghost`,"disabled":!!actionBusy[c.id] || postingAll,"onclick":() => declineComment(c),});
                  actionBusy[c.id] === 'decline' ? 'Declining…' : 'Decline';
                 }
              } else if (c.state === 'approved' && c.posted){
                 { svelteHTML.createElement("span", { "class":`chip ok rp-badge`,});
                   { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});} 
                 }
              } else if (c.state === 'approved'){
                 { svelteHTML.createElement("span", {   "class":`chip rp-badge dim`,"title":`Approved in Otto, but the provider refused the post`,});  }
                 { svelteHTML.createElement("button", {           "class":`btn small`,"disabled":!!actionBusy[c.id] || postingAll,...__sveltets_2_empty({"data-testid":`rp-retry-post`}),"title":`Post this comment to ${prWhere} again`,"onclick":() => void postComment(c),});
                  actionBusy[c.id] === 'approve' ? 'Posting…' : 'Retry post…'; 
                 }
              }else{
                 { svelteHTML.createElement("span", { "class":`chip rp-badge dim`,});  }
                if(!c.posted){
                   { svelteHTML.createElement("button", {           "class":`btn small ghost`,"disabled":!!actionBusy[c.id],...__sveltets_2_empty({"data-testid":`rp-restore-comment`}),"title":`Move this comment back to draft`,"onclick":() => void editComment(c, { restore_draft: true }),});
                    actionBusy[c.id] === 'edit' ? 'Restoring…' : 'Restore to draft';
                   }
                }
              }
             }
            if(c.state === 'draft' && c.id in editingBody){
               { svelteHTML.createElement("textarea", {            "dir":`auto`,"class":`rp-edit-body`,"rows":5,"aria-label":`Comment text`,...__sveltets_2_empty({"data-testid":`rp-edit-body`}),"bind:value":editingBody[c.id],});/*Ωignore_startΩ*/() => editingBody[c.id] = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
               { svelteHTML.createElement("div", { "class":`rp-edit-actions`,});
                 { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":!!actionBusy[c.id],"onclick":() => void saveEdit(c),});
                  actionBusy[c.id] === 'edit' ? 'Saving…' : 'Save';
                 }
                 { svelteHTML.createElement("button", {     "class":`btn small ghost`,"disabled":!!actionBusy[c.id],"onclick":() => stopEditing(c.id),});
                  
                 }
               }
            }else{
               { svelteHTML.createElement("p", { "class":`rp-comment-body`,});c.body; }
            }

            
            if(snippetLines !== null){
               { svelteHTML.createElement("div", { "class":`rp-diff-toggle-row`,});
                 { svelteHTML.createElement("button", {       "class":`rp-diff-toggle`,"onclick":() => toggleDiff(c.id, defaultExpanded),"aria-expanded":expanded,});
                   { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":expanded ? 'chevronDown' : 'chevronRight',"size":12,}});} expanded ? 'Hide diff' : 'Show diff';
                 }
               }
              if(expanded){
                 { svelteHTML.createElement("div", {   "class":`rp-diff-snippet`,"dir":`ltr`,});
                    for(let dl of __sveltets_2_ensureArray(snippetLines)){
                    const isTarget = (dl.new_line !== null && dl.new_line === c.line) || (dl.new_line === null && dl.old_line !== null && dl.old_line === c.line);
                     { svelteHTML.createElement("div", {   "class":`rp-diff-line rp-diff-${dl.origin}${isTarget ? ' rp-diff-target' : ''}`,});
                       { svelteHTML.createElement("span", { "class":`rp-diff-gutter`,});
                        dl.origin === 'add' ? '+' : dl.origin === 'del' ? '-' : ' ';
                       }
                       { svelteHTML.createElement("span", { "class":`rp-diff-linenum`,});dl.new_line ?? dl.old_line ?? ''; }
                       { svelteHTML.createElement("span", { "class":`rp-diff-content`,});dl.content; }
                     }
                  }
                 }
              }
            }
           }
        }
       }
    }
  }

  
  if(history.length > 1){
    const headerOpen = !!historyExpanded['_header' as unknown as number];
     { svelteHTML.createElement("div", { "class":`rp-history`,});
       { svelteHTML.createElement("button", {       "class":`rp-history-toggle`,"onclick":() => { historyExpanded = { ...historyExpanded, ['_header' as unknown as number]: !headerOpen }; },"aria-expanded":headerOpen,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":headerOpen ? 'chevronDown' : 'chevronRight',"size":12,}});}   history.length - 1;
       }
      if(headerOpen){
         { svelteHTML.createElement("div", { "class":`rp-history-list`,});
              for(let run of __sveltets_2_ensureArray(history.slice(1))){let i = 1;run.id;
            const isOpen = !!historyExpanded[i];
             { svelteHTML.createElement("div", { "class":`rp-history-run card`,});
               { svelteHTML.createElement("button", {       "class":`rp-history-run-header`,"onclick":() => toggleHistoryRun(i),"aria-expanded":isOpen,});
                 { svelteHTML.createElement("span", {   "class":`dim`,"style":`font-size: var(--fs-xs)`,});timeAgo(run.created_at); }
                 { const $$_egdaBsutatS5C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS5C({ target: __sveltets_2_any(), props: {  "status":runStatus(run.status),}});}
                if(run.agents && run.agents.length > 0){
                   { svelteHTML.createElement("span", {   "class":`dim`,"style":`font-size:var(--fs-xs)`,});run.agents.filter(a => a.status === 'done').length; run.agents.length;  }
                }
                 { svelteHTML.createElement("span", {   "class":`dim`,"style":`font-size:var(--fs-xs)`,});plural(run.comments.length, 'comment'); }
                 { svelteHTML.createElement("span", { "class":`grow`,}); }
                 { svelteHTML.createElement("span", {   "class":`dim`,"aria-hidden":`true`,}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":isOpen ? 'chevronDown' : 'chevronRight',"size":12,}});} }
               }
              if(isOpen){
                 { svelteHTML.createElement("div", { "class":`rp-history-run-body`,});
                  if(run.comments.length === 0){
                     { svelteHTML.createElement("p", {   "class":`dim`,"style":`font-size: var(--fs-xs);padding:4px 0`,});     }
                  }else{
                       for(let c of __sveltets_2_ensureArray(run.comments)){c.id;
                       { svelteHTML.createElement("div", { "class":`rp-comment card rp-history-comment`,});
                         { svelteHTML.createElement("div", { "class":`rp-comment-head`,});
                           { svelteHTML.createElement("span", { "class":`severity-chip sev-${c.severity}`,});c.severity; }
                          if(c.path !== null){
                             { svelteHTML.createElement("span", {     "class":`mono rp-loc`,"dir":`ltr`,"title":c.path,});c.path;c.line !== null ? `:${c.line}` : ''; }
                          }
                           { svelteHTML.createElement("span", { "class":`grow`,}); }
                          if(c.state === 'approved' && c.posted){
                             { svelteHTML.createElement("span", { "class":`chip ok rp-badge`,}); { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}  }
                          } else if (c.state === 'approved'){
                             { svelteHTML.createElement("span", { "class":`chip rp-badge dim`,});  }
                          } else if (c.state === 'declined'){
                             { svelteHTML.createElement("span", { "class":`chip rp-badge dim`,});  }
                          }else{
                             { svelteHTML.createElement("span", { "class":`chip rp-badge dim`,});  }
                          }
                         }
                         { svelteHTML.createElement("p", { "class":`rp-comment-body`,});c.body; }
                       }
                    }
                  }
                 }
              }
             }
          }
         }
      }
     }
  }
 }


if(showConfig){
   { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {     "title":`Configure review agents`,"width":580,"onclose":closeConfig,children:() => { async ()/*Ωignore_positionΩ*/ => {
      if(configLoading){
         { const $$_notelekS0C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS0C({ target: __sveltets_2_any(), props: {    "rows":3,"height":36,}});}
      }else{
         { svelteHTML.createElement("p", { "class":`dim cfg-note`,});
                            
         }

        
         { svelteHTML.createElement("div", {     "class":`segmented cfg-scope`,"role":`group`,"aria-label":`Review config scope`,});
           { svelteHTML.createElement("button", {     "aria-pressed":configScope === 'global',"onclick":() => setConfigScope('global'),});configScope === 'global';
             
           }
           { svelteHTML.createElement("button", {     "aria-pressed":configScope === 'repo',"onclick":() => setConfigScope('repo'),});configScope === 'repo';
             
           }
         }
        if(configScope === 'global'){
          if(repoCfg && repoCfg.scope !== 'global'){
             { svelteHTML.createElement("p", { "class":`dim cfg-scope-note`,});
                    
              repoCfg.scope === 'preset' ? `preset “${repoCfg.preset_name ?? repoCfg.preset_id}”` : 'custom';
                           
             }
          }
        }else{
           { svelteHTML.createElement("div", { "class":`cfg-repo-row`,});
             { svelteHTML.createElement("select", {        "aria-label":`Review preset`,"class":`cfg-select cfg-preset-select`,"value":selectedPresetId,"onchange":(e) => pickNamedPreset((e.currentTarget as HTMLSelectElement).value),});
               { svelteHTML.createElement("option", {"value":"",});     }
                 for(let p of __sveltets_2_ensureArray(namedPresets)){p.id;
                 { svelteHTML.createElement("option", { "value":p.id,});p.name; }
              }
             }
             { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":() => (presetSaveOpen = !presetSaveOpen),});
                
             }
            if(repoCfg && repoCfg.scope !== 'global'){
               { svelteHTML.createElement("button", {     "class":`btn small ghost`,"disabled":configSaving,"onclick":useGlobalConfig,});
                 
               }
            }
           }
          if(presetSaveOpen){
             { svelteHTML.createElement("div", { "class":`cfg-repo-row cfg-preset-save-row`,});
               { svelteHTML.createElement("input", {            "dir":`auto`,"aria-label":`Preset name`,"class":`cfg-input`,"bind:value":presetNameDraft,"placeholder":`Preset name (e.g. Backend services)`,"onkeydown":(e) => { if (e.key === 'Enter') void saveAsNamedPreset(); },});/*Ωignore_startΩ*/() => presetNameDraft = __sveltets_2_any(null);/*Ωignore_endΩ*/}
               { svelteHTML.createElement("button", {       "class":`btn small primary`,"disabled":configSaving || presetNameDraft.trim() === '',"onclick":saveAsNamedPreset,});
                  
               }
             }
          }
           { svelteHTML.createElement("p", { "class":`dim cfg-scope-note`,});
            if(selectedPresetId === ''){      }else{            
                          }
           }
        }

         { svelteHTML.createElement("h3", { "class":`cfg-section-title`,});  }
            for(let agent of __sveltets_2_ensureArray(editAgents)){let i = 1;i;
          const agentProviders = agent.providers && agent.providers.length > 0 ? agent.providers : [agent.provider || defaultAgentProvider()];
           { svelteHTML.createElement("div", { "class":`cfg-agent-row card`,});
             { svelteHTML.createElement("div", { "class":`cfg-agent-fields`,});
               { svelteHTML.createElement("div", { "class":`cfg-field`,});
                 { svelteHTML.createElement("span", { "class":`cfg-label`,});  }
                 { svelteHTML.createElement("input", {        "dir":`auto`,"aria-label":`Reviewer name`,"class":`cfg-input`,"bind:value":editAgents[i].name,});/*Ωignore_startΩ*/() => editAgents[i].name = __sveltets_2_any(null);/*Ωignore_endΩ*/}
               }
               { svelteHTML.createElement("div", { "class":`cfg-field`,});
                 { svelteHTML.createElement("span", { "class":`cfg-label`,});   }
                 { svelteHTML.createElement("div", { "class":`cfg-provider-checks`,});
                    for(let p of __sveltets_2_ensureArray(PROVIDER_OPTIONS)){
                     { svelteHTML.createElement("label", { "class":`cfg-check-label`,});
                       { svelteHTML.createElement("input", {       "type":`checkbox`,"checked":agentProviders.includes(p),"onchange":() => toggleProvider(i, p),});}
                      p;
                     }
                  }
                 }
               }
               { svelteHTML.createElement("div", { "class":`cfg-field`,});
                 { svelteHTML.createElement("span", { "class":`cfg-label`,});   }
                 { svelteHTML.createElement("textarea", {         "dir":`auto`,"aria-label":`Reviewer lens / instructions`,"class":`cfg-textarea`,"rows":3,"bind:value":editAgents[i].prompt,});/*Ωignore_startΩ*/() => editAgents[i].prompt = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
               }
             }
             { svelteHTML.createElement("div", { "class":`cfg-agent-actions`,});
               { svelteHTML.createElement("button", {       "class":`btn small ghost cfg-save-preset`,"title":`Save as preset`,"onclick":() => saveAsPreset(i),}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`star`,"size":12,}});}    }
               { svelteHTML.createElement("button", {       "class":`icon-btn cfg-remove`,"onclick":() => removeAgent(i),"aria-label":`Remove agent ${editAgents[i].name || i + 1}`,"title":`Remove agent`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});} }
             }
           }
        }
         { svelteHTML.createElement("div", { "class":`cfg-add-row`,});
           { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":addAgent,}); { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}   }
           { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":openPresetMenu,}); { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}    { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`chevronDown`,"size":12,}});} }
         }

        
        if(editPresets.length > 0){
           { svelteHTML.createElement("h3", { "class":`cfg-section-title cfg-presets-title`,});  }
           { svelteHTML.createElement("div", { "class":`cfg-presets-list`,});
                for(let preset of __sveltets_2_ensureArray(editPresets)){let i = 1;i;
               { svelteHTML.createElement("div", { "class":`cfg-preset-chip`,});
                 { svelteHTML.createElement("span", { "class":`cfg-preset-name`,});preset.name; }
                 { svelteHTML.createElement("button", {         "class":`cfg-preset-action`,"title":`Add to agents`,"onclick":() => addPresetToAgents(preset),"aria-label":`Add preset ${preset.name} to agents`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});} }
                 { svelteHTML.createElement("button", {         "class":`cfg-preset-action cfg-preset-del`,"title":`Delete preset`,"onclick":() => removePreset(i),"aria-label":`Delete preset ${preset.name}`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});} }
               }
            }
           }
        }

         { svelteHTML.createElement("h3", {   "class":`cfg-section-title`,"style":`margin-top:18px`,});  }
         { svelteHTML.createElement("div", { "class":`cfg-agent-row card`,});
           { svelteHTML.createElement("div", { "class":`cfg-agent-fields`,});
             { svelteHTML.createElement("div", { "class":`cfg-field`,});
               { svelteHTML.createElement("span", { "class":`cfg-label`,});  }
               { svelteHTML.createElement("input", {        "dir":`auto`,"aria-label":`Summarizer name`,"class":`cfg-input`,"bind:value":editSummarizer.name,});/*Ωignore_startΩ*/() => editSummarizer.name = __sveltets_2_any(null);/*Ωignore_endΩ*/}
             }
             { svelteHTML.createElement("div", { "class":`cfg-field`,});
               { svelteHTML.createElement("span", { "class":`cfg-label`,});  { svelteHTML.createElement("span", { "class":`dim`,});    } }
               { svelteHTML.createElement("select", {     "aria-label":`Summarizer provider`,"class":`cfg-select`,"bind:value":editSummarizer.provider,});/*Ωignore_startΩ*/() => editSummarizer.provider = __sveltets_2_any(null);/*Ωignore_endΩ*/
                  for(let p of __sveltets_2_ensureArray(PROVIDER_OPTIONS)){
                   { svelteHTML.createElement("option", { "value":p,});p; }
                }
               }
             }
             { svelteHTML.createElement("div", { "class":`cfg-field`,});
               { svelteHTML.createElement("span", { "class":`cfg-label`,});    }
               { svelteHTML.createElement("textarea", {         "dir":`auto`,"aria-label":`Summarizer merge / dedupe instructions`,"class":`cfg-textarea`,"rows":3,"bind:value":editSummarizer.prompt,});/*Ωignore_startΩ*/() => editSummarizer.prompt = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
             }
           }
         }
      }
    };return __sveltets_2_any(0)},footer:() => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("button", {   "class":`btn ghost`,"onclick":closeConfig,});  }
       { svelteHTML.createElement("button", {     "class":`btn primary`,"disabled":configSaving || configLoading,"onclick":saveConfig,});
        configSaving ? 'Saving…' : 'Save';
       }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {children, footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
    
    
   Modal}
}


if(showJiraPicker){
   { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {     "title":`Attach story`,"width":520,"onclose":closeJiraPicker,children:() => { async ()/*Ωignore_positionΩ*/ => {
       { const $$_rekciPeussIariJ0C = __sveltets_2_ensureComponent(JiraIssuePicker); new $$_rekciPeussIariJ0C({ target: __sveltets_2_any(), props: {  "onpick":(iss) => { attachedIssue = iss; closeJiraPicker(); },}});}
    };return __sveltets_2_any(0)},footer:() => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("button", {   "class":`btn ghost`,"onclick":closeJiraPicker,});  }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {children, footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
    
    
   Modal}
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ReviewPanel__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ReviewPanel__SvelteComponent_ = ReturnType<typeof ReviewPanel__SvelteComponent_>;
/*Ωignore_endΩ*/export default ReviewPanel__SvelteComponent_;