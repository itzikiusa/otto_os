<script lang="ts">
  // Shared per-agent results block for BOTH the PR review (ReviewPanel) and the
  // local working-tree review (LocalReviewPanel) — so "Open" (inline live
  // terminal), "Retry", per-agent findings and status pills behave identically
  // and live in one place. The backend is already shared: retry hits
  // POST /reviews/{id}/agents/{index}/retry, keyed by review id.
  import { api } from '../../lib/api/client';
  import type { Review } from '../../lib/api/types';
  import { toasts } from '../../lib/toast.svelte';
  import Terminal from '../../lib/components/Terminal.svelte';

  interface Props {
    review: Review;
    /** 'running' shows every agent (incl. the trailing summarizer, not
     *  retryable); 'done' shows the reviewers only (summarizer sliced off). */
    view: 'running' | 'done';
    /** Called after a successful retry with the refreshed review, so the parent
     *  can update its state and resume polling. */
    onretried?: (review: Review) => void;
  }
  let { review, view, onretried }: Props = $props();

  // Rows to render: during a run, all agents; when done, drop the summarizer
  // (the last entry) — it has no standalone session/findings to open.
  const rows = $derived(view === 'done' ? review.agents.slice(0, -1) : review.agents);
  // The summarizer is the last entry of the FULL list and is never retryable.
  const lastRetryable = $derived(review.agents.length - 1);

  // Inline live terminals (multiple may be open at once), keyed by session id.
  let openTerminals = $state<Set<string>>(new Set());
  function toggleTerminal(sessionId: string | null | undefined): void {
    if (!sessionId) return;
    const next = new Set(openTerminals);
    if (next.has(sessionId)) next.delete(sessionId);
    else next.add(sessionId);
    openTerminals = next;
  }

  // Which agent rows are expanded to show their individual findings.
  let agentExpanded: Record<string, boolean> = $state({});
  function toggleAgent(name: string): void {
    agentExpanded = { ...agentExpanded, [name]: !agentExpanded[name] };
  }

  // Re-run a single agent (e.g. one whose prompt never landed). Kills its old
  // session and spawns a fresh one server-side; the parent resumes polling.
  let retrying: Record<number, boolean> = $state({});
  async function retryAgent(index: number): Promise<void> {
    if (retrying[index]) return;
    retrying = { ...retrying, [index]: true };
    try {
      const r = await api.post<Review>(`/reviews/${review.id}/agents/${index}/retry`);
      onretried?.(r);
      toasts.info('Retrying agent…');
    } catch (e) {
      toasts.error('Retry failed', e instanceof Error ? e.message : String(e));
    } finally {
      retrying = { ...retrying, [index]: false };
    }
  }
</script>

<div class="rp-agents" class:rp-agents-done={view === 'done'}>
  {#each rows as agent, i (agent.name)}
    <div class="rp-agent card">
      <div class="rp-agent-top">
        <span class="rp-agent-name">{agent.name}</span>
        <span class="chip rp-agent-chip">{agent.provider}{agent.model ? ' · ' + agent.model : ''}</span>
        <span class="grow"></span>
        {#if agent.session_id}
          <button class="btn small ghost" onclick={() => toggleTerminal(agent.session_id)}>
            {openTerminals.has(agent.session_id) ? 'Hide' : 'Open'}
          </button>
        {/if}
        {#if i < lastRetryable}
          <button
            class="btn small ghost"
            disabled={retrying[i]}
            onclick={() => retryAgent(i)}
            title="Re-run this agent"
          >
            {retrying[i] ? 'Retrying…' : 'Retry'}
          </button>
        {/if}
        {#if agent.findings && agent.findings.length > 0}
          <button class="btn small ghost" onclick={() => toggleAgent(agent.name)}>
            {agentExpanded[agent.name]
              ? 'Hide'
              : `${agent.findings.length} finding${agent.findings.length === 1 ? '' : 's'}`}
          </button>
        {/if}
        <span class="rp-status-pill rp-status-{agent.status}">
          {#if agent.status === 'running' || agent.status === 'waiting'}
            <span class="spinner-xs"></span>
          {/if}
          {agent.status}
        </span>
      </div>
      {#if agent.note && (view === 'running' || agent.status !== 'done')}
        <p class="rp-agent-note">{agent.note}</p>
      {/if}
      {#if agent.status === 'waiting'}
        <p class="rp-agent-waiting">
          ⚠ This agent looks blocked on input. Click <strong>Open</strong> to view its session and
          respond (e.g. approve folder access).
        </p>
      {/if}
      {#if agent.session_id && openTerminals.has(agent.session_id)}
        <div class="rp-term">
          <!-- No {#key} here. Terminal retargets its WS internally when
               `sessionId` changes (no xterm/WS teardown — see Terminal's
               session-switch effect, commit e72edf78). Wrapping it in
               {#key agent.session_id} forced a full teardown + reconnect on
               every parent re-render; during a RUNNING review (the panel
               refetches `review` on every poll/bus tick) that became a ~90/sec
               WS reconnect storm, leaving a running agent's terminal stuck on
               "reconnecting". The {#each} is keyed by agent.name, so the
               component instance is stable across refetches. -->
          <Terminal sessionId={agent.session_id} />
        </div>
      {/if}
      {#if agentExpanded[agent.name] && agent.findings}
        <ul class="rp-agent-findings">
          {#each agent.findings as f (f.fingerprint ?? f.body)}
            <li class="rp-finding">
              <span class="severity-chip sev-{f.severity}">{f.severity}</span>
              {#if f.path}<span class="mono rp-loc">{f.path}{f.line ? ':' + f.line : ''}</span>{/if}
              <!-- Lifecycle state chip (A1): shown when a persisted state is available. -->
              {#if f.state && f.state !== 'open'}
                <span class="chip rp-state-chip rp-state-{f.state}" title="Finding state">{f.state}</span>
              {/if}
              <span class="rp-finding-body">{f.body}</span>
            </li>
          {/each}
        </ul>
      {/if}
    </div>
  {/each}
</div>

<style>
  .rp-agents {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-top: 4px;
  }
  .rp-agent {
    padding: 8px 12px;
  }
  .rp-agent-top {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .rp-agent-name {
    font-size: 12.5px;
    font-weight: 600;
  }
  .rp-agent-chip {
    font-size: 10.5px;
  }
  .rp-agent-note {
    margin: 4px 0 0;
    font-size: 11.5px;
    color: var(--text-dim);
    line-height: 1.4;
  }

  .rp-status-pill {
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
  .rp-status-pending {
    background: color-mix(in srgb, var(--text-dim) 12%, transparent);
    color: var(--text-dim);
  }
  .rp-status-running {
    background: color-mix(in srgb, var(--accent) 15%, transparent);
    color: var(--accent);
  }
  .rp-status-done {
    background: color-mix(in srgb, var(--status-working) 15%, transparent);
    color: var(--status-working);
  }
  .rp-status-error {
    background: color-mix(in srgb, var(--status-exited) 15%, transparent);
    color: var(--status-exited);
  }
  .rp-status-waiting {
    background: var(--status-warn-soft);
    color: var(--status-warn);
  }

  .rp-agent-waiting {
    margin: 6px 0 0;
    font-size: 11.5px;
    line-height: 1.45;
    color: var(--status-warn);
  }
  .rp-term {
    height: min(360px, 65vh);
    margin: 8px 0 2px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    overflow: hidden;
    overscroll-behavior: contain;
    background: var(--term-bg);
  }
  .rp-agent-findings {
    list-style: none;
    margin: 6px 0 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 5px;
  }
  .rp-finding {
    display: flex;
    align-items: baseline;
    gap: 6px;
    font-size: 11.5px;
    line-height: 1.4;
  }
  .rp-finding-body {
    flex: 1;
    min-width: 0;
  }
  .rp-loc {
    font-size: 11px;
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 280px;
  }

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
  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  .grow { flex: 1; }
  .mono { font-family: var(--font-mono, monospace); }

  /* A1: finding lifecycle state chips */
  .rp-state-chip {
    font-size: 9.5px;
    padding: 1px 5px;
    text-transform: uppercase;
    font-weight: 700;
    letter-spacing: 0.03em;
    flex-shrink: 0;
  }
  .rp-state-fixing    { background: color-mix(in srgb, var(--status-warn) 15%, transparent); color: var(--status-warn); }
  .rp-state-resolved  { background: color-mix(in srgb, var(--status-working) 12%, transparent); color: var(--status-working); }
  .rp-state-regressed { background: color-mix(in srgb, var(--status-exited) 12%, transparent); color: var(--status-exited); }
  .rp-state-declined  { background: color-mix(in srgb, var(--text-dim) 12%, transparent); color: var(--text-dim); }

  /* ── Mobile + tablet (≤1024px) ──────────────────────────────────────────────
     The per-agent header is a dense row of name + chip + Open/Retry/findings
     buttons + status pill — it already wraps, but on a phone we give the action
     buttons real touch height, let the agent name shrink/ellipsis instead of
     forcing the row wider, and cap the inline terminal so it fits a short
     viewport. The findings list body keeps its own min-width:0 so long text
     wraps within the card rather than pushing the page. */
  @media (max-width: 1024px) {
    .rp-agent { padding: 10px 12px; }
    .rp-agent-name {
      flex: 1 1 auto;
      min-width: 0;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }
    .rp-agent-top .btn { min-height: 32px; }
  }
  @media (max-width: 640px) {
    .rp-agent-top .btn { min-height: 38px; }
    .rp-loc { max-width: 100%; }
    .rp-term { height: min(280px, 60vh); }
  }
</style>
