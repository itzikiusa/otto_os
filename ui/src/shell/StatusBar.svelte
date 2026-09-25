<script lang="ts">
  // Status bar: working agents · needs you · event stream · current branch ·
  // network listener · clock. While the events socket is down the live counts
  // are stale: the working dot stops pulsing and the stream says
  // "Reconnecting…" (patterns.md §1, "Stale is a state").
  import Icon from '../lib/components/Icon.svelte';
  import { ws } from '../lib/stores/workspace.svelte';
  import { auth } from '../lib/stores/auth.svelte';
  import { events } from '../lib/events.svelte';
  import { git } from '../lib/stores/git.svelte';
  import { router } from '../lib/router.svelte';

  let now = $state(new Date());
  $effect(() => {
    const t = setInterval(() => (now = new Date()), 15_000);
    return () => clearInterval(t);
  });

  const live = $derived(events.state === 'connected');

  /** "· N need you" → narrow the sidebar to the sessions waiting on the user. */
  function showNeedsYou(): void {
    ws.needsYouFilter = true;
    if (router.module !== 'agents') router.go('agents');
  }

  const clock = $derived(
    now.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }),
  );
</script>

<footer class="statusbar chrome-material">
  <div class="sb-group">
    <span class="sb-item" title={live ? 'Agents working' : 'Agents working (last known — reconnecting)'}>
      <span class="working-dot" class:on={ws.workingCount > 0} class:stale={!live} aria-hidden="true"></span>
      {ws.workingCount} working
    </span>
    {#if ws.needsYouCount > 0}
      <button
        class="sb-item sb-btn needs-you"
        onclick={showNeedsYou}
        title="Show only the sessions waiting on you"
        data-testid="statusbar-needs-you"
      >
        · {ws.needsYouCount} need{ws.needsYouCount === 1 ? 's' : ''} you
      </button>
    {/if}
    {#if live}
      <span class="sb-item dim" title="Event stream: live">
        <span class="conn-dot connected" aria-hidden="true"></span>
        live
      </span>
    {:else}
      <button
        class="sb-item sb-btn dim"
        onclick={() => events.reconnectNow()}
        title="Event stream: {events.state} — live status may be out of date. Click to reconnect now"
      >
        <span class="conn-dot {events.state}" aria-hidden="true"></span>
        Reconnecting…
      </button>
    {/if}
  </div>

  <div class="sb-group">
    {#if git.primaryStatus}
      <button
        class="sb-item sb-btn sb-branch"
        onclick={() => router.go('git')}
        title={`Current branch: ${git.primaryStatus.branch} — open Git`}
      >
        <Icon name="branch" size={11} />
        <span class="sb-branch-name">{git.primaryStatus.branch}</span>
        {#if git.primaryStatus.ahead > 0}<span class="dim">↑{git.primaryStatus.ahead}</span>{/if}
        {#if git.primaryStatus.behind > 0}<span class="dim">↓{git.primaryStatus.behind}</span>{/if}
      </button>
    {/if}
    <span class="sb-item dim" title="Network listener">
      <Icon name="globe" size={11} />
      {auth.meta?.network_listener ? 'network' : 'loopback'}
    </span>
    <span class="sb-item dim">{clock}</span>
  </div>
</footer>

<style>
  .statusbar {
    height: 24px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 12px;
    /* Chrome: toolbar glass over the ambient (.chrome-material). */
    border-top: 1px solid var(--separator);
    font-size: var(--fs-xs);
    color: var(--text);
    flex-shrink: 0;
  }
  .sb-group {
    display: flex;
    align-items: center;
    gap: 14px;
  }
  .sb-item {
    display: inline-flex;
    align-items: center;
    gap: 5px;
  }
  .sb-btn {
    border: none;
    background: transparent;
    cursor: pointer;
    font-size: var(--fs-xs);
    color: var(--text);
    padding: 1px 4px;
    border-radius: 4px;
  }
  .sb-btn:hover {
    background: var(--surface-2);
  }
  /* A long branch name (feature/…/…) ellipsizes instead of running into the
     docked "Ask Otto" chip centred in this bar; the tooltip has it whole. */
  .sb-branch {
    min-width: 0;
  }
  .sb-branch-name {
    max-width: 220px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .working-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--status-idle);
  }
  .working-dot.on {
    background: var(--status-working);
    animation: pulse 1.6s ease-in-out infinite;
  }
  /* Stale: last known count, not live — hollow ring, no pulse. */
  .working-dot.on.stale {
    background: transparent;
    box-shadow: inset 0 0 0 1.5px var(--status-working);
    animation: none;
  }
  @media (prefers-reduced-motion: reduce) {
    .working-dot.on {
      animation: none;
    }
  }
  .needs-you {
    color: var(--warning);
    font-weight: 600;
  }
  .conn-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--status-exited);
  }
  .conn-dot.connected {
    background: var(--status-working);
  }
  .conn-dot.connecting {
    background: var(--status-warn);
  }
  @keyframes pulse {
    0%,
    100% {
      opacity: 1;
    }
    50% {
      opacity: 0.4;
    }
  }
</style>
