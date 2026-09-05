<script lang="ts">
  // Agent Mode: tabbed split panes, tiled grid, or Mission Control work queue.
  import Splits from './Splits.svelte';
  import TiledView from './TiledView.svelte';
  import MissionControl from './MissionControl.svelte';
  import FirstRunCoach from './FirstRunCoach.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { ui } from '../../lib/stores/ui.svelte';
  import { router } from '../../lib/router.svelte';
  // Importing the History module here (not only from App's `#/history` branch)
  // registers its palette command at boot, so "Go to History" works from any page.
  import '../agents/history';

  const tiled = $derived(ws.viewMode === 'tiled');
  const mission = $derived(ws.viewMode === 'mission');

  // First-run coach: a guided path from a fresh account to a launched agent.
  // Shown only on a truly empty Agents view (no agent sessions at all) and only
  // until the user dismisses it or launches a session (remembered per-machine).
  let coachDismissed = $state(
    (() => {
      try {
        return localStorage.getItem('otto_firstrun_dismissed') === '1';
      } catch {
        return false;
      }
    })(),
  );
  const showCoach = $derived(!coachDismissed && ws.agentSessions.length === 0);
</script>

<div class="agents">
  <!-- Slim header: the one place every Agents view (tabs / tiled / mission)
       shares; hosts the History entry point (⌘K "Go to History" and the
       sidebar row are the others). -->
  <div class="agents-bar">
    <button
      class="bar-btn"
      onclick={() => router.go('history')}
      title="Browse past conversations — every Claude/Codex session, resumable"
      data-testid="agents-history-btn"
    >
      <Icon name="clock" size={12} /> History
    </button>
  </div>
  <div class="agents-body">
  {#if ws.sessionsLoading && ws.sessions.length === 0}
    <div style="padding: 16px">
      <Skeleton rows={3} height={48} />
    </div>
  {:else if mission}
    <MissionControl />
  {:else if tiled}
    <TiledView />
  {:else if ws.panes.length === 0}
    {#if showCoach}
      <FirstRunCoach ondismiss={() => (coachDismissed = true)} />
    {:else if ws.activeSessions.length === 0}
      <EmptyState
        icon="terminal"
        title="No sessions yet"
        body="Spawn an agent (claude, codex) or a plain shell in this workspace. Sessions keep running even when you close the app."
        actionLabel="New Session  ⌘T"
        onaction={() => (ui.newSessionOpen = true)}
      />
    {:else}
      <EmptyState
        icon="terminal"
        title="No open tabs"
        body="Pick a session from the navigator on the left, switch to tiled view, or start a new one."
        actionLabel="New Session  ⌘T"
        onaction={() => (ui.newSessionOpen = true)}
      />
    {/if}
  {:else}
    <Splits />
  {/if}
  </div>
</div>

<style>
  .agents {
    height: 100%;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .agents-bar {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 6px;
    height: 26px;
    padding: 0 8px;
    border-bottom: 1px solid var(--border);
    background: var(--bg);
  }
  .bar-btn {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 20px;
    padding: 0 8px;
    border: 1px solid var(--border);
    border-radius: 999px;
    background: transparent;
    color: var(--text-dim);
    font: inherit;
    font-size: 11px;
    cursor: pointer;
  }
  .bar-btn:hover {
    border-color: var(--accent);
    color: var(--accent);
  }
  .agents-body {
    flex: 1;
    min-height: 0;
  }
</style>
