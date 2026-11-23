<script lang="ts">
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import { plural } from '../../lib/plural';
  // Read-only ERD / relationship diagram for the active connection.
  //
  //  ┌── sidebar ──┐┌──────────── canvas ────────────┐
  //  │ db select   ││  auto-laid-out table cards with │
  //  │ table picker││  columns (PK/FK marked) + FK    │
  //  │  (subset)   ││  relationship edges; pan + zoom │
  //  └─────────────┘└─────────────────────────────────┘
  //
  // Reuses the JOIN-canvas card/edge model (geometry constants + horizontal
  // bezier edges) but is purely presentational — no editing, no SQL. A full
  // schema can be huge, so the user picks which tables to show; the backend
  // caps how many it introspects. Engines without FK metadata (Redis/Mongo)
  // render table cards with a "no relationships" hint.
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import { announceShown } from '../../lib/announce.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { database } from '../../lib/stores/database.svelte';
  import type { DbSchemaGraph, DbGraphTable } from '../../lib/api/types';

  // ── Card geometry (shared with QueryBuilder so the look matches) ───────────
  const CARD_W = 234;
  const HEADER_H = 30;
  const ROW_H = 21;
  const BODY_PAD = 5;
  const COL_GAP = 70; // horizontal gap between auto-laid-out columns of cards
  const ROW_GAP = 40; // vertical gap between cards in a layout column

  // How many tables to show on the canvas at once by default (a sane cap so a
  // 200-table schema doesn't render as an unreadable wall of cards).
  const DEFAULT_SHOWN = 12;

  // ── State ──────────────────────────────────────────────────────────────────
  let databases = $state<{ name: string; path: string }[]>([]);
  let selectedDb = $state<string>(''); // db path (NodePath id, e.g. `db:shop`)
  // Raw: the graph is only ever replaced wholesale — a deep proxy made every
  // edge/column read on the drag path go through the proxy traps.
  let graph = $state.raw<DbSchemaGraph | null>(null);
  let loading = $state(false);
  /** Cause of the last failed graph load — inline with Retry, never the
   *  "No schema selected" empty state. */
  let graphError = $state<string | null>(null);
  let tableSearch = $state('');
  // Which table ids are shown on the canvas (the selected subset).
  let shown = $state<Set<string>>(new Set());
  // Per-table card positions in content coordinates, keyed by table id.
  let positions = $state<Map<string, { x: number; y: number }>>(new Map());

  // Pan/zoom of the whole canvas (a CSS transform on the content layer).
  let zoom = $state(1);
  let panX = $state(0);
  let panY = $state(0);
  let viewportEl = $state<HTMLDivElement | null>(null);

  // Drag state — either panning the canvas or moving one card.
  let panning = $state(false);
  let panStart = { x: 0, y: 0, px: 0, py: 0 };
  let dragId = $state<string | null>(null);
  let dragOff = { x: 0, y: 0 };

  // ── Load databases for the active connection ───────────────────────────────
  $effect(() => {
    void database.selectedConnId;
    void database.schemaRoot;
    void loadDatabases();
  });

  // Monotonic sequence guarding the two async loaders: a connection/schema
  // switch mid-flight must not let the SLOWER (stale) response land on top of
  // the newer one.
  let loadSeq = 0;

  async function loadDatabases(): Promise<void> {
    const mySeq = ++loadSeq;
    const dbs = await database.listBuilderDatabases();
    if (mySeq !== loadSeq) return; // superseded by a newer load
    databases = dbs;
    if (!dbs.some((d) => d.path === selectedDb)) selectedDb = dbs[0]?.path ?? '';
    // ALWAYS (re)load the graph: an engine with no db list still has a default
    // schema to draw (this used to dead-end on "No schema selected"), and the
    // same db path on a different connection is a different schema.
    await loadGraph();
  }

  function dbName(path: string): string {
    return databases.find((d) => d.path === path)?.name ?? '';
  }

  async function onDbChange(): Promise<void> {
    await loadGraph();
  }

  async function loadGraph(): Promise<void> {
    if (!selectedDb && databases.length && databases[0].path) selectedDb = databases[0].path;
    const mySeq = ++loadSeq;
    loading = true;
    graph = null;
    graphError = null;
    try {
      let g: DbSchemaGraph | null;
      try {
        g = await database.fetchSchemaGraph(dbName(selectedDb) || selectedDb);
      } catch (e) {
        if (mySeq === loadSeq) graphError = loadErrorText(e);
        return;
      }
      if (mySeq !== loadSeq) return; // a newer load owns the view now
      graph = g;
      // Default selection: tables that participate in a relationship first
      // (the interesting ones), then fill up to DEFAULT_SHOWN by name order.
      if (g) {
        const related = new Set<string>();
        const byKey = new Map(g.tables.map((t) => [tableKey(t.schema, t.name), t.id]));
        for (const e of g.edges) {
          const tid = byKey.get(tableKey(e.to_schema, e.to_table));
          const fromId = byKey.get(tableKey(g.schema, e.from_table));
          if (fromId) related.add(fromId);
          if (tid) related.add(tid);
        }
        const pick = new Set<string>();
        for (const t of g.tables) {
          if (related.has(t.id) && pick.size < DEFAULT_SHOWN) pick.add(t.id);
        }
        for (const t of g.tables) {
          if (pick.size >= DEFAULT_SHOWN) break;
          pick.add(t.id);
        }
        shown = pick;
        relayout();
        resetView();
      }
    } finally {
      if (mySeq === loadSeq) loading = false;
    }
  }

  function tableKey(schema: string, name: string): string {
    return `${schema}\u0000${name}`.toLowerCase();
  }

  // ── Table picker ──────────────────────────────────────────────────────────
  const filteredTables = $derived.by(() => {
    const list = graph?.tables ?? [];
    const q = tableSearch.trim().toLowerCase();
    if (!q) return list;
    return list.filter((t) => t.name.toLowerCase().includes(q));
  });

  function toggleTable(id: string): void {
    const next = new Set(shown);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    shown = next;
    relayout();
  }

  function showAll(): void {
    shown = new Set((graph?.tables ?? []).map((t) => t.id));
    relayout();
  }
  function showNone(): void {
    shown = new Set();
  }
  function showRelatedOnly(): void {
    if (!graph) return;
    const byKey = new Map(graph.tables.map((t) => [tableKey(t.schema, t.name), t.id]));
    const related = new Set<string>();
    for (const e of graph.edges) {
      const fromId = byKey.get(tableKey(graph.schema, e.from_table));
      const toId = byKey.get(tableKey(e.to_schema, e.to_table));
      if (fromId) related.add(fromId);
      if (toId) related.add(toId);
    }
    shown = related;
    relayout();
  }

  // ── Auto layout — a simple column-packed grid of the shown cards ───────────
  function cardHeight(t: DbGraphTable): number {
    return HEADER_H + BODY_PAD * 2 + t.columns.length * ROW_H;
  }

  function relayout(): void {
    if (!graph) return;
    const cards = graph.tables.filter((t) => shown.has(t.id));
    if (!cards.length) {
      positions = new Map();
      return;
    }
    // Pack into N columns sized to keep the canvas roughly square-ish.
    const cols = Math.max(1, Math.ceil(Math.sqrt(cards.length)));
    const colHeights = new Array<number>(cols).fill(28);
    const next = new Map<string, { x: number; y: number }>();
    cards.forEach((t, i) => {
      const c = i % cols;
      const x = 28 + c * (CARD_W + COL_GAP);
      const y = colHeights[c];
      next.set(t.id, { x, y });
      colHeights[c] = y + cardHeight(t) + ROW_GAP;
    });
    positions = next;
  }

  // ── Content size (grows to fit the farthest card) ──────────────────────────
  const shownTables = $derived.by(() => (graph?.tables ?? []).filter((t) => shown.has(t.id)));

  const contentSize = $derived.by(() => {
    let w = 600;
    let h = 400;
    for (const t of shownTables) {
      const p = positions.get(t.id);
      if (!p) continue;
      w = Math.max(w, p.x + CARD_W + 80);
      h = Math.max(h, p.y + cardHeight(t) + 80);
    }
    return { w, h };
  });

  // ── Edge geometry (only edges between two shown cards) ─────────────────────
  function colIndex(t: DbGraphTable, col: string): number {
    return Math.max(0, t.columns.findIndex((c) => c.name === col));
  }
  function anchorY(t: DbGraphTable, p: { x: number; y: number }, col: string): number {
    return p.y + HEADER_H + BODY_PAD + colIndex(t, col) * ROW_H + ROW_H / 2;
  }
  function sideFacing(p: { x: number }, otherCenterX: number): 'l' | 'r' {
    return otherCenterX < p.x + CARD_W / 2 ? 'l' : 'r';
  }
  function handleX(p: { x: number }, side: 'l' | 'r'): number {
    return side === 'l' ? p.x : p.x + CARD_W;
  }
  function bezier(x1: number, y1: number, s1: 'l' | 'r', x2: number, y2: number, s2: 'l' | 'r'): string {
    const k = Math.max(40, Math.abs(x2 - x1) / 2);
    const c1x = s1 === 'r' ? x1 + k : x1 - k;
    const c2x = s2 === 'r' ? x2 + k : x2 - k;
    return `M ${x1} ${y1} C ${c1x} ${y1}, ${c2x} ${y2}, ${x2} ${y2}`;
  }

  // Edges resolved to their two cards ONCE per graph (the table lookup used to
  // be rebuilt, and every edge re-resolved, on every drag frame).
  type ResolvedEdge = { id: string; from: DbGraphTable; to: DbGraphTable; fromCol: string; toCol: string; label: string };
  const resolvedEdges = $derived.by<ResolvedEdge[]>(() => {
    const g = graph;
    if (!g) return [];
    const byKey = new Map(g.tables.map((t) => [tableKey(t.schema, t.name), t]));
    const out: ResolvedEdge[] = [];
    g.edges.forEach((e, i) => {
      const from = byKey.get(tableKey(g.schema, e.from_table));
      const to = byKey.get(tableKey(e.to_schema, e.to_table));
      if (!from || !to) return;
      out.push({
        id: `${e.name}_${i}`,
        from,
        to,
        fromCol: e.from_columns[0] ?? '',
        toCol: e.to_columns[0] ?? '',
        label: `${e.from_table}.${e.from_columns.join(',')} → ${e.to_table}.${e.to_columns.join(',')}`,
      });
    });
    return out;
  });

  // Path cache keyed by edge id: a drag replaces only the dragged card's
  // position object, so every edge whose two endpoint objects are unchanged
  // reuses its path — only the edges incident to the moving card recompute.
  // (Plain Map, not state: it is a memo inside the derivation below.)
  let pathMemo = new Map<string, { pa: object; pb: object; d: string }>();
  const edgePaths = $derived.by(() => {
    const next = new Map<string, { pa: object; pb: object; d: string }>();
    const out: { id: string; d: string; label: string }[] = [];
    for (const e of resolvedEdges) {
      if (!shown.has(e.from.id) || !shown.has(e.to.id)) continue;
      const pa = positions.get(e.from.id);
      const pb = positions.get(e.to.id);
      if (!pa || !pb) continue;
      let hit = pathMemo.get(e.id);
      if (!hit || hit.pa !== pa || hit.pb !== pb) {
        const sa = sideFacing(pa, pb.x + CARD_W / 2);
        const sb = sideFacing(pb, pa.x + CARD_W / 2);
        const d = bezier(handleX(pa, sa), anchorY(e.from, pa, e.fromCol), sa, handleX(pb, sb), anchorY(e.to, pb, e.toCol), sb);
        hit = { pa, pb, d };
      }
      next.set(e.id, hit);
      out.push({ id: e.id, d: hit.d, label: e.label });
    }
    pathMemo = next;
    return out;
  });

  // ── Pan / zoom ──────────────────────────────────────────────────────────────
  function resetView(): void {
    zoom = 1;
    panX = 0;
    panY = 0;
  }
  function zoomBy(factor: number): void {
    zoom = Math.min(2, Math.max(0.25, +(zoom * factor).toFixed(3)));
  }
  function onWheel(ev: WheelEvent): void {
    if (!ev.ctrlKey && !ev.metaKey) return; // plain scroll = native scroll
    ev.preventDefault();
    zoomBy(ev.deltaY < 0 ? 1.1 : 0.9);
  }

  function onPointerDown(ev: PointerEvent): void {
    // Background drag = pan (cards stop propagation in their own handler).
    if (ev.button !== 0) return;
    panning = true;
    panStart = { x: ev.clientX, y: ev.clientY, px: panX, py: panY };
    viewportEl?.setPointerCapture?.(ev.pointerId);
  }
  // Card drags apply at most once per frame: pointermove fires at 60–120 Hz and
  // each applied move re-positions the card and its incident edges.
  let dragFrame = 0;
  let dragPoint: { clientX: number; clientY: number } | null = null;
  function applyDrag(): void {
    dragFrame = 0;
    const id = dragId;
    const pt = dragPoint;
    dragPoint = null;
    if (!id || !pt) return;
    const np = clientToContent(pt);
    const next = new Map(positions);
    next.set(id, { x: Math.max(0, np.x - dragOff.x), y: Math.max(0, np.y - dragOff.y) });
    positions = next;
  }
  function onPointerMove(ev: PointerEvent): void {
    if (dragId) {
      dragPoint = { clientX: ev.clientX, clientY: ev.clientY };
      if (!dragFrame) dragFrame = requestAnimationFrame(applyDrag);
    } else if (panning) {
      panX = panStart.px + (ev.clientX - panStart.x);
      panY = panStart.py + (ev.clientY - panStart.y);
    }
  }
  function onPointerUp(): void {
    // Land the last queued move before the drag ends.
    if (dragFrame) {
      cancelAnimationFrame(dragFrame);
      applyDrag();
    }
    panning = false;
    dragId = null;
  }
  $effect(() => () => {
    if (dragFrame) cancelAnimationFrame(dragFrame);
  });

  // Convert client coords → content (pre-transform) coordinates.
  function clientToContent(ev: { clientX: number; clientY: number }): { x: number; y: number } {
    const rect = viewportEl?.getBoundingClientRect();
    if (!rect) return { x: 0, y: 0 };
    return {
      x: (ev.clientX - rect.left - panX) / zoom,
      y: (ev.clientY - rect.top - panY) / zoom,
    };
  }

  function startCardDrag(ev: PointerEvent, id: string): void {
    if (ev.button !== 0) return;
    const p = positions.get(id);
    if (!p) return;
    const pt = clientToContent(ev);
    dragId = id;
    dragOff = { x: pt.x - p.x, y: pt.y - p.y };
    viewportEl?.setPointerCapture?.(ev.pointerId);
    ev.stopPropagation();
  }

  // Open a table in the Structure tab (read-only deep dive).
  function openStructure(t: DbGraphTable): void {
    void database.openObject({ id: t.id, label: t.name, kind: t.kind, has_children: true });
  }
</script>

<div class="diagram">
  <aside class="picker">
    <div class="picker-top">
      <label class="lbl" for="erd-db">Schema</label>
      <select id="erd-db" class="db-select" bind:value={selectedDb} onchange={onDbChange}>
        {#each databases as d (d.path)}
          <option value={d.path}>{d.name || '(default)'}</option>
        {/each}
      </select>
    </div>

    {#if graph}
      <div class="picker-tools">
        <input dir="auto" aria-label="Filter tables" class="search" placeholder="Filter tables…" bind:value={tableSearch} />
        <div class="quick">
          <button class="link" onclick={showRelatedOnly} title="Show only tables with relationships">Related</button>
          <button class="link" onclick={showAll}>All</button>
          <button class="link" onclick={showNone}>None</button>
        </div>
      </div>
      <div class="tlist">
        {#each filteredTables as t (t.id)}
          <label class="trow" class:on={shown.has(t.id)}>
            <input type="checkbox" checked={shown.has(t.id)} onchange={() => toggleTable(t.id)} />
            <Icon name={t.kind === 'view' ? 'eye' : 'grid'} size={12} />
            <span class="tname" title={t.name}>{t.name}</span>
            <span class="tmeta">{t.columns.length}</span>
          </label>
        {/each}
        {#if !filteredTables.length}
          <p class="muted small">No tables match.</p>
        {/if}
      </div>
      <div class="picker-foot muted small">
        {shown.size} / {graph.tables.length} shown
        {#if graph.truncated}· capped{/if}
      </div>
    {/if}
  </aside>

  <div class="stage">
    <div class="toolbar">
      <div class="zoom">
        <button class="zb" onclick={() => zoomBy(0.9)} title="Zoom out" aria-label="Zoom out">−</button>
        <span class="zlvl mono">{Math.round(zoom * 100)}%</span>
        <button class="zb" onclick={() => zoomBy(1.1)} title="Zoom in" aria-label="Zoom in">+</button>
        <button class="zb" onclick={resetView} title="Reset view" aria-label="Reset view"><Icon name="maximize" size={12} /></button>
        <button class="zb" onclick={relayout} title="Re-layout cards" aria-label="Re-layout"><Icon name="grid" size={12} /></button>
      </div>
      <span class="grow"></span>
      {#if graph && !graph.relationships}
        <span class="rel-hint" title="This engine doesn’t expose foreign-key relationships">
          <Icon name="info" size={12} /> No relationships
        </span>
      {:else if graph}
        <span class="rel-hint mono" title="Foreign-key edges drawn">{plural(edgePaths.length, 'relationship')}</span>
      {/if}
    </div>

    {#if graph?.truncated}
      <div class="trunc-banner" use:announceShown={`Schema capped — showing the first ${graph.tables.length} tables.`}>
        <Icon name="info" size={12} />
        <span>
          Schema capped — showing the first {graph.tables.length} tables the server introspected;
          the schema has more. Foreign-key edges to tables beyond the cap are <strong>not drawn</strong>.
        </span>
      </div>
    {/if}

    {#if loading}
      <div class="center"><Skeleton rows={4} height={64} label="the diagram" /></div>
    {:else if graphError}
      <div class="center"><LoadState what="the diagram" error={graphError} empty onretry={() => void loadGraph()} /></div>
    {:else if !graph}
      <div class="center"><EmptyState icon="link" title="No schema selected" body="Pick a schema to draw its tables and relationships." /></div>
    {:else if shown.size === 0}
      <div class="center"><EmptyState icon="grid" title="No tables shown" body="Pick tables on the left to add them to the diagram." /></div>
    {:else}
      <div
        role="group"
        aria-label="Schema diagram — drag to pan, scroll to zoom"
        class="viewport"
        class:panning
        bind:this={viewportEl}
        onpointerdown={onPointerDown}
        onpointermove={onPointerMove}
        onpointerup={onPointerUp}
        onpointercancel={onPointerUp}
        onwheel={onWheel}
      >
        <div
          class="content"
          style="width:{contentSize.w}px; height:{contentSize.h}px; transform: translate({panX}px, {panY}px) scale({zoom}); transform-origin: 0 0;"
        >
          <svg class="edges" width={contentSize.w} height={contentSize.h} aria-hidden="true">
            <defs>
              <marker id="erd-arrow" viewBox="0 0 10 10" refX="8" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse">
                <path d="M0 0 L10 5 L0 10 z" fill="var(--accent)" />
              </marker>
            </defs>
            {#each edgePaths as p (p.id)}
              <path class="edge" d={p.d} marker-end="url(#erd-arrow)"><title>{p.label}</title></path>
            {/each}
          </svg>

          {#each shownTables as t (t.id)}
            {@const p = positions.get(t.id)}
            {#if p}
              <div class="node" style="left:{p.x}px; top:{p.y}px; width:{CARD_W}px">
                <!-- Pointer-only move handle; a table's Structure also opens from the schema tree. -->
                <div class="node-head" role="presentation" onpointerdown={(e) => startCardDrag(e, t.id)} ondblclick={() => openStructure(t)} title="Drag to move · double-click for Structure">
                  <Icon name={t.kind === 'view' ? 'eye' : 'grid'} size={12} />
                  <span class="node-title" title={t.name}>{t.name}</span>
                  {#if t.kind === 'view'}<span class="kind-tag">view</span>{/if}
                </div>
                <div class="node-body">
                  {#each t.columns as c (c.name)}
                    <div class="col-row" style="height:{ROW_H}px">
                      <span class="col-marker">
                        {#if c.primary_key}<span class="keymark pk" title="Primary key">PK</span>
                        {:else if c.foreign_key}<span class="keymark fk" title="Foreign key">FK</span>{/if}
                      </span>
                      <span class="col-name" class:pk={c.primary_key} class:fk={c.foreign_key} title={c.name}>{c.name}</span>
                      <span class="col-ty" class:nul={c.nullable} title={c.data_type}>{c.data_type}</span>
                    </div>
                  {/each}
                  {#if !t.columns.length}
                    <div class="col-row empty muted">no columns</div>
                  {/if}
                </div>
              </div>
            {/if}
          {/each}
        </div>
      </div>
    {/if}
  </div>
</div>

<style>
  .diagram {
    display: flex;
    height: 100%;
    min-height: 0;
  }

  /* ── Left picker ── */
  .picker {
    width: 220px;
    flex: 0 0 220px;
    display: flex;
    flex-direction: column;
    border-inline-end: 1px solid var(--border);
    background: var(--surface);
    min-height: 0;
  }
  .picker-top {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 8px 10px;
    border-bottom: 1px solid var(--border);
  }
  .lbl {
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: .06em;
    color: var(--text-dim);
  }
  .db-select {
    height: 26px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--bg);
    color: var(--text);
    font-size: var(--fs-s);
    padding: 0 6px;
  }
  .picker-tools {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px 10px;
    border-bottom: 1px solid var(--border);
  }
  .search {
    height: 26px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--bg);
    color: var(--text);
    font-size: var(--fs-s);
    padding: 0 8px;
  }
  .quick {
    display: flex;
    gap: 10px;
  }
  .link {
    border: none;
    background: transparent;
    color: var(--accent-text);
    font-size: var(--fs-xs);
    cursor: pointer;
    padding: 0;
  }
  .link:hover {
    text-decoration: underline;
  }
  .tlist {
    flex: 1;
    overflow: auto;
    padding: 4px 6px;
    min-height: 0;
  }
  .trow {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 2px 6px;
    border-radius: var(--radius-s);
    cursor: pointer;
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .trow:hover {
    background: var(--hover);
  }
  .trow.on {
    color: var(--text);
  }
  .trow input {
    margin: 0;
  }
  .tname {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tmeta {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .picker-foot {
    padding: 6px 10px;
    border-top: 1px solid var(--border);
  }

  /* ── Stage ── */
  .stage {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    position: relative;
  }
  .toolbar {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 34px;
    padding: 0 10px;
    border-bottom: 1px solid var(--border);
    background: var(--surface);
  }
  .zoom {
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .zb {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 22px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--bg);
    color: var(--text-dim);
    cursor: pointer;
  }
  .zb:hover {
    color: var(--text);
    border-color: var(--accent);
  }
  .zlvl {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    min-width: 34px;
    text-align: center;
  }
  .grow {
    flex: 1;
  }
  .rel-hint {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  /* Loud, persistent cap warning — the tiny "· capped" footnote was invisible
     and users read a truncated diagram as the whole schema. */
  .trunc-banner {
    display: flex;
    align-items: flex-start;
    gap: 6px;
    padding: 6px 12px;
    border-bottom: 1px solid color-mix(in srgb, var(--warning) 40%, var(--border));
    background: var(--warning-soft);
    color: var(--warning);
    font-size: var(--fs-s);
    line-height: 1.45;
  }
  .trunc-banner :global(svg) {
    flex-shrink: 0;
    margin-top: 2px;
  }

  .center {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  /* ── Canvas ── */
  .viewport {
    flex: 1;
    position: relative;
    overflow: auto;
    background:
      radial-gradient(circle, color-mix(in srgb, var(--text-dim) 22%, transparent) 1px, transparent 1px);
    background-size: 18px 18px;
    background-color: var(--bg);
    cursor: grab;
    touch-action: none;
    min-height: 0;
  }
  .viewport.panning {
    cursor: grabbing;
    user-select: none;
  }
  .content {
    position: relative;
  }
  .edges {
    position: absolute;
    top: 0;
    left: 0; /* ui-guards: allow — SVG edges share the cards' physical px coordinates */
    pointer-events: none;
    z-index: 1;
  }
  .edge {
    fill: none;
    stroke: var(--accent);
    stroke-width: 1.6;
    opacity: 0.8;
    pointer-events: stroke;
  }
  .edge:hover {
    stroke-width: 2.6;
    opacity: 1;
  }

  .node {
    position: absolute;
    z-index: 2;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    box-shadow: var(--glass-shadow);
    overflow: hidden;
  }
  .node-head {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 30px;
    padding: 0 8px;
    background: var(--surface-2);
    border-bottom: 1px solid var(--border);
    cursor: grab;
    color: var(--accent-text);
  }
  .node-head:active {
    cursor: grabbing;
  }
  .node-title {
    flex: 1;
    font-size: var(--fs-s);
    font-weight: 600;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .kind-tag {
    font-size: var(--fs-xs);
    text-transform: uppercase;
    letter-spacing: .06em;
    color: var(--text-dim);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 0 4px;
  }
  .node-body {
    padding: 4px 0;
  }
  .col-row {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 8px;
    font-size: var(--fs-s);
  }
  .col-row.empty {
    height: auto;
    padding: 4px 8px;
    font-style: italic;
  }
  .col-marker {
    width: 24px;
    flex: 0 0 24px;
    display: inline-flex;
  }
  /* PK / FK key marks on a diagram column: a glyph sized to the 24 px marker
     column (not a status Badge — it never wraps or grows the row). */
  .keymark {
    font-size: var(--fs-xs);
    font-weight: 600;
    line-height: 1;
    padding: 2px;
    border-radius: var(--radius-s);
  }
  .keymark.pk {
    background: var(--accent-soft-strong);
    color: var(--accent-text);
  }
  .keymark.fk {
    background: color-mix(in srgb, var(--text-dim) 20%, transparent);
    color: var(--text-dim);
  }
  .col-name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text);
  }
  .col-name.pk {
    font-weight: 600;
  }
  .col-name.fk {
    color: var(--accent-text);
  }
  .col-ty {
    color: var(--text-dim);
    font-size: var(--fs-xs);
    max-width: 80px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .col-ty.nul {
    opacity: 0.75;
  }
  .muted {
    color: var(--text-dim);
  }
  .small {
    font-size: var(--fs-xs);
  }
  .mono {
    font-family: var(--font-mono);
  }
</style>
