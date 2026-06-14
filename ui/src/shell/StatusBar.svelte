<script lang="ts">
  // Status bar: working agents · network listener · current branch · clock.
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

  const clock = $derived(
    now.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }),
  );
</script>

<footer class="statusbar">
  <div class="sb-group">
    <span class="sb-item" title="Agents working">
      <span class="working-dot" class:on={ws.workingCount > 0}></span>
      {ws.workingCount} working
    </span>
    <span class="sb-item dim" title="Event stream: {events.state}">
      <span class="conn-dot {events.state}"></span>
      {events.state === 'connected' ? 'live' : events.state}
    </span>
  </div>

  <div class="sb-group">
    {#if git.primaryStatus}
      <button class="sb-item sb-btn" onclick={() => router.go('git')} title="Current branch">
        <Icon name="branch" size={11} />
        {git.primaryStatus.branch}
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
    border-top: 1px solid var(--border);
    background: var(--bg);
    font-size: 11px;
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
    font-size: 11px;
    color: var(--text);
    padding: 1px 4px;
    border-radius: 4px;
  }
  .sb-btn:hover {
    background: var(--surface-2);
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
    background: #febc2e;
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
