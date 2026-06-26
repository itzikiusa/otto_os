<script lang="ts">
  // Local working-tree review panel: diff against a chosen base branch, run
  // the configured review agents, show findings with checkboxes, and hand
  // selected findings off to a new agent session.
  import { api, ApiError } from '../../lib/api/client';
  import type { Review, ReviewComment, RefsResp, Session } from '../../lib/api/types';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { router } from '../../lib/router.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import ReviewAgents from './ReviewAgents.svelte';
  import FindingsBoard from './FindingsBoard.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';

  interface Props {
    repoId: string;
  }
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
  let pollTimer: ReturnType<typeof setTimeout> | null = null;
  let pollCount = $state(0);
  const MAX_POLLS = 90; // 3 min at 2 s each
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

  const PROVIDER_OPTIONS = ['claude', 'codex', 'agy'];

  // ---------------------------------------------------------------------------
  // Load refs once on mount, and try to load an existing local review
  // ---------------------------------------------------------------------------
  $effect(() => {
    void loadRefs(repoId);
    void loadExisting(repoId);
    return () => {
      if (pollTimer !== null) clearTimeout(pollTimer);
    };
  });

  async function loadRefs(rid: string): Promise<void> {
    refsLoading = true;
    try {
      refs = await api.get<RefsResp>(`/repos/${rid}/refs`);
      // Pick a sensible default base.
      const remotes = refs?.remote ?? [];
      const locals = refs?.local ?? [];
      const preferred = [
        'origin/develop',
        'origin/main',
        'origin/master',
        'develop',
        'main',
        'master',
      ];
      const allNames = [...remotes.map((r) => r.name), ...locals.map((l) => l.name)];
      const hit = preferred.find((p) => allNames.includes(p));
      selectedBase = hit ?? remotes[0]?.name ?? locals[0]?.name ?? '';
    } catch {
      // non-blocking
    } finally {
      refsLoading = false;
    }
  }

  async function loadExisting(rid: string): Promise<void> {
    reviewLoading = true;
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
        toasts.error('Could not load local review', e instanceof Error ? e.message : String(e));
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
    if (pollTimer !== null) clearTimeout(pollTimer);
    pollTimer = setTimeout(() => void poll(), 2000);
  }

  // A child <ReviewAgents> retried one agent: adopt the refreshed review and
  // resume polling so we keep tracking the re-run.
  function onAgentRetried(r: Review): void {
    review = r;
    history = history.length > 0 ? [r, ...history.slice(1)] : [r];
    pollCount = 0;
    schedulePoll();
  }

  async function poll(): Promise<void> {
    pollCount++;
    if (pollCount > MAX_POLLS) {
      toasts.warn('Review is taking too long', 'Try refreshing manually.');
      return;
    }
    try {
      const r = await api.get<Review>(`/repos/${repoId}/local-review`);
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

  async function startReview(): Promise<void> {
    if (!selectedBase) {
      toasts.warn('Select a base branch first');
      return;
    }
    if (pollTimer !== null) clearTimeout(pollTimer);
    starting = true;
    pollCount = 0;
    checked = {};
    try {
      const newRun = await api.post<Review>(`/repos/${repoId}/local-review`, { base: selectedBase });
      // Prepend new run; keep existing runs as history
      review = newRun;
      history = [newRun, ...history];
      if (review.status === 'running') schedulePoll();
      if (review.status === 'done') initChecked(review.comments);
    } catch (e) {
      toasts.error('Could not start review', e instanceof Error ? e.message : String(e));
    } finally {
      starting = false;
    }
  }

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
      toasts.success(`Handed ${checkedIds.length} finding${checkedIds.length === 1 ? '' : 's'} to ${provider}`);
    } catch (e) {
      toasts.error('Handoff failed', e instanceof Error ? e.message : String(e));
    }
  }

  // ---------------------------------------------------------------------------
  // History helpers
  // ---------------------------------------------------------------------------

  function timeAgo(iso: string): string {
    const diff = Math.floor((Date.now() - new Date(iso).getTime()) / 1000);
    if (diff < 60) return `${diff}s ago`;
    if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
    if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
    return `${Math.floor(diff / 86400)}d ago`;
  }

  function toggleHistoryRun(idx: number): void {
    historyExpanded = { ...historyExpanded, [idx]: !historyExpanded[idx] };
  }

  // ---------------------------------------------------------------------------
  // Branch list for the <select>
  // ---------------------------------------------------------------------------

  const allBranches = $derived.by(() => {
    if (!refs) return [] as string[];
    return [
      ...refs.remote.map((r) => r.name),
      ...refs.local.map((l) => l.name),
    ];
  });
</script>

<div class="lrp">
  <!-- Base selector + run button -->
  <div class="lrp-toolbar">
    <label class="lrp-label" for="lrp-base">Compare to</label>
    {#if refsLoading}
      <div class="lrp-select-skeleton"></div>
    {:else}
      <select id="lrp-base" class="lrp-select" bind:value={selectedBase} disabled={starting}>
        {#each allBranches as b (b)}
          <option value={b}>{b}</option>
        {/each}
        {#if allBranches.length === 0}
          <option value="">No branches found</option>
        {/if}
      </select>
    {/if}
    <button
      class="btn primary small"
      disabled={starting || !selectedBase || refsLoading}
      onclick={startReview}
    >
      {#if starting}
        <span class="spinner-xs"></span>Starting…
      {:else}
        <Icon name="zap" size={11} />
        Review changes
      {/if}
    </button>
    <span class="lrp-cfg-note dim">
      Uses the same agents as
      <a href="#/settings/pr-review" onclick={(e) => { e.preventDefault(); router.go('settings/pr-review'); }}>PR review</a>
    </span>
  </div>

  <!-- Body -->
  {#if reviewLoading}
    <div style="padding: 16px"><Skeleton rows={4} height={36} /></div>
  {:else if !review}
    <EmptyState
      icon="zap"
      title="No active review"
      body={pastRuns.length > 0
        ? "Click 'Review changes' to review your current changes. Earlier runs are kept under 'Past reviews' below."
        : "Select a base branch and click 'Review changes' to run AI agents on your current uncommitted work."}
    />
  {:else if review.status === 'running'}
    <div class="lrp-running-header">
      <div class="spinner"></div>
      <span class="lrp-running-title">Reviewing…</span>
    </div>
    {#if review.agents && review.agents.length > 0}
      <ReviewAgents {review} view="running" onretried={onAgentRetried} />
    {:else}
      <p class="dim" style="font-size:12px;padding:8px 0">Agents starting…</p>
    {/if}
  {:else if review.status === 'error'}
    <div class="lrp-error card">
      <Icon name="zap" size={14} />
      <span class="lrp-error-msg">{review.error ?? 'An unknown error occurred.'}</span>
      <button class="btn small" disabled={starting} onclick={startReview}>
        {starting ? 'Starting…' : 'Try again'}
      </button>
    </div>
  {:else}
    <!-- done -->
    <!-- Findings workflow board: persisted Finding rows with the 6-state status,
         the 7 triage actions, and the Proof Pack. -->
    {#if ws.currentId}
      <div class="lrp-findings-section">
        <h3 class="lrp-findings-title">Findings</h3>
        <FindingsBoard reviewId={review.id} workspaceId={ws.currentId} />
      </div>
    {/if}

    <!-- Per-agent breakdown: open each agent's session + its own findings,
         shared with the PR review (same Open / Retry). -->
    {#if review.agents.length > 1}
      <ReviewAgents {review} view="done" onretried={onAgentRetried} />
    {/if}
    {#if review.comments.length === 0}
      <p class="dim" style="font-size: 12.5px; padding: 16px 0">
        No findings — your changes look clean vs <strong>{selectedBase || 'base'}</strong>.
      </p>
    {:else}
      <div class="lrp-findings-header">
        <span class="lrp-findings-count">{review.comments.length} finding{review.comments.length === 1 ? '' : 's'}</span>
        <div class="lrp-sel-btns">
          <button class="btn small ghost" onclick={selectAll} disabled={allSelected}>Select all</button>
          <button class="btn small ghost" onclick={selectNone} disabled={noneSelected}>None</button>
        </div>
        <span class="grow"></span>
        <button
          class="btn small primary"
          disabled={checkedIds.length === 0}
          onclick={openHandoffMenu}
        >
          Send to agent ({checkedIds.length}) ▾
        </button>
      </div>

      <div class="lrp-list">
        {#each review.comments as c (c.id)}
          <label class="lrp-comment card" for="lrp-chk-{c.id}">
            <input
              id="lrp-chk-{c.id}"
              type="checkbox"
              class="lrp-chk"
              checked={!!checked[c.id]}
              onchange={(e) => { checked = { ...checked, [c.id]: (e.target as HTMLInputElement).checked }; }}
            />
            <div class="lrp-comment-body">
              <div class="lrp-comment-head">
                <span class="severity-chip sev-{c.severity}">{c.severity}</span>
                {#if c.path !== null}
                  <span class="mono lrp-loc">{c.path}{c.line !== null ? `:${c.line}` : ''}</span>
                {/if}
              </div>
              <p class="lrp-comment-text">{c.body}</p>
            </div>
          </label>
        {/each}
      </div>

      <!-- Bottom handoff bar -->
      <div class="lrp-handoff-bar">
        <span class="dim" style="font-size:12px">{checkedIds.length} of {review.comments.length} selected</span>
        <button
          class="btn primary"
          disabled={checkedIds.length === 0}
          onclick={openHandoffMenu}
        >
          Send to agent ({checkedIds.length}) ▾
        </button>
      </div>
    {/if}
  {/if}

  <!-- Past local reviews history: all stored runs except the active one -->
  {#if pastRuns.length > 0}
    {@const headerOpen = !!historyExpanded['_header' as unknown as number]}
    <div class="lrp-history">
      <button
        class="lrp-history-toggle"
        onclick={() => { historyExpanded = { ...historyExpanded, ['_header' as unknown as number]: !headerOpen }; }}
        aria-expanded={headerOpen}
      >
        Past reviews ({pastRuns.length}){headerOpen ? ' ▾' : ' ▸'}
      </button>
      {#if headerOpen}
        <div class="lrp-history-list">
          {#each pastRuns as run, i (run.id)}
            {@const isOpen = !!historyExpanded[i]}
            <div class="lrp-history-run card">
              <button
                class="lrp-history-run-header"
                onclick={() => toggleHistoryRun(i)}
                aria-expanded={isOpen}
              >
                <span class="dim" style="font-size:11px">{timeAgo(run.created_at)}</span>
                <span class="chip lrp-status-{run.status}" style="font-size:10px;padding:1px 5px">{run.status}</span>
                {#if run.agents && run.agents.length > 0}
                  <span class="dim" style="font-size:10.5px">{run.agents.filter(a => a.status === 'done').length}/{run.agents.length} agents</span>
                {/if}
                <span class="dim" style="font-size:10.5px">{run.comments.length} finding{run.comments.length === 1 ? '' : 's'}</span>
                <span class="grow"></span>
                <span class="dim" style="font-size:10px">{isOpen ? '▾' : '▸'}</span>
              </button>
              {#if isOpen}
                <div class="lrp-history-run-body">
                  {#if run.comments.length === 0}
                    <p class="dim" style="font-size:11.5px;padding:4px 0">No findings for this run.</p>
                  {:else}
                    {#each run.comments as c (c.id)}
                      <div class="lrp-comment card lrp-history-comment" style="cursor:default">
                        <div class="lrp-comment-body">
                          <div class="lrp-comment-head">
                            <span class="severity-chip sev-{c.severity}">{c.severity}</span>
                            {#if c.path !== null}
                              <span class="mono lrp-loc">{c.path}{c.line !== null ? `:${c.line}` : ''}</span>
                            {/if}
                          </div>
                          <p class="lrp-comment-text">{c.body}</p>
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

<style>
  .lrp {
    padding: 12px 0 32px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  /* Findings workflow board section */
  .lrp-findings-section {
    margin: 4px 0 8px;
    padding-top: 10px;
    border-top: 1px solid var(--border);
  }
  .lrp-findings-title {
    font-size: 12.5px;
    font-weight: 600;
    margin: 0 0 4px;
  }

  /* Toolbar */
  .lrp-toolbar {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    padding-bottom: 8px;
    border-bottom: 1px solid var(--border);
    margin-bottom: 4px;
  }
  .lrp-label {
    font-size: 12px;
    color: var(--text-dim);
    white-space: nowrap;
  }
  .lrp-select {
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-s, 4px);
    color: var(--text);
    font-size: 12.5px;
    padding: 4px 8px;
    min-width: 180px;
    max-width: 260px;
  }
  .lrp-select-skeleton {
    width: 200px;
    height: 28px;
    background: var(--surface-2);
    border-radius: var(--radius-s);
    animation: pulse 1.4s ease-in-out infinite;
  }
  .lrp-cfg-note {
    font-size: 11px;
    margin-inline-start: auto;
  }
  .lrp-cfg-note a {
    color: var(--accent);
    text-decoration: none;
  }
  .lrp-cfg-note a:hover {
    text-decoration: underline;
  }

  /* Running */
  .lrp-running-header {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 12px 0 8px;
  }
  .lrp-running-title {
    font-size: 13px;
    font-weight: 600;
  }
  .spinner {
    width: 18px;
    height: 18px;
    border: 2.5px solid var(--border);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
    flex-shrink: 0;
  }
  .spinner-xs {
    display: inline-block;
    width: 9px;
    height: 9px;
    border: 1.5px solid currentColor;
    border-top-color: transparent;
    border-radius: 50%;
    animation: spin 0.7s linear infinite;
    vertical-align: middle;
    margin-inline-end: 3px;
  }
  @keyframes spin { to { transform: rotate(360deg); } }
  @keyframes pulse { 0%, 100% { opacity: 1; } 50% { opacity: 0.5; } }

  /* Agent cards */
  .lrp-agents {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-top: 4px;
  }
  .lrp-agent {
    padding: 8px 12px;
  }
  .lrp-agent-top {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .lrp-agent-name {
    font-size: 12.5px;
    font-weight: 600;
  }
  .lrp-agent-chip {
    font-size: 10.5px;
  }
  .lrp-agent-note {
    margin: 4px 0 0;
    font-size: 11.5px;
    color: var(--text-dim);
    line-height: 1.4;
  }
  .lrp-status-pill {
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    padding: 2px 6px;
    border-radius: var(--radius-s, 4px);
    display: inline-flex;
    align-items: center;
    gap: 3px;
  }
  .lrp-status-pending {
    background: color-mix(in srgb, var(--text-dim) 12%, transparent);
    color: var(--text-dim);
  }
  .lrp-status-running {
    background: color-mix(in srgb, var(--accent) 15%, transparent);
    color: var(--accent);
  }
  .lrp-status-done {
    background: color-mix(in srgb, var(--status-working) 15%, transparent);
    color: var(--status-working);
  }
  .lrp-status-error {
    background: color-mix(in srgb, var(--status-exited) 15%, transparent);
    color: var(--status-exited);
  }

  /* Error */
  .lrp-error {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 12px 14px;
    color: var(--status-exited);
    margin-top: 8px;
  }
  .lrp-error-msg {
    flex: 1;
    min-width: 0;
    overflow-wrap: anywhere;
    font-size: 12.5px;
  }

  /* Findings */
  .lrp-findings-header {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 8px;
    flex-wrap: wrap;
  }
  .lrp-findings-count {
    font-size: 12.5px;
    font-weight: 600;
  }
  .lrp-sel-btns {
    display: flex;
    gap: 4px;
  }
  .lrp-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  /* Comment card with checkbox */
  .lrp-comment {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 10px 14px;
    cursor: pointer;
    user-select: none;
  }
  .lrp-comment:hover {
    background: var(--surface-2);
  }
  .lrp-chk {
    margin-top: 2px;
    flex-shrink: 0;
    cursor: pointer;
  }
  .lrp-comment-body {
    flex: 1;
    min-width: 0;
  }
  .lrp-comment-head {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 4px;
    flex-wrap: wrap;
  }
  .lrp-comment-text {
    margin: 0;
    font-size: 12.5px;
    line-height: 1.55;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .lrp-loc {
    font-size: 11px;
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 280px;
  }

  /* Handoff bar at bottom */
  .lrp-handoff-bar {
    display: flex;
    align-items: center;
    gap: 12px;
    padding-top: 12px;
    margin-top: 8px;
    border-top: 1px solid var(--border);
  }

  /* Severity chips */
  .severity-chip {
    display: inline-block;
    padding: 2px 7px;
    border-radius: var(--radius-s, 4px);
    font-size: 10.5px;
    font-weight: 700;
    letter-spacing: 0.04em;
    text-transform: uppercase;
  }
  .sev-info {
    background: color-mix(in srgb, var(--accent) 15%, transparent);
    color: var(--accent);
  }
  .sev-warn {
    background: color-mix(in srgb, var(--status-warn) 15%, transparent);
    color: var(--status-warn);
  }
  .sev-bug {
    background: color-mix(in srgb, var(--status-exited) 15%, transparent);
    color: var(--status-exited);
  }

  .grow { flex: 1; }
  .dim { color: var(--text-dim); }
  .mono { font-family: var(--font-mono, monospace); }

  /* History section */
  .lrp-history {
    margin-top: 16px;
    border-top: 1px solid var(--border);
    padding-top: 10px;
  }
  .lrp-history-toggle {
    background: none;
    border: none;
    cursor: pointer;
    font-size: 11.5px;
    font-weight: 600;
    color: var(--text-dim);
    padding: 0;
    line-height: 1.4;
  }
  .lrp-history-toggle:hover {
    color: var(--text);
  }
  .lrp-history-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-top: 8px;
  }
  .lrp-history-run {
    padding: 0;
    overflow: hidden;
  }
  .lrp-history-run-header {
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
  .lrp-history-run-header:hover {
    background: var(--surface-2);
  }
  .lrp-history-run-body {
    padding: 4px 8px 8px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .lrp-history-comment {
    opacity: 0.85;
    user-select: text;
  }

  /* ── Mobile + tablet (≤1024px) ──────────────────────────────────────────────
     The panel is a single vertical flow inside RepoView's own scroll container,
     so the main risks are dense button rows overflowing and small touch targets.
     The toolbar + findings header already wrap; here we give the base <select>
     and the action buttons real width/height on a phone, and let the "Compare
     to" note drop to its own line instead of being pinned to the far edge. */
  @media (max-width: 1024px) {
    .lrp-select { min-width: 0; flex: 1 1 160px; max-width: 100%; }
    .lrp-handoff-bar { flex-wrap: wrap; }
    /* Touch targets across the panel's action rows — incl. the Select all/None
       findings-header buttons (a real tablet/landscape concern, not phone-only). */
    .lrp-toolbar .btn,
    .lrp-findings-header .btn,
    .lrp-handoff-bar .btn { min-height: 36px; }
    /* The "Past reviews" disclosure is a zero-padding text button — give it real
       tap height without changing its desktop look. */
    .lrp-history-toggle { padding: 6px 0; min-height: 36px; }
  }
  @media (max-width: 640px) {
    .lrp-cfg-note { margin-inline-start: 0; flex-basis: 100%; }
    .lrp-toolbar .btn,
    .lrp-findings-header .btn,
    .lrp-handoff-bar .btn { min-height: 38px; }
    /* Stretch the primary handoff actions so they're easy to tap. */
    .lrp-handoff-bar { gap: 8px; }
    .lrp-handoff-bar .btn.primary { flex: 1 1 auto; }
    .lrp-loc { max-width: 100%; }
    .lrp-chk { width: 18px; height: 18px; }
  }
</style>
