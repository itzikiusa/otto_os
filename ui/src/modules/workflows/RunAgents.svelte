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
  import Terminal from '../../lib/components/Terminal.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import type { WorkflowRun, Review, Session } from '../../lib/api/types';
  import { api } from '../../lib/api/client';
  import { mapLimit } from '../../lib/poll';
  import { liveQuery, type LiveEvent } from '../../lib/live';
  import { untrack } from 'svelte';
  import { runStatus, sessionState, sentenceCase, type SessionStateInfo } from '../../lib/status';
  import StatusBadge from '../../lib/components/StatusBadge.svelte';
  import StatusDot from '../../lib/components/StatusDot.svelte';
  import { events } from '../../lib/events.svelte';
  import { reviewIds, reviewSessions, reviewAgentStatus } from './reviewAgents';

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
  let reviews = $state<Record<string, Review>>({});
  let sessionDetails = $state<Record<string, Session>>({});
  // A STRING key: the id array was rebuilt on every run merge (a WS tick), so
  // the effect below restarted — and refetched every review — per merge
  // (backlog B6 / SD-20).
  const relatedKey = $derived([...new Set((run.nodes ?? []).flatMap(reviewIds))].join(','));
  const relatedIds = $derived(relatedKey ? relatedKey.split(',') : []);
  // Reviews outlive await:false steps and may be retried after the workflow is
  // complete. Keep their durable association active while this panel is open:
  // running reviews every 2 s, finished ones (a retry can revive them) every
  // 30 s, nothing while the window is hidden (shared poll helper). Each effect
  // owns its requests, so a late response cannot enter another run.
  const TERMINAL_RECHECK_MS = 30_000;
  $effect(() => {
    const runId = run.id;
    const ids = relatedKey ? relatedKey.split(',') : [];
    if (!ids.length) return;
    const fetchedAt = new Map<string, number>();
    const lastJson = new Map<string, string>();
    // Event-fed: `review_changed` for one of these reviews re-reads the due
    // ones (coalesced); 30 s safety net while events flow, the old 2 s
    // cadence only while the event socket is down.
    // An event for a TERMINAL review (a summarizer retry re-opens a done one)
    // makes it due now instead of after TERMINAL_RECHECK_MS.
    const onEvent = (ev: LiveEvent): boolean => {
      const id = ev.review_id as string;
      if (!ids.includes(id)) return false;
      fetchedAt.delete(id);
      return true;
    };
    const poller = liveQuery({ on: ['review_changed'], match: onEvent, fallbackMs: 2000, safetyMs: 30_000, run: async (signal) => {
      const now = Date.now();
      const due = ids.filter((id) => {
        const r = untrack(() => reviews[id]);
        if (!r || r.status === 'running') return true;
        return now - (fetchedAt.get(id) ?? 0) >= TERMINAL_RECHECK_MS;
      });
      if (!due.length) return;
      const results = await mapLimit(due, 3, async (id) => {
        try { return await api.bg.get<Review>(`/reviews/${id}`, signal); } catch { return null; }
      });
      if (signal.aborted || run.id !== runId) return;
      for (const review of results) {
        if (!review) continue;
        fetchedAt.set(review.id, Date.now());
        // Assign only a changed review: an unchanged tick re-renders nothing.
        const json = JSON.stringify(review);
        if (lastJson.get(review.id) === json) continue;
        lastJson.set(review.id, json);
        reviews[review.id] = review;
      }
    } });
    return () => poller.stop();
  });
  const groups = $derived(
    (run.nodes ?? [])
      .map((n) => ({
        id: n.node_id,
        status: n.status,
        sessions: reviewSessions(n, reviews),
        reviews: reviewIds(n).flatMap((id) => reviews[id] ? [reviews[id]] : []),
        activity: n.activity ?? null,
      }))
      .filter((g) => g.sessions.length > 0 || g.reviews.length > 0 || (g.activity?.subagents.length ?? 0) > 0),
  );
  // Suspended sessions may be absent from the workspace's live list. Load
  // metadata once for titles/provider and an elapsed clock that survives reload.
  $effect(() => {
    const ids = groups.flatMap((g) => g.sessions);
    let alive = true;
    void Promise.all(ids.filter((id) => !sessOf(id)).map(async (id) => {
      try {
        const session = await api.get<Session>(`/sessions/${id}`);
        if (alive) sessionDetails[id] = session;
      } catch { /* An old deleted session still retains its review label. */ }
    }));
    return () => { alive = false; };
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
  function sStatus(sid: string): string {
    const row = reviewAgent(sid);
    if (row) return reviewAgentStatus(row.review, row.agent);
    return ws.statusMap[sid] ?? sessOf(sid)?.status ?? 'idle';
  }
  /** The one dot + word for a session row. Review agents report their own
   *  vocabulary (pending / running / waiting / done / error / skipped, plus
   *  cancelled / fallback) — mapped onto the shared session states so the raw
   *  engine word never reaches the screen. */
  function sState(sid: string): SessionStateInfo {
    const stale = events.state !== 'connected';
    const raw = sStatus(sid);
    const row = reviewAgent(sid);
    const base = { resumable: false, inactive: true };
    if (!row) return sessionState(sessOf(sid), raw, ws.needsYou[sid] === true, { stale });
    switch (raw) {
      case 'running':
        return sessionState(null, 'working', false, { stale });
      case 'waiting':
        return sessionState(null, 'idle', true, { stale });
      case 'pending':
        return { ...sessionState(null, 'idle', false, { stale }), label: 'Queued' };
      case 'done':
        return { ...base, key: 'ended', label: 'Done', tone: 'success', hint: 'Finished' };
      case 'skipped':
        return { ...base, key: 'ended', label: 'Skipped', tone: 'neutral', hint: 'Skipped' };
      case 'cancelled':
        return { ...base, key: 'ended', label: 'Cancelled', tone: 'neutral', hint: 'The review was cancelled' };
      case 'fallback':
        return { ...base, key: 'failed', label: 'Fallback', tone: 'warning', hint: 'The configured summarizer was unavailable — a deterministic fallback ran' };
      case 'error':
        return { ...base, key: 'failed', label: 'Failed', tone: 'danger', hint: 'The agent failed' };
      default:
        return { ...base, key: 'ended', label: sentenceCase(raw), tone: 'neutral' };
    }
  }
  function shortId(id: string): string {
    return id.length > 6 ? id.slice(-6) : id;
  }
</script>

{#if groups.length === 0}
  <div class="empty">
    No agent sessions for this run yet.{#if runActive}
      <br />They’ll appear here as steps spawn them.
    {/if}
  </div>
{:else}
  <div class="agents" data-testid="run-agents">
    {#each groups as g (g.id)}
      <div class="grp">
        <div class="grp-h">
          <!-- Step status in the shared run vocabulary (Succeeded / Failed /
               Queued…), like the Steps list and timeline — not the raw
               engine word ("success", "error", "pending"). -->
          <span class="grp-name" title={nodeName(g.id)}>{nodeName(g.id)}</span>
          <span class="grp-status"><StatusBadge status={runStatus(g.status)} variant="text" /></span>
          <span class="grow"></span>
          <span class="grp-count" title="{g.sessions.length} session(s)">{g.sessions.length}</span>
        </div>
        {#each g.sessions as sid (sid)}
          <div class="sess" data-sess={sid}>
            <button class="sess-h" onclick={() => toggle(sid)} aria-expanded={!!expanded[sid]} title={expanded[sid] ? 'Hide live terminal' : 'Show live terminal'}>
              <Icon name={expanded[sid] ? 'chevronDown' : 'chevronRight'} size={12} />
              <StatusDot state={sState(sid)} size={8} />
              <span class="s-title">{sTitle(sid)}</span>
              <span class="s-provider">{sProvider(sid)}</span>
              <span class="s-status">{sState(sid).label}{#if ['working', 'running', 'needs-you'].includes(sState(sid).key)}{' ' + fmtSince(sessOf(sid)?.created_at)}{/if}</span>
              <span class="grow"></span>
              <code class="s-id" title={sid}>{shortId(sid)}</code>
            </button>
            {#if reviewAgent(sid)?.agent.fallback}
              <div class="fallback" data-testid="summarizer-fallback">Deterministic fallback — {reviewAgent(sid)?.agent.note}</div>
            {/if}
            {#if expanded[sid]}
              <div class="term">
                {#key sid}
                  <Terminal sessionId={sid} resumable preferDom resumeOnOpen={false} />
                {/key}
              </div>
            {/if}
          </div>
        {/each}
        {#each g.reviews as review (review.id)}
          {@const summarizer = review.agents.at(-1)}
          {#if summarizer && !summarizer.session_id}
            <div class="sub" data-testid="summarizer-without-session">Summarizer · {summarizer.provider || 'claude'} — {sentenceCase(reviewAgentStatus(review, summarizer))}{summarizer.fallback ? ' · Deterministic fallback' : ''}</div>
          {/if}
        {/each}
        <!-- Sub-agents / background tasks the step launched. Display only —
             they have no PTY of their own, so there is nothing to attach. -->
        {#if g.activity && g.activity.subagents.length}
          {@const subs = g.activity.subagents}
          <div class="subs" data-testid="subagent-rows">
            {#each subs as sa (sa.id)}
              <div class="sub" data-status={sa.status} title={sa.description}>└ {sa.description} — {sa.status === 'running' ? `running ${fmtSince(sa.started_at)}` : sa.status === 'done' ? `done${sa.started_at && sa.finished_at ? ' in ' + fmtBetween(sa.started_at, sa.finished_at) : ''}` : 'failed'}</div>
            {/each}
          </div>
        {/if}
      </div>
    {/each}
  </div>
{/if}

<style>
  .empty {
    padding: 16px 12px;
    font-size: var(--fs-s);
    color: var(--text-dim);
    line-height: 1.5;
  }
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
    gap: 7px;
    padding: 3px 4px;
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
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
    background: color-mix(in srgb, var(--accent) 12%, transparent);
    padding: 0 6px;
    border-radius: 99px;
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
    gap: 6px;
    width: 100%;
    padding: 6px 8px;
    background: transparent;
    border: none;
    cursor: pointer;
    color: var(--text-dim);
    font-size: var(--fs-s);
    text-align: start;
  }
  .sess-h:hover {
    background: color-mix(in srgb, var(--accent) 8%, transparent);
  }
  .s-title {
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 50%;
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
  }
  .subs {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin-inline-start: 18px;
  }
  .sub {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .sub[data-status='failed'] {
    color: var(--danger);
  }
  .term {
    height: 320px;
    border-top: 1px solid var(--border);
    display: flex;
    min-height: 0;
  }
</style>
