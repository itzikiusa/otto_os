<script lang="ts">
  // Tiled view: every active session in the workspace shown at once as a live
  // terminal tile (loom-style). Auto-flowing grid; the focused tile is the one
  // that keyboard input / new commands target.
  import SessionView from './SessionView.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { ui } from '../../lib/stores/ui.svelte';

  // Column count scales with tile count: 1→1, 2-4→2, 5-9→3, 10+→4.
  const cols = $derived.by(() => {
    const n = ws.mainSessions.length;
    if (n <= 1) return 1;
    if (n <= 4) return 2;
    if (n <= 9) return 3;
    return 4;
  });

  // Explicit row count so every tile fits the viewport (no clipped bottom row).
  const rows = $derived(Math.max(1, Math.ceil(ws.mainSessions.length / cols)));

  // When a tile is maximized, show only it (zoomed in).
  const maxed = $derived(
    ws.maximizedId ? ws.mainSessions.find((s) => s.id === ws.maximizedId) ?? null : null,
  );
</script>

{#if ws.mainSessions.length === 0}
  <div class="tiled-empty">
    <EmptyState
      icon="terminal"
      title="No active sessions"
      body="Spawn an agent or a shell. In tiled view you'll see every session at once."
      actionLabel="New Session  ⌘T"
      onaction={() => (ui.newSessionOpen = true)}
    />
  </div>
{:else if maxed}
  <div class="tiled single">
    {#key maxed.id}
      <SessionView
        sessionId={maxed.id}
        focused={true}
        showClose={false}
        showZoom={true}
        onfocus={() => ws.openSession(maxed.id)}
        onclosepane={() => {}}
      />
    {/key}
  </div>
{:else}
  <div
    class="tiled"
    style="grid-template-columns: repeat({cols}, minmax(0, 1fr)); grid-template-rows: repeat({rows}, minmax(0, 1fr));"
  >
    {#each ws.mainSessions as s, i (s.id)}
      <SessionView
        sessionId={s.id}
        focused={ws.activeSessionId === s.id}
        showClose={false}
        showZoom={true}
        onfocus={() => {
          ws.openSession(s.id);
          // make this the focused/active target without leaving tiled view
          ws.focusedPane = 0;
        }}
        onclosepane={() => {}}
      />
    {/each}
  </div>
{/if}

<style>
  .tiled {
    display: grid;
    gap: 8px;
    height: 100%;
    padding: 8px;
    overflow: auto;
    grid-auto-rows: minmax(220px, 1fr);
  }
  .tiled.single {
    display: block;
    overflow: hidden;
  }
  .tiled-empty {
    height: 100%;
  }
</style>
