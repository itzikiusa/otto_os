<script lang="ts">
  // Per-agent results block for a Skills Lab review — the same embedded-terminal
  // pattern as the code-review ReviewAgents: Open mounts a live <Terminal> for
  // the agent's session inline (multiple can be open at once), Retry re-runs one
  // reviewer, and each agent's own findings expand below its row. The summarizer
  // is the trailing row (not retryable; its aggregate renders in the panel).
  import type { SkillReview } from '../../lib/api/types';
  import { skillReviewApi } from '../../lib/api/skillReview';
  import { toasts } from '../../lib/toast.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import Terminal from '../../lib/components/Terminal.svelte';
  import StatusBadge from '../../lib/components/StatusBadge.svelte';
  import { runStatus } from '../../lib/status';

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
      toasts.error("Couldn't re-run the agent", e instanceof Error ? e.message : String(e));
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
        <span class="rp-agent-name" title={agent.name}>{agent.name}</span>
        <span class="chip rp-agent-chip">{agent.provider}{agent.model ? ' · ' + agent.model : ''}</span>
        <span class="grow"></span>
        {#if agent.session_id}
          <button class="btn small ghost" aria-expanded={openTerminals.has(agent.session_id)} onclick={() => toggleTerminal(agent.session_id)}>
            {openTerminals.has(agent.session_id) ? 'Hide session' : 'Open session'}
          </button>
        {/if}
        {#if i < lastRetryable && agent.name !== 'summarizer'}
          <button class="btn small ghost" disabled={retrying[i]} onclick={() => retryAgent(i)} title="Re-run this agent">
            {retrying[i] ? 'Retrying…' : 'Retry'}
          </button>
        {/if}
        {#if agent.findings && agent.findings.length > 0}
          <button class="btn small ghost" aria-expanded={!!agentExpanded[agent.name]} onclick={() => toggleAgent(agent.name)}>
            {agentExpanded[agent.name] ? 'Hide findings' : `${agent.findings.length} finding${agent.findings.length === 1 ? '' : 's'}`}
          </button>
        {/if}
        <span class="rp-status-pill" data-status={agent.status}><StatusBadge status={runStatus(agent.status)} /></span>
      </div>
      {#if agent.note && (view === 'running' || agent.status !== 'done')}
        <p class="rp-agent-note">{agent.note}</p>
      {/if}
      {#if agent.status === 'waiting'}
        <p class="rp-agent-waiting">
          <Icon name="warning" size={12} /> This agent looks blocked on input. <strong>Open session</strong> to respond.
        </p>
      {/if}
      {#if agent.session_id && openTerminals.has(agent.session_id)}
        <div class="rp-term">
          <!-- No {#key}: Terminal retargets its own WS when sessionId changes; a
               {#key} under a frequently-refetching parent causes a reconnect storm. -->
          <Terminal sessionId={agent.session_id} preferDom />
        </div>
      {/if}
      {#if agentExpanded[agent.name] && agent.findings}
        <ul class="rp-agent-findings">
          {#each agent.findings as f, i (i + f.code)}
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
  .rp-agent-name { font-size: var(--fs-s); font-weight: 600; }
  .rp-agent-chip { font-size: var(--fs-xs); }
  .rp-agent-note { margin: 4px 0 0; font-size: var(--fs-xs); color: var(--text-dim); line-height: 1.4; }

  /* Status pill: the shared StatusBadge; this wrapper is the layout hook. */
  .rp-status-pill { display: inline-flex; align-items: center; }

  .rp-agent-waiting { margin: 6px 0 0; font-size: var(--fs-xs); line-height: 1.45; color: var(--warning); }
  .rp-term {
    height: min(360px, 65vh); margin: 8px 0 2px; border: 1px solid var(--border);
    border-radius: var(--radius-m); overflow: hidden; overscroll-behavior: contain; background: var(--term-bg);
  }
  .rp-agent-findings { list-style: none; margin: 6px 0 0; padding: 0; display: flex; flex-direction: column; gap: 5px; }
  .rp-finding { display: flex; align-items: baseline; gap: 6px; font-size: var(--fs-xs); line-height: 1.4; }
  .rp-finding-body { flex: 1; min-width: 0; }
  .rp-loc { font-size: var(--fs-xs); color: var(--text-dim); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 200px; }

  .severity-chip {
    display: inline-block; padding: 1px 8px; border-radius: 999px;
    font-size: var(--fs-xs); font-weight: 500; text-transform: capitalize;
  }
  .sev-critical { background: var(--danger-soft); color: var(--danger); }
  .sev-high { background: var(--danger-soft); color: var(--danger); }
  .sev-medium { background: var(--warning-soft); color: var(--warning); }
  .sev-low { background: var(--info-soft); color: var(--info); }

  .grow { flex: 1; }
  .mono { font-family: var(--font-mono); }

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
