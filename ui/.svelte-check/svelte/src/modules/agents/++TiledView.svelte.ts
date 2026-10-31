///<reference types="svelte" />
;// Tiled view: every active session in the workspace shown at once (loom-style).
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
import { EMBED_SCROLLBACK } from '../../lib/components/termFlow';
import EmptyState from '../../lib/components/EmptyState.svelte';
import StatusDot from '../../lib/components/StatusDot.svelte';
import { events } from '../../lib/events.svelte';
import { sessionState, type SessionLike, type SessionStateInfo } from '../../lib/status';
import Icon from '../../lib/components/Icon.svelte';
import { splitter } from '../../lib/paneResizer';
import { ws } from '../../lib/stores/workspace.svelte';
import { ui } from '../../lib/stores/ui.svelte';
import { viewport } from '../../lib/stores/viewport.svelte';
import { winKey } from '../../lib/win';
import { MAX_PANES, applyTileOrder } from '../../lib/stores/splitLayout';
import { layout } from '../../lib/stores/splitLayout.svelte';
import { liveTracks, type Tracks } from './tileTracks';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  // Max number of tiles allowed to hold a live terminal/WS at once. Visible,
  // recently-focused tiles win the budget; everything else stays a placeholder.
  // ONE source of truth with the split view's pane cap.
  const MAX_LIVE_TILES = MAX_PANES;
  // Per-tile xterm scrollback (SA-05): up to MAX_LIVE_TILES terminals are live
  // at once, each line ~12 B/cell. The maximized tile keeps SessionView's
  // 10k primary default, and the daemon's 4000-row snapshot restores depth on
  // maximize/reconnect.
  const TILE_SCROLLBACK = EMBED_SCROLLBACK;

  // C3a: the user's drag order on top of the store order (unknown ids append,
  // dead ids are skipped) — everything below counts/lays out `ordered`.
  const ordered = $derived(applyTileOrder(ws.mainSessions, layout.tileOrder));

  // Column count scales with tile count: 1→1, 2-4→2, 5-9→3, 10+→4, capped on
  // phone (1 col) and tablet (2 cols) so tiles stay readable.
  const cols = $derived.by(() => {
    const n = ordered.length;
    const raw = n <= 1 ? 1 : n <= 4 ? 2 : n <= 9 ? 3 : 4;
    if (viewport.isPhone) return 1;
    if (viewport.isTablet) return Math.min(2, raw);
    return raw;
  });

  // Explicit row count so every tile fits the viewport (no clipped bottom row).
  const rows = $derived(Math.max(1, Math.ceil(ordered.length / cols)));

  // ── Resizable tiles ──────────────────────────────────────────────────────
  // Every ROW of the grid is its own track list, so dragging the edge between
  // two tiles moves ONLY those two (the tile above / below keeps its width),
  // and the edge between two rows moves only those rows. Each tile also has a
  // corner grip that drags its width and height at once, like a window.
  // Weights are kept per grid SHAPE (`cols×rows`) and per workspace, so a 2×2
  // arrangement remembers its sizes when a third row comes and goes.
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
  /** Tiles grouped into rows of `cols` (the last row may be shorter). */
  const tileRows = $derived.by(() => {
    const out: (typeof ordered)[] = [];
    for (let r = 0; r < rows; r++) out.push(ordered.slice(r * cols, (r + 1) * cols));
    return out;
  });
  /** The weights as rendered — the stored ones normalized to the live row
   *  lengths (see tileTracks.ts). */
  const rendered = $derived(liveTracks(tracks[shapeKey], tileRows.map((tiles) => tiles.length)));
  const rowFr = $derived(rendered.rows);
  const colFr = $derived(rendered.cols);
  const gridStyle = $derived(`grid-template-rows: ${rowFr.map((f) => `minmax(220px, ${f}fr)`).join(' 8px ')};`);
  const rowStyle = (r: number): string => `grid-template-columns: ${colFr[r].map((f) => `minmax(0, ${f}fr)`).join(' 8px ')};`;
  function saveTracks(): void {
    try {
      localStorage.setItem(tracksKey(), JSON.stringify(tracks));
    } catch {
      /* private mode */
    }
  }
  /** A fresh, editable copy of the RENDERED weights. Edits start here, never
   *  from the raw stored arrays (S14-302: a stale per-shape row was dropped
   *  as a length mismatch, so the divider would not move). */
  function currentTracks(): Tracks {
    return liveTracks(tracks[shapeKey], tileRows.map((tiles) => tiles.length));
  }
  let resizing = $state(false);
  /** One pair of neighbouring tracks measured for a drag: the pointer offset
   *  inside `span` (the two neighbours together) becomes their weight split. */
  interface PairDrag {
    start: number;
    span: number;
    total: number;
    rtl?: boolean;
    apply: (t: Tracks, first: number) => void;
  }
  function colPair(r: number, k: number): PairDrag | null {
    const grid = gridEl;
    if (!grid || k < 1 || k >= (colFr[r]?.length ?? 0)) return null;
    const a = grid.querySelector<HTMLElement>(`.tile-slot[data-row="${r}"][data-col="${k - 1}"]`);
    const b = grid.querySelector<HTMLElement>(`.tile-slot[data-row="${r}"][data-col="${k}"]`);
    if (!a || !b) return null;
    const ra = a.getBoundingClientRect();
    const rb = b.getBoundingClientRect();
    const rtl = getComputedStyle(grid).direction === 'rtl';
    const total = colFr[r][k - 1] + colFr[r][k];
    return {
      start: rtl ? ra.right : ra.left,
      span: rtl ? ra.right - rb.left : rb.right - ra.left,
      total,
      rtl,
      apply: (t, first) => {
        t.cols[r][k - 1] = first;
        t.cols[r][k] = total - first;
      },
    };
  }
  function rowPair(k: number): PairDrag | null {
    const grid = gridEl;
    if (!grid || k < 1 || k >= rowFr.length) return null;
    const a = grid.querySelector<HTMLElement>(`.trow[data-row="${k - 1}"]`);
    const b = grid.querySelector<HTMLElement>(`.trow[data-row="${k}"]`);
    if (!a || !b) return null;
    const ra = a.getBoundingClientRect();
    const rb = b.getBoundingClientRect();
    const total = rowFr[k - 1] + rowFr[k];
    return {
      start: ra.top,
      span: rb.bottom - ra.top,
      total,
      apply: (t, first) => {
        t.rows[k - 1] = first;
        t.rows[k] = total - first;
      },
    };
  }
  function splitOf(p: PairDrag, pos: number): number {
    const lo = Math.min(MIN_TRACK_PX, p.span / 2);
    const clamped = Math.min(p.span - lo, Math.max(lo, pos));
    return (clamped / p.span) * p.total;
  }
  /** Drag `col` (the divider after column k-1 in row r) and/or `row` (the
   *  divider after row k-1) — both at once from a tile's corner grip. */
  function startDrag(e: PointerEvent, col: PairDrag | null, row: PairDrag | null): void {
    if (e.button !== 0 || (!col && !row)) return;
    e.preventDefault();
    e.stopPropagation();
    const shape = shapeKey;
    resizing = true;
    const move = (ev: PointerEvent): void => {
      if (shapeKey !== shape) return; // the grid reshaped under the drag
      const t = currentTracks();
      if (col) col.apply(t, splitOf(col, col.rtl ? col.start - ev.clientX : ev.clientX - col.start));
      if (row) row.apply(t, splitOf(row, ev.clientY - row.start));
      tracks = { ...tracks, [shape]: t };
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
  /** Double-click any divider or grip: every tile back to equal shares. */
  function resetTracks(): void {
    const { [shapeKey]: _gone, ...rest } = tracks;
    tracks = rest;
    saveTracks();
  }
  /** Keyboard resize on a focused divider: arrows nudge 5 % of the pair. */
  function trackKeydown(e: KeyboardEvent, col: PairDrag | null, row: PairDrag | null): void {
    const p = col ?? row;
    if (!p) return;
    const rtl = gridEl ? getComputedStyle(gridEl).direction === 'rtl' : false;
    const dec = col ? (rtl ? e.key === 'ArrowRight' : e.key === 'ArrowLeft') : e.key === 'ArrowUp';
    const inc = col ? (rtl ? e.key === 'ArrowLeft' : e.key === 'ArrowRight') : e.key === 'ArrowDown';
    if (!dec && !inc) return;
    e.preventDefault();
    const t = currentTracks();
    // Current first-track share, nudged and clamped to 10..90 % of the pair.
    const first = Math.min(p.total * 0.9, Math.max(p.total * 0.1, firstOf(t, col, row) + p.total * 0.05 * (inc ? 1 : -1)));
    p.apply(t, first);
    tracks = { ...tracks, [shapeKey]: t };
    saveTracks();
  }
  function firstOf(t: Tracks, col: PairDrag | null, row: PairDrag | null): number {
    // Recover the pair's current first weight by applying a probe: apply(first)
    // writes first + (total - first), so read it back from a scratch copy.
    const probe: Tracks = { rows: [...t.rows], cols: t.cols.map((c) => [...c]) };
    const p = (col ?? row)!;
    p.apply(probe, -1);
    // The slot written with -1 is the first track; its current value lives in t.
    for (let r = 0; r < probe.cols.length; r++) for (let k = 0; k < probe.cols[r].length; k++) if (probe.cols[r][k] === -1) return t.cols[r][k];
    for (let k = 0; k < probe.rows.length; k++) if (probe.rows[k] === -1) return t.rows[k];
    return p.total / 2;
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

  /** The shared session state (lib/status.ts) for a placeholder tile. */
  function tileState(s: SessionLike & { id: string }): SessionStateInfo {
    return sessionState(s, ws.statusMap[s.id], ws.needsYou[s.id] === true, { stale: events.state !== 'connected' });
  }
  /** Placeholder subline: only a session with no live PTY is "suspended" /
   *  "ended" — a running-but-over-budget/off-screen one is just not attached. */
  function placeholderHint(st: SessionStateInfo): string {
    return st.inactive ? (st.hint ?? st.label) : `${st.label} — not attached`;
  }
;
async () => {

if(ws.mainSessions.length === 0){
   { svelteHTML.createElement("div", { "class":`tiled-empty`,});
     { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {               "icon":`terminal`,"title":`No active sessions`,"body":`Start an agent or a shell (⌘T). In tiled view you’ll see every session at once.`,"variant":`page`,"actionIcon":`plus`,"actionLabel":`New session…`,"onaction":() => (ui.newSessionOpen = true),}});}
   }
} else if (maxed){
   { svelteHTML.createElement("div", { "class":`tiled single`,});
    maxed.id; {
       { const $$_weiVnoisseS1C = __sveltets_2_ensureComponent(SessionView); new $$_weiVnoisseS1C({ target: __sveltets_2_any(), props: {             "sessionId":maxed.id,"focused":true,"showClose":false,"showZoom":true,"onfocus":() => ws.openSession(maxed.id),"onclosepane":() => {},}});}
    }
   }
}else{
   { svelteHTML.createElement("div", { "class":`tiled-wrap`,});
  if(ordered.length > 1){
    
     { svelteHTML.createElement("div", { "class":`tiled-bar`,});
     { svelteHTML.createElement("button", {         "class":`btn small free-layout`,"onclick":() => {
        layout.layoutSessions(ordered.map((s) => s.id), 'grid');
        ws.setViewMode('tabs');
      },"title":`Turn these tiles into a free layout: every pane edge resizable, panes nestable, presets in the ⋯ menu`,...__sveltets_2_empty({"data-testid":`tiled-free-layout`}),}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`split`,"size":12,}});}   }
     }
  }
   { const $$_div1 = svelteHTML.createElement("div", {     "class":`tiled`,"style":gridStyle,});resizing;gridEl = $$_div1;
        for(let tiles of __sveltets_2_ensureArray(tileRows)){let r = 1;r;
      if(r > 0){
        
         {const $$action_0 = __sveltets_2_ensureAction(splitter(svelteHTML.mapElementTag('div'),({ onkeydown: (e) => trackKeydown(e, null, rowPair(r)), onpointerdown: (e) => startDrag(e, null, rowPair(r)), ondblclick: resetTracks })));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {                 "class":`tgut r-gut`,"role":`separator`,"aria-orientation":`horizontal`,"aria-label":`Resize rows (drag, arrow keys; double-click to equalise)`,"title":`Drag to resize · double-click to equalise`,"style":`grid-row: ${2 * r};`,...__sveltets_2_empty({"data-testid":`tile-divider-row`}),}); }}
      }
       { svelteHTML.createElement("div", {     "class":`trow`,...__sveltets_2_empty({"data-row":r}),"style":`grid-row: ${2 * r + 1}; ${rowStyle(r)}`,});
            for(let s of __sveltets_2_ensureArray(tiles)){let c = 1;s.id;
          if(c > 0){
            
             {const $$action_0 = __sveltets_2_ensureAction(splitter(svelteHTML.mapElementTag('div'),({ onkeydown: (e) => trackKeydown(e, colPair(r, c), null), onpointerdown: (e) => startDrag(e, colPair(r, c), null), ondblclick: resetTracks })));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {                 "class":`tgut col`,"role":`separator`,"aria-orientation":`vertical`,"aria-label":`Resize tiles (drag, arrow keys; double-click to equalise)`,"title":`Drag to resize · double-click to equalise`,"style":`grid-column: ${2 * c};`,...__sveltets_2_empty({"data-testid":`tile-divider-col`}),}); }}
          }
          
           {const $$action_0 = __sveltets_2_ensureAction(observeTile(svelteHTML.mapElementTag('div'),(s.id)));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {                        "role":`presentation`,"class":`tile-slot`,...__sveltets_2_empty({"data-tile-id":s.id}),...__sveltets_2_empty({"data-col":c}),...__sveltets_2_empty({"data-row":r}),"style":`grid-column: ${2 * c + 1};`,"ondragover":(e) => onTileDragOver(e, s.id),"ondragleave":() => onTileDragLeave(s.id),"ondrop":(e) => onTileDrop(e, s.id),"ondragend":onTileDragEnd,});tileDragOverId === s.id;
        if(liveIds.has(s.id)){
           { const $$_weiVnoisseS4C = __sveltets_2_ensureComponent(SessionView); new $$_weiVnoisseS4C({ target: __sveltets_2_any(), props: {                       "sessionId":s.id,"focused":ws.activeSessionId === s.id,"showClose":false,"showZoom":true,"showGrip":ordered.length > 1,"scrollback":TILE_SCROLLBACK,"resumeOnOpen":false,"dragKey":s.id,"ondragpane":(phase) => (tileDragId = phase === 'start' ? s.id : null),"onfocus":() => {
              ws.openSession(s.id);
              // make this the focused/active target without leaving tiled view
              layout.focusIndex(0);
            },"onclosepane":() => {},}});}
        }else{
          
           { svelteHTML.createElement("button", {       "class":`tile-placeholder`,"onclick":() => attach(s.id),"title":`Attach this session (opens its live terminal)`,});
            
             { svelteHTML.createElement("span", {           "role":`presentation`,"class":`ph-head`,"draggable":ordered.length > 1,"ondragstart":(e) => onTileDragStart(e, s.id),"ondragend":onTileDragEnd,});
               { const $$_toDsutatS6C = __sveltets_2_ensureComponent(StatusDot); new $$_toDsutatS6C({ target: __sveltets_2_any(), props: {  "state":tileState(s),}});}
               { svelteHTML.createElement("span", {   "class":`ph-title`,"title":s.title ?? s.id,});s.title ?? s.id; }
               { svelteHTML.createElement("span", { "class":`chip ph-chip`,});s.provider ?? '?'; }
             }
             { svelteHTML.createElement("div", { "class":`ph-body`,});
               { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`terminal`,"size":24,}});}
               { svelteHTML.createElement("span", { "class":`ph-cta`,});   }
              if(atCapacity){
                 { svelteHTML.createElement("span", { "class":`ph-hint`,});    MAX_LIVE_TILES;    }
              }else{
                 { svelteHTML.createElement("span", { "class":`ph-hint`,});placeholderHint(tileState(s)); }
              }
             }
           }
        }
            
            if(c < tiles.length - 1 || r < tileRows.length - 1){
              
               { svelteHTML.createElement("div", {               "role":`presentation`,"class":`tile-corner`,"title":`Drag to resize this tile · double-click to equalise`,...__sveltets_2_empty({"data-testid":`tile-corner`}),"onpointerdown":(e) => startDrag(e, c < tiles.length - 1 ? colPair(r, c + 1) : null, r < tileRows.length - 1 ? rowPair(r + 1) : null),"ondblclick":resetTracks,});c < tiles.length - 1;r < tileRows.length - 1; }
            }
           }}
        }
       }
    }
   }
   }
}


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const TiledView__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type TiledView__SvelteComponent_ = ReturnType<typeof TiledView__SvelteComponent_>;
/*Ωignore_endΩ*/export default TiledView__SvelteComponent_;