<script lang="ts">
  // Agents box: the live session roster straight from the workspace store (no
  // fetch of its own — `ws` is already kept current by the daemon WS), with
  // working / needs-you / idle counts and a click-to-open row per session.
  import Icon from '../../../lib/components/Icon.svelte';
  import StatusDot from '../../../lib/components/StatusDot.svelte';
  import ProviderIcon from '../../../lib/components/ProviderIcon.svelte';
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import { ws } from '../../../lib/stores/workspace.svelte';
  import { now } from '../../../lib/stores/now.svelte';
  import { relTime } from '../../mission-control/lib';
  import type { HomeBox } from '../home.svelte';

  interface Props {
    box: HomeBox;
    zoomed: boolean;
    tick: number;
  }
  let { box: _box, zoomed: _zoomed, tick: _tick }: Props = $props();

  const sessions = $derived(
    [...ws.plainAgentSessions].sort((a, b) => {
      // Working first, then needs-you, then most recently active.
      const rank = (id: string): number =>
        ws.statusMap[id] === 'working' ? 0 : ws.needsYou[id] ? 1 : 2;
      return rank(a.id) - rank(b.id) || Date.parse(b.last_active_at) - Date.parse(a.last_active_at);
    }),
  );
  const idle = $derived(sessions.filter((s) => (ws.statusMap[s.id] ?? s.status) === 'idle').length);
  const working = $derived(ws.workingCount);
  const needsYou = $derived(ws.needsYouCount);
</script>

<div class="sessions">
  <div class="stats">
    <div class="stat"><span class="n working">{working}</span><span class="l">working</span></div>
    <div class="stat"><span class="n needs">{needsYou}</span><span class="l">need you</span></div>
    <div class="stat"><span class="n">{idle}</span><span class="l">idle</span></div>
    <div class="stat"><span class="n">{sessions.length}</span><span class="l">open</span></div>
  </div>
  {#if sessions.length === 0}
    <EmptyState icon="terminal" title="No open agent sessions" body="Start one from the Agents tab (⌘T)." />
  {:else}
    <ul class="rows">
      {#each sessions as s (s.id)}
        <li>
          <button class="row" onclick={() => ws.navigateToSession(s.id)} title="Open {s.title}">
            <StatusDot status={ws.statusMap[s.id] ?? s.status} needsYou={ws.needsYou[s.id] === true} />
            <ProviderIcon provider={s.provider} size={12} />
            <span class="title ellipsis">{s.title}</span>
            {#if ws.needsYou[s.id]}<span class="pill needs">needs you</span>{/if}
            <span class="ago">{now() && relTime(s.last_active_at)}</span>
            <Icon name="external" size={11} />
          </button>
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .sessions {
    display: flex;
    flex-direction: column;
    gap: 8px;
    height: 100%;
    min-height: 0;
  }
  .stats {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 6px;
  }
  .stat {
    display: flex;
    flex-direction: column;
    align-items: center;
    padding: 6px 4px;
    background: var(--surface-2);
    border-radius: var(--radius-s);
  }
  .n {
    font-size: 18px;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
    line-height: 1.1;
  }
  .n.working {
    color: var(--status-working);
  }
  .n.needs {
    color: var(--status-warn);
  }
  .l {
    font-size: 10px;
    color: var(--text-dim);
  }
  .rows {
    list-style: none;
    margin: 0;
    padding: 0;
    overflow-y: auto;
    min-height: 0;
    flex: 1;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 6px 8px;
    border: none;
    background: transparent;
    color: var(--text);
    border-radius: var(--radius-s);
    cursor: pointer;
    text-align: start;
    font: inherit;
    font-size: 12px;
  }
  .row:hover {
    background: color-mix(in srgb, var(--text-dim) 12%, transparent);
  }
  .title {
    flex: 1;
    min-width: 0;
  }
  .pill {
    padding: 0 6px;
    border-radius: 999px;
    font-size: 10px;
  }
  .pill.needs {
    color: var(--status-warn);
    background: var(--status-warn-soft);
  }
  .ago {
    color: var(--text-dim);
    font-size: 11px;
    font-variant-numeric: tabular-nums;
  }
  .ellipsis {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
