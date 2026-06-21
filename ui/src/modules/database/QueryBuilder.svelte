<script lang="ts">
  // Visual JOIN canvas (Navicat-style) for SQL engines.
  //
  //  ┌── palette ──┐┌──────────── canvas ────────────┐
  //  │ db select   ││  draggable table cards; drag    │
  //  │ table search││  from a column handle to another│
  //  │ + add tables││  column to draw a JOIN edge     │
  //  └─────────────┘└─────────────────────────────────┘
  //  ┌──────── bottom: Generated SQL · Filters · Limit · actions ────────┐
  //
  // Add ANY table from ANY database via the palette. Joins are drawn by the
  // user (FK suggestions are an optional helper). SQL is generated live by
  // walking the edge graph from the first-added (base) table.
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import { database } from '../../lib/stores/database.svelte';
  import {
    buildSql,
    nextId,
    uniqueAlias,
    walkJoins,
    aggregateOptions,
    JOIN_TYPES,
    OPS,
    type Aggregate,
    type CanvasTable,
    type ExprCol,
    type JoinEdge,
    type JoinType,
    type OrderRow,
    type WhereRow,
  } from './joinCanvas';
  import type { ObjectDetail } from '../../lib/api/types';

  // ── Card geometry (constants → anchor math is exact, no measurement) ───────
  const CARD_W = 244;
  const HEADER_H = 30;
  const ROW_H = 22;
  const BODY_PAD = 6; // top padding of the column list
  const BODY_MAX_H = 900; // matches .node-body max-height in CSS (fits ~40 cols, no scroll)

  // ── State ──────────────────────────────────────────────────────────────────
  let tables = $state<CanvasTable[]>([]);
  let edges = $state<JoinEdge[]>([]);
  let wheres = $state<WhereRow[]>([]);
  let orders = $state<OrderRow[]>([]);
  let exprs = $state<ExprCol[]>([]);
  let limit = $state<number>(100);
  let bottomOpen = $state(true);
  let exprsOpen = $state(false);
  let fnMenuFor = $state<string | null>(null); // expr id whose fn menu is open

  // Engine-specific aggregate list for the per-column dropdown.
  const aggOptions = $derived(aggregateOptions(database.capabilities?.engine));

  // Common built-in functions for the expression quick-insert menu.
  const FN_SNIPPETS = [
    'COALESCE(a, b)',
    'CONCAT(a, b)',
    'COUNT(*)',
    'SUM(x)',
    'AVG(x)',
    'ROUND(x, 2)',
    'DATE(x)',
    'LOWER(x)',
    'UPPER(x)',
  ];

  // Palette
  let databases = $state<{ name: string; path: string }[]>([]);
  let selectedDb = $state<string>(''); // db path
  let paletteTables = $state<{ label: string; path: string; kind: string }[]>([]);
  let paletteSearch = $state('');
  let paletteLoading = $state(false);

  // Canvas scroll container. Cards + edges are positioned in content
  // coordinates inside an inner sized layer, so native scrolling just works.
  let canvasEl = $state<HTMLDivElement | null>(null);

  // Per-card body geometry — the column list scrolls independently, so an
  // edge anchor must subtract its card's body scrollTop and clamp to the
  // body's visible band. Keyed by table uid; updated by the `cardBody` action.
  let bodyScroll = $state<Map<string, number>>(new Map());
  let bodyHeight = $state<Map<string, number>>(new Map());

  /**
   * Track a card body's scroll + visible height into the reactive maps so
   * edge anchors recompute on body scroll and resize. Returns a Svelte action.
   */
  function cardBody(node: HTMLElement, uid: string) {
    const sync = (): void => {
      bodyScroll.set(uid, node.scrollTop);
      bodyScroll = new Map(bodyScroll);
      bodyHeight.set(uid, node.clientHeight);
      bodyHeight = new Map(bodyHeight);
    };
    sync();
    node.addEventListener('scroll', sync, { passive: true });
    const ro = new ResizeObserver(sync);
    ro.observe(node);
    return {
      destroy() {
        node.removeEventListener('scroll', sync);
        ro.disconnect();
        bodyScroll.delete(uid);
        bodyScroll = new Map(bodyScroll);
        bodyHeight.delete(uid);
        bodyHeight = new Map(bodyHeight);
      },
    };
  }

  // Drag a card by its header.
  let dragUid = $state<string | null>(null);
  let dragOffX = 0;
  let dragOffY = 0;

  // Pending connection (drawing a join).
  let pending = $state<{ fromUid: string; fromCol: string; side: 'l' | 'r' } | null>(null);
  let pendingPt = $state<{ x: number; y: number } | null>(null);

  // Edge popover.
  let editEdge = $state<string | null>(null);

  // ── Palette data loading ────────────────────────────────────────────────
  $effect(() => {
    // Reload the database list when the connection or its schema root changes
    // (the root populates asynchronously after a connection is selected).
    void database.selectedConnId;
    void database.schemaRoot;
    void loadDatabases();
  });

  async function loadDatabases(): Promise<void> {
    if (!database.supportsBuilder) return;
    const dbs = await database.listBuilderDatabases();
    databases = dbs;
    // Keep the current db if still present; otherwise select the first.
    if (dbs.length && !dbs.some((d) => d.path === selectedDb)) {
      selectedDb = dbs[0].path;
      await loadPaletteTables();
    }
  }

  async function loadPaletteTables(): Promise<void> {
    paletteLoading = true;
    try {
      paletteTables = await database.listBuilderTables(selectedDb);
    } finally {
      paletteLoading = false;
    }
  }

  const filteredPalette = $derived.by(() => {
    const q = paletteSearch.trim().toLowerCase();
    if (!q) return paletteTables;
    return paletteTables.filter((t) => t.label.toLowerCase().includes(q));
  });

  function dbName(path: string): string {
    return databases.find((d) => d.path === path)?.name ?? '';
  }

  // ── Add / remove tables ─────────────────────────────────────────────────
  async function addTable(t: { label: string; path: string }): Promise<void> {
    const db = dbName(selectedDb);
    const taken = new Set(tables.map((x) => x.alias));
    const alias = uniqueAlias(t.label, taken);
    // Stagger new cards so they don't stack exactly on top of each other,
    // offset into the currently-scrolled region of the canvas.
    const n = tables.length;
    const sx = canvasEl?.scrollLeft ?? 0;
    const sy = canvasEl?.scrollTop ?? 0;
    const x = 28 + (n % 4) * (CARD_W + 40) + sx;
    const y = 28 + Math.floor(n / 4) * 60 + (n % 4) * 26 + sy;
    const isFirst = tables.length === 0;

    const card: CanvasTable = {
      uid: nextId('tbl'),
      db,
      table: t.label,
      alias,
      path: t.path,
      detail: null,
      cols: new Set(),
      aggs: new Map(),
      x,
      y,
    };
    tables = [...tables, card];

    const detail = await database.fetchObject(t.path);
    // Default columns: all for the FIRST table (the base), none for the rest —
    // keeps the SELECT focused as you add joined tables.
    tables = tables.map((c) =>
      c.uid === card.uid
        ? {
            ...c,
            detail,
            cols: isFirst && detail ? new Set(detail.columns.map((col) => col.name)) : new Set(),
          }
        : c,
    );
  }

  function removeTable(uid: string): void {
    tables = tables.filter((t) => t.uid !== uid);
    edges = edges.filter((e) => e.fromUid !== uid && e.toUid !== uid);
    // Drop filters whose column lived on the removed table.
    const aliases = new Set(tables.map((t) => t.alias));
    wheres = wheres.filter((w) => aliases.has(w.ref.split('.')[0]));
    orders = orders.filter((o) => aliases.has(o.ref.split('.')[0]));
  }

  function setAlias(uid: string, alias: string): void {
    const clean = alias.replace(/[^A-Za-z0-9_]/g, '_');
    tables = tables.map((t) => (t.uid === uid ? { ...t, alias: clean } : t));
  }

  /** Column names that participate in any foreign key (for the FK badge). */
  function fkCols(detail: ObjectDetail): Set<string> {
    const out = new Set<string>();
    for (const fk of detail.foreign_keys) for (const c of fk.columns) out.add(c);
    return out;
  }

  function toggleCol(uid: string, col: string): void {
    tables = tables.map((t) => {
      if (t.uid !== uid) return t;
      const cols = new Set(t.cols);
      const aggs = new Map(t.aggs);
      if (cols.has(col)) {
        cols.delete(col);
        aggs.delete(col); // deselecting clears any aggregate on that column
      } else {
        cols.add(col);
      }
      return { ...t, cols, aggs };
    });
  }

  /** Set/clear the aggregate for a selected column. */
  function setAgg(uid: string, col: string, agg: Aggregate): void {
    tables = tables.map((t) => {
      if (t.uid !== uid) return t;
      const aggs = new Map(t.aggs);
      if (agg) aggs.set(col, agg);
      else aggs.delete(col);
      return { ...t, aggs };
    });
  }

  // ── Expression columns ────────────────────────────────────────────────────
  function addExpr(): void {
    exprsOpen = true;
    exprs = [...exprs, { id: nextId('expr'), expression: '', alias: '' }];
  }
  function removeExpr(id: string): void {
    exprs = exprs.filter((e) => e.id !== id);
    if (fnMenuFor === id) fnMenuFor = null;
  }
  function setExprField(id: string, field: 'expression' | 'alias', value: string): void {
    exprs = exprs.map((e) => (e.id === id ? { ...e, [field]: value } : e));
  }
  /** Drop a snippet into an expression row's text field. */
  function insertSnippet(id: string, snippet: string): void {
    exprs = exprs.map((e) =>
      e.id === id ? { ...e, expression: e.expression ? `${e.expression} ${snippet}` : snippet } : e,
    );
    fnMenuFor = null;
  }

  // ── Card dragging ────────────────────────────────────────────────────────
  function startDrag(ev: PointerEvent, uid: string): void {
    if (ev.button !== 0) return;
    const t = tables.find((x) => x.uid === uid);
    if (!t) return;
    const pt = canvasPoint(ev);
    dragUid = uid;
    dragOffX = pt.x - t.x;
    dragOffY = pt.y - t.y;
    (ev.currentTarget as HTMLElement).setPointerCapture?.(ev.pointerId);
    ev.preventDefault();
  }

  // ── Connector handles (drawing joins) ─────────────────────────────────────
  // We capture the pointer on the canvas (so moves keep flowing) and resolve
  // the drop target by hit-testing at pointer-up — capture redirects the
  // up event to the canvas, so the target handle's own listener can't fire.
  function startConnect(ev: PointerEvent, uid: string, col: string, side: 'l' | 'r'): void {
    if (ev.button !== 0) return;
    pending = { fromUid: uid, fromCol: col, side };
    pendingPt = canvasPoint(ev);
    canvasEl?.setPointerCapture?.(ev.pointerId);
    ev.stopPropagation();
    ev.preventDefault();
  }

  function commitConnect(uid: string, col: string): void {
    if (!pending) return;
    if (pending.fromUid === uid && pending.fromCol === col) return; // same handle
    // No duplicate edges between the same two columns.
    const dup = edges.some(
      (e) =>
        (e.fromUid === pending!.fromUid && e.fromCol === pending!.fromCol && e.toUid === uid && e.toCol === col) ||
        (e.fromUid === uid && e.fromCol === col && e.toUid === pending!.fromUid && e.toCol === pending!.fromCol),
    );
    if (dup) return;
    edges = [
      ...edges,
      {
        id: nextId('edge'),
        fromUid: pending.fromUid,
        fromCol: pending.fromCol,
        toUid: uid,
        toCol: col,
        type: 'INNER',
      },
    ];
  }

  // ── Pointer-move / up at the canvas level ─────────────────────────────────
  // Convert client coords → content coords (account for the scroll offset).
  function canvasPoint(ev: { clientX: number; clientY: number }): { x: number; y: number } {
    const rect = canvasEl?.getBoundingClientRect();
    if (!rect || !canvasEl) return { x: 0, y: 0 };
    return { x: ev.clientX - rect.left + canvasEl.scrollLeft, y: ev.clientY - rect.top + canvasEl.scrollTop };
  }

  function onCanvasMove(ev: PointerEvent): void {
    if (dragUid) {
      const pt = canvasPoint(ev);
      const nx = Math.max(0, pt.x - dragOffX);
      const ny = Math.max(0, pt.y - dragOffY);
      tables = tables.map((t) => (t.uid === dragUid ? { ...t, x: nx, y: ny } : t));
    } else if (pending) {
      pendingPt = canvasPoint(ev);
    }
  }

  function onCanvasUp(ev: PointerEvent): void {
    dragUid = null;
    if (pending) {
      // Resolve the drop target by hit-testing the element under the cursor.
      // Accept a drop anywhere on the column row (not just the tiny handle).
      const el = document.elementFromPoint(ev.clientX, ev.clientY) as HTMLElement | null;
      const target = el?.closest<HTMLElement>('.handle, .col-row');
      const uid = target?.dataset.uid;
      const col = target?.dataset.col;
      if (uid && col) commitConnect(uid, col);
    }
    pending = null;
    pendingPt = null;
  }

  // ── Anchor geometry (handle positions for edges, in content coords) ──────
  function colIndex(t: CanvasTable, col: string): number {
    if (!t.detail) return 0;
    return Math.max(0, t.detail.columns.findIndex((c) => c.name === col));
  }

  /** The card body's visible height (falls back to the CSS max-height). */
  function visibleBodyH(t: CanvasTable): number {
    const measured = bodyHeight.get(t.uid);
    if (measured && measured > 0) return measured;
    const rows = t.detail ? t.detail.columns.length : 1;
    return Math.min(BODY_MAX_H, rows * ROW_H + BODY_PAD * 2);
  }

  /**
   * On-screen Y of a column's connector dot, in content coordinates, with the
   * card body's scroll subtracted and the result CLAMPED to the body's visible
   * band — so a scrolled-out column anchors to the card's top/bottom edge
   * rather than floating in empty space.
   */
  function handleY(t: CanvasTable, col: string): number {
    const bodyTop = t.y + HEADER_H; // first pixel of the scrollable body
    const scroll = bodyScroll.get(t.uid) ?? 0;
    // Row center within the body's content (pre-scroll), then shift by scroll.
    const rowCenter = BODY_PAD + colIndex(t, col) * ROW_H + ROW_H / 2;
    const y = bodyTop + rowCenter - scroll;
    // Clamp into the body's visible band so a scrolled-out column anchors to
    // the card's top/bottom edge (a 2px inset keeps the dot just on-card).
    const vis = visibleBodyH(t);
    const lo = bodyTop + 2;
    const hi = bodyTop + Math.max(2, vis - 2);
    return Math.min(hi, Math.max(lo, y));
  }

  /** True when the column's row center is scrolled out of the visible band. */
  function isClamped(t: CanvasTable, col: string): boolean {
    const scroll = bodyScroll.get(t.uid) ?? 0;
    const rowCenter = BODY_PAD + colIndex(t, col) * ROW_H + ROW_H / 2 - scroll;
    return rowCenter < 0 || rowCenter > visibleBodyH(t);
  }

  function handleX(t: CanvasTable, side: 'l' | 'r'): number {
    return side === 'l' ? t.x : t.x + CARD_W;
  }

  /** Pick the side (left/right) of a card that faces the other anchor. */
  function sideFacing(t: CanvasTable, otherX: number): 'l' | 'r' {
    const center = t.x + CARD_W / 2;
    return otherX < center ? 'l' : 'r';
  }

  // Inner content size — grows to fit the farthest card so native scroll works.
  const contentSize = $derived.by(() => {
    let w = 800;
    let h = 500;
    for (const t of tables) {
      w = Math.max(w, t.x + CARD_W + 80);
      const rows = t.detail ? t.detail.columns.length : 1;
      h = Math.max(h, t.y + HEADER_H + BODY_PAD * 2 + rows * ROW_H + 80);
    }
    return { w, h };
  });

  // SVG paths for committed edges (content coordinates). Each endpoint anchors
  // to its column's connector dot (scroll-aware + clamped via handleX/handleY).
  // The exit/enter side is chosen from the cards' relative horizontal position
  // so the curve leaves the source's outer dot and the arrowhead enters the
  // target's facing dot.
  const edgePaths = $derived.by(() => {
    const byUid = new Map(tables.map((t) => [t.uid, t]));
    return edges
      .map((e) => {
        const a = byUid.get(e.fromUid);
        const b = byUid.get(e.toUid);
        if (!a || !b) return null;
        const sa = sideFacing(a, b.x + CARD_W / 2);
        const x1 = handleX(a, sa);
        const y1 = handleY(a, e.fromCol);
        const sb = sideFacing(b, a.x + CARD_W / 2);
        const x2 = handleX(b, sb);
        const y2 = handleY(b, e.toCol);
        // Dim/dash the edge when either end is scrolled out of view — it points
        // at the card edge, signalling the real column is hidden.
        const offscreen = isClamped(a, e.fromCol) || isClamped(b, e.toCol);
        return {
          id: e.id,
          type: e.type,
          d: bezier(x1, y1, sa, x2, y2, sb),
          mx: (x1 + x2) / 2,
          my: (y1 + y2) / 2,
          offscreen,
        };
      })
      .filter((p): p is NonNullable<typeof p> => p !== null);
  });

  // Live path for the in-progress connection (content coordinates).
  const pendingPath = $derived.by(() => {
    if (!pending || !pendingPt) return null;
    const t = tables.find((x) => x.uid === pending!.fromUid);
    if (!t) return null;
    const x1 = handleX(t, pending.side);
    const y1 = handleY(t, pending.fromCol);
    const x2 = pendingPt.x;
    const y2 = pendingPt.y;
    const sb = x2 < x1 ? 'l' : 'r';
    return bezier(x1, y1, pending.side, x2, y2, sb);
  });

  /** Horizontal-tangent cubic bezier between two handles. */
  function bezier(x1: number, y1: number, s1: 'l' | 'r', x2: number, y2: number, s2: 'l' | 'r'): string {
    const k = Math.max(40, Math.abs(x2 - x1) / 2);
    const c1x = s1 === 'r' ? x1 + k : x1 - k;
    const c2x = s2 === 'r' ? x2 + k : x2 - k;
    return `M ${x1} ${y1} C ${c1x} ${y1}, ${c2x} ${y2}, ${x2} ${y2}`;
  }

  // ── FK suggestions (optional helper) ───────────────────────────────────────
  // For each table, FKs that reference another table also on the canvas, where
  // no edge already exists between them.
  const fkSuggestions = $derived.by(() => {
    const out: { fromUid: string; fromCol: string; toUid: string; toCol: string; label: string }[] = [];
    for (const t of tables) {
      if (!t.detail) continue;
      for (const fk of t.detail.foreign_keys) {
        const target = tables.find(
          (x) => x.uid !== t.uid && x.table.toLowerCase() === fk.ref_table.toLowerCase(),
        );
        if (!target) continue;
        const fromCol = fk.columns[0];
        const toCol = fk.ref_columns[0];
        if (!fromCol || !toCol) continue;
        const exists = edges.some(
          (e) =>
            (e.fromUid === t.uid && e.toUid === target.uid) ||
            (e.fromUid === target.uid && e.toUid === t.uid),
        );
        if (exists) continue;
        out.push({
          fromUid: t.uid,
          fromCol,
          toUid: target.uid,
          toCol,
          label: `${t.alias}.${fromCol} → ${target.alias}.${toCol}`,
        });
      }
    }
    return out;
  });

  function applySuggestion(s: { fromUid: string; fromCol: string; toUid: string; toCol: string }): void {
    edges = [
      ...edges,
      { id: nextId('edge'), fromUid: s.fromUid, fromCol: s.fromCol, toUid: s.toUid, toCol: s.toCol, type: 'INNER' },
    ];
  }

  // ── Edge editing ────────────────────────────────────────────────────────
  function setEdgeType(id: string, type: JoinType): void {
    edges = edges.map((e) => (e.id === id ? { ...e, type } : e));
  }
  function removeEdge(id: string): void {
    edges = edges.filter((e) => e.id !== id);
    if (editEdge === id) editEdge = null;
  }

  // ── Filters ────────────────────────────────────────────────────────────
  // Column options across every canvas table, as `alias.col`.
  const colOptions = $derived.by(() => {
    const out: { ref: string; label: string }[] = [];
    for (const t of tables) {
      if (!t.detail) continue;
      for (const c of t.detail.columns) {
        out.push({ ref: `${t.alias}.${c.name}`, label: `${t.alias}.${c.name}` });
      }
    }
    return out;
  });

  function addWhere(): void {
    wheres = [...wheres, { ref: colOptions[0]?.ref ?? '', op: '=', value: '' }];
  }
  function removeWhere(idx: number): void {
    wheres = wheres.filter((_, i) => i !== idx);
  }

  function addOrder(): void {
    orders = [...orders, { ref: colOptions[0]?.ref ?? '', dir: 'ASC' }];
  }
  function removeOrder(idx: number): void {
    orders = orders.filter((_, i) => i !== idx);
  }

  // ── Connectivity warning + SQL ─────────────────────────────────────────
  const baseUid = $derived(tables[0]?.uid ?? '');

  const unreached = $derived.by(() => {
    if (tables.length <= 1) return [];
    const { unreached } = walkJoins(tables, edges, baseUid);
    return tables.filter((t) => unreached.includes(t.uid));
  });

  const generatedSql = $derived.by(() => {
    if (!baseUid) return '';
    return buildSql(tables, edges, wheres, exprs, orders, limit, baseUid);
  });

  function openInQuery(andRun: boolean): void {
    if (!generatedSql) return;
    database.newTab(generatedSql);
    if (andRun) void database.runQuery(generatedSql);
  }
</script>

{#if !database.supportsBuilder}
  <EmptyState
    icon="split"
    title="Builder unavailable"
    body="The visual JOIN builder is only available for SQL engines."
  />
{:else}
  <div class="builder" class:bottom-open={bottomOpen}>
    <!-- ── Palette ─────────────────────────────────────────────────────── -->
    <aside class="palette">
      <div class="pal-head">Tables</div>
      <select class="input db-select" bind:value={selectedDb} onchange={loadPaletteTables}>
        {#each databases as d (d.path)}
          <option value={d.path}>{d.name || 'default'}</option>
        {/each}
      </select>
      <div class="pal-search">
        <Icon name="search" size={12} />
        <input class="pal-search-input" placeholder="Filter tables…" bind:value={paletteSearch} spellcheck="false" />
      </div>
      <div class="pal-list">
        {#if paletteLoading}
          <div class="pal-hint"><Icon name="refresh" size={12} /> Loading…</div>
        {:else if filteredPalette.length === 0}
          <div class="pal-hint">No tables found.</div>
        {:else}
          {#each filteredPalette as t (t.path)}
            <button class="pal-item" onclick={() => addTable(t)} title="Add to canvas">
              <Icon name={t.kind === 'view' ? 'eye' : 'grid'} size={12} />
              <span class="pal-item-label">{t.label}</span>
              <Icon name="plus" size={11} />
            </button>
          {/each}
        {/if}
      </div>
    </aside>

    <!-- ── Canvas ──────────────────────────────────────────────────────── -->
    <!-- Non-scrolling wrapper holds the pinned overlays (hint, FK bar). -->
    <div class="canvas">
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="canvas-scroll"
        class:dragging={dragUid !== null || pending !== null}
        role="application"
        aria-label="Join canvas"
        bind:this={canvasEl}
        onpointermove={onCanvasMove}
        onpointerup={onCanvasUp}
        onpointercancel={onCanvasUp}
      >
        <!-- Inner content layer, sized to fit all cards (enables native scroll).
             Cards, edges and popovers are all positioned in its coordinates. -->
        <div class="content" style="width:{contentSize.w}px; height:{contentSize.h}px">
          <!-- Edge overlay spans the full content so paths line up with cards. -->
          <svg class="edges" width={contentSize.w} height={contentSize.h} aria-hidden="true">
            <defs>
              <marker id="qb-arrow" viewBox="0 0 10 10" refX="8" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse">
                <path d="M0 0 L10 5 L0 10 z" fill="var(--accent)" />
              </marker>
            </defs>
            {#each edgePaths as p (p.id)}
              <!-- svelte-ignore a11y_click_events_have_key_events -->
              <path
                class="edge"
                class:active={editEdge === p.id}
                class:offscreen={p.offscreen}
                d={p.d}
                marker-end="url(#qb-arrow)"
                onclick={() => (editEdge = editEdge === p.id ? null : p.id)}
                role="button"
                tabindex="-1"
                aria-label="Join — click to edit"
              />
            {/each}
            {#if pendingPath}
              <path class="edge pending" d={pendingPath} marker-end="url(#qb-arrow)" />
            {/if}
          </svg>

          <!-- Edge popover (join type + delete) -->
          {#each edgePaths as p (p.id)}
            {#if editEdge === p.id}
              <div class="edge-pop" style="left:{p.mx}px; top:{p.my}px">
                <div class="seg">
                  {#each JOIN_TYPES as jt (jt)}
                    <button
                      class="seg-btn"
                      class:on={edges.find((e) => e.id === p.id)?.type === jt}
                      onclick={() => setEdgeType(p.id, jt)}>{jt}</button>
                  {/each}
                </div>
                <button class="icon-btn" aria-label="Delete join" onclick={() => removeEdge(p.id)}>
                  <Icon name="trash" size={11} />
                </button>
              </div>
            {/if}
          {/each}

          <!-- Table cards -->
          {#each tables as t (t.uid)}
            <div class="node" style="left:{t.x}px; top:{t.y}px; width:{CARD_W}px">
              <!-- svelte-ignore a11y_no_static_element_interactions -->
              <div class="node-head" onpointerdown={(e) => startDrag(e, t.uid)}>
                <Icon name="grid" size={11} />
                <input
                  class="alias-input"
                  value={t.alias}
                  onpointerdown={(e) => e.stopPropagation()}
                  onchange={(e) => setAlias(t.uid, (e.currentTarget as HTMLInputElement).value)}
                  spellcheck="false"
                />
                <span class="node-src">{t.db ? `${t.db}.` : ''}{t.table}</span>
                <button
                  class="node-x"
                  aria-label="Remove table"
                  onpointerdown={(e) => e.stopPropagation()}
                  onclick={() => removeTable(t.uid)}
                >
                  <Icon name="x" size={10} />
                </button>
              </div>
              <div class="node-body" use:cardBody={t.uid}>
                {#if !t.detail}
                  <div class="node-loading"><Icon name="refresh" size={11} /> loading…</div>
                {:else}
                  {@const fks = fkCols(t.detail)}
                  {#each t.detail.columns as c (c.name)}
                    {@const pk = t.detail.primary_key.includes(c.name)}
                    {@const fk = fks.has(c.name)}
                    <div class="col-row" data-uid={t.uid} data-col={c.name} style="height:{ROW_H}px">
                      <!-- svelte-ignore a11y_no_static_element_interactions -->
                      <span
                        class="handle l"
                        data-uid={t.uid}
                        data-col={c.name}
                        onpointerdown={(e) => startConnect(e, t.uid, c.name, 'l')}
                      ></span>
                      <input
                        class="col-cb"
                        type="checkbox"
                        checked={t.cols.has(c.name)}
                        onchange={() => toggleCol(t.uid, c.name)}
                        title="Include in SELECT"
                      />
                      <span class="col-name" class:pk class:fk title={c.name}>{c.name}</span>
                      <span class="col-ty" title={c.data_type}>{c.data_type}</span>
                      {#if t.cols.has(c.name)}
                        <select
                          class="col-agg"
                          class:set={(t.aggs.get(c.name) ?? '') !== ''}
                          value={t.aggs.get(c.name) ?? ''}
                          title="Aggregate"
                          onpointerdown={(e) => e.stopPropagation()}
                          onchange={(e) =>
                            setAgg(t.uid, c.name, (e.currentTarget as HTMLSelectElement).value as Aggregate)}
                        >
                          {#each aggOptions as a (a)}
                            <option value={a}>{a === '' ? 'fx' : a}</option>
                          {/each}
                        </select>
                      {:else}
                        <span class="col-agg spacer" aria-hidden="true"></span>
                      {/if}
                      {#if pk}
                        <span class="col-badge pk" title="Primary key">PK</span>
                      {:else if fk}
                        <span class="col-badge fk" title="Foreign key">FK</span>
                      {:else}
                        <span class="col-badge spacer" aria-hidden="true"></span>
                      {/if}
                      <!-- svelte-ignore a11y_no_static_element_interactions -->
                      <span
                        class="handle r"
                        data-uid={t.uid}
                        data-col={c.name}
                        onpointerdown={(e) => startConnect(e, t.uid, c.name, 'r')}
                      ></span>
                    </div>
                  {/each}
                {/if}
              </div>
            </div>
          {/each}
        </div>
      </div>

      <!-- Empty hint — pinned to the visible canvas, not the content layer. -->
      {#if tables.length === 0}
        <div class="canvas-hint">
          <Icon name="merge" size={26} />
          <p>Add tables from the palette, then drag from one column to another to create a join.</p>
        </div>
      {/if}

      <!-- FK suggestion chips (optional helper) -->
      {#if fkSuggestions.length}
        <div class="fk-bar">
          <span class="fk-label">Suggested joins</span>
          {#each fkSuggestions as s (s.fromUid + s.fromCol + s.toUid)}
            <button class="fk-chip" onclick={() => applySuggestion(s)}>
              <Icon name="merge" size={10} />{s.label}
            </button>
          {/each}
        </div>
      {/if}
    </div>

    <!-- ── Bottom panel ────────────────────────────────────────────────── -->
    <div class="bottom">
      <button class="bottom-toggle" onclick={() => (bottomOpen = !bottomOpen)}>
        <Icon name={bottomOpen ? 'chevronDown' : 'chevronRight'} size={12} />
        SQL &amp; Filters
        {#if unreached.length}<span class="warn-pill"><Icon name="info" size={10} />{unreached.length} unconnected</span>{/if}
      </button>
      {#if bottomOpen}
        <div class="bottom-body">
          <!-- Filters + limit -->
          <div class="b-col filters">
            <div class="b-col-head">
              Filters
              <button class="btn small ghost" onclick={addWhere} disabled={colOptions.length === 0}>
                <Icon name="plus" size={10} />Add
              </button>
            </div>
            <div class="where-list">
              {#each wheres as w, wi (wi)}
                <div class="where-row">
                  <select class="input" bind:value={w.ref}>
                    {#each colOptions as o (o.ref)}<option value={o.ref}>{o.label}</option>{/each}
                  </select>
                  <select class="input op" bind:value={w.op}>
                    {#each OPS as op (op)}<option value={op}>{op}</option>{/each}
                  </select>
                  {#if w.op !== 'IS NULL' && w.op !== 'IS NOT NULL'}
                    <input class="input mono" bind:value={w.value} placeholder="value" spellcheck="false" />
                  {:else}<span class="grow"></span>{/if}
                  <button class="icon-btn" aria-label="Remove filter" onclick={() => removeWhere(wi)}>
                    <Icon name="x" size={11} />
                  </button>
                </div>
              {/each}
              {#if wheres.length === 0}
                <div class="b-hint">No filters. Add a WHERE clause across any canvas column.</div>
              {/if}
            </div>

            <div class="b-col-head">
              Sort
              <button class="btn small ghost" onclick={addOrder} disabled={colOptions.length === 0}>
                <Icon name="plus" size={10} />Add
              </button>
            </div>
            <div class="where-list">
              {#each orders as o, oi (oi)}
                <div class="where-row">
                  <select class="input" bind:value={o.ref}>
                    {#each colOptions as c (c.ref)}<option value={c.ref}>{c.label}</option>{/each}
                  </select>
                  <select class="input op" bind:value={o.dir}>
                    <option value="ASC">ASC</option>
                    <option value="DESC">DESC</option>
                  </select>
                  <span class="grow"></span>
                  <button class="icon-btn" aria-label="Remove sort" onclick={() => removeOrder(oi)}>
                    <Icon name="x" size={11} />
                  </button>
                </div>
              {/each}
              {#if orders.length === 0}
                <div class="b-hint">No sorting. Add an ORDER BY across any canvas column.</div>
              {/if}
            </div>

            <div class="limit-row">
              <label for="qb-limit">Limit</label>
              <input id="qb-limit" class="input mono" type="number" bind:value={limit} min="0" />
            </div>

            <!-- Expression / computed columns (IF, CASE, built-ins) -->
            <div class="expr-section">
              <button class="expr-toggle" onclick={() => (exprsOpen = !exprsOpen)}>
                <Icon name={exprsOpen ? 'chevronDown' : 'chevronRight'} size={11} />
                Expressions
                {#if exprs.length}<span class="expr-count">{exprs.length}</span>{/if}
                <span class="grow"></span>
                <span
                  class="btn small ghost"
                  role="button"
                  tabindex="0"
                  onclick={(e) => {
                    e.stopPropagation();
                    addExpr();
                  }}
                  onkeydown={(e) => {
                    if (e.key === 'Enter' || e.key === ' ') {
                      e.preventDefault();
                      addExpr();
                    }
                  }}
                >
                  <Icon name="plus" size={10} />Add
                </span>
              </button>
              {#if exprsOpen}
                <div class="expr-list">
                  {#each exprs as e, ei (e.id)}
                    <div class="expr-row">
                      <div class="expr-main">
                        <input
                          class="input mono expr-input"
                          value={e.expression}
                          placeholder="IF(a.x > 0, 'yes', 'no')"
                          spellcheck="false"
                          oninput={(ev) =>
                            setExprField(e.id, 'expression', (ev.currentTarget as HTMLInputElement).value)}
                        />
                        <span class="expr-as">AS</span>
                        <input
                          class="input mono expr-alias"
                          value={e.alias}
                          placeholder={`expr_${ei + 1}`}
                          spellcheck="false"
                          oninput={(ev) =>
                            setExprField(e.id, 'alias', (ev.currentTarget as HTMLInputElement).value)}
                        />
                        <button class="icon-btn" aria-label="Remove expression" onclick={() => removeExpr(e.id)}>
                          <Icon name="x" size={11} />
                        </button>
                      </div>
                      <div class="expr-snips">
                        <button
                          class="snip"
                          onclick={() => insertSnippet(e.id, 'IF(condition, then_value, else_value)')}
                        >IF</button>
                        <button
                          class="snip"
                          onclick={() =>
                            insertSnippet(e.id, 'CASE WHEN condition THEN value ELSE value END')}
                        >CASE</button>
                        <div class="fn-wrap">
                          <button
                            class="snip"
                            onclick={() => (fnMenuFor = fnMenuFor === e.id ? null : e.id)}
                          >fn ▾</button>
                          {#if fnMenuFor === e.id}
                            <div class="fn-menu">
                              {#each FN_SNIPPETS as fn (fn)}
                                <button class="fn-item mono" onclick={() => insertSnippet(e.id, fn)}>{fn}</button>
                              {/each}
                            </div>
                          {/if}
                        </div>
                      </div>
                    </div>
                  {/each}
                  {#if exprs.length === 0}
                    <div class="b-hint">No expression columns. Add IF / CASE / function columns to SELECT.</div>
                  {/if}
                </div>
              {/if}
            </div>

            {#if unreached.length}
              <div class="b-warn">
                <Icon name="info" size={11} />
                Not connected (excluded from SQL): {unreached.map((t) => t.alias).join(', ')} — drag a join to include.
              </div>
            {/if}
          </div>

          <!-- Generated SQL -->
          <div class="b-col sql">
            <div class="b-col-head">
              Generated SQL
              <div class="b-actions">
                <button class="btn small" onclick={() => openInQuery(false)} disabled={!generatedSql}>
                  <Icon name="external" size={11} />Open in Query
                </button>
                <button class="btn small primary" onclick={() => openInQuery(true)} disabled={!generatedSql}>
                  <Icon name="play" size={11} />Run
                </button>
              </div>
            </div>
            <pre class="gen-sql mono">{generatedSql || '— add a table to start —'}</pre>
          </div>
        </div>
      {/if}
    </div>
  </div>
{/if}

<style>
  .builder {
    height: 100%;
    display: grid;
    grid-template-columns: 220px 1fr;
    grid-template-rows: 1fr auto;
    grid-template-areas:
      'palette canvas'
      'bottom  bottom';
    gap: 0;
    overflow: hidden;
  }

  /* ── Palette ── */
  .palette {
    grid-area: palette;
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px;
    border-inline-end: 1px solid var(--border);
    background: var(--surface);
    min-height: 0;
  }
  .pal-head {
    font-size: 11px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-dim);
  }
  .db-select {
    width: 100%;
  }
  .pal-search {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 27px;
    padding: 0 9px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text-dim);
  }
  .pal-search-input {
    flex: 1;
    min-width: 0;
    border: none;
    background: transparent;
    color: var(--text);
    font-size: 12.5px;
    outline: none;
  }
  .pal-list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 1px;
    margin: 0 -4px;
    padding: 0 4px;
  }
  .pal-item {
    display: flex;
    align-items: center;
    gap: 7px;
    width: 100%;
    height: 26px;
    padding: 0 8px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text);
    font-size: 12px;
    cursor: pointer;
    text-align: start;
  }
  .pal-item:hover {
    background: var(--surface-2);
  }
  .pal-item :global(svg:first-child) {
    color: var(--text-dim);
    flex-shrink: 0;
  }
  .pal-item-label {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .pal-item :global(svg:last-child) {
    color: var(--accent);
    opacity: 0;
    flex-shrink: 0;
  }
  .pal-item:hover :global(svg:last-child) {
    opacity: 1;
  }
  .pal-hint {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px;
    font-size: 11.5px;
    color: var(--text-dim);
  }

  /* ── Canvas ── */
  .canvas {
    grid-area: canvas;
    position: relative;
    overflow: hidden;
    min-width: 0;
  }
  .canvas-scroll {
    position: absolute;
    inset: 0;
    overflow: auto;
    background:
      radial-gradient(circle, color-mix(in srgb, var(--text-dim) 22%, transparent) 1px, transparent 1px);
    background-size: 18px 18px;
    background-color: var(--bg);
    touch-action: none;
  }
  .canvas-scroll.dragging {
    user-select: none;
    cursor: grabbing;
  }
  .content {
    position: relative;
    /* width/height set inline to fit all cards */
  }
  .edges {
    position: absolute;
    top: 0;
    left: 0;
    pointer-events: none;
    z-index: 1;
  }
  .edge {
    fill: none;
    stroke: var(--accent);
    stroke-width: 1.6;
    pointer-events: stroke;
    cursor: pointer;
    opacity: 0.85;
    transition: stroke-width 120ms ease-out, opacity 120ms ease-out;
  }
  .edge:hover {
    stroke-width: 2.6;
    opacity: 1;
  }
  .edge.active {
    stroke-width: 2.6;
    opacity: 1;
  }
  .edge.pending {
    stroke-dasharray: 5 4;
    opacity: 0.7;
    pointer-events: none;
  }
  /* An endpoint is scrolled out of its card — anchored to the card edge and
     dimmed/dashed to signal the real column is hidden. */
  .edge.offscreen {
    stroke-dasharray: 3 3;
    opacity: 0.5;
  }

  .edge-pop {
    position: absolute;
    z-index: 5;
    transform: translate(-50%, -50%);
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px;
    border-radius: var(--radius-m);
    background: var(--surface);
    border: 1px solid var(--border);
    box-shadow: var(--shadow);
  }
  .seg {
    display: inline-flex;
    background: var(--surface-2);
    border-radius: var(--radius-s);
    padding: 2px;
    gap: 2px;
  }
  .seg-btn {
    border: none;
    background: transparent;
    height: 20px;
    padding: 0 7px;
    border-radius: 4px;
    font-size: 10.5px;
    font-weight: 600;
    color: var(--text-dim);
    cursor: pointer;
  }
  .seg-btn.on {
    background: var(--accent);
    color: var(--accent-contrast);
  }

  /* ── Table card ── */
  .node {
    position: absolute;
    z-index: 2;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    box-shadow: var(--shadow);
    overflow: hidden;
  }
  .node-head {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 30px;
    padding: 0 6px 0 9px;
    background: var(--surface-2);
    border-bottom: 1px solid var(--border);
    cursor: grab;
    color: var(--accent);
  }
  .node-head:active {
    cursor: grabbing;
  }
  .alias-input {
    width: 78px;
    height: 20px;
    padding: 0 5px;
    border: 1px solid transparent;
    border-radius: 4px;
    background: transparent;
    color: var(--text);
    font-size: 12px;
    font-weight: 600;
    outline: none;
  }
  .alias-input:hover {
    border-color: var(--border);
  }
  .alias-input:focus {
    border-color: var(--accent);
    background: var(--surface);
  }
  .node-src {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 10.5px;
    color: var(--text-dim);
    text-align: right;
  }
  .node-x {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 18px;
    height: 18px;
    border: none;
    border-radius: 4px;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    flex-shrink: 0;
  }
  .node-x:hover {
    background: color-mix(in srgb, var(--status-exited) 16%, transparent);
    color: var(--status-exited);
  }
  .node-body {
    padding: 6px 0;
    /* Tall enough that typical tables fit without an internal scrollbar; only
       very wide tables (~40+ columns) scroll. Keep in sync with BODY_MAX_H. */
    max-height: 900px;
    overflow-y: auto;
  }
  .node-loading {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 12px;
    font-size: 11px;
    color: var(--text-dim);
  }
  /* 5-part row: checkbox · name (flex) · type · aggregate · key badge.
     min-width:0 + ellipsis on the text cells means long names never collide. */
  .col-row {
    position: relative;
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) auto auto auto;
    align-items: center;
    column-gap: 6px;
    padding: 0 11px;
    font-size: 11.5px;
  }
  .col-row:hover {
    background: color-mix(in srgb, var(--accent) 8%, transparent);
  }
  .col-cb {
    accent-color: var(--accent);
    cursor: pointer;
    margin: 0;
  }
  .col-name {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: var(--font-mono);
    font-size: 11px;
  }
  .col-name.pk {
    color: var(--accent);
    font-weight: 600;
  }
  .col-name.fk {
    font-weight: 600;
  }
  .col-ty {
    max-width: 70px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 9.5px;
    color: var(--text-dim);
    text-align: right;
  }
  /* Per-column aggregate dropdown — compact; the spacer keeps the grid track
     aligned for unselected columns. */
  .col-agg {
    height: 16px;
    max-width: 58px;
    padding: 0 2px;
    border: 1px solid transparent;
    border-radius: 3px;
    background: transparent;
    color: var(--text-dim);
    font-size: 9px;
    cursor: pointer;
  }
  .col-agg:hover {
    border-color: var(--border);
  }
  .col-agg.set {
    color: var(--accent);
    border-color: color-mix(in srgb, var(--accent) 45%, transparent);
    font-weight: 700;
  }
  .col-agg.spacer {
    width: 0;
    max-width: 0;
    padding: 0;
    pointer-events: none;
  }
  /* Key badge — small pill, right-aligned; spacer keeps grid columns aligned
     across rows that have no PK/FK. */
  .col-badge {
    justify-self: end;
    min-width: 18px;
    height: 14px;
    padding: 0 4px;
    border-radius: 3px;
    font-size: 8.5px;
    font-weight: 700;
    line-height: 14px;
    text-align: center;
    letter-spacing: 0.03em;
  }
  .col-badge.pk {
    background: color-mix(in srgb, var(--accent) 18%, transparent);
    color: var(--accent);
  }
  .col-badge.fk {
    background: color-mix(in srgb, var(--text-dim) 18%, transparent);
    color: var(--text-dim);
  }
  .col-badge.spacer {
    background: transparent;
  }
  .handle {
    position: absolute;
    top: 50%;
    width: 11px;
    height: 11px;
    margin-top: -5.5px;
    border-radius: 50%;
    background: var(--surface);
    border: 1.5px solid color-mix(in srgb, var(--accent) 55%, transparent);
    cursor: crosshair;
    z-index: 3;
    transition: transform 110ms ease-out, background 110ms ease-out;
  }
  .handle.l {
    left: -5.5px;
  }
  .handle.r {
    right: -5.5px;
  }
  .col-row:hover .handle,
  .handle:hover {
    background: var(--accent);
    transform: scale(1.25);
  }

  .canvas-hint {
    position: absolute;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    max-width: 280px;
    text-align: center;
    color: var(--text-dim);
    pointer-events: none;
  }
  .canvas-hint p {
    margin: 0;
    font-size: 12.5px;
    line-height: 1.5;
  }

  .fk-bar {
    position: absolute;
    inset-inline-start: 12px;
    bottom: 12px;
    inset-inline-end: 12px;
    z-index: 4;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    padding: 8px 10px;
    border-radius: var(--radius-m);
    background: color-mix(in srgb, var(--surface) 92%, transparent);
    backdrop-filter: blur(8px);
    border: 1px solid var(--border);
  }
  .fk-label {
    font-size: 10.5px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-dim);
  }
  .fk-chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 22px;
    padding: 0 9px;
    border-radius: 999px;
    border: 1px dashed color-mix(in srgb, var(--accent) 45%, transparent);
    background: transparent;
    color: var(--accent);
    font-size: 11px;
    font-family: var(--font-mono);
    cursor: pointer;
  }
  .fk-chip:hover {
    background: color-mix(in srgb, var(--accent) 12%, transparent);
  }

  /* ── Bottom panel ── */
  .bottom {
    grid-area: bottom;
    border-top: 1px solid var(--border);
    background: var(--surface);
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .bottom-toggle {
    display: flex;
    align-items: center;
    gap: 7px;
    height: 30px;
    padding: 0 12px;
    border: none;
    background: transparent;
    color: var(--text-dim);
    font-size: 11px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    cursor: pointer;
  }
  .bottom-toggle:hover {
    color: var(--text);
  }
  .warn-pill {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    margin-inline-start: 6px;
    padding: 0 7px;
    height: 17px;
    border-radius: 999px;
    background: color-mix(in srgb, #d2691e 18%, transparent);
    color: #d2691e;
    font-size: 10px;
    text-transform: none;
    letter-spacing: 0;
  }
  .bottom-body {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 14px;
    padding: 0 14px 14px;
    max-height: 280px;
    overflow-y: auto;
  }
  .b-col {
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-width: 0;
  }
  .b-col-head {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 11px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-dim);
  }
  .b-col-head .btn,
  .b-actions {
    margin-inline-start: auto;
    text-transform: none;
    letter-spacing: 0;
  }
  .b-actions {
    display: flex;
    gap: 8px;
  }
  .where-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .where-row {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .where-row .input {
    flex: 1;
    min-width: 0;
  }
  .where-row .op {
    flex: 0 0 88px;
  }
  .b-hint,
  .b-warn {
    font-size: 11.5px;
    color: var(--text-dim);
    line-height: 1.5;
  }
  .b-warn {
    display: flex;
    align-items: flex-start;
    gap: 6px;
    color: #d2691e;
  }
  .limit-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .limit-row label {
    font-size: 11.5px;
    color: var(--text-dim);
  }
  .limit-row .input {
    width: 90px;
  }

  /* ── Expression columns ── */
  .expr-section {
    display: flex;
    flex-direction: column;
    gap: 6px;
    border-top: 1px solid var(--border);
    padding-top: 8px;
  }
  .expr-toggle {
    display: flex;
    align-items: center;
    gap: 6px;
    border: none;
    background: transparent;
    color: var(--text-dim);
    font-size: 11px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    cursor: pointer;
    padding: 0;
  }
  .expr-toggle:hover {
    color: var(--text);
  }
  .expr-toggle .btn {
    text-transform: none;
    letter-spacing: 0;
  }
  .expr-count {
    display: inline-grid;
    place-items: center;
    min-width: 16px;
    height: 16px;
    padding: 0 4px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--accent) 18%, transparent);
    color: var(--accent);
    font-size: 10px;
    text-transform: none;
  }
  .expr-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .expr-row {
    display: flex;
    flex-direction: column;
    gap: 5px;
  }
  .expr-main {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .expr-input {
    flex: 1;
    min-width: 0;
  }
  .expr-as {
    font-size: 10px;
    font-weight: 700;
    color: var(--text-dim);
  }
  .expr-alias {
    width: 96px;
    flex: 0 0 auto;
  }
  .expr-snips {
    display: flex;
    align-items: center;
    gap: 5px;
    padding-inline-start: 2px;
  }
  .snip {
    height: 19px;
    padding: 0 8px;
    border-radius: 999px;
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text-dim);
    font-size: 10.5px;
    font-weight: 600;
    cursor: pointer;
  }
  .snip:hover {
    color: var(--accent);
    border-color: color-mix(in srgb, var(--accent) 45%, transparent);
  }
  .fn-wrap {
    position: relative;
  }
  .fn-menu {
    position: absolute;
    top: calc(100% + 4px);
    inset-inline-start: 0;
    z-index: 6;
    display: flex;
    flex-direction: column;
    min-width: 140px;
    padding: 4px;
    border-radius: var(--radius-m);
    background: var(--surface);
    border: 1px solid var(--border);
    box-shadow: var(--shadow);
  }
  .fn-item {
    text-align: start;
    padding: 4px 7px;
    border: none;
    border-radius: 4px;
    background: transparent;
    color: var(--text);
    font-size: 11px;
    cursor: pointer;
  }
  .fn-item:hover {
    background: var(--surface-2);
    color: var(--accent);
  }

  .gen-sql {
    margin: 0;
    flex: 1;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 12px;
    font-size: 12px;
    line-height: 1.55;
    white-space: pre-wrap;
    user-select: text;
    color: var(--text);
    min-height: 80px;
  }
  .grow {
    flex: 1;
  }
</style>
