<script lang="ts">
  // Status bar: working agents · needs you · event stream · current branch ·
  // network listener · clock. While the events socket is down the live counts
  // are stale: the working dot stops pulsing and the stream says
  // "Reconnecting…" (patterns.md §1, "Stale is a state").
  import Icon from '../lib/components/Icon.svelte';
  import { ws } from '../lib/stores/workspace.svelte';
  import { agentRunsWorking } from '../lib/stores/agentRunsWorking.svelte';
  import { auth } from '../lib/stores/auth.svelte';
  import { events } from '../lib/events.svelte';
  import { pageChrome } from '../lib/stores/pageChrome.svelte';
  import { git } from '../lib/stores/git.svelte';
  import { router } from '../lib/router.svelte';
  import { nowMinute } from '../lib/stores/now.svelte';
  import { daemonStatus } from '../modules/settings/daemonStatus.svelte';
  import { listenerIndicator } from '../modules/settings/daemonIndicators';

  const live = $derived(events.state === 'connected');

  /** "· N need you" → narrow the sidebar to the sessions waiting on the user. */
  function showNeedsYou(): void {
    ws.needsYouFilter = true;
    if (router.module !== 'agents') router.go('agents');
  }

  // Root-only daemon facts, read once and never polled (daemonStatus): the
  // secret store's mode, and what the network listener is BOUND to. `/meta`
  // is re-read on boot, on a daemon restart (new boot id) and after a Daemon
  // save, so re-reading the posture with it is what clears "after restart".
  $effect(() => {
    daemonStatus.ensureSecrets(auth.me?.id, auth.isRoot);
  });
  $effect(() => {
    void auth.meta;
    daemonStatus.refreshPosture(auth.isRoot);
  });
  const listener = $derived(listenerIndicator(auth.meta?.network_listener, daemonStatus.posture));

  // The shared clock's minute tick (skips while the window is hidden) — no
  // interval of our own, and the label turns over on the minute.
  const clock = $derived(
    new Date(nowMinute()).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }),
  );
</script>

<!-- A page may trim the bar (Threads); a lost connection always shows it. -->
{#if !pageChrome.hideStatusBar || events.state !== 'connected'}
<footer class="statusbar chrome-material">
  <div class="sb-group">
    <button
      class="sb-item sb-btn"
      onclick={() => router.go('agents')}
      title={live ? 'Agents working — open Agents' : 'Agents working (last known — reconnecting) — open Agents'}
    >
      <span class="working-dot" class:on={ws.workingCount + agentRunsWorking.count > 0} class:stale={!live} aria-hidden="true"></span>
      {ws.workingCount + agentRunsWorking.count} working
    </button>
    <!-- Persistent live region: the needs-you count appears/changes silently
         otherwise. Only that count is announced (the working count churns). -->
    <span class="sr-only" role="status" aria-live="polite">
      {ws.needsYouCount > 0 ? `${ws.needsYouCount} ${ws.needsYouCount === 1 ? 'session needs' : 'sessions need'} you` : ''}
    </span>
    {#if ws.needsYouCount > 0}
      <button
        class="sb-item sb-btn needs-you"
        onclick={showNeedsYou}
        title="Show only the sessions waiting on you"
        data-testid="statusbar-needs-you"
      >
        · {ws.needsYouCount} {ws.needsYouCount === 1 ? 'needs' : 'need'} you
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
        <Icon name="branch" size={12} />
        <span class="sb-branch-name">{git.primaryStatus.branch}</span>
        {#if git.primaryStatus.ahead > 0}<span class="dim">↑{git.primaryStatus.ahead}</span>{/if}
        {#if git.primaryStatus.behind > 0}<span class="dim">↓{git.primaryStatus.behind}</span>{/if}
      </button>
    {/if}
    {#if daemonStatus.secretsUnencrypted}
      <button
        class="sb-item sb-btn sb-warn"
        onclick={() => router.go('settings/trust-safety')}
        title="Integration secrets are stored in a plaintext file. Open Trust & safety to encrypt them"
        aria-label="Secrets not encrypted — open Trust & safety"
        data-testid="statusbar-secrets-warning"
      >
        <Icon name="warning" size={12} />
        <span class="sb-warn-text">Secrets not encrypted</span>
      </button>
    {/if}
    <span class="sb-item dim" class:pending={listener.pending} title={listener.title} data-testid="statusbar-listener">
      <Icon name="globe" size={12} />
      {listener.text}
    </span>
    <span class="sb-item dim">{clock}</span>
  </div>
</footer>
{/if}

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
    gap: 4px;
  }
  .sb-btn {
    border: none;
    background: transparent;
    cursor: pointer;
    font-size: var(--fs-xs);
    color: var(--text);
    padding: 1px 4px;
    border-radius: var(--radius-s);
  }
  .sb-btn:hover {
    background: var(--hover);
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
    animation: otto-pulse 1.6s ease-in-out infinite;
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
  /* Root-only: the secret store is unencrypted (Trust & safety fixes it). */
  .sb-warn {
    color: var(--warning);
    font-weight: 600;
  }
  /* A saved listener setting waiting on a daemon restart. */
  .pending {
    color: var(--warning);
  }
  /* Phone: the icon alone (the button keeps its aria-label + title). */
  @media (max-width: 640px) {
    .sb-warn-text {
      display: none;
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
</style>
