<script lang="ts">
  // Agent Mode: tabbed split panes, tiled grid, or Mission Control work queue.
  import Splits from './Splits.svelte';
  import TiledView from './TiledView.svelte';
  import MissionControl from './MissionControl.svelte';
  import FirstRunCoach from './FirstRunCoach.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { ui } from '../../lib/stores/ui.svelte';

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

  // Never open onto a "pick one" void when there are sessions: once per
  // workspace, if its restored layout has no panes, open the most recently
  // active session. Only on ARRIVAL — closing the last tab later leaves the
  // empty state alone (the user just chose that).
  let autoOpenedFor: string | null | undefined = undefined;
  $effect(() => {
    if (!ws.layoutReady || ws.sessionsLoading) return;
    const key = ws.currentId;
    if (autoOpenedFor === key) return;
    autoOpenedFor = key;
    if (ws.panes.length > 0 || tiled || mission) return;
    const latest = [...ws.mainSessions].sort(
      (a, b) => Date.parse(b.last_active_at) - Date.parse(a.last_active_at),
    )[0];
    if (latest) ws.navigateToSession(latest.id);
  });
</script>

<div class="agents">
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
        body="Start an agent (Claude, Codex) or a plain shell in this workspace (⌘T). Sessions keep running even when you close the app."
        variant="page"
        actionIcon="plus"
        actionLabel="New session…"
        onaction={() => (ui.newSessionOpen = true)}
      />
    {:else}
      <EmptyState
        icon="terminal"
        title="No open tabs"
        body="Pick a session in the sidebar, switch to tiled view, or start a new one (⌘T)."
        variant="page"
        actionIcon="plus"
        actionLabel="New session…"
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
  .agents-body {
    flex: 1;
    min-height: 0;
  }
</style>
