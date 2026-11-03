<script lang="ts">
  // Run-scoped agent view (embedded in the WF page's right sidebar "Agents" tab).
  // Shows ONLY the currently-selected run's sessions — every session each step
  // spawned, grouped by step, in run order. This includes a review_run step's
  // reviewer sessions AND its summarizer (the backend now surfaces them into
  // `nodes[*].sessions`), so a running review has full inline visibility.
  //
  // Each session expands to a live <Terminal> attached in place — no navigation
  // to the global Agents panel. Nothing here is a general/all-workflows list.
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import LazyTerminal from '../../lib/components/LazyTerminal.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import type { WorkflowRun, Review, ReviewAgentState, Session } from '../../lib/api/types';
  import { api, ApiError } from '../../lib/api/client';
  import { untrack } from 'svelte';
  import { runStatus, sessionState, sentenceCase, type SessionStateInfo } from '../../lib/status';
  import StatusBadge from '../../lib/components/StatusBadge.svelte';
  import StatusDot from '../../lib/components/StatusDot.svelte';
  import { events } from '../../lib/events.svelte';
  import { reviewIds, reviewSessions, reviewAgentStatus } from './reviewAgents';
  import { runReviews } from './runReviews.svelte';
  import { repoProgress, stepProgress } from './stepProgress';
  import { git } from '../../lib/stores/git.svelte';
  import { summaryFallback, summaryFallbackTitle } from '../git/reviewFallback';
  import { plural } from '../../lib/plural';

  interface Props {
    run: WorkflowRun;
    /** Resolve a node id to a friendly label. */
    nodeName?: (id: string) => string;
    /** A session id to auto-expand + scroll to (set when opened from a step). */
    focusSid?: string | null;
  }
  let { run, nodeName = (id) => id, focusSid = null }: Props = $props();

  // Steps that spawned sessions OR are running sub-agents, in run order. An
  // agent step reports its sub-agents (from the parent transcript) before its
  // own session id lands, so an activity-only step is still a group — the
  // sub-agent rows must not wait for the session.
  let sessionDetails = $state<Record<string, Session>>({});
  // A STRING key: the id array was rebuilt on every run merge (a WS tick), so
  // the effect below restarted — and refetched every review — per merge
  // (backlog B6 / SD-20).
  const relatedKey = $derived([...new Set((run.nodes ?? []).flatMap(reviewIds))].join(','));
  const relatedIds = $derived(relatedKey ? relatedKey.split(',') : []);
  // Reviews outlive await:false steps and may be retried after the workflow is
  // complete: keep their association fresh while this panel is open (shared
  // with the Steps list — one poller per id set, see runReviews).
  $effect(() => runReviews.watch(relatedKey ? relatedKey.split(',') : []));
  const reviews = $derived(runReviews.reviews);
  const groups = $derived(
    (run.nodes ?? [])
      .map((n) => ({
        id: n.node_id,
        status: n.status,
        attempts: n.attempts ?? 0,
        recovery: (n.logs ?? []).filter((line) => line.includes('↻ retry') || line.startsWith('🔁 iteration') || line.includes(' — attempt ')).at(-1),
        sessions: reviewSessions(n, reviews),
        reviews: reviewIds(n).flatMap((id) => reviews[id] ? [reviews[id]] : []),
        // Sessions no review row owns (an agent step, the goals grader…).
        loose: (n.sessions ?? []).filter((sid) => !reviewIds(n).some((id) => reviews[id]?.agents.some((a) => a.session_id === sid))),
        activity: n.activity ?? null,
      }))
      .filter((g) => g.sessions.length > 0 || g.reviews.length > 0 || (g.activity?.subagents.length ?? 0) > 0),
  );
  // Suspended sessions may be absent from the workspace's live list. Load
  // metadata once for titles/provider and an elapsed clock that survives reload.
  // Keyed on the joined id set, not on `groups` (rebuilt on every live merge —
  // up to once per log line): each id is fetched at most once per mount, and a
  // 404 (deleted session) is remembered instead of re-requested forever.
  const sessionIdsKey = $derived([...new Set(groups.flatMap((g) => g.sessions))].sort().join('\n'));
  const attemptedSessions = new Set<string>();
  let mounted = true;
  $effect(() => () => { mounted = false; });
  $effect(() => {
    const ids = sessionIdsKey ? sessionIdsKey.split('\n') : [];
    const todo = untrack(() => ids.filter((id) => !attemptedSessions.has(id) && !sessOf(id)));
    for (const id of todo) attemptedSessions.add(id);
    void Promise.all(todo.map(async (id) => {
      try {
        const session = await api.get<Session>(`/sessions/${id}`);
        if (mounted) sessionDetails[id] = session;
      } catch (e) {
        // Only a 404 (a deleted session — its review label still shows) is
        // remembered. A timeout or a restarting daemon is forgotten, so the
        // next id-set change looks it up again instead of leaving the row
        // without title, provider or clock for the rest of the mount.
        if (!(e instanceof ApiError && e.status === 404)) attemptedSessions.delete(id);
      }
    }));
  });
  const runActive = $derived(run.status === 'running' || run.status === 'pending');

  // Live "running 3m40s" on sub-agent rows: the same 1s client-side ticker
  // RunSteps uses, alive only while a step actually runs (no network).
  let now = $state(Date.now());
  $effect(() => {
    if (!(run.nodes ?? []).some((n) => n.status === 'running') && !Object.values(reviews).some((r) => r.status === 'running')) return;
    const iv = setInterval(() => (now = Date.now()), 1000);
    return () => clearInterval(iv);
  });
  function fmtDur(ms: number): string {
    const s = Math.max(0, Math.round(ms / 1000));
    return s < 60 ? `${s}s` : `${Math.floor(s / 60)}m${s % 60}s`;
  }
  function stamp(v?: string | null): number | null {
    if (!v) return null;
    const t = new Date(v).getTime();
    return Number.isFinite(t) ? t : null;
  }
  /** Elapsed since a sub-agent was launched (ticks every second). */
  function fmtSince(from?: string | null): string {
    const t = stamp(from);
    return t == null ? '' : fmtDur(now - t);
  }
  /** How long a finished sub-agent took (both stamps come from the parent). */
  function fmtBetween(from?: string | null, to?: string | null): string {
    const a = stamp(from);
    const b = stamp(to);
    return a == null || b == null ? '' : fmtDur(b - a);
  }

  // Which session terminals are mounted (collapsed by default so we don't attach
  // dozens of PTYs at once). id-keyed, reset when the viewed run changes.
  let expanded = $state<Record<string, boolean>>({});
  let expandedRunId: string | null = null;
  $effect(() => {
    if (run.id !== expandedRunId) {
      expandedRunId = run.id;
      expanded = {};
      lastFocus = null;
    }
  });
  function toggle(sid: string): void {
    expanded[sid] = !expanded[sid];
  }

  // Auto-expand + reveal a session opened from a step's "Open session" button.
  let lastFocus: string | null = null;
  $effect(() => {
    const f = focusSid;
    if (f && f !== lastFocus) {
      lastFocus = f;
      expanded[f] = true;
      queueMicrotask(() =>
        document.querySelector(`[data-sess="${f}"]`)?.scrollIntoView({ block: 'nearest' }),
      );
    }
  });

  function sessOf(sid: string) {
    return ws.getSession(sid) ?? sessionDetails[sid] ?? null;
  }
  function reviewAgent(sid: string) {
    for (const id of relatedIds) {
      const review = reviews[id];
      const agent = review?.agents.find((a) => a.session_id === sid);
      if (agent) return { review, agent, summarizer: review.agents.at(-1) === agent };
    }
    return null;
  }
  function sTitle(sid: string): string {
    const row = reviewAgent(sid);
    if (row) return row.summarizer ? 'Summarizer' : row.agent.name;
    return sessOf(sid)?.title || 'Session';
  }
  function sProvider(sid: string): string {
    return reviewAgent(sid)?.agent.provider || sessOf(sid)?.provider || '';
  }
  /** The one dot + word for a session row. Review agents report their own
   *  vocabulary (pending / running / waiting / done / error / skipped, plus
   *  cancelled / fallback) — mapped onto the shared session states so the raw
   *  engine word never reaches the screen. */
  function sState(sid: string): SessionStateInfo {
    const row = reviewAgent(sid);
    if (row) return agentState(row.review, row.agent);
    const st = sessionState(sessOf(sid), ws.statusMap[sid] ?? sessOf(sid)?.status ?? 'idle', ws.needsYou[sid] === true, { stale: events.state !== 'connected' });
    // A finished step's session is suspended to free its PTY — it did its
    // work. Say "Done" like a review agent, and keep why in the hint.
    const group = groups.find((g) => g.sessions.includes(sid));
    if (st.key === 'suspended' && group && !['running', 'pending'].includes(group.status)) {
      return { ...st, key: 'ended', label: 'Done', tone: 'success', hint: 'Finished — its terminal is suspended to save memory; open it to view or resume' };
    }
    return st;
  }
  /** The one dot + word for a review agent row. Review agents report their own
   *  vocabulary (pending / running / waiting / done / error / skipped, plus
   *  cancelled / fallback) — mapped onto the shared session states so the raw
   *  engine word never reaches the screen. */
  function agentState(review: Review, agent: ReviewAgentState): SessionStateInfo {
    const stale = events.state !== 'connected';
    const raw = reviewAgentStatus(review, agent);
    const base = { resumable: false, inactive: true };
    switch (raw) {
      case 'running':
        return sessionState(null, 'working', false, { stale });
      case 'waiting':
        return sessionState(null, 'idle', true, { stale });
      case 'pending': {
        const summarizer = review.agents.at(-1) === agent;
        return { ...sessionState(null, 'idle', false, { stale }), label: 'Queued', hint: summarizer ? 'Starts when every reviewer has finished' : 'Waiting for a free reviewer slot — slots are shared by every review on this Mac' };
      }
      case 'done':
        return { ...base, key: 'ended', label: 'Done', tone: 'success', hint: 'Finished' };
      case 'skipped':
        return { ...base, key: 'ended', label: 'Skipped', tone: 'neutral', hint: 'Not needed — another provider already covered this lens' };
      case 'cancelled':
        return { ...base, key: 'ended', label: 'Canceled', tone: 'neutral', hint: 'The review was canceled' };
      case 'fallback':
        // Names the summarizer that actually ran and why its output wasn't
        // used (error-handling-5-05), not a hard-coded "claude unavailable".
        return { ...base, key: 'failed', label: 'Fallback', tone: 'warning', hint: summaryFallbackTitle(summaryFallback(review.agents), 'the findings') };
      case 'error':
        return { ...base, key: 'failed', label: 'Failed', tone: 'danger', hint: 'The agent failed' };
      default:
        return { ...base, key: 'ended', label: sentenceCase(raw), tone: 'neutral' };
    }
  }
  function repoLabel(review: Review, i: number): string {
    const r = git.allRepos.find((x) => x.id === review.repo_id) ?? git.repos.find((x) => x.id === review.repo_id);
    return r?.name ?? `Repo ${i + 1}`;
  }
  const LEGEND = 'Queued — waiting to start (a free reviewer slot, or the summarizer waiting for the reviewers)\nWorking — running now\nNeeds you — waiting on your input\nDone — finished its work; its terminal is suspended to save memory and resumes when opened\nSuspended — a session of a step still in progress, paused until opened\nSkipped — another provider already covered that lens\nFailed — the agent errored';
  function shortId(id: string): string {
    return id.length > 6 ? id.slice(-6) : id;
  }
</script>

{#snippet sessRow(sid: string)}
  <div class="sess" data-sess={sid}>
    <button class="sess-h" onclick={() => toggle(sid)} aria-expanded={!!expanded[sid]} title={expanded[sid] ? 'Hide live terminal' : 'Show live terminal'}>
      <Icon name={expanded[sid] ? 'chevronDown' : 'chevronRight'} size={12} />
      <StatusDot state={sState(sid)} size={8} />
      <span class="s-title">{sTitle(sid)}</span>
      <span class="s-provider">{sProvider(sid)}</span>
      <span class="s-status" title={sState(sid).hint}>{sState(sid).label}{#if ['working', 'running', 'needs-you'].includes(sState(sid).key)}{' ' + fmtSince(sessOf(sid)?.created_at)}{/if}</span>
      <span class="grow"></span>
      <code class="s-id" title={sid}>{shortId(sid)}</code>
    </button>
    {#if reviewAgent(sid)?.agent.fallback}
      {@const fb = summaryFallback(reviewAgent(sid)?.review.agents)}
      <div class="fallback" data-testid="summarizer-fallback" title={summaryFallbackTitle(fb, 'the findings')}>
        Deterministic fallback — the {fb.provider} summarizer’s output wasn’t used{fb.cause ? ` (${fb.cause})` : ''}.
      </div>
    {/if}
    {#if expanded[sid]}
      <div class="term">
        {#key sid}
          <LazyTerminal sessionId={sid} resumable preferDom resumeOnOpen={false} />
        {/key}
      </div>
    {/if}
  </div>
{/snippet}

{#if groups.length === 0}
  <EmptyState
    icon="terminal"
    title="No agent sessions for this run yet"
    body={runActive ? 'They’ll appear here as steps spawn them.' : undefined}
  />
{:else}
  <div class="agents" data-testid="run-agents">
    <div class="legend-row">
      <span class="grow"></span>
      <span class="legend" role="img" aria-label={LEGEND} title={LEGEND} data-testid="agent-state-legend"><Icon name="info" size={12} /> What the states mean</span>
    </div>
    {#each groups as g (g.id)}
      <div class="grp">
        <div class="grp-h">
          <!-- Step status in the shared run vocabulary (Succeeded / Failed /
               Queued…), like the Steps list and timeline — not the raw
               engine word ("success", "error", "pending"). -->
          <span class="grp-name" title={nodeName(g.id)}>{nodeName(g.id)}</span>
          <span class="grp-status"><StatusBadge status={runStatus(g.status)} variant="text" /></span>
          {#if g.attempts > 1}<span class="grp-status">Attempt {g.attempts}</span>{/if}
          <span class="grow"></span>
          <span class="grp-count" role="img" title={plural(g.sessions.length, 'session')} aria-label={plural(g.sessions.length, 'session')}>{g.sessions.length}</span>
        </div>
        {#if g.recovery}<div class="sub recovery-status" data-testid="workflow-recovery-status">{g.recovery}</div>{/if}
        {#if g.status === 'running' && g.reviews.length}
          {@const prog = stepProgress(g.reviews)}
          {#if prog}
            <div class="sub waiting" data-testid="step-waiting-on">
              <span class="phase">{prog.phase}</span>
              <span>{prog.detail}</span>
              {#if prog.waitingOn}<span class="sub-desc">· waiting for {prog.waitingOn}</span>{/if}
            </div>
          {/if}
        {/if}
        {#each g.reviews as review, ri (review.id)}
          {@const rp = repoProgress(review)}
          {#if g.reviews.length > 1}
            <div class="repo-h" data-testid="review-repo-group">
              <Icon name="branch" size={12} />
              <span class="repo-name">{repoLabel(review, ri)}</span>
              <span class="grow"></span>
              <span>{rp.finished ? sentenceCase(review.status) : `${rp.done}/${rp.reviewers.length} reviewers done`}</span>
            </div>
          {/if}
          {#each review.agents as agent, ai (`${review.id}:${ai}`)}
            {#if agent.session_id}
              {@render sessRow(agent.session_id)}
            {:else}
              {@const st = agentState(review, agent)}
              <div class="sess queued" data-testid={review.agents.at(-1) === agent ? 'summarizer-without-session' : 'agent-without-session'} title={st.hint}>
                <div class="sess-h static">
                  <StatusDot state={st} size={8} />
                  <span class="s-title">{review.agents.at(-1) === agent && review.agents.length > 1 ? 'Summarizer' : agent.name}</span>
                  <span class="s-provider">{agent.provider || 'claude'}</span>
                  <span class="s-status">{st.label}</span>
                </div>
                {#if agent.fallback}
                  {@const fb = summaryFallback(review.agents)}
                  <div class="fallback" data-testid="summarizer-fallback" title={summaryFallbackTitle(fb, 'the findings')}>
                    Deterministic fallback — the {fb.provider} summarizer’s output wasn’t used{fb.cause ? ` (${fb.cause})` : ''}.
                  </div>
                {/if}
              </div>
            {/if}
          {/each}
        {/each}
        {#each g.loose as sid (sid)}
          {@render sessRow(sid)}
        {/each}
        <!-- Sub-agents / background tasks the step launched. Display only —
             they have no PTY of their own, so there is nothing to attach. -->
        {#if g.activity && g.activity.subagents.length}
          {@const subs = g.activity.subagents}
          <div class="subs" data-testid="subagent-rows">
            {#each subs as sa (sa.id)}
              <div class="sub" data-status={sa.status} title={sa.description}>
                <span class="sub-desc">{sa.description}</span>
                <StatusBadge status={runStatus(sa.status)} variant="text" />
                {#if sa.status === 'running'}
                  <span class="sub-when">{fmtSince(sa.started_at)}</span>
                {:else if sa.status === 'done' && sa.started_at && sa.finished_at}
                  <span class="sub-when">{fmtBetween(sa.started_at, sa.finished_at)}</span>
                {/if}
              </div>
            {/each}
          </div>
        {/if}
      </div>
    {/each}
  </div>
{/if}

<style>
  .agents {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 8px;
    overflow: auto;
    min-height: 0;
    height: 100%;
  }
  .grp {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .grp-h {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 2px 4px;
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: .06em;
    color: var(--text-dim);
  }
  .grp-name {
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  /* The header is an uppercase eyebrow; the status word is not. */
  .grp-status {
    text-transform: none;
    letter-spacing: 0;
    font-weight: 500;
  }
  .grow {
    flex: 1;
  }
  .grp-count {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    color: var(--text-dim);
    background: var(--accent-soft);
    padding: 0 6px;
    border-radius: 999px;
  }
  .sess {
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    overflow: hidden;
  }
  .sess-h {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
    width: 100%;
    min-width: 0;
    padding: 6px 8px;
    background: transparent;
    border: none;
    cursor: pointer;
    color: var(--text-dim);
    font-size: var(--fs-s);
    text-align: start;
  }
  .sess-h:hover {
    background: var(--accent-faint);
  }
  .s-title {
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1 1 8rem;
    min-width: 0;
    max-width: 100%;
  }
  .s-provider {
    font-size: var(--fs-xs);
  }
  .fallback {
    padding: 4px 8px 8px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .s-status {
    font-size: var(--fs-xs);
  }
  .s-id {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    color: var(--text-dim);
    min-width: 0;
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    direction: ltr;
  }
  .subs {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin-inline-start: 18px;
  }
  .sub {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .sub-desc {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .sub-when {
    flex-shrink: 0;
  }
  .recovery-status {
    white-space: normal;
    overflow-wrap: anywhere;
    overflow: visible;
    text-overflow: clip;
  }
  .sub[data-status='failed'] {
    color: var(--danger);
  }
  .waiting {
    flex-wrap: wrap;
    padding: 0 4px;
  }
  .phase {
    color: var(--accent-text);
    font-weight: 600;
  }
  .repo-h {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 4px 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .repo-name {
    color: var(--text);
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .sess-h.static {
    cursor: default;
    padding-inline-start: 28px;
  }
  .sess-h.static:hover {
    background: transparent;
  }
  .legend-row {
    display: flex;
  }
  .legend {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    cursor: help;
  }
  .term {
    height: 320px;
    border-top: 1px solid var(--border);
    display: flex;
    min-height: 0;
  }
</style>
