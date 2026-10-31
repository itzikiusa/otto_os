///<reference types="svelte" />
;// Run-scoped agent view (embedded in the WF page's right sidebar "Agents" tab).
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
import type { WorkflowRun, Review, Session } from '../../lib/api/types';
import { api, ApiError } from '../../lib/api/client';
import { mapLimit } from '../../lib/poll';
import { liveQuery, type LiveEvent } from '../../lib/live';
import { untrack } from 'svelte';
import { runStatus, sessionState, sentenceCase, type SessionStateInfo } from '../../lib/status';
import StatusBadge from '../../lib/components/StatusBadge.svelte';
import StatusDot from '../../lib/components/StatusDot.svelte';
import { events } from '../../lib/events.svelte';
import { reviewIds, reviewSessions, reviewAgentStatus } from './reviewAgents';
import { summaryFallback, summaryFallbackTitle } from '../git/reviewFallback';
import { plural } from '../../lib/plural';
;

  interface Props {
    run: WorkflowRun;
    /** Resolve a node id to a friendly label. */
    nodeName?: (id: string) => string;
    /** A session id to auto-expand + scroll to (set when opened from a step). */
    focusSid?: string | null;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

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
        attempts: n.attempts ?? 0,
        recovery: (n.logs ?? []).filter((line) => line.includes('↻ retry') || line.startsWith('🔁 iteration') || line.includes(' — attempt ')).at(-1),
        sessions: reviewSessions(n, reviews),
        reviews: reviewIds(n).flatMap((id) => reviews[id] ? [reviews[id]] : []),
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
        return { ...base, key: 'ended', label: 'Canceled', tone: 'neutral', hint: 'The review was canceled' };
      case 'fallback':
        // Names the summarizer that actually ran and why its output wasn't
        // used (error-handling-5-05), not a hard-coded "claude unavailable".
        return { ...base, key: 'failed', label: 'Fallback', tone: 'warning', hint: summaryFallbackTitle(summaryFallback(row.review.agents), 'the findings') };
      case 'error':
        return { ...base, key: 'failed', label: 'Failed', tone: 'danger', hint: 'The agent failed' };
      default:
        return { ...base, key: 'ended', label: sentenceCase(raw), tone: 'neutral' };
    }
  }
  function shortId(id: string): string {
    return id.length > 6 ? id.slice(-6) : id;
  }
;
async () => {

if(groups.length === 0){
   { const $$_etatSytpmE0C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE0C({ target: __sveltets_2_any(), props: {       "icon":`terminal`,"title":`No agent sessions for this run yet`,"body":runActive ? 'They’ll appear here as steps spawn them.' : undefined,}});}
}else{
   { svelteHTML.createElement("div", {   "class":`agents`,...__sveltets_2_empty({"data-testid":`run-agents`}),});
       for(let g of __sveltets_2_ensureArray(groups)){g.id;
       { svelteHTML.createElement("div", { "class":`grp`,});
         { svelteHTML.createElement("div", { "class":`grp-h`,});
          
           { svelteHTML.createElement("span", {   "class":`grp-name`,"title":nodeName(g.id),});nodeName(g.id); }
           { svelteHTML.createElement("span", { "class":`grp-status`,}); { const $$_egdaBsutatS4C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS4C({ target: __sveltets_2_any(), props: {    "status":runStatus(g.status),"variant":`text`,}});} }
          if(g.attempts > 1){ { svelteHTML.createElement("span", { "class":`grp-status`,}); g.attempts; }}
           { svelteHTML.createElement("span", { "class":`grow`,}); }
           { svelteHTML.createElement("span", {       "class":`grp-count`,"role":`img`,"title":plural(g.sessions.length, 'session'),"aria-label":plural(g.sessions.length, 'session'),});g.sessions.length; }
         }
        if(g.recovery){ { svelteHTML.createElement("div", {   "class":`sub recovery-status`,...__sveltets_2_empty({"data-testid":`workflow-recovery-status`}),});g.recovery; }}
           for(let sid of __sveltets_2_ensureArray(g.sessions)){sid;
           { svelteHTML.createElement("div", {   "class":`sess`,...__sveltets_2_empty({"data-sess":sid}),});
             { svelteHTML.createElement("button", {       "class":`sess-h`,"onclick":() => toggle(sid),"aria-expanded":!!expanded[sid],"title":expanded[sid] ? 'Hide live terminal' : 'Show live terminal',});
               { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":expanded[sid] ? 'chevronDown' : 'chevronRight',"size":12,}});}
               { const $$_toDsutatS4C = __sveltets_2_ensureComponent(StatusDot); new $$_toDsutatS4C({ target: __sveltets_2_any(), props: {    "state":sState(sid),"size":8,}});}
               { svelteHTML.createElement("span", { "class":`s-title`,});sTitle(sid); }
               { svelteHTML.createElement("span", { "class":`s-provider`,});sProvider(sid); }
               { svelteHTML.createElement("span", { "class":`s-status`,});sState(sid).label;if(['working', 'running', 'needs-you'].includes(sState(sid).key)){' ' + fmtSince(sessOf(sid)?.created_at);} }
               { svelteHTML.createElement("span", { "class":`grow`,}); }
               { svelteHTML.createElement("code", {   "class":`s-id`,"title":sid,});shortId(sid); }
             }
            if(reviewAgent(sid)?.agent.fallback){
              const fb = summaryFallback(reviewAgent(sid)?.review.agents);
               { svelteHTML.createElement("div", {     "class":`fallback`,...__sveltets_2_empty({"data-testid":`summarizer-fallback`}),"title":summaryFallbackTitle(fb, 'the findings'),});
                    fb.provider;    fb.cause ? ` (${fb.cause})` : '';
               }
            }
            if(expanded[sid]){
               { svelteHTML.createElement("div", { "class":`term`,});
                sid; {
                   { const $$_lanimreTyzaL4C = __sveltets_2_ensureComponent(LazyTerminal); new $$_lanimreTyzaL4C({ target: __sveltets_2_any(), props: {    "sessionId":sid,"resumable":true,"preferDom":true,"resumeOnOpen":false,}});}
                }
               }
            }
           }
        }
           for(let review of __sveltets_2_ensureArray(g.reviews)){review.id;
          const summarizer = review.agents.at(-1);
          if(summarizer && !summarizer.session_id){
             { svelteHTML.createElement("div", {   "class":`sub`,...__sveltets_2_empty({"data-testid":`summarizer-without-session`}),});  summarizer.provider || 'claude';  sentenceCase(reviewAgentStatus(review, summarizer));summarizer.fallback ? ` · Deterministic fallback${summaryFallback(review.agents).cause ? ` (${summaryFallback(review.agents).cause})` : ''}` : ''; }
          }
        }
        
        if(g.activity && g.activity.subagents.length){
          const subs = g.activity.subagents;
           { svelteHTML.createElement("div", {   "class":`subs`,...__sveltets_2_empty({"data-testid":`subagent-rows`}),});
               for(let sa of __sveltets_2_ensureArray(subs)){sa.id;
               { svelteHTML.createElement("div", {     "class":`sub`,...__sveltets_2_empty({"data-status":sa.status}),"title":sa.description,});
                 { svelteHTML.createElement("span", { "class":`sub-desc`,});sa.description; }
                 { const $$_egdaBsutatS4C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS4C({ target: __sveltets_2_any(), props: {    "status":runStatus(sa.status),"variant":`text`,}});}
                if(sa.status === 'running'){
                   { svelteHTML.createElement("span", { "class":`sub-when`,});fmtSince(sa.started_at); }
                } else if (sa.status === 'done' && sa.started_at && sa.finished_at){
                   { svelteHTML.createElement("span", { "class":`sub-when`,});fmtBetween(sa.started_at, sa.finished_at); }
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
const RunAgents__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type RunAgents__SvelteComponent_ = ReturnType<typeof RunAgents__SvelteComponent_>;
/*Ωignore_endΩ*/export default RunAgents__SvelteComponent_;