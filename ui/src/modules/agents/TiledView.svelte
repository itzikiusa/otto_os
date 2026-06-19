<script lang="ts">
  // Tiled view: every active session in the workspace shown at once (loom-style).
  // Auto-flowing grid; the focused tile is the one that keyboard input / new
  // commands target.
  //
  // Live-tile budget — why this matters: each mounted SessionView opens a live
  // terminal WebSocket, and the daemon's ensure_live RESUMES a suspended session
  // on attach (~200 MB/agent). Mounting one terminal per session would therefore
  // wake every suspended agent the instant the tiled view opens, defeating the
  // idle-suspend memory design. Instead we only keep a bounded set of tiles
  // "live": tiles that are actually scrolled into view (IntersectionObserver),
  // capped at MAX_LIVE_TILES, plus the focused tile and any the user explicitly
  // attached. Off-screen / over-budget tiles render a lightweight placeholder
  // ("click to attach") and open no socket. Demoting a tile unmounts its
  // SessionView → Terminal's cleanup closes the WS → memory is reclaimed.
  import SessionView from './SessionView.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import StatusDot from '../../lib/components/StatusDot.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { ui } from '../../lib/stores/ui.svelte';

  // Max number of tiles allowed to hold a live terminal/WS at once. Visible,
  // recently-focused tiles win the budget; everything else stays a placeholder.
  const MAX_LIVE_TILES = 6;

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

  // ── Live-tile bookkeeping ─────────────────────────────────────────────────
  // Set of session ids currently scrolled into the viewport (driven by the
  // IntersectionObserver below).
  let visible = $state(new Set<string>());
  // Tiles the user explicitly attached via "click to attach" — pinned live even
  // if scrolled off-screen, so a deliberate attach is never silently dropped.
  let pinned = $state(new Set<string>());
  // Per-tile elements we observe for visibility.
  const tileEls = new Map<string, HTMLElement>();
  let observer: IntersectionObserver | null = null;

  // Grid scroll container — the IntersectionObserver root.
  let gridEl: HTMLElement | null = $state(null);

  function observeTile(node: HTMLElement, id: string) {
    tileEls.set(id, node);
    observer?.observe(node);
    return {
      destroy() {
        observer?.unobserve(node);
        tileEls.delete(id);
      },
    };
  }

  $effect(() => {
    // Observe the scroll container (the grid itself) as the viewport so we only
    // count tiles the user can actually see right now. Re-runs when gridEl binds
    // (it's null until the grid mounts).
    const root = gridEl;
    if (!root) return;
    const obs = new IntersectionObserver(
      (entries) => {
        const next = new Set(visible);
        for (const e of entries) {
          const id = (e.target as HTMLElement).dataset.tileId;
          if (!id) continue;
          if (e.isIntersecting) next.add(id);
          else next.delete(id);
        }
        visible = next;
      },
      { root, threshold: 0.01 },
    );
    observer = obs;
    for (const node of tileEls.values()) obs.observe(node);
    return () => {
      obs.disconnect();
      if (observer === obs) observer = null;
    };
  });

  // Drop pins/visibility for sessions that no longer exist so the sets don't
  // leak and stale ids never count against the live budget.
  $effect(() => {
    const ids = new Set(ws.mainSessions.map((s) => s.id));
    for (const id of [...pinned]) if (!ids.has(id)) pinned.delete(id);
    for (const id of [...visible]) if (!ids.has(id)) visible.delete(id);
  });

  // Decide which tiles get a live terminal. The focused/active tile is always
  // live (keeps the normal attach path intact), then pinned tiles, then visible
  // tiles in grid order — capped at MAX_LIVE_TILES.
  const liveIds = $derived.by(() => {
    const live = new Set<string>();
    const active = ws.activeSessionId;
    if (active && ws.mainSessions.some((s) => s.id === active)) live.add(active);
    // Explicit attaches next — honor the user's deliberate choice before
    // best-effort visible tiles.
    for (const s of ws.mainSessions) {
      if (live.size >= MAX_LIVE_TILES) break;
      if (pinned.has(s.id)) live.add(s.id);
    }
    for (const s of ws.mainSessions) {
      if (live.size >= MAX_LIVE_TILES) break;
      if (visible.has(s.id)) live.add(s.id);
    }
    return live;
  });

  // True when the live budget is full and some visible tiles had to stay
  // placeholders — surfaced as a hint on those tiles.
  const atCapacity = $derived(liveIds.size >= MAX_LIVE_TILES);

  function attach(id: string): void {
    // Pin so it survives scrolling, and if we're at capacity, focusing it makes
    // it the always-live active tile (evicting the least-prioritized one).
    pinned.add(id);
    pinned = new Set(pinned);
    ws.openSession(id);
    ws.focusedPane = 0;
  }
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
    bind:this={gridEl}
    style="grid-template-columns: repeat({cols}, minmax(0, 1fr)); grid-template-rows: repeat({rows}, minmax(0, 1fr));"
  >
    {#each ws.mainSessions as s (s.id)}
      <div class="tile-slot" data-tile-id={s.id} use:observeTile={s.id}>
        {#if liveIds.has(s.id)}
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
        {:else}
          <!-- Lightweight placeholder: no terminal, no WebSocket, no resume. -->
          <button
            class="tile-placeholder"
            onclick={() => attach(s.id)}
            title="Attach this session (opens its live terminal)"
          >
            <header class="ph-head">
              <StatusDot status={ws.statusMap[s.id] ?? s.status ?? 'idle'} />
              <span class="ph-title">{s.title ?? s.id}</span>
              <span class="chip ph-chip">{s.provider ?? '?'}</span>
            </header>
            <div class="ph-body">
              <Icon name="terminal" size={20} />
              <span class="ph-cta">Click to attach</span>
              {#if atCapacity}
                <span class="ph-hint">Live tiles capped at {MAX_LIVE_TILES} to save memory</span>
              {:else}
                <span class="ph-hint">Suspended — not connected</span>
              {/if}
            </div>
          </button>
        {/if}
      </div>
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
  /* Each grid cell wraps either a live SessionView or a placeholder; it is the
     element the IntersectionObserver watches. */
  .tile-slot {
    min-width: 0;
    min-height: 0;
    display: flex;
  }
  .tile-slot > :global(*) {
    flex: 1;
    min-width: 0;
    min-height: 0;
  }
  /* Placeholder: looks like a pane but holds no terminal/WS until attached. */
  .tile-placeholder {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    width: 100%;
    text-align: left;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    overflow: hidden;
    background: var(--term-bg);
    color: var(--text);
    cursor: pointer;
    transition: border-color 140ms ease-out;
  }
  .tile-placeholder:hover {
    border-color: color-mix(in srgb, var(--accent) 55%, transparent);
  }
  .ph-head {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 30px;
    padding: 0 8px 0 10px;
    background: var(--surface);
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
  }
  .ph-title {
    font-size: 12px;
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 180px;
  }
  .ph-chip {
    height: 16px;
    font-size: 9.5px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  .ph-body {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 6px;
    color: var(--text-dim);
  }
  .ph-cta {
    font-size: 12px;
    font-weight: 600;
    color: var(--text);
  }
  .ph-hint {
    font-size: 10.5px;
    color: var(--text-dim);
  }
</style>
