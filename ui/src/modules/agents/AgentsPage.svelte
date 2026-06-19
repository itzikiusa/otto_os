<script lang="ts">
  // Agent Mode: tabbed split panes OR a tiled grid of every active session.
  import Splits from './Splits.svelte';
  import TiledView from './TiledView.svelte';
  import FirstRunCoach from './FirstRunCoach.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { ui } from '../../lib/stores/ui.svelte';

  const tiled = $derived(ws.viewMode === 'tiled');

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
  {#if ws.sessionsLoading && ws.sessions.length === 0}
    <div style="padding: 16px">
      <Skeleton rows={3} height={48} />
    </div>
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

<style>
  .agents {
    height: 100%;
    min-height: 0;
  }
</style>
