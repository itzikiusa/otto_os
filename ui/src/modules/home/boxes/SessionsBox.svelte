<script lang="ts">
  // Agents box: the live session roster straight from the workspace store (no
  // fetch of its own — `ws` is already kept current by the daemon WS), with
  // working / needs-you / idle counts and a click-to-open row per session.
  import Icon from '../../../lib/components/Icon.svelte';
  import StatusDot from '../../../lib/components/StatusDot.svelte';
  import ProviderIcon from '../../../lib/components/ProviderIcon.svelte';
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import { ws } from '../../../lib/stores/workspace.svelte';
  import { ui } from '../../../lib/stores/ui.svelte';
  import { router } from '../../../lib/router.svelte';
  import { events } from '../../../lib/events.svelte';
  import { sessionState, type SessionStateInfo } from '../../../lib/status';
  import type { Session } from '../../../lib/api/types';
  import { now } from '../../../lib/stores/now.svelte';
  import { relTime } from '../../mission-control/lib';
  import type { HomeBox } from '../home.svelte';

  interface Props {
    box: HomeBox;
    zoomed: boolean;
    tick: number;
  }
  let { box: _box, zoomed: _zoomed, tick: _tick }: Props = $props();

  // The one session state (lib/status.ts) — the same dot/label the sidebar,
  // tabs and tiles show, incl. suspended (resumable) and stale (socket down).
  const stateOf = (s: Session): SessionStateInfo =>
    sessionState(s, ws.statusMap[s.id], ws.needsYou[s.id] === true, { stale: events.state !== 'connected' });
  const sessions = $derived(
    [...ws.plainAgentSessions].sort((a, b) => {
      // Needs-you first (blocked on you), then working, then most recently active.
      const rank = (s: Session): number => {
        const k = stateOf(s).key;
        return k === 'needs-you' ? 0 : k === 'working' ? 1 : 2;
      };
      return rank(a) - rank(b) || Date.parse(b.last_active_at) - Date.parse(a.last_active_at);
    }),
  );
  const idle = $derived(sessions.filter((s) => (ws.statusMap[s.id] ?? s.status) === 'idle').length);
  const working = $derived(ws.workingCount);
  const needsYou = $derived(ws.needsYouCount);
</script>

<div class="sessions">
  <div class="stats">
    <div class="stat"><span class="n" class:working={working > 0}>{working}</span><span class="l">working</span></div>
    <div class="stat"><span class="n" class:needs={needsYou > 0}>{needsYou}</span><span class="l">need you</span></div>
    <div class="stat"><span class="n">{idle}</span><span class="l">idle</span></div>
    <div class="stat"><span class="n">{sessions.length}</span><span class="l">open</span></div>
  </div>
  {#if sessions.length === 0}
    <!-- A quiet secondary CTA: the page's one primary is "Add widget". -->
    <EmptyState icon="terminal" title="No open agent sessions" body="Start Claude, Codex or a shell in this workspace.">
      <button class="btn small" onclick={() => (ui.newSessionOpen = true)}><Icon name="plus" size={12} />New session</button>
    </EmptyState>
  {:else}
    <ul class="rows">
      {#each sessions as s (s.id)}
        <li>
          <button class="srow" onclick={() => ws.navigateToSession(s.id)} title="Open {s.title}">
            <StatusDot state={stateOf(s)} />
            <ProviderIcon provider={s.provider} size={12} />
            <span class="title ellipsis">{s.title}</span>
            {#if ws.needsYou[s.id]}<span class="pill needs">needs you</span>{/if}
            <span class="ago" title={new Date(s.last_active_at).toLocaleString()}>{now() && relTime(s.last_active_at)}</span>
            <span class="go" aria-hidden="true"><Icon name="chevronRight" size={12} /></span>
          </button>
        </li>
      {/each}
    </ul>
    <!-- Anchors the card's foot (a short roster no longer floats in a tall,
         empty box) and gives the widget an obvious way into the module. -->
    <button class="foot" onclick={() => router.go('agents')}>
      Open Agents<Icon name="chevronRight" size={12} />
    </button>
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
  /* Widget figures: number over label, split by hairlines — no tile fills
     (the calm desktop-widget look shared by every Home box). */
  .stats {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 0;
  }
  .stat {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    min-width: 0;
    padding: 2px 12px;
    border-inline-start: 1px solid var(--separator);
  }
  .stat:first-child {
    padding-inline-start: 2px;
    border-inline-start: none;
  }
  .n {
    font-size: var(--fs-xl);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    line-height: 1.1;
  }
  /* Tone only when there is something to see: a green "0 working" was noise. */
  .n.working {
    color: var(--success);
  }
  .n.needs {
    color: var(--warning);
  }
  .l {
    font-size: var(--fs-xs);
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
  .srow {
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
    font-size: var(--fs-s);
  }
  .srow:hover {
    background: var(--hover);
  }
  /* The row's "opens" cue shows on hover/focus only — a permanent icon on
     every row was noise. */
  .go {
    display: inline-flex;
    color: var(--text-dim);
    opacity: 0;
    transition: opacity 120ms ease-out;
  }
  .srow:hover .go,
  .srow:focus-visible .go {
    opacity: 1;
  }
  :global([dir='rtl']) .go {
    transform: scaleX(-1);
  }
  .foot {
    flex: none;
    align-self: flex-start;
    display: inline-flex;
    align-items: center;
    gap: 2px;
    padding: 2px 0;
    border: none;
    background: none;
    color: var(--accent-text);
    font: inherit;
    font-size: var(--fs-s);
    cursor: pointer;
  }
  .foot:hover {
    text-decoration: underline;
  }
  :global([dir='rtl']) .foot :global(svg) {
    transform: scaleX(-1);
  }
  .title {
    flex: 1;
    min-width: 0;
  }
  .pill {
    flex: none;
    padding: 0 6px;
    border-radius: 999px;
    font-size: var(--fs-xs);
  }
  .pill.needs {
    color: var(--status-warn);
    background: var(--status-warn-soft);
  }
  .ago {
    color: var(--text-dim);
    font-size: var(--fs-xs);
    font-variant-numeric: tabular-nums;
  }
  .ellipsis {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
