<!--
  The Personal Agents rail: one large avatar tile per agent — the main agent
  first, with a star — each with a live presence dot (working / needs you /
  idle) and its name. "+" quick-creates an agent. ⌘1–9 jump to an agent.
-->
<script lang="ts">
  import Icon from '../../lib/components/Icon.svelte';
  import { personalAgents, type AgentPresence } from '../../lib/stores/personalAgents.svelte';
  import type { PersonalAgent } from '../../lib/api/types';
  import AgentAvatar from './AgentAvatar.svelte';

  interface Props {
    agents: PersonalAgent[];
    selectedId: string | null;
    onselect: (id: string) => void;
    oncreate: () => void;
  }
  let { agents, selectedId, onselect, oncreate }: Props = $props();

  const PRESENCE: Record<AgentPresence, string> = {
    needs_you: 'Needs you',
    working: 'Working…',
    idle: 'Idle',
  };

  function tip(a: PersonalAgent, p: AgentPresence, main: boolean): string {
    const parts = [a.name];
    if (main) parts.push('Main agent');
    parts.push(a.enabled ? PRESENCE[p] : 'Paused');
    return parts.join(' · ');
  }

  function onkey(e: KeyboardEvent): void {
    // ⌘1–9 switches agents (rail order).
    if (!(e.metaKey || e.ctrlKey) || e.altKey || e.shiftKey) return;
    const n = Number(e.key);
    if (!Number.isInteger(n) || n < 1 || n > 9) return;
    const a = agents[n - 1];
    if (!a) return;
    e.preventDefault();
    onselect(a.id);
  }
</script>

<svelte:window onkeydown={onkey} />

<nav class="rail" aria-label="Personal agents" data-testid="pa-rail">
  <button type="button" class="rail-add" aria-label="New agent" title="New agent" data-testid="pa-rail-add" onclick={oncreate}>
    <Icon name="plus" size={16} />
  </button>
  <ul class="rail-list">
    {#each agents as a, i (a.id)}
      {@const p = personalAgents.presence(a.id)}
      {@const main = !!personalAgents.autonomyById[a.id]?.primary}
      <li>
        <button
          type="button"
          class="tile"
          class:selected={a.id === selectedId}
          class:paused={!a.enabled}
          aria-current={a.id === selectedId ? 'true' : undefined}
          aria-label={a.name}
          title="{tip(a, p, main)}{i < 9 ? ` (⌘${i + 1})` : ''}"
          data-testid="pa-rail-{a.id}"
          data-presence={p}
          onclick={() => onselect(a.id)}
        >
          <span class="tile-face">
            <AgentAvatar avatar={a.avatar} name={a.name} size={44} />
            {#if main}<span class="tile-star" aria-hidden="true"><Icon name="star" size={12} /></span>{/if}
            {#if a.enabled}<span class="tile-dot {p}" aria-hidden="true"></span>{/if}
          </span>
          <span class="tile-name ellipsis" dir="auto">{a.name}</span>
        </button>
      </li>
    {/each}
  </ul>
</nav>

<style>
  .rail {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    inline-size: 84px;
    flex: none;
    padding-block: 10px;
    border-inline-end: 1px solid var(--border);
    overflow-y: auto;
    min-block-size: 0;
  }
  .rail-add {
    display: grid;
    place-items: center;
    inline-size: 44px;
    block-size: 44px;
    border-radius: var(--radius-m);
    border: 1px dashed var(--border-strong);
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
  }
  .rail-add:hover {
    color: var(--text);
    background: var(--hover);
  }
  .rail-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
    inline-size: 100%;
  }
  .tile {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
    inline-size: 100%;
    padding: 6px 4px;
    border: 0;
    border-radius: var(--radius-m);
    background: transparent;
    color: var(--text-dim);
    font: inherit;
    cursor: pointer;
  }
  .tile:hover {
    background: var(--hover);
    color: var(--text);
  }
  .tile:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: -2px;
  }
  .tile.selected {
    background: var(--accent-soft);
    color: var(--text);
  }
  .tile.selected :global(.pa-avatar) {
    border-color: var(--accent-line-strong);
  }
  .tile.paused .tile-face {
    opacity: 0.55;
  }
  .tile-face {
    position: relative;
    display: inline-grid;
  }
  .tile-star {
    position: absolute;
    inset-block-start: -4px;
    inset-inline-end: -4px;
    display: grid;
    place-items: center;
    inline-size: 18px;
    block-size: 18px;
    border-radius: 50%;
    background: var(--accent-solid);
    color: var(--accent-contrast);
    border: 2px solid var(--surface);
  }
  .tile-dot {
    position: absolute;
    inset-block-end: -2px;
    inset-inline-end: -2px;
    inline-size: 12px;
    block-size: 12px;
    border-radius: 50%;
    border: 2px solid var(--surface);
    background: var(--text-dim);
  }
  .tile-dot.working {
    background: var(--success);
  }
  .tile-dot.needs_you {
    background: var(--warning);
  }
  @media (prefers-reduced-motion: no-preference) {
    .tile-dot.working {
      animation: otto-pulse 1.6s ease-in-out infinite alternate;
    }
  }
  .tile-name {
    max-inline-size: 76px;
    font-size: var(--fs-xs);
    text-align: center;
  }
  @media (max-width: 640px) {
    .rail {
      flex-direction: row;
      inline-size: 100%;
      padding: 8px 12px;
      border-inline-end: 0;
      border-block-end: 1px solid var(--border);
      overflow-x: auto;
      overflow-y: hidden;
    }
    .rail-list {
      flex-direction: row;
      inline-size: auto;
    }
    .tile {
      inline-size: 68px;
    }
  }
</style>
