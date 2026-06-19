<script lang="ts">
  // Recursive org tree (CEO → … → devs) by `reports_to`. Each node shows the
  // agent, a status dot, task/run counts, and its open sessions (click → open).
  import Icon from '../../lib/components/Icon.svelte';
  import { swarm } from '../../lib/stores/swarm.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import type { SwarmAgent } from './types';

  interface Props {
    onedit: (a: SwarmAgent) => void;
    onruntask: (a: SwarmAgent) => void;
  }
  let { onedit, onruntask }: Props = $props();

  const agents = $derived(swarm.detail?.agents ?? []);
  const roots = $derived(agents.filter((a) => !a.reports_to));
  const childrenOf = (id: string) => agents.filter((a) => a.reports_to === id);

  // Sessions this agent holds (tagged at spawn with meta.swarm_id + meta.agent_id).
  function agentSessions(agentId: string) {
    const sid = swarm.detail?.id;
    return ws.sessions.filter((s) => {
      const m = (s.meta ?? {}) as Record<string, unknown>;
      return !s.archived && m.swarm_id === sid && m.agent_id === agentId;
    });
  }

  function runCount(agentId: string): number {
    return swarm.runs.filter((r) => r.agent_id === agentId && (r.status === 'running' || r.status === 'waiting')).length;
  }

  let open = $state<Record<string, boolean>>({});
  function toggle(id: string) {
    open[id] = !(open[id] ?? true);
    open = { ...open };
  }

  function menu(e: MouseEvent, a: SwarmAgent) {
    ctxMenu.show(e, [
      { label: 'Edit agent', icon: 'edit', action: () => onedit(a) },
      { label: 'Run a task…', icon: 'play', action: () => onruntask(a) },
      { separator: true },
      { label: 'Delete agent', icon: 'trash', danger: true, action: () => swarm.deleteAgent(a.id) },
    ]);
  }
</script>

<div class="tree">
  {#if roots.length === 0}
    <p class="dim pad">No agents yet. Use <strong>Recruit</strong> to add one.</p>
  {/if}
  {#each roots as r (r.id)}
    {@render node(r, 0)}
  {/each}
</div>

{#snippet node(a: SwarmAgent, depth: number)}
  {@const kids = childrenOf(a.id)}
  {@const sessions = agentSessions(a.id)}
  {@const isOpen = open[a.id] ?? true}
  {@const running = runCount(a.id)}
  <div class="row" style="padding-left:{depth * 14 + 6}px" oncontextmenu={(e) => menu(e, a)} role="treeitem" aria-selected="false" tabindex="-1">
    {#if kids.length > 0 || sessions.length > 0}
      <button class="twist" onclick={() => toggle(a.id)} aria-label="toggle">
        <Icon name={isOpen ? 'chevronDown' : 'chevronRight'} size={12} />
      </button>
    {:else}
      <span class="twist-spacer"></span>
    {/if}
    <span class="avatar">{a.avatar || a.name.slice(0, 1)}</span>
    <span class="who grow">
      <span class="name">{a.name}</span>
      <span class="title dim">{a.title}</span>
    </span>
    {#if a.schedule?.enabled}
      <span class="badge" title="scheduled"><Icon name="clock" size={11} /></span>
    {/if}
    {#if running > 0}
      <span class="chip accent" title="active runs">{running}●</span>
    {/if}
    <span class="state {a.status}" title={a.status}></span>
    <button class="icon-btn small" onclick={(e) => menu(e, a)} aria-label="agent menu">
      <Icon name="dot" size={14} />
    </button>
  </div>
  {#if isOpen}
    {#each sessions as s (s.id)}
      <button
        class="session-row"
        class:selected={swarm.selectedSessionId === s.id}
        style="padding-left:{depth * 14 + 30}px"
        onclick={() => (swarm.selectedSessionId = s.id)}
      >
        <Icon name="terminal" size={12} />
        <span class="grow mono">{s.title || s.provider}</span>
        <span class="state {ws.statusMap[s.id] ?? s.status}"></span>
      </button>
    {/each}
    {#each kids as k (k.id)}
      {@render node(k, depth + 1)}
    {/each}
  {/if}
{/snippet}

<style>
  .tree {
    overflow: auto;
    height: 100%;
    padding: 4px 0;
  }
  .pad {
    padding: 12px;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 30px;
    cursor: default;
  }
  .row:hover {
    background: color-mix(in srgb, var(--text-dim) 10%, transparent);
  }
  .twist,
  .twist-spacer {
    width: 16px;
    height: 16px;
    display: grid;
    place-items: center;
    border: none;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    flex: none;
  }
  .avatar {
    width: 20px;
    height: 20px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    font-size: 12px;
    background: color-mix(in srgb, var(--accent) 22%, transparent);
    flex: none;
  }
  .who {
    display: flex;
    flex-direction: column;
    line-height: 1.1;
    overflow: hidden;
  }
  .name {
    font-size: 12.5px;
    font-weight: 600;
  }
  .title {
    font-size: 10.5px;
  }
  .state {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    flex: none;
    background: var(--text-dim);
  }
  .state.active,
  .state.running,
  .state.working {
    background: var(--status-working);
  }
  .state.idle,
  .state.reconnectable {
    background: var(--status-idle, var(--text-dim));
  }
  .state.exited,
  .state.paused {
    background: var(--status-exited);
  }
  .badge {
    color: var(--text-dim);
    display: grid;
    place-items: center;
  }
  .session-row {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 26px;
    width: 100%;
    border: none;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    text-align: left;
    font-size: 11.5px;
  }
  .session-row:hover {
    background: color-mix(in srgb, var(--text-dim) 10%, transparent);
    color: var(--text);
  }
  .session-row.selected {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    color: var(--accent);
  }
</style>
