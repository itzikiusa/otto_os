<script lang="ts">
  // Per-agent results block for a Skills Lab review — the same embedded-terminal
  // pattern as the code-review ReviewAgents: Open mounts a live <Terminal> for
  // the agent's session inline (multiple can be open at once), Retry re-runs one
  // reviewer, and each agent's own findings expand below its row. The summarizer
  // is the trailing row (not retryable; its aggregate renders in the panel).
  import type { SkillReview } from '../../lib/api/types';
  import { skillReviewApi } from '../../lib/api/skillReview';
  import { toasts } from '../../lib/toast.svelte';
  import Terminal from '../../lib/components/Terminal.svelte';

  interface Props {
    review: SkillReview;
    view: 'running' | 'done';
    onretried?: (review: SkillReview) => void;
  }
  let { review, view, onretried }: Props = $props();

  // Running: show every agent incl. the summarizer. Done: drop the summarizer
  // (its aggregate renders separately as the summary report).
  const rows = $derived(view === 'done' ? review.agents.slice(0, -1) : review.agents);
  const lastRetryable = $derived(review.agents.length - 1);

  let openTerminals = $state<Set<string>>(new Set());
  function toggleTerminal(sessionId: string | null | undefined): void {
    if (!sessionId) return;
    const next = new Set(openTerminals);
    if (next.has(sessionId)) next.delete(sessionId);
    else next.add(sessionId);
    openTerminals = next;
  }

  let agentExpanded: Record<string, boolean> = $state({});
  function toggleAgent(name: string): void {
    agentExpanded = { ...agentExpanded, [name]: !agentExpanded[name] };
  }

  let retrying: Record<number, boolean> = $state({});
  async function retryAgent(index: number): Promise<void> {
    if (retrying[index]) return;
    retrying = { ...retrying, [index]: true };
    try {
      const r = await skillReviewApi.retryAgent(review.id, index);
      onretried?.(r);
      toasts.info('Retrying agent…');
    } catch (e) {
      toasts.error('Retry failed', e instanceof Error ? e.message : String(e));
    } finally {
      retrying = { ...retrying, [index]: false };
    }
  }

  function sevClass(sev: string): string {
    return `sev-${sev.toLowerCase()}`;
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
        {#if i < lastRetryable && agent.name !== 'summarizer'}
          <button class="btn small ghost" disabled={retrying[i]} onclick={() => retryAgent(i)} title="Re-run this agent">
            {retrying[i] ? 'Retrying…' : 'Retry'}
          </button>
        {/if}
        {#if agent.findings && agent.findings.length > 0}
          <button class="btn small ghost" onclick={() => toggleAgent(agent.name)}>
            {agentExpanded[agent.name] ? 'Hide' : `${agent.findings.length} finding${agent.findings.length === 1 ? '' : 's'}`}
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
          ⚠ This agent looks blocked on input. Click <strong>Open</strong> to view its session and respond.
        </p>
      {/if}
      {#if agent.session_id && openTerminals.has(agent.session_id)}
        <div class="rp-term">
          <!-- No {#key}: Terminal retargets its own WS when sessionId changes; a
               {#key} under a frequently-refetching parent causes a reconnect storm. -->
          <Terminal sessionId={agent.session_id} />
        </div>
      {/if}
      {#if agentExpanded[agent.name] && agent.findings}
        <ul class="rp-agent-findings">
          {#each agent.findings as f (f.code + f.title)}
            <li class="rp-finding">
              <span class="severity-chip {sevClass(f.severity)}">{f.severity}</span>
              {#if f.code}<span class="mono rp-loc">{f.code}</span>{/if}
              <span class="rp-finding-body"><strong>{f.title}</strong>{f.fix ? ` — ${f.fix}` : ''}</span>
            </li>
          {/each}
        </ul>
      {/if}
    </div>
  {/each}
</div>

<style>
  .rp-agents { display: flex; flex-direction: column; gap: 6px; margin-top: 4px; }
  .rp-agent { padding: 8px 12px; }
  .rp-agent-top { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .rp-agent-name { font-size: 12.5px; font-weight: 600; }
  .rp-agent-chip { font-size: 10.5px; }
  .rp-agent-note { margin: 4px 0 0; font-size: 11.5px; color: var(--text-dim); line-height: 1.4; }

  .rp-status-pill {
    font-size: 10px; font-weight: 700; letter-spacing: 0.04em; text-transform: uppercase;
    padding: 2px 6px; border-radius: var(--radius-s, 4px); display: inline-flex; align-items: center; gap: 3px;
  }
  .rp-status-pending { background: color-mix(in srgb, var(--text-dim) 12%, transparent); color: var(--text-dim); }
  .rp-status-running { background: color-mix(in srgb, var(--accent) 15%, transparent); color: var(--accent); }
  .rp-status-done { background: color-mix(in srgb, var(--status-working) 15%, transparent); color: var(--status-working); }
  .rp-status-error { background: color-mix(in srgb, var(--status-exited) 15%, transparent); color: var(--status-exited); }
  .rp-status-waiting { background: var(--status-warn-soft); color: var(--status-warn); }

  .rp-agent-waiting { margin: 6px 0 0; font-size: 11.5px; line-height: 1.45; color: var(--status-warn); }
  .rp-term {
    height: min(360px, 65vh); margin: 8px 0 2px; border: 1px solid var(--border);
    border-radius: var(--radius-m); overflow: hidden; overscroll-behavior: contain; background: var(--term-bg);
  }
  .rp-agent-findings { list-style: none; margin: 6px 0 0; padding: 0; display: flex; flex-direction: column; gap: 5px; }
  .rp-finding { display: flex; align-items: baseline; gap: 6px; font-size: 11.5px; line-height: 1.4; }
  .rp-finding-body { flex: 1; min-width: 0; }
  .rp-loc { font-size: 11px; color: var(--text-dim); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 200px; }

  .severity-chip {
    display: inline-block; padding: 2px 7px; border-radius: var(--radius-s, 4px);
    font-size: 10.5px; font-weight: 700; letter-spacing: 0.04em; text-transform: uppercase;
  }
  .sev-critical { background: color-mix(in srgb, var(--status-exited) 22%, transparent); color: var(--status-exited); }
  .sev-high { background: color-mix(in srgb, var(--status-exited) 15%, transparent); color: var(--status-exited); }
  .sev-medium { background: color-mix(in srgb, var(--status-warn) 15%, transparent); color: var(--status-warn); }
  .sev-low { background: color-mix(in srgb, var(--accent) 15%, transparent); color: var(--accent); }

  .spinner-xs {
    display: inline-block; width: 9px; height: 9px; border: 1.5px solid currentColor;
    border-top-color: transparent; border-radius: 50%; animation: spin 0.7s linear infinite;
    vertical-align: middle; margin-inline-end: 3px;
  }
  @keyframes spin { to { transform: rotate(360deg); } }
  .grow { flex: 1; }
  .mono { font-family: var(--font-mono, monospace); }

  @media (max-width: 1024px) {
    .rp-agent { padding: 10px 12px; }
    .rp-agent-name { flex: 1 1 auto; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .rp-agent-top .btn { min-height: 32px; }
  }
  @media (max-width: 640px) {
    .rp-agent-top .btn { min-height: 38px; }
    .rp-loc { max-width: 100%; }
    .rp-term { height: min(280px, 60vh); }
  }
</style>
