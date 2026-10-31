///<reference types="svelte" />
;
import { registry } from '../../lib/commands.svelte';
import { plural } from '../../lib/plural';
// Local working-tree review panel: diff against a chosen base branch, run
// the configured review agents, show findings with checkboxes, and hand
// selected findings off to a new agent session.
import { untrack } from 'svelte';
import { toastError } from '../../lib/toastError';
import StatusBadge from '../../lib/components/StatusBadge.svelte';
import { runStatus } from '../../lib/status';
import { api, ApiError } from '../../lib/api/client';
import { git } from '../../lib/stores/git.svelte';
import type { Review, ReviewComment, RefsResp, Session } from '../../lib/api/types';
import { ws } from '../../lib/stores/workspace.svelte';
import { router } from '../../lib/router.svelte';
import { toasts } from '../../lib/toast.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
import Icon from '../../lib/components/Icon.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../lib/loadError';
import { rel } from '../../lib/stores/now.svelte';
import ReviewAgents from './ReviewAgents.svelte';
import FindingsBoard from './FindingsBoard.svelte';
import ProviderIcon from '../../lib/components/ProviderIcon.svelte';
import { agentProviders } from '../../lib/providers';
import { ctxMenu } from '../../lib/contextmenu.svelte';
import { reviewBus } from '../../lib/events.svelte';
import { appLive } from '../../lib/live';
import { routeReviewId } from './deepLink';
import { runWithOtto } from '../../lib/stores/runWithOtto.svelte';
import { summaryFallback, summaryFallbackSentence, summaryFallbackTitle } from './reviewFallback';
;

  interface Props {
    repoId: string;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { repoId }: Props = $props();

  // ---------------------------------------------------------------------------
  // Branch/ref selector state
  // ---------------------------------------------------------------------------
  let refs: RefsResp | null = $state(null);
  let refsLoading = $state(false);
  let selectedBase = $state('');

  // ---------------------------------------------------------------------------
  // Review state
  // ---------------------------------------------------------------------------
  let review: Review | null = $state(null);
  // Full history of all runs, newest first
  let history: Review[] = $state([]);
  let reviewLoading = $state(true);
  let starting = $state(false);
  let cancelling = $state(false);
  let pollTimer: ReturnType<typeof setTimeout> | null = null;
  /** False once the panel unmounts: a poll already in flight then must not
   *  re-arm the timer (it kept polling — and could toast "taking too long" —
   *  on another page for up to MAX_POLLS). */
  let alive = true;
  let pollCount = $state(0);
  /** Fallback polls before "taking too long": 3 min at 2 s with no event
   *  socket; with one, `review_changed` drives the refresh and the 10 s
   *  safety-net chain gives up after 15 min. */
  const MAX_POLLS = 90;
  // Which older run indices (history[1+]) are expanded
  let historyExpanded: Record<number, boolean> = $state({});

  // ---------------------------------------------------------------------------
  // Per-comment checkbox state (comment id → checked)
  // ---------------------------------------------------------------------------
  let checked: Record<string, boolean> = $state({});

  const checkedIds = $derived.by(() => {
    return Object.entries(checked)
      .filter(([, v]) => v)
      .map(([k]) => k);
  });

  // Runs shown under "Past reviews": everything that isn't the currently active
  // run. Whenever a review is active it is always history[0] (mount adopts only
  // history[0]; start/poll/retry all write history[0]), so drop it; on a clean
  // slate (review === null) every stored run is "past".
  const pastRuns = $derived(review ? history.slice(1) : history);

  // ---------------------------------------------------------------------------
  // Deep link to ONE review (S20-301): `#/git/<repo>/review/<review_id>` —
  // Run with Otto's "Open findings". That review is shown read-only on its own
  // (findings board with severities, the run it came from), instead of the
  // clean-slate panel where the run's findings hid in "Past reviews".
  // ---------------------------------------------------------------------------
  const focusId = $derived(routeReviewId(router.parts, repoId));
  let pinned: Review | null = $state(null);
  let pinnedLoading = $state(false);
  let pinnedError = $state<string | null>(null);
  /** The run that produced the linked review, when the run list knows it. */
  const pinnedRun = $derived(focusId ? (runWithOtto.list.find((r) => r.review_id === focusId) ?? null) : null);
  $effect(() => {
    const id = focusId;
    pinned = null;
    pinnedError = null;
    if (id) void loadPinned(id);
  });

  async function loadPinned(id: string): Promise<void> {
    pinnedLoading = true;
    try {
      const r = await api.get<Review>(`/reviews/${encodeURIComponent(id)}`);
      if (focusId === id) pinned = r;
    } catch (e) {
      if (focusId === id) pinnedError = e instanceof ApiError && e.status === 404 ? 'This review no longer exists.' : loadErrorText(e);
    } finally {
      if (focusId === id) pinnedLoading = false;
    }
  }

  /** Leave the linked review for the normal local-review panel. */
  function closePinned(): void {
    router.go(`git/${encodeURIComponent(repoId)}/review`);
  }

  // Reviewer providers from the live registry (built-ins + custom, e.g. grok).
  const PROVIDER_OPTIONS = $derived(agentProviders());

  // ---------------------------------------------------------------------------
  // Load refs once on mount, and try to load an existing local review
  // ---------------------------------------------------------------------------
  $effect(() => {
    void loadRefs(repoId);
    void loadExisting(repoId);
    alive = true;
    document.addEventListener('visibilitychange', onVisibility);
    return () => {
      alive = false;
      if (pollTimer !== null) clearTimeout(pollTimer);
      document.removeEventListener('visibilitychange', onVisibility);
    };
  });

  // A hidden window never polls; it reads once when it is shown again.
  function onVisibility(): void {
    if (document.hidden) {
      if (pollTimer !== null) clearTimeout(pollTimer);
      pollTimer = null;
    } else if (review?.status === 'running') {
      void poll(true);
    }
  }

  // `review_changed` for the running local review re-reads it at once (the
  // shared review runner emits it per agent step), so the timer chain below
  // is only a safety net while the event socket is up.
  $effect(() => {
    void reviewBus.tick;
    const evId = reviewBus.reviewId;
    const cur = untrack(() => review);
    if (!evId || !cur || cur.id !== evId) return;
    void poll(true);
  });

  /** Why the base-branch list or the review history failed to load — shown
   *  inline with Retry (a silent failure left an empty "Compare to" select). */
  let refsError = $state<string | null>(null);
  let historyError = $state<string | null>(null);

  async function loadRefs(rid: string): Promise<void> {
    refsLoading = true;
    refsError = null;
    try {
      const read = await git.refs(rid);
      if (read.error !== null) throw read.error;
      refs = read.refs;
      // Pick a sensible default base.
      const remotes = refs?.remote ?? [];
      const locals = refs?.local ?? [];
      const preferred = ['origin/develop', 'origin/main', 'origin/master', 'develop', 'main', 'master'];
      const allNames = [...remotes.map((r) => r.name), ...locals.map((l) => l.name)];
      const hit = preferred.find((p) => allNames.includes(p));
      selectedBase = allBranches.find((b) => b.name === hit)?.value ?? allBranches[0]?.value ?? '';
    } catch (e) {
      refsError = loadErrorText(e);
    } finally {
      refsLoading = false;
    }
  }

  async function loadExisting(rid: string): Promise<void> {
    reviewLoading = true;
    historyError = null;
    try {
      const runs = await api.get<Review[]>(`/repos/${rid}/local-reviews`);
      history = runs;
      // Clean slate by default: a finished run (done/error) is kept in history
      // only — never resurrected as the active panel, because its findings were
      // computed against an earlier working tree that may no longer exist. Only
      // a still-running review is re-adopted (and polling resumed) so an
      // in-flight run survives navigating away from the tab and back.
      const newest = runs.length > 0 ? runs[0] : null;
      if (newest && newest.status === 'running') {
        review = newest;
        schedulePoll();
      } else {
        review = null;
      }
    } catch (e) {
      if (e instanceof ApiError && e.status === 404) {
        review = null;
        history = [];
      } else {
        historyError = loadErrorText(e);
      }
    } finally {
      reviewLoading = false;
    }
  }

  function initChecked(comments: ReviewComment[]): void {
    const next: Record<string, boolean> = {};
    for (const c of comments) {
      // Default: check all non-declined comments.
      next[c.id] = c.state !== 'declined';
    }
    checked = next;
  }

  function schedulePoll(): void {
    if (!alive) return;
    if (pollTimer !== null) clearTimeout(pollTimer);
    pollTimer = null;
    if (typeof document !== 'undefined' && document.hidden) return;
    pollTimer = setTimeout(() => void poll(), appLive.connected() ? 10_000 : 2000);
  }

  // A child <ReviewAgents> retried one agent: adopt the refreshed review and
  // resume polling so we keep tracking the re-run.
  function onAgentRetried(r: Review): void {
    review = r;
    history = history.length > 0 ? [r, ...history.slice(1)] : [r];
    pollCount = 0;
    schedulePoll();
  }

  /** Re-read the running review. `fromBus` (an event or a re-shown window)
   *  doesn't count toward the fallback budget. */
  async function poll(fromBus = false): Promise<void> {
    if (!alive) return;
    // Only the TIMER safety net has a budget. A bus/visibility poll is proof
    // the review is alive: it always re-reads and resets the budget — gating
    // it too froze the panel (and re-toasted) on every event after 15 min.
    if (fromBus) {
      pollCount = 0;
    } else if (++pollCount > MAX_POLLS) {
      if (!budgetWarned) {
        budgetWarned = true;
        toasts.warn('Review is taking too long', 'Still running — it refreshes on its next update, or refresh manually.');
      }
      return;
    }
    try {
      const r = await api.get<Review>(`/repos/${repoId}/local-review`);
      if (!alive) return;
      review = r;
      if (history.length > 0) {
        history = [r, ...history.slice(1)];
      } else {
        history = [r];
      }
      if (r.status === 'running') {
        schedulePoll();
      } else {
        pollCount = 0;
        if (r.status === 'done') initChecked(r.comments);
      }
    } catch {
      schedulePoll();
    }
  }

  /** The over-budget toast fires once per review, not once per event. */
  let budgetWarned = false;

  async function startReview(): Promise<void> {
    if (!selectedBase) {
      toasts.warn('Select a base branch first');
      return;
    }
    if (pollTimer !== null) clearTimeout(pollTimer);
    starting = true;
    pollCount = 0;
    budgetWarned = false;
    checked = {};
    try {
      const newRun = await api.post<Review>(`/repos/${repoId}/local-review`, { base: selectedBase });
      // Prepend new run; keep existing runs as history
      review = newRun;
      history = [newRun, ...history];
      if (review.status === 'running') schedulePoll();
      if (review.status === 'done') initChecked(review.comments);
    } catch (e) {
      toastError('Couldn’t start review', e);
    } finally {
      starting = false;
    }
  }

  async function cancelReview(): Promise<void> {
    if (cancelling || !review || review.status !== 'running') return;
    if (pollTimer !== null) clearTimeout(pollTimer);
    cancelling = true;
    try {
      const updated = await api.post<Review>(`/reviews/${review.id}/cancel`, {});
      if (!alive) return;
      review = updated;
      history = history.map((r) => (r.id === updated.id ? updated : r));
      toasts.info('Review stopped');
    } catch (e) {
      toastError('Couldn’t stop the review', e);
    } finally {
      if (alive) {
        cancelling = false;
        if (review?.status === 'running') schedulePoll();
      }
    }
  }

  const draftedBy = $derived.by(() => {
    const ag = review?.agents ?? [];
    if (ag.length === 0) return null;
    const summ = ag[ag.length - 1];
    const reviewers = ag.length > 1 ? ag.slice(0, -1) : ag;
    const lenses = [...new Set(reviewers.map((a) => a.lens ?? a.name).filter((n) => n.trim() !== ''))];
    return { provider: summ.provider, model: summ.model, lenses };
  });

  // ---------------------------------------------------------------------------
  // Select all / none helpers
  // ---------------------------------------------------------------------------

  function allComments(): ReviewComment[] {
    return review?.comments ?? [];
  }

  function selectAll(): void {
    const next: Record<string, boolean> = {};
    for (const c of allComments()) {
      next[c.id] = true;
    }
    checked = next;
  }

  function selectNone(): void {
    const next: Record<string, boolean> = {};
    for (const c of allComments()) {
      next[c.id] = false;
    }
    checked = next;
  }

  const allSelected = $derived.by(() => {
    const cs = allComments();
    return cs.length > 0 && cs.every((c) => checked[c.id]);
  });

  const noneSelected = $derived.by(() => {
    return allComments().every((c) => !checked[c.id]);
  });

  // ---------------------------------------------------------------------------
  // Handoff to agent
  // ---------------------------------------------------------------------------

  function openHandoffMenu(e: MouseEvent): void {
    if (checkedIds.length === 0) {
      toasts.warn('Select at least one finding to hand off');
      return;
    }
    ctxMenu.show(
      e,
      PROVIDER_OPTIONS.map((p) => ({
        label: p,
        action: () => void handoff(p),
      })),
    );
  }

  async function handoff(provider: string): Promise<void> {
    if (!review) return;
    if (checkedIds.length === 0) {
      toasts.warn('Select at least one finding to hand off');
      return;
    }
    try {
      const session = await api.post<Session>(`/reviews/${review.id}/handoff`, {
        provider,
        comment_ids: checkedIds,
      });
      ws.addSession(session); // navigates to the new session via addSession → navigateToSession
      toasts.success(`Handed ${plural(checkedIds.length, 'finding')} to ${provider}`);
    } catch (e) {
      toastError('Couldn’t hand off the review', e);
    }
  }

  // ---------------------------------------------------------------------------
  // History helpers
  // ---------------------------------------------------------------------------

  // The shared, self-ticking relative time (lib/stores/now) — no local formatter.
  const timeAgo = (iso: string): string => rel(iso);

  function toggleHistoryRun(idx: number): void {
    historyExpanded = { ...historyExpanded, [idx]: !historyExpanded[idx] };
  }

  // ---------------------------------------------------------------------------
  // Branch list for the <select>
  // ---------------------------------------------------------------------------

  const allBranches = $derived.by(() => {
    if (!refs) return [];
    const locals = new Set(refs.local.map((b) => b.name));
    const remotes = new Set(refs.remote.map((b) => b.name));
    // A local branch named origin/foo is distinct from the remote origin/foo.
    // Keep both choices and pass Git an unambiguous revision in that case.
    return [
      ...refs.remote.map((b) => ({ name: b.name, value: locals.has(b.name) ? `refs/remotes/${b.name}` : b.name, label: locals.has(b.name) ? `${b.name} (remote)` : b.name })),
      ...refs.local.map((b) => ({ name: b.name, value: remotes.has(b.name) ? `refs/heads/${b.name}` : b.name, label: remotes.has(b.name) ? `${b.name} (local)` : b.name })),
    ];
  });
  $effect(() => registry.register(`local-review:${repoId}`, reviewLoading || focusId ? [] : [
    review?.status === 'running'
      ? { id: `local-review.stop.${repoId}`, title: 'Stop local review', group: 'Git', keywords: 'review findings changes', disabled: cancelling, run: cancelReview }
      : { id: `local-review.start.${repoId}`, title: 'Send local changes to review agents', group: 'Git', keywords: 'review findings changes', disabled: starting || !selectedBase || refsLoading, run: startReview },
  ]));
;
async () => {

 { svelteHTML.createElement("div", { "class":`lrp`,});
  if(focusId){
    
     { svelteHTML.createElement("section", {     "class":`lrp-pinned`,...__sveltets_2_empty({"data-testid":`linked-review`}),"aria-label":`Linked review`,});
       { svelteHTML.createElement("header", { "class":`lrp-pinned-head`,});
         { svelteHTML.createElement("div", { "class":`lrp-pinned-title`,});
           { svelteHTML.createElement("h3", {});  }
           { svelteHTML.createElement("p", { "class":`dim`,});
            if(pinnedRun){   { svelteHTML.createElement("button", {     "type":`button`,"class":`link`,"onclick":() => router.go(`run-with-otto/${encodeURIComponent(pinnedRun.id)}`),}); pinnedRun.title;  }pinnedRun.branch ? ` on ${pinnedRun.branch}` : '';  }else{       }
           }
         }
         { svelteHTML.createElement("button", {     "type":`button`,"class":`btn small`,"onclick":closePinned,});   }
       }
      if(pinnedLoading && !pinned){
         { svelteHTML.createElement("div", { "style":`padding: 16px`,}); { const $$_notelekS3C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS3C({ target: __sveltets_2_any(), props: {    "rows":3,"height":36,}});} }
      } else if (pinnedError){
         { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {      "what":`the review`,"error":pinnedError,"empty":true,"onretry":() => focusId && void loadPinned(focusId),}});}
      } else if (pinned){
         { svelteHTML.createElement("p", { "class":`lrp-pinned-meta dim`,});
           { const $$_egdaBsutatS3C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS3C({ target: __sveltets_2_any(), props: {  "status":runStatus(pinned.status),}});}
           { svelteHTML.createElement("span", {});timeAgo(pinned.created_at); }
          if(pinned.summary_fallback){ { svelteHTML.createElement("span", { "title":summaryFallbackTitle(summaryFallback(pinned.agents), 'these findings'),});    summaryFallback(pinned.agents).provider;    }}
         }
        if(pinned.status === 'running'){
           { svelteHTML.createElement("p", { "class":`dim`,});            }
        } else if (ws.currentId){
           { const $$_draoBsgnidniF2C = __sveltets_2_ensureComponent(FindingsBoard); new $$_draoBsgnidniF2C({ target: __sveltets_2_any(), props: {    "reviewId":pinned.id,"workspaceId":ws.currentId,}});}
        }
      }
     }
  }else{
  
   { svelteHTML.createElement("div", { "class":`lrp-toolbar`,});
     { svelteHTML.createElement("label", {   "class":`lrp-label`,"for":`lrp-base`,});  }
    if(refsLoading){
       { svelteHTML.createElement("div", { "class":`lrp-select-skeleton`,}); }
    } else if (refsError){
       { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {        "what":`branches`,"error":refsError,"empty":true,"variant":`compact`,"onretry":() => void loadRefs(repoId),}});}
    }else{
       { svelteHTML.createElement("select", {       "id":`lrp-base`,"class":`lrp-select`,"bind:value":selectedBase,"disabled":starting,});/*Ωignore_startΩ*/() => selectedBase = __sveltets_2_any(null);/*Ωignore_endΩ*/
           for(let b of __sveltets_2_ensureArray(allBranches)){b.value;
           { svelteHTML.createElement("option", { "value":b.value,});b.label; }
        }
        if(allBranches.length === 0){
           { svelteHTML.createElement("option", {"value":"",});   }
        }
       }
    }
    
     { svelteHTML.createElement("button", {        "class":`btn small`,"disabled":starting || !selectedBase || refsLoading,"onclick":startReview,});review?.status !== 'done';
      if(starting){
         { svelteHTML.createElement("span", {     "class":`spinner`,"style":`--spinner-size: 10px`,"aria-hidden":`true`,}); } }else{
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`zap`,"size":12,}});}
        review?.status === 'done' ? 'Review again' : 'Review changes';
      }
     }
     { svelteHTML.createElement("span", {   "class":`lrp-cfg-note dim`,"title":`Configure them from a pull request’s Review tab`,});
             
     }
   }

  
  if(reviewLoading){
     { svelteHTML.createElement("div", { "style":`padding: 16px`,}); { const $$_notelekS2C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS2C({ target: __sveltets_2_any(), props: {    "rows":4,"height":36,}});} }
  } else if (historyError && !review){
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {      "what":`past local reviews`,"error":historyError,"empty":true,"onretry":() => void loadExisting(repoId),}});}
  } else if (!review){
    if(pastRuns.length > 0){
       { svelteHTML.createElement("p", { "class":`lrp-idle-note`,}); { svelteHTML.createElement("strong", {});   }           }
    }else{
     { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {       "icon":`zap`,"title":`No active review`,"body":`Select a base branch and click 'Review changes' to run AI agents on your current uncommitted work.`,}});}
    }
  } else if (review.status === 'running'){
     { svelteHTML.createElement("div", { "class":`lrp-running-header`,});
       { svelteHTML.createElement("span", {     "class":`spinner`,"style":`--spinner-size: 18px`,"aria-hidden":`true`,}); }
       { svelteHTML.createElement("span", { "class":`lrp-running-title`,});  }
       { svelteHTML.createElement("span", { "class":`grow`,}); }
       { svelteHTML.createElement("button", {         "class":`btn small ghost`,"disabled":cancelling,"onclick":cancelReview,...__sveltets_2_empty({"data-testid":`local-review-cancel`}),});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`stop`,"size":12,}});} cancelling ? 'Stopping…' : 'Stop review';
       }
     }
    if(review.agents && review.agents.length > 0){
       { const $$_stnegAweiveR1C = __sveltets_2_ensureComponent(ReviewAgents); new $$_stnegAweiveR1C({ target: __sveltets_2_any(), props: {     review,"view":`running`,"onretried":onAgentRetried,}});}
    }else{
       { svelteHTML.createElement("p", {   "class":`dim`,"style":`font-size: var(--fs-s);padding:8px 0`,});  }
    }
  } else if (review.status === 'cancelled'){
     { svelteHTML.createElement("div", {   "class":`lrp-error card`,...__sveltets_2_empty({"data-testid":`local-review-cancelled`}),});
       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":14,}});}
       { svelteHTML.createElement("span", { "class":`lrp-error-msg`,});  }
       { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":starting,"onclick":startReview,});
        starting ? 'Starting…' : 'Run again';
       }
     }
  } else if (review.status === 'error'){
     { svelteHTML.createElement("div", {   "class":`lrp-error card`,"role":`alert`,});
       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":14,}});}
       { svelteHTML.createElement("span", { "class":`lrp-error-msg`,});    { svelteHTML.createElement("span", { "class":`dim`,});review.error ?? 'No reason was reported — check Settings → Logs.'; } }
       { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":starting,"onclick":startReview,});
        starting ? 'Starting…' : 'Try again';
       }
     }
  }else{
    
    if(review.summary_fallback){
       { svelteHTML.createElement("p", {   "class":`lrp-fallback-note dim`,"title":summaryFallbackTitle(summaryFallback(review.agents), 'these findings'),});
         { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});} summaryFallbackSentence(summaryFallback(review.agents)); }
    }
    
    if(ws.currentId){
       { svelteHTML.createElement("div", { "class":`lrp-findings-section`,});
         { svelteHTML.createElement("h3", { "class":`lrp-findings-title`,});  }
         { const $$_draoBsgnidniF2C = __sveltets_2_ensureComponent(FindingsBoard); new $$_draoBsgnidniF2C({ target: __sveltets_2_any(), props: {    "reviewId":review.id,"workspaceId":ws.currentId,}});}
       }
    }

    
    if(review.agents.length > 1){
       { const $$_stnegAweiveR1C = __sveltets_2_ensureComponent(ReviewAgents); new $$_stnegAweiveR1C({ target: __sveltets_2_any(), props: {     review,"view":`done`,"onretried":onAgentRetried,}});}
    }
    if(review.comments.length === 0){
       { svelteHTML.createElement("p", {   "class":`dim`,"style":`font-size: var(--fs-s); padding: 16px 0`,});
                 { svelteHTML.createElement("strong", {});selectedBase || 'base'; }
       }
    }else{
      if(draftedBy){
         { svelteHTML.createElement("p", { "class":`lrp-attrib dim`,});
           { const $$_nocIredivorP2C = __sveltets_2_ensureComponent(ProviderIcon); new $$_nocIredivorP2C({ target: __sveltets_2_any(), props: {    "provider":draftedBy.provider,"size":12,}});}
           { svelteHTML.createElement("span", {});
                draftedBy.provider;draftedBy.model ? ` · ${draftedBy.model}` : ''; draftedBy.lenses.length > 0 ? ` from ${draftedBy.lenses.join(', ')}` : '';        
           }
         }
      }
       { svelteHTML.createElement("div", { "class":`lrp-findings-header`,});
         { svelteHTML.createElement("span", { "class":`lrp-findings-count`,});plural(review.comments.length, 'finding'); }
         { svelteHTML.createElement("div", { "class":`lrp-sel-btns`,});
           { svelteHTML.createElement("button", {     "class":`btn small ghost`,"onclick":selectAll,"disabled":allSelected,});  }
           { svelteHTML.createElement("button", {     "class":`btn small ghost`,"onclick":selectNone,"disabled":noneSelected,});  }
         }
       }

       { svelteHTML.createElement("div", { "class":`lrp-list`,});
           for(let c of __sveltets_2_ensureArray(review.comments)){c.id;
           { svelteHTML.createElement("div", { "class":`lrp-comment card`,});
             { svelteHTML.createElement("input", {             "id":`lrp-chk-${c.id}`,"type":`checkbox`,"class":`lrp-chk`,"aria-label":`Select ${c.severity} finding: ${c.body}`,"checked":!!checked[c.id],"onchange":(e) => { checked = { ...checked, [c.id]: (e.target as HTMLInputElement).checked }; },});}
             { svelteHTML.createElement("div", { "class":`lrp-comment-body`,});
               { svelteHTML.createElement("div", { "class":`lrp-comment-head`,});
                 { svelteHTML.createElement("span", { "class":`severity-chip sev-${c.severity}`,});c.severity; }
                if(c.path !== null){
                   { svelteHTML.createElement("span", { "class":`mono lrp-loc`,});c.path;c.line !== null ? `:${c.line}` : ''; }
                }
               }
               { svelteHTML.createElement("p", { "class":`lrp-comment-text`,});c.body; }
             }
           }
        }
       }

      
       { svelteHTML.createElement("div", { "class":`lrp-handoff-bar`,});
         { svelteHTML.createElement("span", {   "class":`dim`,"style":`font-size: var(--fs-s)`,});checkedIds.length;  review.comments.length;  }
         { svelteHTML.createElement("button", {       "class":`btn primary`,"disabled":checkedIds.length === 0,"onclick":openHandoffMenu,});
             checkedIds.length;  { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`chevronDown`,"size":14,}});}
         }
       }
    }
  }

  
  if(pastRuns.length > 0){
    const headerOpen = !!historyExpanded['_header' as unknown as number];
     { svelteHTML.createElement("div", { "class":`lrp-history`,});
       { svelteHTML.createElement("button", {       "class":`lrp-history-toggle`,"onclick":() => { historyExpanded = { ...historyExpanded, ['_header' as unknown as number]: !headerOpen }; },"aria-expanded":headerOpen,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":headerOpen ? 'chevronDown' : 'chevronRight',"size":12,}});}   pastRuns.length;
       }
      if(headerOpen){
         { svelteHTML.createElement("div", { "class":`lrp-history-list`,});
              for(let run of __sveltets_2_ensureArray(pastRuns)){let i = 1;run.id;
            const isOpen = !!historyExpanded[i];
             { svelteHTML.createElement("div", { "class":`lrp-history-run card`,});
               { svelteHTML.createElement("button", {       "class":`lrp-history-run-header`,"onclick":() => toggleHistoryRun(i),"aria-expanded":isOpen,});
                 { svelteHTML.createElement("span", {   "class":`dim`,"style":`font-size: var(--fs-xs)`,});timeAgo(run.created_at); }
                 { const $$_egdaBsutatS5C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS5C({ target: __sveltets_2_any(), props: {  "status":runStatus(run.status),}});}
                if(run.agents && run.agents.length > 0){
                   { svelteHTML.createElement("span", {   "class":`dim`,"style":`font-size:var(--fs-xs)`,});run.agents.filter(a => a.status === 'done').length; run.agents.length;  }
                }
                 { svelteHTML.createElement("span", {   "class":`dim`,"style":`font-size:var(--fs-xs)`,});plural(run.comments.length, 'finding'); }
                 { svelteHTML.createElement("span", { "class":`grow`,}); }
                 { svelteHTML.createElement("span", {   "class":`dim`,"aria-hidden":`true`,}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":isOpen ? 'chevronDown' : 'chevronRight',"size":12,}});} }
               }
              if(isOpen){
                 { svelteHTML.createElement("div", { "class":`lrp-history-run-body`,});
                  if(run.comments.length === 0){
                     { svelteHTML.createElement("p", {   "class":`dim`,"style":`font-size: var(--fs-xs);padding:4px 0`,});     }
                  }else{
                       for(let c of __sveltets_2_ensureArray(run.comments)){c.id;
                       { svelteHTML.createElement("div", {   "class":`lrp-comment card lrp-history-comment`,"style":`cursor:default`,});
                         { svelteHTML.createElement("div", { "class":`lrp-comment-body`,});
                           { svelteHTML.createElement("div", { "class":`lrp-comment-head`,});
                             { svelteHTML.createElement("span", { "class":`severity-chip sev-${c.severity}`,});c.severity; }
                            if(c.path !== null){
                               { svelteHTML.createElement("span", { "class":`mono lrp-loc`,});c.path;c.line !== null ? `:${c.line}` : ''; }
                            }
                           }
                           { svelteHTML.createElement("p", { "class":`lrp-comment-text`,});c.body; }
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
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const LocalReviewPanel__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type LocalReviewPanel__SvelteComponent_ = ReturnType<typeof LocalReviewPanel__SvelteComponent_>;
/*Ωignore_endΩ*/export default LocalReviewPanel__SvelteComponent_;