<script lang="ts">
  // All runs/iterations as a filterable list (per assignee / project / status).
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import { swarm } from '../../lib/stores/swarm.svelte';
  import type { RunStatus, SwarmRun } from './types';

  let agentFilter = $state<string>('');
  let projectFilter = $state<string>('');
  let statusFilter = $state<string>('');

  const STATUSES: RunStatus[] = ['queued', 'running', 'waiting', 'done', 'error', 'stopped'];

  const filtered = $derived(
    swarm.runs.filter(
      (r) =>
        (!agentFilter || r.agent_id === agentFilter) &&
        (!projectFilter || r.project_id === projectFilter) &&
        (!statusFilter || r.status === statusFilter),
    ),
  );

  function rel(ts?: string | null): string {
    if (!ts) return '—';
    const d = new Date(ts).getTime();
    const s = Math.floor((Date.now() - d) / 1000);
    if (s < 60) return `${s}s ago`;
    if (s < 3600) return `${Math.floor(s / 60)}m ago`;
    if (s < 86400) return `${Math.floor(s / 3600)}h ago`;
    return `${Math.floor(s / 86400)}d ago`;
  }

  function active(r: SwarmRun): boolean {
    return r.status === 'queued' || r.status === 'running' || r.status === 'waiting';
  }
</script>

<div class="runs">
  <div class="filters">
    <select class="input small" bind:value={agentFilter}>
      <option value="">All assignees</option>
      {#each swarm.detail?.agents ?? [] as a (a.id)}
        <option value={a.id}>{a.name}</option>
      {/each}
    </select>
    <select class="input small" bind:value={projectFilter}>
      <option value="">All projects</option>
      {#each swarm.detail?.projects ?? [] as p (p.id)}
        <option value={p.id}>{p.name}</option>
      {/each}
    </select>
    <div class="chips">
      <button class="chip" class:accent={statusFilter === ''} onclick={() => (statusFilter = '')}>all</button>
      {#each STATUSES as s (s)}
        <button class="chip" class:accent={statusFilter === s} onclick={() => (statusFilter = s)}>{s}</button>
      {/each}
    </div>
    <span class="grow"></span>
    <button class="icon-btn" onclick={() => swarm.detail && swarm.loadRuns({ swarm_id: swarm.detail.id })} aria-label="refresh">
      <Icon name="refresh" size={14} />
    </button>
  </div>

  {#if filtered.length === 0}
    <EmptyState icon="clock" title="No runs" body="Runs appear here as agents work tasks." />
  {:else}
    <div class="table">
      <div class="thead">
        <span class="c-agent">Agent</span>
        <span class="c-kind">Work</span>
        <span class="c-status">Status</span>
        <span class="c-time">Started</span>
        <span class="c-tok">Tokens</span>
        <span class="c-act"></span>
      </div>
      {#each filtered as r (r.id)}
        {@const agent = swarm.agentById(r.agent_id)}
        <div class="trow">
          <span class="c-agent">{agent?.name ?? r.agent_id.slice(0, 6)}</span>
          <span class="c-kind dim">{r.kind}{r.summary ? ` · ${r.summary}` : ''}</span>
          <span class="c-status"><span class="badge {r.status}">{r.status}</span></span>
          <span class="c-time dim">{rel(r.started_at ?? r.enqueued_at)}</span>
          <span class="c-tok mono dim">
            {r.tokens_input != null || r.tokens_output != null
              ? `${r.tokens_input ?? 0}/${r.tokens_output ?? 0}`
              : '—'}
          </span>
          <span class="c-act">
            {#if r.session_id}
              <button class="btn small ghost" onclick={() => (swarm.selectedSessionId = r.session_id!)}>Open</button>
            {/if}
            {#if active(r)}
              <button class="btn small danger" onclick={() => swarm.stopRun(r.id)}>Stop</button>
            {/if}
          </span>
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .runs {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .filters {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 10px;
    border-bottom: 1px solid var(--border);
    flex-wrap: wrap;
  }
  .chips {
    display: flex;
    gap: 4px;
    flex-wrap: wrap;
  }
  .chip {
    cursor: pointer;
    border: 1px solid var(--border);
    background: transparent;
  }
  .table {
    overflow: auto;
    flex: 1;
    min-height: 0;
  }
  .thead,
  .trow {
    display: grid;
    grid-template-columns: 120px 1fr 90px 90px 90px 130px;
    gap: 8px;
    align-items: center;
    padding: 6px 10px;
    font-size: 12px;
  }
  .thead {
    position: sticky;
    top: 0;
    background: var(--surface-2);
    color: var(--text-dim);
    font-size: 11px;
    border-bottom: 1px solid var(--border);
  }
  .trow {
    border-bottom: 1px solid color-mix(in srgb, var(--border) 60%, transparent);
  }
  .trow:hover {
    background: color-mix(in srgb, var(--text-dim) 8%, transparent);
  }
  .c-kind {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .c-act {
    display: flex;
    gap: 4px;
    justify-content: flex-end;
  }
  .badge {
    font-size: 10.5px;
    padding: 1px 7px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--text-dim) 18%, transparent);
    color: var(--text-dim);
  }
  .badge.running,
  .badge.waiting {
    background: color-mix(in srgb, var(--status-working) 22%, transparent);
    color: var(--status-working);
  }
  .badge.done {
    background: color-mix(in srgb, var(--accent) 20%, transparent);
    color: var(--accent);
  }
  .badge.error {
    background: color-mix(in srgb, var(--status-exited) 22%, transparent);
    color: var(--status-exited);
  }
</style>
