<script lang="ts">
  // All runs/iterations as a filterable list (per assignee / project / status).
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import RunInspector from './RunInspector.svelte';
  import VirtualList from '../../lib/components/VirtualList.svelte';
  import { swarm } from '../../lib/stores/swarm.svelte';
  import { rel } from '../../lib/stores/now.svelte';
  import type { RunStatus, SwarmRun } from './types';

  let agentFilter = $state<string>('');
  let projectFilter = $state<string>('');
  let statusFilter = $state<string>('');
  let inspecting = $state<SwarmRun | null>(null);

  // Keep the open inspector in sync with live WS run updates (tokens land on the
  // terminal patch, after the inspector may already be open).
  const live = $derived(inspecting ? (swarm.runs.find((r) => r.id === inspecting!.id) ?? inspecting) : null);

  const STATUSES: RunStatus[] = ['queued', 'running', 'waiting', 'done', 'error', 'stopped'];

  const filtered = $derived(
    swarm.runs.filter(
      (r) =>
        (!agentFilter || r.agent_id === agentFilter) &&
        (!projectFilter || r.project_id === projectFilter) &&
        (!statusFilter || r.status === statusFilter),
    ),
  );

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
      <VirtualList items={filtered} estimateHeight={37} class="vlist-runs">
        {#snippet row(r: SwarmRun)}
          {@const agent = swarm.agentById(r.agent_id)}
          <div class="trow" role="button" tabindex="0" onclick={() => (inspecting = r)} onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && (inspecting = r)}>
            <span class="c-agent">{agent?.name ?? r.agent_id.slice(0, 6)}</span>
            <span class="c-kind dim">{r.kind}{r.summary ? ` · ${r.summary}` : ''}</span>
            <span class="c-status"><span class="badge {r.status}">{r.status}</span></span>
            <span class="c-time dim">{rel(r.started_at ?? r.enqueued_at ?? '')}</span>
            <span class="c-tok mono dim">
              {r.tokens_input != null || r.tokens_output != null
                ? `${r.tokens_input ?? 0}/${r.tokens_output ?? 0}`
                : '—'}
            </span>
            <span class="c-act">
              <button class="icon-btn" title="Inspect run" aria-label="inspect run" onclick={(e) => { e.stopPropagation(); inspecting = r; }}>
                <Icon name="eye" size={14} />
              </button>
              {#if r.session_id}
                <button class="btn small ghost" onclick={(e) => { e.stopPropagation(); swarm.selectedSessionId = r.session_id!; }}>Open</button>
              {/if}
              {#if active(r)}
                <button class="btn small danger" onclick={(e) => { e.stopPropagation(); swarm.stopRun(r.id); }}>Stop</button>
              {/if}
            </span>
          </div>
        {/snippet}
      </VirtualList>
    </div>
  {/if}
</div>

{#if live}
  <RunInspector run={live} onclose={() => (inspecting = null)} />
{/if}

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
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
  }
  .vlist-runs {
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
    cursor: pointer;
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
