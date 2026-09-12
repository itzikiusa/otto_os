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
  import { winKey } from '../../lib/win';
  import { MAX_PANES, applyTileOrder } from '../../lib/stores/splitLayout';
  import { layout } from '../../lib/stores/splitLayout.svelte';

  // Max number of tiles allowed to hold a live terminal/WS at once. Visible,
  // recently-focused tiles win the budget; everything else stays a placeholder.
  // ONE source of truth with the split view's pane cap.
  const MAX_LIVE_TILES = MAX_PANES;

  // C3a: the user's drag order on top of the store order (unknown ids append,
  // dead ids are skipped) — everything below counts/lays out `ordered`.
  const ordered = $derived(applyTileOrder(ws.mainSessions, layout.tileOrder));

  // Column count scales with tile count: 1→1, 2-4→2, 5-9→3, 10+→4.
  const cols = $derived.by(() => {
    const n = ordered.length;
    if (n <= 1) return 1;
    if (n <= 4) return 2;
    if (n <= 9) return 3;
    return 4;
  });

  // Explicit row count so every tile fits the viewport (no clipped bottom row).
  const rows = $derived(Math.max(1, Math.ceil(ordered.length / cols)));

  // ── Resizable columns / rows ─────────────────────────────────────────────
  // The grid's tracks are `fr` weights the user can drag (a divider sits
  // between every two columns and every two rows, spanning the whole grid), so
  // a tile can be made wide or tall without leaving the tiled view. Weights are
  // kept per grid SHAPE (`cols×rows`) and per workspace — a 2×2 arrangement
  // remembers its own sizes when a third row appears and disappears.
  type Tracks = { cols: number[]; rows: number[] };
  const MIN_TRACK_PX = 120;
  const tracksKey = (): string => winKey(`otto_tile_tracks_${ws.currentId ?? 'scratch'}`);
  function loadTracks(): Record<string, Tracks> {
    try {
      const raw = JSON.parse(localStorage.getItem(tracksKey()) ?? '{}') as Record<string, Tracks>;
      return raw && typeof raw === 'object' ? raw : {};
    } catch {
      return {};
    }
  }
  let tracks = $state<Record<string, Tracks>>(loadTracks());
  $effect(() => {
    void ws.currentId;
    tracks = loadTracks();
  });
  const shapeKey = $derived(`${cols}x${rows}`);
  /** Positive finite weights of exactly `n` entries, else all-equal. */
  function normalize(fr: number[] | undefined, n: number): number[] {
    if (!fr || fr.length !== n || fr.some((f) => !Number.isFinite(f) || f <= 0)) return Array(n).fill(1);
    return fr;
  }
  const colFr = $derived(normalize(tracks[shapeKey]?.cols, cols));
  const rowFr = $derived(normalize(tracks[shapeKey]?.rows, rows));
  const gridStyle = $derived(
    `grid-template-columns: ${colFr.map((f) => `minmax(0, ${f}fr)`).join(' 8px ')};` +
      ` grid-template-rows: ${rowFr.map((f) => `minmax(220px, ${f}fr)`).join(' 8px ')};`,
  );
  function saveTracks(): void {
    try {
      localStorage.setItem(tracksKey(), JSON.stringify(tracks));
    } catch {
      /* private mode */
    }
  }
  let resizing = $state(false);
  /** Drag the divider between track `k-1` and `k` on `axis`. The cursor is
   *  measured against the two neighbouring tiles' rects, so the weights follow
   *  the pointer exactly and neither side can drop below MIN_TRACK_PX. */
  function startTrackDrag(e: PointerEvent, axis: 'cols' | 'rows', k: number): void {
    const grid = gridEl;
    if (!grid || e.button !== 0) return;
    e.preventDefault();
    const attr = axis === 'cols' ? 'data-col' : 'data-row';
    const slots = [...grid.querySelectorAll<HTMLElement>('.tile-slot')];
    const before = slots.find((el) => el.getAttribute(attr) === String(k - 1));
    const after = slots.find((el) => el.getAttribute(attr) === String(k));
    if (!before || !after) return;
    const a = before.getBoundingClientRect();
    const b = after.getBoundingClientRect();
    const start = axis === 'cols' ? a.left : a.top;
    const end = axis === 'cols' ? b.right : b.bottom;
    const span = end - start;
    if (span <= 0) return;
    const base = axis === 'cols' ? [...colFr] : [...rowFr];
    const total = base[k - 1] + base[k];
    const shape = shapeKey;
    resizing = true;
    const move = (ev: PointerEvent): void => {
      const pos = axis === 'cols' ? ev.clientX - start : ev.clientY - start;
      const lo = Math.min(MIN_TRACK_PX, span / 2);
      const clamped = Math.min(span - lo, Math.max(lo, pos));
      const next = [...base];
      next[k - 1] = (clamped / span) * total;
      next[k] = total - next[k - 1];
      const cur = tracks[shape] ?? { cols: [...colFr], rows: [...rowFr] };
      tracks = { ...tracks, [shape]: { ...cur, [axis]: next } };
    };
    const up = (): void => {
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', up);
      window.removeEventListener('pointercancel', up);
      resizing = false;
      saveTracks();
    };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', up);
    window.addEventListener('pointercancel', up);
  }
  /** Double-click a divider: every column and row back to equal shares. */
  function resetTracks(): void {
    const { [shapeKey]: _gone, ...rest } = tracks;
    tracks = rest;
    saveTracks();
  }
  /** Keyboard resize on a focused divider: arrows nudge 5 % of the pair. */
  function trackKeydown(e: KeyboardEvent, axis: 'cols' | 'rows', k: number): void {
    const dec = axis === 'cols' ? e.key === 'ArrowLeft' : e.key === 'ArrowUp';
    const inc = axis === 'cols' ? e.key === 'ArrowRight' : e.key === 'ArrowDown';
    if (!dec && !inc) return;
    e.preventDefault();
    const base = axis === 'cols' ? [...colFr] : [...rowFr];
    const total = base[k - 1] + base[k];
    const step = total * 0.05 * (inc ? 1 : -1);
    const next = [...base];
    next[k - 1] = Math.min(total * 0.9, Math.max(total * 0.1, base[k - 1] + step));
    next[k] = total - next[k - 1];
    const cur = tracks[shapeKey] ?? { cols: [...colFr], rows: [...rowFr] };
    tracks = { ...tracks, [shapeKey]: { ...cur, [axis]: next } };
    saveTracks();
  }

  // When a tile is maximized, show only it (zoomed in).
  const maxed = $derived(
    ws.maximizedId ? ordered.find((s) => s.id === ws.maximizedId) ?? null : null,
  );

  // ── Drag-to-reorder tiles ────────────────────────────────────────────────
  // Same HTML5 idiom as the tab bar: the SessionView header grip (or a
  // placeholder's own header) is the source, the tile slot is the target.
  let tileDragId = $state<string | null>(null);
  let tileDragOverId = $state<string | null>(null);

  function onTileDragStart(e: DragEvent, id: string): void {
    tileDragId = id;
    e.dataTransfer?.setData('text/plain', id);
    if (e.dataTransfer) e.dataTransfer.effectAllowed = 'move';
  }
  function onTileDragOver(e: DragEvent, id: string): void {
    if (!tileDragId || id === tileDragId) return;
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
    tileDragOverId = id;
  }
  function onTileDragLeave(id: string): void {
    if (tileDragOverId === id) tileDragOverId = null;
  }
  function onTileDrop(e: DragEvent, id: string): void {
    e.preventDefault();
    if (tileDragId && tileDragId !== id) {
      layout.moveTile(ordered.map((s) => s.id), tileDragId, id);
    }
    tileDragId = null;
    tileDragOverId = null;
  }
  function onTileDragEnd(): void {
    tileDragId = null;
    tileDragOverId = null;
  }

  // ── Live-tile bookkeeping ─────────────────────────────────────────────────
  // Set of session ids currently scrolled into the viewport (driven by the
  // IntersectionObserver below).
  let visible = $state(new Set<string>());
  // Tiles the user explicitly attached via "click to attach" — pinned live even
  // if scrolled off-screen, so a deliberate attach is never silently dropped.
  // Persisted per workspace so deliberate attaches survive view switches/reloads.
  const pinKey = (): string => winKey(`otto_tiled_pinned_${ws.currentId ?? ''}`);
  function loadPinned(): Set<string> {
    try {
      return new Set(JSON.parse(localStorage.getItem(pinKey()) ?? '[]') as string[]);
    } catch {
      return new Set();
    }
  }
  let pinned = $state(loadPinned());
  function savePinned(): void {
    try {
      localStorage.setItem(pinKey(), JSON.stringify([...pinned]));
    } catch {
      /* private mode */
    }
  }
  // Reload the pin set when the workspace changes.
  $effect(() => {
    void ws.currentId;
    pinned = loadPinned();
  });
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
    let pinsChanged = false;
    for (const id of [...pinned]) {
      if (!ids.has(id)) {
        pinned.delete(id);
        pinsChanged = true;
      }
    }
    if (pinsChanged) savePinned();
    for (const id of [...visible]) if (!ids.has(id)) visible.delete(id);
  });

  // Decide which tiles get a live terminal. The focused/active tile is always
  // live (keeps the normal attach path intact), then pinned tiles, then visible
  // tiles in grid order — capped at MAX_LIVE_TILES.
  const liveIds = $derived.by(() => {
    const live = new Set<string>();
    const active = ws.activeSessionId;
    if (active && ordered.some((s) => s.id === active)) live.add(active);
    // Explicit attaches next — honor the user's deliberate choice before
    // best-effort visible tiles.
    for (const s of ordered) {
      if (live.size >= MAX_LIVE_TILES) break;
      if (pinned.has(s.id)) live.add(s.id);
    }
    for (const s of ordered) {
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
    savePinned();
    ws.openSession(id);
    layout.focusIndex(0);
  }

  /** Placeholder subline: only a session with no live PTY is "suspended" — a
   *  running-but-over-budget/off-screen one is just not attached here. */
  function placeholderHint(id: string, fallback: string): string {
    const st = ws.statusMap[id] ?? fallback;
    return st === 'working' || st === 'running' || st === 'idle'
      ? 'Running — not attached'
      : 'Suspended — not connected';
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
  <div class="tiled-wrap">
  {#if ordered.length > 1}
    <!-- Full per-pane control lives in the split tree: hand the tiles over in
         their current order as an equal grid, then every edge is its own
         gutter and panes can be nested any way (presets in the pane ⋯ menu). -->
    <button
      class="btn small free-layout"
      onclick={() => {
        layout.layoutSessions(ordered.map((s) => s.id), 'grid');
        ws.setViewMode('tabs');
      }}
      title="Turn these tiles into a free layout: every pane edge resizable, panes nestable, presets in the ⋯ menu"
      data-testid="tiled-free-layout"
    ><Icon name="split" size={11} /> Free layout</button>
  {/if}
  <div class="tiled" class:resizing bind:this={gridEl} style={gridStyle}>
    <!-- Dividers: one per gap, spanning the whole grid, so dragging between
         any two tiles resizes the whole column / row (a 20px grab zone reaches
         into both neighbours' edges; the drawn line stays 2px). -->
    {#each Array.from({ length: cols - 1 }, (_, i) => i + 1) as k (`c${k}`)}
      <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
      <div
        class="tgut col"
        role="separator"
        tabindex="0"
        aria-orientation="vertical"
        aria-label="Resize columns (drag, arrow keys; double-click to equalise)"
        title="Drag to resize · double-click to equalise"
        style="grid-column: {2 * k}; grid-row: 1 / -1;"
        data-testid="tile-divider-col"
        onpointerdown={(e) => startTrackDrag(e, 'cols', k)}
        ondblclick={resetTracks}
        onkeydown={(e) => trackKeydown(e, 'cols', k)}
      ></div>
    {/each}
    {#each Array.from({ length: rows - 1 }, (_, i) => i + 1) as k (`r${k}`)}
      <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
      <div
        class="tgut row"
        role="separator"
        tabindex="0"
        aria-orientation="horizontal"
        aria-label="Resize rows (drag, arrow keys; double-click to equalise)"
        title="Drag to resize · double-click to equalise"
        style="grid-row: {2 * k}; grid-column: 1 / -1;"
        data-testid="tile-divider-row"
        onpointerdown={(e) => startTrackDrag(e, 'rows', k)}
        ondblclick={resetTracks}
        onkeydown={(e) => trackKeydown(e, 'rows', k)}
      ></div>
    {/each}
    {#each ordered as s, i (s.id)}
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="tile-slot"
        class:drag-over={tileDragOverId === s.id}
        data-tile-id={s.id}
        data-col={i % cols}
        data-row={Math.floor(i / cols)}
        style="grid-column: {2 * (i % cols) + 1}; grid-row: {2 * Math.floor(i / cols) + 1};"
        use:observeTile={s.id}
        ondragover={(e) => onTileDragOver(e, s.id)}
        ondragleave={() => onTileDragLeave(s.id)}
        ondrop={(e) => onTileDrop(e, s.id)}
        ondragend={onTileDragEnd}
      >
        {#if liveIds.has(s.id)}
          <SessionView
            sessionId={s.id}
            focused={ws.activeSessionId === s.id}
            showClose={false}
            showZoom={true}
            showGrip={ordered.length > 1}
            dragKey={s.id}
            ondragpane={(phase) => (tileDragId = phase === 'start' ? s.id : null)}
            onfocus={() => {
              ws.openSession(s.id);
              // make this the focused/active target without leaving tiled view
              layout.focusIndex(0);
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
            <!-- A placeholder has no SessionView grip — its own header is the
                 drag source so every tile can be reordered. -->
            <!-- svelte-ignore a11y_no_static_element_interactions -->
            <header
              class="ph-head"
              draggable={ordered.length > 1}
              ondragstart={(e) => onTileDragStart(e, s.id)}
              ondragend={onTileDragEnd}
            >
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
                <span class="ph-hint">{placeholderHint(s.id, s.status ?? 'idle')}</span>
              {/if}
            </div>
          </button>
        {/if}
      </div>
    {/each}
  </div>
  </div>
{/if}

<style>
  .tiled-wrap {
    position: relative;
    height: 100%;
    min-height: 0;
  }
  .free-layout {
    position: absolute;
    top: 12px;
    inset-inline-end: 22px;
    z-index: 6;
    gap: 4px;
    box-shadow: var(--shadow);
  }
  .tiled {
    display: grid;
    /* No `gap`: the 8px dividers are their own tracks (see gridStyle). */
    height: 100%;
    padding: 8px;
    overflow: auto;
    grid-auto-rows: minmax(220px, 1fr);
  }
  .tiled.resizing {
    user-select: none;
  }
  .tiled.resizing > :global(*) {
    pointer-events: none;
  }
  /* Column / row dividers: an 8px track with a 2px line drawn at rest so the
     handle is discoverable, brighter on hover / focus; the hit area (::before)
     reaches 6px into both neighbouring tiles like a window frame. */
  .tgut {
    position: relative;
    /* Above the panes' own stacking contexts (header chrome, drop veils). */
    z-index: 20;
    min-width: 0;
    min-height: 0;
  }
  .tiled.resizing > .tgut {
    pointer-events: auto;
  }
  .tgut.col {
    cursor: col-resize;
  }
  .tgut.row {
    cursor: row-resize;
  }
  .tgut::before {
    content: '';
    position: absolute;
    inset: 0;
  }
  .tgut.col::before {
    inset: 0 -6px;
  }
  .tgut.row::before {
    inset: -6px 0;
  }
  .tgut::after {
    content: '';
    position: absolute;
    inset: 0;
    margin: auto;
    background: var(--border);
    border-radius: 2px;
    transition: background 120ms ease-out;
  }
  .tgut.col::after {
    width: 2px;
  }
  .tgut.row::after {
    height: 2px;
  }
  .tgut:hover::after,
  .tgut:focus-visible::after {
    background: color-mix(in srgb, var(--accent) 55%, transparent);
  }
  .tgut:focus-visible {
    outline: none;
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
    border-radius: var(--radius-m);
  }
  .tile-slot.drag-over {
    box-shadow: inset 0 0 0 2px var(--accent);
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
    text-align: start;
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
