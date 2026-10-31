///<reference types="svelte" />
;
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
import LoadState from '../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../lib/loadError';
import { database } from '../../lib/stores/database.svelte';
import type { DbSchemaGraph, DbGraphTable } from '../../lib/api/types';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

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
;
async () => {

 { svelteHTML.createElement("div", { "class":`diagram`,});
   { svelteHTML.createElement("aside", { "class":`picker`,});
     { svelteHTML.createElement("div", { "class":`picker-top`,});
       { svelteHTML.createElement("label", {   "class":`lbl`,"for":`erd-db`,});  }
       { svelteHTML.createElement("select", {       "id":`erd-db`,"class":`db-select`,"bind:value":selectedDb,"onchange":onDbChange,});/*Ωignore_startΩ*/() => selectedDb = __sveltets_2_any(null);/*Ωignore_endΩ*/
           for(let d of __sveltets_2_ensureArray(databases)){d.path;
           { svelteHTML.createElement("option", { "value":d.path,});d.name || '(default)'; }
        }
       }
     }

    if(graph){
       { svelteHTML.createElement("div", { "class":`picker-tools`,});
         { svelteHTML.createElement("input", {          "dir":`auto`,"aria-label":`Filter tables`,"class":`search`,"placeholder":`Filter tables…`,"bind:value":tableSearch,});/*Ωignore_startΩ*/() => tableSearch = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         { svelteHTML.createElement("div", { "class":`quick`,});
           { svelteHTML.createElement("button", {     "class":`link`,"onclick":showRelatedOnly,"title":`Show only tables with relationships`,});  }
           { svelteHTML.createElement("button", {   "class":`link`,"onclick":showAll,});  }
           { svelteHTML.createElement("button", {   "class":`link`,"onclick":showNone,});  }
         }
       }
       { svelteHTML.createElement("div", { "class":`tlist`,});
           for(let t of __sveltets_2_ensureArray(filteredTables)){t.id;
           { svelteHTML.createElement("label", {  "class":`trow`,});shown.has(t.id);
             { svelteHTML.createElement("input", {      "type":`checkbox`,"checked":shown.has(t.id),"onchange":() => toggleTable(t.id),});}
             { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":t.kind === 'view' ? 'eye' : 'grid',"size":12,}});}
             { svelteHTML.createElement("span", {   "class":`tname`,"title":t.name,});t.name; }
             { svelteHTML.createElement("span", { "class":`tmeta`,});t.columns.length; }
           }
        }
        if(!filteredTables.length){
           { svelteHTML.createElement("p", { "class":`muted small`,});   }
        }
       }
       { svelteHTML.createElement("div", { "class":`picker-foot muted small`,});
        shown.size;  graph.tables.length; 
        if(graph.truncated){ }
       }
    }
   }

   { svelteHTML.createElement("div", { "class":`stage`,});
     { svelteHTML.createElement("div", { "class":`toolbar`,});
       { svelteHTML.createElement("div", { "class":`zoom`,});
         { svelteHTML.createElement("button", {       "class":`zb`,"onclick":() => zoomBy(0.9),"title":`Zoom out`,"aria-label":`Zoom out`,});  }
         { svelteHTML.createElement("span", { "class":`zlvl mono`,});Math.round(zoom * 100);  }
         { svelteHTML.createElement("button", {       "class":`zb`,"onclick":() => zoomBy(1.1),"title":`Zoom in`,"aria-label":`Zoom in`,});  }
         { svelteHTML.createElement("button", {       "class":`zb`,"onclick":resetView,"title":`Reset view`,"aria-label":`Reset view`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`maximize`,"size":12,}});} }
         { svelteHTML.createElement("button", {       "class":`zb`,"onclick":relayout,"title":`Re-layout cards`,"aria-label":`Re-layout`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`grid`,"size":12,}});} }
       }
       { svelteHTML.createElement("span", { "class":`grow`,}); }
      if(graph && !graph.relationships){
         { svelteHTML.createElement("span", {   "class":`rel-hint`,"title":`This engine doesn’t expose foreign-key relationships`,});
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`info`,"size":12,}});}  
         }
      } else if (graph){
         { svelteHTML.createElement("span", {   "class":`rel-hint mono`,"title":`Foreign-key edges drawn`,});plural(edgePaths.length, 'relationship'); }
      }
     }

    if(graph?.truncated){
      
       { svelteHTML.createElement("div", {   "class":`trunc-banner`,"role":`status`,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`info`,"size":12,}});}
         { svelteHTML.createElement("span", {});
                graph.tables.length;    
                       { svelteHTML.createElement("strong", {});  }
         }
       }
    }

    if(loading){
       { svelteHTML.createElement("div", { "class":`center`,}); { const $$_notelekS3C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS3C({ target: __sveltets_2_any(), props: {      "rows":4,"height":64,"label":`the diagram`,}});} }
    } else if (graphError){
       { svelteHTML.createElement("div", { "class":`center`,}); { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {      "what":`the diagram`,"error":graphError,"empty":true,"onretry":() => void loadGraph(),}});} }
    } else if (!graph){
       { svelteHTML.createElement("div", { "class":`center`,}); { const $$_etatSytpmE3C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE3C({ target: __sveltets_2_any(), props: {      "icon":`link`,"title":`No schema selected`,"body":`Pick a schema to draw its tables and relationships.`,}});} }
    } else if (shown.size === 0){
       { svelteHTML.createElement("div", { "class":`center`,}); { const $$_etatSytpmE3C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE3C({ target: __sveltets_2_any(), props: {      "icon":`grid`,"title":`No tables shown`,"body":`Pick tables on the left to add them to the diagram.`,}});} }
    }else{
       { const $$_div2 = svelteHTML.createElement("div", {                   "role":`group`,"aria-label":`Schema diagram — drag to pan, scroll to zoom`,"class":`viewport`,"onpointerdown":onPointerDown,"onpointermove":onPointerMove,"onpointerup":onPointerUp,"onpointercancel":onPointerUp,"onwheel":onWheel,});panning;viewportEl = $$_div2;
         { svelteHTML.createElement("div", {     "class":`content`,"style":`width:${contentSize.w}px; height:${contentSize.h}px; transform: translate(${panX}px, ${panY}px) scale(${zoom}); transform-origin: 0 0;`,});
           { svelteHTML.createElement("svg", {       "class":`edges`,"width":contentSize.w,"height":contentSize.h,"aria-hidden":`true`,});
             { svelteHTML.createElement("defs", {});
               { svelteHTML.createElement("marker", {             "id":`erd-arrow`,"viewBox":`0 0 10 10`,"refX":`8`,"refY":`5`,"markerWidth":`7`,"markerHeight":`7`,"orient":`auto-start-reverse`,});
                 { svelteHTML.createElement("path", {    "d":`M0 0 L10 5 L0 10 z`,"fill":`var(--accent)`,});}
               }
             }
               for(let p of __sveltets_2_ensureArray(edgePaths)){p.id;
               { svelteHTML.createElement("path", {     "class":`edge`,"d":p.d,"marker-end":`url(#erd-arrow)`,}); { svelteHTML.createElement("title", {});p.label; } }
            }
           }

             for(let t of __sveltets_2_ensureArray(shownTables)){t.id;
            const p = positions.get(t.id);
            if(p){
               { svelteHTML.createElement("div", {   "class":`node`,"style":`left:${p.x}px; top:${p.y}px; width:${CARD_W}px`,});
                
                 { svelteHTML.createElement("div", {         "class":`node-head`,"role":`presentation`,"onpointerdown":(e) => startCardDrag(e, t.id),"ondblclick":() => openStructure(t),"title":`Drag to move · double-click for Structure`,});
                   { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":t.kind === 'view' ? 'eye' : 'grid',"size":12,}});}
                   { svelteHTML.createElement("span", {   "class":`node-title`,"title":t.name,});t.name; }
                  if(t.kind === 'view'){ { svelteHTML.createElement("span", { "class":`kind-tag`,});  }}
                 }
                 { svelteHTML.createElement("div", { "class":`node-body`,});
                     for(let c of __sveltets_2_ensureArray(t.columns)){c.name;
                     { svelteHTML.createElement("div", {   "class":`col-row`,"style":`height:${ROW_H}px`,});
                       { svelteHTML.createElement("span", { "class":`col-marker`,});
                        if(c.primary_key){ { svelteHTML.createElement("span", {   "class":`keymark pk`,"title":`Primary key`,});  }
                        } else if (c.foreign_key){ { svelteHTML.createElement("span", {   "class":`keymark fk`,"title":`Foreign key`,});  }}
                       }
                       { svelteHTML.createElement("span", {     "class":`col-name`,"title":c.name,});c.primary_key;c.foreign_key;c.name; }
                       { svelteHTML.createElement("span", {    "class":`col-ty`,"title":c.data_type,});c.nullable;c.data_type; }
                     }
                  }
                  if(!t.columns.length){
                     { svelteHTML.createElement("div", { "class":`col-row empty muted`,});  }
                  }
                 }
               }
            }
          }
         }
       }
    }
   }
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const DiagramView__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type DiagramView__SvelteComponent_ = ReturnType<typeof DiagramView__SvelteComponent_>;
/*Ωignore_endΩ*/export default DiagramView__SvelteComponent_;