///<reference types="svelte" />
;// Vault v3 graph — Obsidian-style force-directed view of the whole vault, or
// (local mode) the depth-N neighborhood of the open note. Layout runs in a
// Web Worker (graph.worker.ts — Barnes-Hut over typed arrays, positions
// ping-pong back as transferred double-buffers); this component owns the
// Canvas2D renderer, camera, picking and the controls panel. Built to stay
// usable at 100k nodes / 1-2M edges: ONE batched path per edge style,
// deterministic index-stride edge sampling past a draw budget, viewport
// culling on nodes AND edges, a uniform spatial grid for hover/click/drag
// hit tests (never O(n) per mousemove), degree-capped label budget, and a
// dirty-flag rAF that is only requested when something changed.
//
// The payload the server sends is the RAW graph; it is never rendered
// directly. `project()` sits in between — it applies the focus filters
// (service / type / tag / anchor+hops) and the optional service rollup, and
// only its output reaches the renderer and the worker. That keeps every
// filter toggle a pure in-browser index walk: no refetch, no relayout stall.

import { vaultGraph, vaultSwitcher, type VaultGraphQuery } from '../../lib/api/vault';
import type { VaultGraphPayload, VaultSwitchHit } from '../../lib/api/types';
import { vault } from './vault.svelte';
import Icon from '../../lib/components/Icon.svelte';
import Badge from '../../lib/components/Badge.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../lib/loadError';
import { confirmer } from '../../lib/confirm.svelte';
import { ui } from '../../lib/stores/ui.svelte';
import { viewport } from '../../lib/stores/viewport.svelte';
import { paneResizer, pxWide, RESIZE_TITLE } from '../../lib/paneResizer';
import { startMouseDrag } from '../../lib/dragCursor';
import type { GraphWorkerIn, GraphWorkerOut } from './graph.worker';
import { carrySeed } from './graphSeed';
;

  // Facet lists for the rail (label + how many raw nodes carry it).
  type Facet = { label: string; count: number };;
;type $$ComponentProps =  { local?: boolean; onNewNote?: () => void };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { local = false, onNewNote }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();

  // ── Budgets / constants ─────────────────────────────────────────────────
  const EDGE_DRAW_BUDGET = 150_000; // past this: stride-sample edges + fade
  const LABEL_BUDGET = 200; // labels per frame (top-degree first)
  const LABEL_SCAN_CAP = 20_000; // max label-order entries scanned per frame
  const GRID_CELL_MIN = 64; // spatial-grid cell in world units (grows to cap cells)
  const CLICK_SLOP = 4; // px of pointer travel that still counts as a click

  // Node flag bits (VaultGraphPayload.flags).
  const F_GHOST = 1; // unresolved wikilink target
  const F_TAG = 2; // tag node
  const F_RESERVED = 4; // reserved file (openable, non-markdown)
  const F_GROUP = 128; // client-only: a rolled-up service/type node

  // ── Raw payload (plain lets — big arrays must NOT become $state proxies) ──
  let rawPaths: string[] = [];
  let rawTitles: string[] = [];
  let rawFlags = new Uint8Array(0);
  let rawTypes = new Uint16Array(0);
  let rawServices = new Uint16Array(0);
  let rawTagOff = new Uint32Array(0);
  let rawTagIds = new Uint16Array(0);
  let rawEdges = new Uint32Array(0);
  let typeLabels: string[] = [];
  let serviceLabels: string[] = [];
  let tagLabels: string[] = [];
  let rawIndex = new Map<string, number>(); // path → raw node index (anchors)
  let rawN = 0;
  let rawE = 0;
  // Raw adjacency (CSR) — rebuilt per payload, walked by the anchor BFS.
  let rawAdjOff = new Int32Array(1);
  let rawAdjList = new Int32Array(0);

  // ── Projected data (what actually renders) ──────────────────────────────
  let paths: string[] = [];
  let titles: string[] = [];
  let metas: string[] = []; // extra tooltip line (rolled nodes: member counts)
  let flagsArr = new Uint8Array(0);
  let groupsArr = new Uint16Array(0);
  let groupLabels: string[] = [];
  let edgeW: Float32Array | null = null; // per-edge weight (rollup only)
  let edges = new Uint32Array(0); // flat [a,b,…] kept for rendering
  let n = 0;
  let E = 0;
  let deg = new Uint32Array(0);
  let radii = new Float32Array(0); // world radius per node
  let adjOff = new Int32Array(0); // CSR adjacency: neighbors of i are
  let adjList = new Int32Array(0); //   adjList[adjOff[i] .. adjOff[i+1])
  let labelOrder = new Uint32Array(0); // node indices, degree-descending
  let pos: Float32Array<ArrayBuffer> | null = null; // latest tick [x0,y0,…] (worker-owned buffers)
  let matched: Uint8Array | null = null; // title-filter match per node (null = no filter)
  let hoverSet = new Set<number>(); // hovered node + its neighbors

  // ── UI state ────────────────────────────────────────────────────────────
  let loading = $state(false);
  let errorMsg = $state('');
  let nodeCount = $state(0);
  let edgeCount = $state(0);
  let truncated = $state(false);
  let alphaLive = $state(0); // last tick's alpha (drives "simulating…")
  let stopped = $state(false);
  let hoverIdx = $state(-1);
  let hoverTip = $state({ x: 0, y: 0, title: '', meta: '' });
  let draggingNode = $state(false); // suppresses the tooltip while dragging
  let dataRev = $state(0); // bumped per adopted payload (re-runs dependent effects)
  let panelOpen = $state(!viewport.isPhone);

  // Panel width is drag-resizable and persisted: service/tag names are long and
  // a fixed 210px truncated most of them. The handle is on the panel's LEFT edge
  // (the panel is anchored right), so dragging left widens it.
  let panelResizing = $state(false);
  function startPanelResize(e: MouseEvent): void {
    e.stopPropagation();
    panelResizing = true;
    const startX = e.clientX;
    const startW = ui.vaultGraphPanelWidth;
    // Anchored to the right edge, so a leftward drag (negative dx) GROWS it.
    // Mirrored under RTL, where the panel sits on the left instead.
    const dir = document.dir === 'rtl' ? 1 : -1;
    // lib/dragCursor (r3-03-08): overlay cursor, per-frame moves, one persist.
    startMouseDrag(e, {
      cursor: 'col-resize',
      onMove: (ev) => ui.setVaultGraphPanelWidth(startW + dir * (ev.clientX - startX), false),
      onEnd: () => {
        panelResizing = false;
        ui.setVaultGraphPanelWidth(ui.vaultGraphPanelWidth);
      },
    });
  }
  let rawNodeCount = $state(0); // pre-filter totals, for the "312 / 9,179" readout
  let rawEdgeCount = $state(0);

  // Server-side filters (these DO re-fetch — the server prunes them).
  let filter = $state('');
  let showTags = $state(true);
  let showOrphans = $state(true);
  let showGhosts = $state(true);
  // Reserved scaffolding (index.md / log.md) is navigation, not knowledge —
  // off by default so the graph shows real notes; the toggle brings them back.
  let showReserved = $state(false);
  let depth = $state(2); // local mode only

  // ── Focus filters (client-side — every toggle re-projects, never refetches).
  // AND across axes, OR within an axis; an empty axis means "no constraint".
  let selServices = $state<string[]>([]);
  let selTypes = $state<string[]>([]);
  let selTags = $state<string[]>([]);
  let anchorPaths = $state<string[]>([]); // BFS seeds; survive the axes above
  let hops = $state(1);
  let rollup = $state<'none' | 'service' | 'type'>('none');
  let expandedGroups = $state<string[]>([]); // rolled groups drilled open
  let colorBy = $state<'service' | 'type'>('service');

  let serviceFacets = $state<Facet[]>([]);
  let typeFacets = $state<Facet[]>([]);
  let tagFacets = $state<Facet[]>([]);
  let svcQuery = $state('');
  let tagQuery = $state('');

  // Anchor typeahead.
  let anchorQuery = $state('');
  let anchorHits = $state<VaultSwitchHit[]>([]);

  const hasFocus = $derived(
    selServices.length > 0 ||
      selTypes.length > 0 ||
      selTags.length > 0 ||
      anchorPaths.length > 0 ||
      rollup !== 'none',
  );
  const svcShown = $derived(matchFacets(serviceFacets, svcQuery));
  const tagShown = $derived(matchFacets(tagFacets, tagQuery));

  /** Facet list narrowed by its search box; selected entries always stay
   *  visible so a filtered search box can never hide what you turned on. */
  function matchFacets(all: Facet[], q: string): Facet[] {
    const t = q.trim().toLowerCase();
    if (!t) return all;
    return all.filter((f) => f.label.toLowerCase().includes(t));
  }

  // Forces (posted live to the worker).
  let fCenter = $state(0.1);
  let fRepel = $state(1);
  let fLink = $state(1);
  let fDist = $state(60);

  // Display (renderer-only — no re-fetch, no worker).
  let nodeScale = $state(1);
  let linkWidth = $state(1);
  let labelZoom = $state(1.2); // camera k at which labels start fading in

  // ── Canvas / camera (plain — everything renders imperatively) ───────────
  let rootEl: HTMLDivElement;
  let canvasEl: HTMLCanvasElement;
  let ctx: CanvasRenderingContext2D | null = null;
  let cssW = 0;
  let cssH = 0;
  let cam = { x: 0, y: 0, k: 1 }; // screen = (world - cam) * k + center
  let dirty = false;
  let raf = 0;
  let painting = false; // the canvas is mounted (between onMount and teardown)
  /** Mark the frame dirty and request ONE paint. Frames are requested only when
   *  something changed — an idle graph schedules no rAF at all (a perpetual
   *  "check dirty" loop woke WebContent 60×/s for nothing). */
  function invalidate(): void {
    dirty = true;
    if (painting && raf === 0) raf = requestAnimationFrame(paint);
  }
  function paint(): void {
    raf = 0;
    if (!dirty) return;
    dirty = false;
    draw();
  }
  let didFit = false; // auto zoom-to-fit once, on the first tick of a payload
  let ticks = 0;

  // Theme colors read from the app's tokens (lib/tokens.css) at mount and on
  // every theme/scheme flip — nothing is drawn before the first read.
  let theme = {
    text: '',
    dim: '',
    accent: '',
    surface: '',
    dark: true,
  };
  let groupColors: string[] = [];

  function readTheme(): void {
    const cs = getComputedStyle(document.documentElement);
    const v = (name: string) => cs.getPropertyValue(name).trim();
    theme = {
      text: v('--text'),
      dim: v('--text-dim'),
      accent: v('--accent'),
      surface: v('--surface'),
      dark: document.documentElement.getAttribute('data-scheme') !== 'light',
    };
    buildColors();
  }

  /** Stable HSL palette from group index: golden-angle hue walk, sat/lum tuned
   *  per scheme so clusters read on both dark and light backgrounds. */
  function buildColors(): void {
    groupColors = groupLabels.map((_, gi) => {
      const hue = (gi * 137.508) % 360;
      return theme.dark ? `hsl(${hue.toFixed(1)}, 62%, 62%)` : `hsl(${hue.toFixed(1)}, 60%, 42%)`;
    });
  }

  // ── Worker ──────────────────────────────────────────────────────────────
  let worker: Worker | null = null;

  function wpost(msg: GraphWorkerIn, transfer?: Transferable[]): void {
    if (worker) worker.postMessage(msg, transfer ?? []);
  }

  function spawnWorker(): void {
    worker?.terminate();
    worker = new Worker(new URL('./graph.worker.ts', import.meta.url), { type: 'module' });
    worker.onmessage = (e: MessageEvent) => {
      const m = e.data as GraphWorkerOut;
      if (m.t !== 'tick' || m.pos.length !== n * 2) return;
      // Swap buffers: keep the fresh one, return the old one to the worker's
      // pool (ping-pong — no per-frame allocation on either side).
      const old = pos;
      pos = m.pos;
      if (old && old.length === m.pos.length) wpost({ t: 'buf', buf: old.buffer }, [old.buffer]);
      alphaLive = m.alpha;
      ticks++;
      // Grid rebuild "when positions settle": every N ticks while hot, and one
      // final rebuild when the sim cools, so hit tests track the layout.
      if (ticks === 1 || ticks % 12 === 0 || m.alpha < 0.004) rebuildGrid();
      if (!didFit) {
        didFit = true;
        fit();
      }
      invalidate();
    };
  }

  // ── Raw payload ingestion ───────────────────────────────────────────────
  /** Adopt a server payload: keep it whole, build the facets, then project. */
  function ingest(p: VaultGraphPayload): void {
    rawPaths = p.paths;
    rawTitles = p.titles;
    rawFlags = Uint8Array.from(p.flags);
    rawTypes = Uint16Array.from(p.types);
    rawServices = Uint16Array.from(p.services);
    rawTagOff = Uint32Array.from(p.tag_off);
    rawTagIds = Uint16Array.from(p.tag_ids);
    rawEdges = Uint32Array.from(p.edges);
    typeLabels = p.type_labels;
    serviceLabels = p.service_labels;
    tagLabels = p.tag_labels;
    rawN = rawPaths.length;
    rawE = rawEdges.length >> 1;
    rawNodeCount = rawN;
    rawEdgeCount = rawE;
    truncated = p.truncated;

    rawIndex = new Map();
    for (let i = 0; i < rawN; i++) rawIndex.set(rawPaths[i], i);

    // Raw CSR adjacency — the anchor BFS walks this, never the edge list.
    const d = new Uint32Array(rawN);
    for (let e = 0; e < rawE; e++) {
      d[rawEdges[e * 2]]++;
      d[rawEdges[e * 2 + 1]]++;
    }
    rawAdjOff = new Int32Array(rawN + 1);
    for (let i = 0; i < rawN; i++) rawAdjOff[i + 1] = rawAdjOff[i] + d[i];
    rawAdjList = new Int32Array(rawE * 2);
    {
      const cur = Int32Array.from(rawAdjOff.subarray(0, rawN));
      for (let e = 0; e < rawE; e++) {
        const a = rawEdges[e * 2], b = rawEdges[e * 2 + 1];
        rawAdjList[cur[a]++] = b;
        rawAdjList[cur[b]++] = a;
      }
    }

    buildFacets();
    // Drop selections and anchors the new payload no longer contains, so a
    // rescan (or a server-side toggle) can't leave the graph filtered to
    // nothing by an invisible ghost selection.
    const svc = new Set(serviceLabels), ty = new Set(typeLabels), tg = new Set(tagLabels);
    selServices = selServices.filter((s) => svc.has(s));
    selTypes = selTypes.filter((s) => ty.has(s));
    selTags = selTags.filter((s) => tg.has(s));
    anchorPaths = anchorPaths.filter((p2) => rawIndex.has(p2));
    project();
  }

  /** Facet label + count over the raw nodes (drives the rail's lists). */
  function buildFacets(): void {
    const count = (ids: Uint16Array, labels: string[]): Facet[] => {
      const c = new Uint32Array(labels.length);
      for (let i = 0; i < ids.length; i++) c[ids[i]]++;
      return labels
        .map((label, i) => ({ label, count: c[i] }))
        .filter((f) => f.count > 0)
        .sort((a, b) => b.count - a.count || a.label.localeCompare(b.label));
    };
    serviceFacets = count(rawServices, serviceLabels);
    typeFacets = count(rawTypes, typeLabels);
    const tc = new Uint32Array(tagLabels.length);
    for (let i = 0; i < rawTagIds.length; i++) tc[rawTagIds[i]]++;
    tagFacets = tagLabels
      .map((label, i) => ({ label, count: tc[i] }))
      .filter((f) => f.count > 0)
      .sort((a, b) => b.count - a.count || a.label.localeCompare(b.label));
  }

  // ── Projection: focus filters + rollup → the rendered graph ─────────────
  function project(): void {
    if (rawN === 0) {
      applyProjection([], [], [], new Uint8Array(0), new Uint32Array(0), null);
      return;
    }
    const svcSel = new Set(selServices);
    const typeSel = new Set(selTypes);
    const tagSel = new Set(selTags);

    // 1. Attribute filters — AND across axes, OR within one.
    let keep = new Uint8Array(rawN);
    for (let i = 0; i < rawN; i++) {
      if (svcSel.size && !svcSel.has(serviceLabels[rawServices[i]])) continue;
      if (typeSel.size && !typeSel.has(typeLabels[rawTypes[i]])) continue;
      if (tagSel.size) {
        let hit = false;
        for (let t = rawTagOff[i]; t < rawTagOff[i + 1] && !hit; t++) {
          if (tagSel.has(tagLabels[rawTagIds[t]])) hit = true;
        }
        if (!hit) continue;
      }
      keep[i] = 1;
    }

    // 2. Anchors + hops — BFS over the ALREADY-FILTERED subgraph, so every
    // edge you can see is a hop you could have walked. Anchors themselves
    // survive the axes above unconditionally.
    if (anchorPaths.length) {
      const seeds: number[] = [];
      for (const p of anchorPaths) {
        const i = rawIndex.get(p);
        if (i !== undefined) seeds.push(i);
      }
      if (seeds.length) {
        const reach = new Uint8Array(rawN);
        let frontier: number[] = [];
        for (const s of seeds) {
          if (!reach[s]) {
            reach[s] = 1;
            frontier.push(s);
          }
        }
        for (let d = 0; d < hops; d++) {
          const next: number[] = [];
          for (const u of frontier) {
            for (let s = rawAdjOff[u]; s < rawAdjOff[u + 1]; s++) {
              const nb = rawAdjList[s];
              if (keep[nb] && !reach[nb]) {
                reach[nb] = 1;
                next.push(nb);
              }
            }
          }
          frontier = next;
          if (!frontier.length) break;
        }
        keep = reach;
      }
    }

    // 3. Node projection. Without rollup every kept node maps 1:1; with it,
    // nodes collapse into one node per service/type unless that group has
    // been drilled open.
    const rolled = rollup !== 'none';
    const open = new Set(expandedGroups);
    const groupOf = (i: number) =>
      rollup === 'type' ? typeLabels[rawTypes[i]] : serviceLabels[rawServices[i]];

    const pPaths: string[] = [];
    const pTitles: string[] = [];
    const pMetas: string[] = [];
    const pFlags: number[] = [];
    const pKeys: string[] = []; // color-group key per projected node
    const pWeight: number[] = []; // members behind a node (1 = a real note)
    const projOf = new Int32Array(rawN).fill(-1);
    const groupNode = new Map<string, number>();
    const internal = new Map<string, number>();

    for (let i = 0; i < rawN; i++) {
      if (!keep[i]) continue;
      const g = rolled ? groupOf(i) : '';
      if (rolled && !open.has(g)) {
        let gi = groupNode.get(g);
        if (gi === undefined) {
          gi = pPaths.length;
          groupNode.set(g, gi);
          pPaths.push(`group:${g}`);
          pTitles.push(g);
          pMetas.push('');
          pFlags.push(F_GROUP);
          pKeys.push(g);
          pWeight.push(0);
        }
        pWeight[gi]++;
        projOf[i] = gi;
        continue;
      }
      const groupKey = colorBy === 'type' ? typeLabels[rawTypes[i]] : serviceLabels[rawServices[i]];
      projOf[i] = pPaths.length;
      pPaths.push(rawPaths[i]);
      pTitles.push(rawTitles[i]);
      pMetas.push(groupKey || '');
      pFlags.push(rawFlags[i]);
      pKeys.push(groupKey);
      pWeight.push(1);
    }

    // 4. Edges. The plain path stays allocation-light (map + emit); rollup
    // aggregates parallel links into one weighted edge per pair.
    let pEdges: Uint32Array<ArrayBuffer>;
    let pWeights: Float32Array | null = null;
    if (!rolled) {
      const out: number[] = [];
      for (let e = 0; e < rawE; e++) {
        const a = projOf[rawEdges[e * 2]], b = projOf[rawEdges[e * 2 + 1]];
        if (a < 0 || b < 0 || a === b) continue;
        out.push(a, b);
      }
      pEdges = Uint32Array.from(out);
    } else {
      const agg = new Map<number, number>();
      const P = pPaths.length;
      for (let e = 0; e < rawE; e++) {
        const ra = rawEdges[e * 2], rb = rawEdges[e * 2 + 1];
        const a = projOf[ra], b = projOf[rb];
        if (a < 0 || b < 0) continue;
        if (a === b) {
          // Intra-group link: off the canvas, onto the node's tooltip.
          if (pFlags[a] & F_GROUP) internal.set(pPaths[a], (internal.get(pPaths[a]) ?? 0) + 1);
          continue;
        }
        const key = a < b ? a * P + b : b * P + a;
        agg.set(key, (agg.get(key) ?? 0) + 1);
      }
      pEdges = new Uint32Array(agg.size * 2);
      pWeights = new Float32Array(agg.size);
      let w = 0;
      for (const [key, weight] of agg) {
        pEdges[w * 2] = Math.floor(key / P);
        pEdges[w * 2 + 1] = key % P;
        pWeights[w] = weight;
        w++;
      }
      for (let i = 0; i < pPaths.length; i++) {
        if (!(pFlags[i] & F_GROUP)) continue;
        const inner = internal.get(pPaths[i]) ?? 0;
        pMetas[i] =
          `${pWeight[i].toLocaleString()} notes` +
          (inner ? ` · ${inner.toLocaleString()} internal links` : '');
      }
    }

    applyProjection(pPaths, pTitles, pMetas, Uint8Array.from(pFlags), pEdges, pWeights, pKeys, pWeight);
  }

  /** Hand a projected graph to the renderer + layout worker. */
  function applyProjection(
    pPaths: string[],
    pTitles: string[],
    pMetas: string[],
    pFlags: Uint8Array<ArrayBuffer>,
    pEdges: Uint32Array<ArrayBuffer>,
    pWeights: Float32Array | null,
    pKeys: string[] = [],
    pWeight: number[] = [],
  ): void {
    // SD-03: a real change (same vault + query, new generation) carries the
    // surviving nodes' positions into the new layout and keeps the camera.
    const carry = carryNext ? carrySeed(paths, pos, pPaths) : null;
    carryNext = false;
    paths = pPaths;
    titles = pTitles;
    metas = pMetas;
    flagsArr = pFlags;
    edges = pEdges;
    edgeW = pWeights;
    // Color groups are a client-side derivation now (colorBy), interned here.
    {
      const ix = new Map<string, number>();
      const labels: string[] = [];
      const g = new Uint16Array(pKeys.length);
      for (let i = 0; i < pKeys.length; i++) {
        let id = ix.get(pKeys[i]);
        if (id === undefined) {
          id = labels.length;
          ix.set(pKeys[i], id);
          labels.push(pKeys[i]);
        }
        g[i] = id;
      }
      groupsArr = g;
      groupLabels = labels;
    }
    n = paths.length;
    E = edges.length >> 1;
    nodeCount = n;
    edgeCount = E;

    // Degrees → world radii (clamped sqrt curve) + CSR adjacency for hover
    // highlighting (neighbors of one node without scanning 2M edges).
    deg = new Uint32Array(n);
    for (let e = 0; e < E; e++) {
      deg[edges[e * 2]]++;
      deg[edges[e * 2 + 1]]++;
    }
    radii = new Float32Array(n);
    for (let i = 0; i < n; i++) {
      // A rolled node sizes by the notes BEHIND it, not by its own degree —
      // otherwise a 400-note service reads the same as a 3-note one.
      radii[i] =
        pFlags[i] & F_GROUP
          ? Math.min(30, Math.max(5, 4 + Math.sqrt(pWeight[i] ?? 1) * 1.4))
          : Math.min(12, Math.max(1.5, 1.5 + Math.sqrt(deg[i]) * 0.6));
    }
    adjOff = new Int32Array(n + 1);
    for (let i = 0; i < n; i++) adjOff[i + 1] = adjOff[i] + deg[i];
    adjList = new Int32Array(E * 2);
    {
      const cur = Int32Array.from(adjOff.subarray(0, n));
      for (let e = 0; e < E; e++) {
        const a = edges[e * 2], b = edges[e * 2 + 1];
        adjList[cur[a]++] = b;
        adjList[cur[b]++] = a;
      }
    }
    // Label priority: highest-degree nodes first.
    labelOrder = Uint32Array.from({ length: n }, (_, i) => i);
    labelOrder = labelOrder.sort((a, b) => deg[b] - deg[a]);

    buildColors();
    pos = null;
    gridValid = false;
    didFit = !!carry; // carried layout: keep the user's camera
    ticks = 0;
    setHover(-1);
    stopped = false;
    alphaLive = n > 0 ? 1 : 0;

    if (n === 0) {
      // Nothing to lay out — drop any running worker instead of idling one.
      worker?.terminate();
      worker = null;
    } else {
      spawnWorker();
      // The worker gets its own edge copy TRANSFERRED (we keep `edges` for
      // drawing); groups ride along past the cluster-seeding threshold.
      const we = edges.slice();
      const transfer: Transferable[] = [we.buffer];
      let wg: Uint16Array<ArrayBuffer> | undefined;
      if (n > 30_000) {
        wg = groupsArr.slice();
        transfer.push(wg.buffer);
      }
      if (carry) transfer.push(carry.seed.buffer);
      wpost({ t: 'init', n, edges: we, groups: wg, carry: carry?.seed }, transfer);
      wpost({ t: 'params', center: fCenter, repel: fRepel, link: fLink, dist: fDist });
    }
    dataRev++;
    invalidate();
  }

  // ── Fetch (own data via vaultGraph; only the SERVER-side toggles refetch) ─
  const EMPTY_PAYLOAD: VaultGraphPayload = {
    paths: [], titles: [], types: [], type_labels: [], services: [], service_labels: [],
    tag_off: [0], tag_ids: [], tag_labels: [], flags: [], edges: [], truncated: false,
  };
  let reqSeq = 0;
  // A PRIMITIVE change token: effects reading it re-run only when its value
  // changes — not whenever the status poll hands back an equal object.
  // Keyed on the GRAPH generation (V1): a body-only autosave moves
  // `generation` but not the graph's shape, so it no longer refetches.
  const gen = $derived(
    vault.status?.graph_generation ?? vault.status?.generation ?? vault.status?.last_scan_at ?? null,
  );
  /** FNV-1a over everything `ingest` reads: an identical refetch (an edit that
   *  moved no link, title, tag or type) must not restart the layout. */
  function payloadSig(p: VaultGraphPayload): string {
    let h = 0x811c9dc5;
    const mix = (x: number): void => {
      h = Math.imul(h ^ x, 0x01000193);
    };
    const str = (t: string | null | undefined): void => {
      const u = t ?? '';
      for (let i = 0; i < u.length; i++) mix(u.charCodeAt(i));
      mix(0x1f);
    };
    for (const a of [p.flags, p.types, p.services, p.tag_off, p.tag_ids, p.edges]) {
      mix(a.length);
      for (let i = 0; i < a.length; i++) mix(a[i]);
    }
    for (const a of [p.paths, p.titles, p.type_labels, p.service_labels, p.tag_labels]) {
      mix(a.length);
      for (const t of a) str(t);
    }
    mix(p.truncated ? 1 : 0);
    return `${p.paths.length}:${p.edges.length}:${h >>> 0}`;
  }
  let lastPayloadSig = '';
  /** Set right before `ingest` of a refetch of the SAME vault + query. */
  let carryNext = false;
  let lastQueryKey = '';
  let lastVaultKey = '';
  /** Bumped by the error state's Retry — the load effect below re-runs. */
  let retryTick = $state(0);
  $effect(() => {
    const v = vault.current;
    void gen; // content/link changes only
    void retryTick;
    const wsId = vault.wsId;
    const path = local ? vault.notePath : null;
    const q: VaultGraphQuery = {
      tags: showTags,
      orphans: showOrphans,
      ghosts: showGhosts,
      reserved: showReserved,
    };
    if (local) {
      q.mode = 'local';
      q.path = path ?? undefined;
      q.depth = depth;
    } else {
      q.mode = 'full';
    }
    if (!v || !wsId || (local && !path)) {
      reqSeq++;
      lastPayloadSig = '';
      lastVaultKey = '';
      ingest(EMPTY_PAYLOAD);
      return;
    }
    const seq = ++reqSeq;
    const vaultKey = `${wsId}:${v.id}`;
    const queryKey = JSON.stringify([wsId, v.id, q]);
    if (lastVaultKey && lastVaultKey !== vaultKey) {
      lastPayloadSig = '';
      ingest(EMPTY_PAYLOAD);
    }
    loading = true;
    errorMsg = '';
    vaultGraph(wsId, v.id, q)
      .then((p) => {
        if (seq !== reqSeq) return;
        const sig = payloadSig(p);
        if (sig === lastPayloadSig && lastVaultKey === vaultKey) return; // same graph: keep layout, pins, camera
        // Same vault + query, different graph = a real change: carry the
        // layout (a vault / mode / toggle switch lays out fresh).
        carryNext = lastPayloadSig !== '' && queryKey === lastQueryKey;
        lastPayloadSig = sig;
        lastQueryKey = queryKey;
        lastVaultKey = vaultKey;
        ingest(p);
      })
      .catch((e: unknown) => {
        if (seq === reqSeq) errorMsg = loadErrorText(e);
      })
      .finally(() => {
        if (seq === reqSeq) loading = false;
      });
  });

  // Focus filters → re-project in place. No refetch: the raw payload already
  // carries every attribute these read.
  // The signature compare stops `ingest`'s own project() from being repeated
  // when it prunes a selection the fresh payload no longer contains.
  let lastSig = '';
  function sigChanged(): boolean {
    const sig = JSON.stringify([
      selServices, selTypes, selTags, anchorPaths, hops, rollup, expandedGroups, colorBy,
    ]);
    if (sig === lastSig) return false;
    lastSig = sig;
    return true;
  }
  $effect(() => {
    // sigChanged() reads every filter — that read is what registers the deps.
    if (sigChanged()) project();
  });

  // Live force params → worker (dataRev keeps a fresh worker in sync too).
  $effect(() => {
    const msg: GraphWorkerIn = { t: 'params', center: fCenter, repel: fRepel, link: fLink, dist: fDist };
    void dataRev;
    wpost(msg);
  });

  // Display sliders only repaint.
  $effect(() => {
    void nodeScale;
    void linkWidth;
    void labelZoom;
    invalidate();
  });

  // Title filter → match mask (dims non-matching); client-side, no re-fetch.
  $effect(() => {
    const f = filter.trim().toLowerCase();
    void dataRev;
    if (!f) {
      matched = null;
    } else {
      const m = new Uint8Array(n);
      for (let i = 0; i < n; i++) if (titles[i].toLowerCase().includes(f)) m[i] = 1;
      matched = m;
    }
    invalidate();
  });

  // ── Filter actions ──────────────────────────────────────────────────────
  function toggleIn(list: string[], label: string): string[] {
    return list.includes(label) ? list.filter((x) => x !== label) : [...list, label];
  }
  function toggleGroup(label: string): void {
    expandedGroups = toggleIn(expandedGroups, label);
  }
  function resetFocus(): void {
    selServices = [];
    selTypes = [];
    selTags = [];
    anchorPaths = [];
    expandedGroups = [];
    rollup = 'none';
    hops = 1;
    svcQuery = '';
    tagQuery = '';
    anchorQuery = '';
    anchorHits = [];
  }
  function addAnchor(path: string): void {
    if (!anchorPaths.includes(path)) anchorPaths = [...anchorPaths, path];
    anchorQuery = '';
    anchorHits = [];
  }

  // Anchor typeahead — server-side fuzzy over title/aliases/path.
  let anchorSeq = 0;
  $effect(() => {
    const q = anchorQuery.trim();
    const v = vault.current;
    void gen; // content/link changes only
    const wsId = vault.wsId;
    if (!q || !v || !wsId) {
      anchorHits = [];
      return;
    }
    const seq = ++anchorSeq;
    vaultSwitcher(wsId, v.id, q)
      .then((hits) => {
        if (seq === anchorSeq) anchorHits = hits.slice(0, 8);
      })
      .catch(() => {
        if (seq === anchorSeq) anchorHits = [];
      });
  });

  // ── Sticky focus, per vault. Labels (never ids) are stored, so a rescan
  // that renumbers the payload's label tables can't scramble a saved filter.
  const FILTER_KEY = 'otto.vault.graph.filter.v1';
  let restoredFor = -1;
  $effect(() => {
    const v = vault.current;
    if (!v || v.id === restoredFor) return;
    restoredFor = v.id;
    resetFocus();
    try {
      const raw = localStorage.getItem(`${FILTER_KEY}.${v.id}`);
      if (!raw) return;
      const s = JSON.parse(raw) as Partial<{
        services: string[]; types: string[]; tags: string[]; anchors: string[];
        hops: number; rollup: 'none' | 'service' | 'type'; colorBy: 'service' | 'type';
      }>;
      selServices = s.services ?? [];
      selTypes = s.types ?? [];
      selTags = s.tags ?? [];
      anchorPaths = s.anchors ?? [];
      hops = Math.min(3, Math.max(0, s.hops ?? 1));
      rollup = s.rollup ?? 'none';
      colorBy = s.colorBy ?? 'service';
    } catch {
      /* unreadable or older shape — the reset above already left it clean */
    }
  });
  $effect(() => {
    const v = vault.current;
    const body = JSON.stringify({
      services: selServices, types: selTypes, tags: selTags,
      anchors: anchorPaths, hops, rollup, colorBy,
    });
    if (!v || v.id !== restoredFor) return; // don't persist over a pending restore
    try {
      localStorage.setItem(`${FILTER_KEY}.${v.id}`, body);
    } catch {
      /* quota / private mode — sticky filters are a nicety, never a hard fail */
    }
  });

  function toggleSim(): void {
    stopped = !stopped;
    wpost({ t: stopped ? 'stop' : 'resume' });
    if (!stopped) alphaLive = Math.max(alphaLive, 0.1);
  }

  // ── Spatial grid (uniform, counting-sort — rebuilt as positions settle) ──
  let gridValid = false;
  let gridCell = GRID_CELL_MIN;
  let gridCols = 0;
  let gridRows = 0;
  let gridMinX = 0;
  let gridMinY = 0;
  let gridStart = new Int32Array(0);
  let gridCursor = new Int32Array(0);
  let gridItems = new Uint32Array(0);

  function rebuildGrid(): void {
    if (!pos || n === 0) {
      gridValid = false;
      return;
    }
    let minx = Infinity, miny = Infinity, maxx = -Infinity, maxy = -Infinity;
    for (let i = 0; i < n; i++) {
      const x = pos[i * 2], y = pos[i * 2 + 1];
      if (x < minx) minx = x;
      if (x > maxx) maxx = x;
      if (y < miny) miny = y;
      if (y > maxy) maxy = y;
    }
    const side = Math.max(maxx - minx, maxy - miny, 1);
    gridCell = Math.max(GRID_CELL_MIN, side / 256); // cap the cell count at ~256²
    gridMinX = minx;
    gridMinY = miny;
    gridCols = Math.max(1, Math.floor((maxx - minx) / gridCell) + 1);
    gridRows = Math.max(1, Math.floor((maxy - miny) / gridCell) + 1);
    const cells = gridCols * gridRows;
    if (gridStart.length < cells + 1) {
      gridStart = new Int32Array(cells + 1);
      gridCursor = new Int32Array(cells + 1);
    } else {
      gridStart.fill(0, 0, cells + 1);
    }
    if (gridItems.length < n) gridItems = new Uint32Array(n);
    // Counting sort: counts → prefix sums → placement.
    for (let i = 0; i < n; i++) {
      const cx = Math.floor((pos[i * 2] - gridMinX) / gridCell);
      const cy = Math.floor((pos[i * 2 + 1] - gridMinY) / gridCell);
      gridStart[cy * gridCols + cx + 1]++;
    }
    for (let c = 0; c < cells; c++) gridStart[c + 1] += gridStart[c];
    gridCursor.set(gridStart.subarray(0, cells + 1));
    for (let i = 0; i < n; i++) {
      const cx = Math.floor((pos[i * 2] - gridMinX) / gridCell);
      const cy = Math.floor((pos[i * 2 + 1] - gridMinY) / gridCell);
      gridItems[gridCursor[cy * gridCols + cx]++] = i;
    }
    gridValid = true;
  }

  /** Nearest node under a world point via the grid's 3×3 neighborhood —
   *  O(cell occupancy), never O(n). Returns -1 for a miss. */
  function pick(wx: number, wy: number): number {
    if (!gridValid || !pos || n === 0) return -1;
    const slop = 5 / cam.k; // a few screen px of forgiveness, in world units
    const cx = Math.floor((wx - gridMinX) / gridCell);
    const cy = Math.floor((wy - gridMinY) / gridCell);
    let best = -1;
    let bestD2 = Infinity;
    for (let gy = cy - 1; gy <= cy + 1; gy++) {
      if (gy < 0 || gy >= gridRows) continue;
      for (let gx = cx - 1; gx <= cx + 1; gx++) {
        if (gx < 0 || gx >= gridCols) continue;
        const c = gy * gridCols + gx;
        for (let s = gridStart[c]; s < gridStart[c + 1]; s++) {
          const i = gridItems[s];
          const dx = pos[i * 2] - wx;
          const dy = pos[i * 2 + 1] - wy;
          const d2 = dx * dx + dy * dy;
          const r = Math.max(radii[i] * nodeScale, 3 / cam.k) + slop;
          if (d2 <= r * r && d2 < bestD2) {
            bestD2 = d2;
            best = i;
          }
        }
      }
    }
    return best;
  }

  // ── Hover ───────────────────────────────────────────────────────────────
  function setHover(i: number): void {
    if (i === hoverIdx) return;
    hoverIdx = i;
    hoverSet = new Set<number>();
    if (i >= 0) {
      hoverSet.add(i);
      for (let s = adjOff[i]; s < adjOff[i + 1]; s++) hoverSet.add(adjList[s]);
      hoverTip = { ...hoverTip, title: titles[i] ?? '', meta: metas[i] ?? '' };
    }
    invalidate();
  }

  // ── Camera helpers ──────────────────────────────────────────────────────
  function toWorld(mx: number, my: number): { x: number; y: number } {
    return { x: (mx - cssW / 2) / cam.k + cam.x, y: (my - cssH / 2) / cam.k + cam.y };
  }

  function fit(): void {
    if (!pos || n === 0) return;
    let minx = Infinity, miny = Infinity, maxx = -Infinity, maxy = -Infinity;
    for (let i = 0; i < n; i++) {
      const x = pos[i * 2], y = pos[i * 2 + 1];
      if (x < minx) minx = x;
      if (x > maxx) maxx = x;
      if (y < miny) miny = y;
      if (y > maxy) maxy = y;
    }
    const bw = Math.max(maxx - minx, 1), bh = Math.max(maxy - miny, 1);
    cam.k = Math.min(4, Math.max(0.02, Math.min((cssW * 0.9) / bw, (cssH * 0.9) / bh)));
    cam.x = (minx + maxx) / 2;
    cam.y = (miny + maxy) / 2;
    invalidate();
  }

  // ── Rendering (dirty flag + on-demand rAF, see invalidate()) ─────────────
  function draw(): void {
    if (!ctx) return;
    const w = cssW, h = cssH;
    ctx.clearRect(0, 0, w, h);
    if (!pos || n === 0) return;
    const k = cam.k;
    const ox = w / 2 - cam.x * k;
    const oy = h / 2 - cam.y * k;
    // Visible world rect + margin (labels/large nodes bleed past centers).
    const mg = 40 / k;
    const wx0 = -ox / k - mg, wx1 = (w - ox) / k + mg;
    const wy0 = -oy / k - mg, wy1 = (h - oy) / k + mg;

    const dimming = hoverIdx >= 0 || matched !== null;

    // ── Edges: one batched path for the base style. Past the draw budget,
    // sample deterministically by index stride (stable frame-to-frame) and
    // fade the survivors so density still reads without 2M strokes.
    const stride = Math.max(1, Math.ceil(E / EDGE_DRAW_BUDGET));
    let eAlpha = Math.min(0.3, 0.3 / Math.sqrt(stride)) + 0.06;
    if (dimming) eAlpha = 0.05;
    ctx.strokeStyle = theme.dim;
    ctx.globalAlpha = eAlpha;
    if (edgeW) {
      // Rolled up: a few hundred edges, each carrying a link count — worth one
      // stroke apiece so weight reads as thickness. Batching would flatten it.
      let maxW = 1;
      for (let e = 0; e < E; e++) if (edgeW[e] > maxW) maxW = edgeW[e];
      ctx.globalAlpha = Math.max(eAlpha, dimming ? 0.05 : 0.22);
      for (let e = 0; e < E; e++) {
        const a = edges[e * 2], b = edges[e * 2 + 1];
        const xa = pos[a * 2], ya = pos[a * 2 + 1];
        const xb = pos[b * 2], yb = pos[b * 2 + 1];
        if ((xa < wx0 && xb < wx0) || (xa > wx1 && xb > wx1)) continue;
        if ((ya < wy0 && yb < wy0) || (ya > wy1 && yb > wy1)) continue;
        const t = Math.sqrt(edgeW[e] / maxW); // sqrt: one hub link can't drown the rest
        ctx.lineWidth = Math.max(0.5, linkWidth * (0.5 + t * 3.5) * Math.min(1.5, k));
        ctx.beginPath();
        ctx.moveTo(xa * k + ox, ya * k + oy);
        ctx.lineTo(xb * k + ox, yb * k + oy);
        ctx.stroke();
      }
    } else {
      ctx.lineWidth = Math.max(0.4, linkWidth * Math.min(1.5, k));
      ctx.beginPath();
      for (let e = 0; e < E; e += stride) {
        const a = edges[e * 2], b = edges[e * 2 + 1];
        const xa = pos[a * 2], ya = pos[a * 2 + 1];
        const xb = pos[b * 2], yb = pos[b * 2 + 1];
        // Cull: both endpoints beyond the same viewport side can't cross it.
        if ((xa < wx0 && xb < wx0) || (xa > wx1 && xb > wx1)) continue;
        if ((ya < wy0 && yb < wy0) || (ya > wy1 && yb > wy1)) continue;
        ctx.moveTo(xa * k + ox, ya * k + oy);
        ctx.lineTo(xb * k + ox, yb * k + oy);
      }
      ctx.stroke();
    }

    // Hovered node's edges: second batched path, accent, full strength
    // (drawn from adjacency — no edge scan).
    if (hoverIdx >= 0) {
      const i = hoverIdx;
      const sx = pos[i * 2] * k + ox, sy = pos[i * 2 + 1] * k + oy;
      ctx.strokeStyle = theme.accent;
      ctx.globalAlpha = 0.85;
      ctx.lineWidth = Math.max(0.7, linkWidth * Math.min(2, k));
      ctx.beginPath();
      for (let s = adjOff[i]; s < adjOff[i + 1]; s++) {
        const nb = adjList[s];
        ctx.moveTo(sx, sy);
        ctx.lineTo(pos[nb * 2] * k + ox, pos[nb * 2 + 1] * k + oy);
      }
      ctx.stroke();
    }

    // ── Nodes. Fill by group color; ghosts gray; tags outline-only when big
    // enough to read. Sub-1.5px nodes use fillRect (much cheaper than arc).
    // fillStyle changes are batched implicitly: folder-grouped payloads keep
    // same-group nodes index-adjacent, so the cache below rarely misses.
    let lastFill = '';
    // Ghost (unresolved-link) nodes: the dim text token, never a hex.
    const ghostFill = theme.dim;
    for (let i = 0; i < n; i++) {
      const x = pos[i * 2], y = pos[i * 2 + 1];
      if (x < wx0 || x > wx1 || y < wy0 || y > wy1) continue;
      const f = flagsArr[i];
      const sr = Math.max(radii[i] * nodeScale * k, 0.7);
      const sx = x * k + ox, sy = y * k + oy;
      let a = 1;
      if (hoverIdx >= 0) a = hoverSet.has(i) ? 1 : 0.12;
      if (matched) a = Math.min(a, matched[i] ? 1 : 0.1);
      ctx.globalAlpha = a;
      const fill = f & F_GHOST ? ghostFill : (groupColors[groupsArr[i]] ?? theme.accent);
      if (f & F_TAG && sr > 2) {
        // Tag node: outline style — visibly "not a note".
        ctx.strokeStyle = fill;
        ctx.lineWidth = Math.max(1, sr * 0.28);
        ctx.beginPath();
        ctx.arc(sx, sy, sr * 0.85, 0, Math.PI * 2);
        ctx.stroke();
        continue;
      }
      if (fill !== lastFill) {
        ctx.fillStyle = fill;
        lastFill = fill;
      }
      if (sr < 1.5) {
        ctx.fillRect(sx - sr, sy - sr, sr * 2, sr * 2);
      } else {
        ctx.beginPath();
        ctx.arc(sx, sy, sr, 0, Math.PI * 2);
        ctx.fill();
        if (f & F_GROUP) {
          // Rolled node: a ring marks it as a container you can click open.
          ctx.strokeStyle = theme.text;
          ctx.globalAlpha = a * 0.55;
          ctx.lineWidth = 1.5;
          ctx.beginPath();
          ctx.arc(sx, sy, sr + 2, 0, Math.PI * 2);
          ctx.stroke();
          ctx.globalAlpha = a;
        }
        if (f & F_GHOST && sr > 3) {
          // Ghost: dashed ring marks the unresolved target.
          ctx.strokeStyle = theme.dim;
          ctx.lineWidth = 1;
          ctx.setLineDash([3, 2]);
          ctx.beginPath();
          ctx.arc(sx, sy, sr + 1.5, 0, Math.PI * 2);
          ctx.stroke();
          ctx.setLineDash([]);
        }
      }
    }

    // ── Labels: only past the zoom threshold, top-degree first, per-frame
    // budget, fading in with zoom. Skipped entirely while the sim is hot so
    // early (fast-moving) frames stay cheap.
    ctx.globalAlpha = 1;
    if (k >= labelZoom && alphaLive < 0.3) {
      const fade = Math.min(1, (k - labelZoom) / (labelZoom * 0.5 + 0.01));
      ctx.font = '11px -apple-system, BlinkMacSystemFont, sans-serif';
      ctx.textAlign = 'center';
      ctx.textBaseline = 'top';
      ctx.fillStyle = theme.text;
      let budget = LABEL_BUDGET;
      const scan = Math.min(n, LABEL_SCAN_CAP);
      for (let o = 0; o < scan && budget > 0; o++) {
        const i = labelOrder[o];
        if (i === hoverIdx) continue; // hovered label drawn last, always
        const x = pos[i * 2], y = pos[i * 2 + 1];
        if (x < wx0 || x > wx1 || y < wy0 || y > wy1) continue;
        let a = fade * 0.85;
        if (hoverIdx >= 0 && !hoverSet.has(i)) a *= 0.25;
        if (matched && !matched[i]) a *= 0.2;
        ctx.globalAlpha = a;
        ctx.fillText(titles[i], x * k + ox, y * k + oy + radii[i] * nodeScale * k + 3);
        budget--;
      }
    }

    // Hovered node: accent ring + its label regardless of zoom.
    if (hoverIdx >= 0) {
      const i = hoverIdx;
      const sx = pos[i * 2] * k + ox, sy = pos[i * 2 + 1] * k + oy;
      const sr = Math.max(radii[i] * nodeScale * k, 0.7);
      ctx.globalAlpha = 1;
      ctx.strokeStyle = theme.accent;
      ctx.lineWidth = 1.5;
      ctx.beginPath();
      ctx.arc(sx, sy, sr + 2.5, 0, Math.PI * 2);
      ctx.stroke();
      ctx.font = '11px -apple-system, BlinkMacSystemFont, sans-serif';
      ctx.textAlign = 'center';
      ctx.textBaseline = 'top';
      ctx.fillStyle = theme.text;
      ctx.fillText(titles[i], sx, sy + sr + 4);
    }
    ctx.globalAlpha = 1;
  }

  // ── Pointer interactions (listeners attached imperatively — the canvas is
  // a pure drawing surface, so no template a11y handlers) ──────────────────
  let dragMode: 'none' | 'pan' | 'node' = 'none';
  let dragI = -1;
  let downX = 0;
  let downY = 0;
  let panCamX = 0;
  let panCamY = 0;
  let movedFar = false;

  function localXY(e: PointerEvent | WheelEvent | MouseEvent): { x: number; y: number } {
    const r = canvasEl.getBoundingClientRect();
    return { x: e.clientX - r.left, y: e.clientY - r.top };
  }

  function onPointerDown(e: PointerEvent): void {
    if (e.button !== 0) return;
    const p = localXY(e);
    downX = p.x;
    downY = p.y;
    movedFar = false;
    const w = toWorld(p.x, p.y);
    const i = pick(w.x, w.y);
    if (i >= 0) {
      dragMode = 'node';
      dragI = i;
      draggingNode = true;
      wpost({ t: 'pin', i, x: w.x, y: w.y }); // dragging pins the node
    } else {
      dragMode = 'pan';
      panCamX = cam.x;
      panCamY = cam.y;
    }
    canvasEl.setPointerCapture(e.pointerId);
  }

  function onPointerMove(e: PointerEvent): void {
    const p = localXY(e);
    if (Math.abs(p.x - downX) + Math.abs(p.y - downY) > CLICK_SLOP) movedFar = true;
    if (dragMode === 'node') {
      const w = toWorld(p.x, p.y);
      wpost({ t: 'drag', i: dragI, x: w.x, y: w.y });
      return;
    }
    if (dragMode === 'pan') {
      cam.x = panCamX - (p.x - downX) / cam.k;
      cam.y = panCamY - (p.y - downY) / cam.k;
      invalidate();
      return;
    }
    // Idle move: hover pick (grid — never O(n)) + tooltip anchor.
    const w = toWorld(p.x, p.y);
    setHover(pick(w.x, w.y));
    if (hoverIdx >= 0) {
      hoverTip = {
        x: Math.max(8, Math.min(cssW - 268, p.x + 14)),
        y: Math.max(8, Math.min(cssH - 36, p.y + 10)),
        title: titles[hoverIdx] ?? '', meta: metas[hoverIdx] ?? '',
      };
    }
  }

  function onPointerUp(e: PointerEvent): void {
    if (dragMode === 'none') return; // capture-less up (e.g. re-entry) — ignore
    if (dragMode === 'node' && !movedFar) {
      // A stationary press on a node is a click: a rolled-up node drills open
      // (that group alone expands into its notes); real notes open; ghost
      // nodes prompt to create the missing note.
      const f = flagsArr[dragI];
      if (f & F_GROUP) {
        toggleGroup(titles[dragI]);
      } else if (f & F_GHOST) {
        const raw = (titles[dragI] || paths[dragI] || '').trim();
        if (raw) {
          const p = raw.endsWith('.md') ? raw : `${raw}.md`;
          void confirmer.ask(`Create “${p}”?`, { title: 'Create note', confirmLabel: 'Create', danger: false }).then((ok) => {
            if (ok) void vault.createNote(p, `# ${raw}\n\n`);
          });
        }
      } else if (!(f & F_TAG)) {
        void vault.open(paths[dragI]);
      }
    }
    if (dragMode === 'node') rebuildGrid(); // dropped node moved — grid must know
    dragMode = 'none';
    dragI = -1;
    draggingNode = false;
    if (canvasEl.hasPointerCapture(e.pointerId)) canvasEl.releasePointerCapture(e.pointerId);
  }

  function onDblClick(e: MouseEvent): void {
    const p = localXY(e);
    const w = toWorld(p.x, p.y);
    const i = pick(w.x, w.y);
    if (i >= 0) {
      wpost({ t: 'release', i }); // double-click a node → unpin it
    } else {
      fit(); // double-click background → reset zoom-to-fit
    }
  }

  function onWheel(e: WheelEvent): void {
    e.preventDefault();
    const p = localXY(e);
    const w = toWorld(p.x, p.y); // keep this world point under the cursor
    cam.k = Math.min(20, Math.max(0.02, cam.k * Math.exp(-e.deltaY * 0.0015)));
    cam.x = w.x - (p.x - cssW / 2) / cam.k;
    cam.y = w.y - (p.y - cssH / 2) / cam.k;
    invalidate();
  }

  function onPointerLeave(): void {
    if (dragMode === 'none') setHover(-1);
  }

  // ── Mount: canvas sizing (DPR-aware), rAF loop, listeners, theme watch ───
  $effect(() => {
    ctx = canvasEl.getContext('2d');
    readTheme();

    const ro = new ResizeObserver(() => {
      const r = rootEl.getBoundingClientRect();
      cssW = Math.max(1, Math.round(r.width));
      cssH = Math.max(1, Math.round(r.height));
      const dpr = window.devicePixelRatio || 1;
      canvasEl.width = Math.round(cssW * dpr);
      canvasEl.height = Math.round(cssH * dpr);
      ctx?.setTransform(dpr, 0, 0, dpr, 0, 0);
      invalidate();
    });
    ro.observe(rootEl);

    // Theme/scheme flips re-read the CSS vars + rebuild the group palette.
    const mo = new MutationObserver(() => {
      readTheme();
      invalidate();
    });
    mo.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme', 'data-scheme'] });

    // Wheel must be non-passive (we preventDefault to own the zoom gesture).
    canvasEl.addEventListener('wheel', onWheel, { passive: false });
    canvasEl.addEventListener('pointerdown', onPointerDown);
    canvasEl.addEventListener('pointermove', onPointerMove);
    canvasEl.addEventListener('pointerup', onPointerUp);
    canvasEl.addEventListener('pointerleave', onPointerLeave);
    canvasEl.addEventListener('dblclick', onDblClick);

    painting = true;
    invalidate();

    return () => {
      painting = false;
      cancelAnimationFrame(raf);
      raf = 0;
      ro.disconnect();
      mo.disconnect();
      canvasEl.removeEventListener('wheel', onWheel);
      canvasEl.removeEventListener('pointerdown', onPointerDown);
      canvasEl.removeEventListener('pointermove', onPointerMove);
      canvasEl.removeEventListener('pointerup', onPointerUp);
      canvasEl.removeEventListener('pointerleave', onPointerLeave);
      canvasEl.removeEventListener('dblclick', onDblClick);
      reqSeq++; // drop any in-flight fetch
      worker?.terminate();
      worker = null;
    };
  });
;
async () => {


  { const $$_div0 = svelteHTML.createElement("div", {       "class":`graph-root`,"role":`region`,"aria-label":nodeCount ? `Link graph of ${vault.current?.name ?? 'this vault'}: ${nodeCount} notes. Use the file tree or search to open a note.` : 'Link graph',});rootEl = $$_div0;
   { const $$_canvas1 = svelteHTML.createElement("canvas", {    "aria-hidden":`true`,});canvasEl = $$_canvas1;hoverIdx < 0; }

  if(!loading && errorMsg && nodeCount > 0){
     { svelteHTML.createElement("div", { "class":`graph-error-bar`,});
       { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {        "what":`the graph`,"variant":`compact`,"error":errorMsg,"onretry":() => retryTick++,}});}
     }
  }

  if(!vault.current){
     { svelteHTML.createElement("div", { "class":`empty`,});   }
  } else if (!loading && nodeCount === 0 && errorMsg){
     { svelteHTML.createElement("div", { "class":`empty empty-error`,});
       { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {          "what":`the graph`,"variant":`compact`,"error":errorMsg,"empty":true,"onretry":() => retryTick++,}});}
     }
  } else if (!loading && nodeCount === 0){
     { svelteHTML.createElement("div", { "class":`empty`,});
      if(local && !vault.notePath){       } else if (hasFocus && rawNodeCount > 0){
         { svelteHTML.createElement("span", {});
               rawNodeCount.toLocaleString();   
           { svelteHTML.createElement("button", {   "class":`mini`,"onclick":resetFocus,});  }
         }
      }else{
         { svelteHTML.createElement("span", {});
                 
          if(onNewNote){
             { svelteHTML.createElement("button", {   "class":`mini`,"onclick":onNewNote,});  }
          }
         }
      }
     }
  }

  
   { svelteHTML.createElement("div", { "class":`statusbar`,});
     { svelteHTML.createElement("span", { "class":`counts`,});
      if(hasFocus){
        nodeCount.toLocaleString();  rawNodeCount.toLocaleString();  
        edgeCount.toLocaleString();  rawEdgeCount.toLocaleString(); }else{
        nodeCount.toLocaleString();   edgeCount.toLocaleString(); }
     }
    if(truncated){
       { const $$_egdaB2C = __sveltets_2_ensureComponent(Badge); new $$_egdaB2C({ target: __sveltets_2_any(), props: {      "tone":`warn`,"label":`Truncated`,"title":`Edge budget hit — some links were omitted by the server`,}});}
    }
    if(loading){
       { const $$_egdaB2C = __sveltets_2_ensureComponent(Badge); new $$_egdaB2C({ target: __sveltets_2_any(), props: {  "label":`Loading graph…`,}});}
    } else if (alphaLive > 0.02 && !stopped){
       { const $$_egdaB2C = __sveltets_2_ensureComponent(Badge); new $$_egdaB2C({ target: __sveltets_2_any(), props: {    "tone":`accent`,"label":`Simulating…`,}});}
    }
    if(nodeCount > 0){
       { svelteHTML.createElement("button", {   "class":`mini`,"onclick":toggleSim,});stopped ? 'Resume' : 'Stop';  }
    }
   }

  
   { svelteHTML.createElement("div", {    "class":`panel`,"style":`width: ${ui.vaultGraphPanelWidth}px`,});panelResizing;
    
     {const $$action_0 = __sveltets_2_ensureAction(paneResizer(svelteHTML.mapElementTag('div'),({ value: ui.vaultGraphPanelWidth, min: 200, max: 560, invert: true, onChange: (w) => ui.setVaultGraphPanelWidth(w), onReset: () => ui.setVaultGraphPanelWidth(210), onDragStart: startPanelResize, text: pxWide })));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {           "class":`panel-resizer`,"role":`separator`,"aria-label":`Resize the graph controls panel`,"title":RESIZE_TITLE,}); }}
     { svelteHTML.createElement("button", {     "class":`panel-head`,"onclick":() => (panelOpen = !panelOpen),"aria-expanded":panelOpen,});
       { svelteHTML.createElement("span", {});  }
       { svelteHTML.createElement("span", { "class":`tri`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":panelOpen ? 'chevronDown' : 'chevronRight',"size":12,}});} }
     }
    if(panelOpen){
       { svelteHTML.createElement("div", { "class":`panel-body`,});
         { svelteHTML.createElement("input", {            "dir":`ltr`,"class":`filter`,"type":`text`,"placeholder":`Filter titles…`,"aria-label":`Filter titles`,"bind:value":filter,});/*Ωignore_startΩ*/() => filter = __sveltets_2_any(null);/*Ωignore_endΩ*/}

         { svelteHTML.createElement("div", { "class":`sec`,});
          
          if(hasFocus){
             { svelteHTML.createElement("button", {   "class":`link`,"onclick":resetFocus,});  }
          }
         }

        
        if(anchorPaths.length){
           { svelteHTML.createElement("div", { "class":`chips`,});
               for(let a of __sveltets_2_ensureArray(anchorPaths)){a;
               { svelteHTML.createElement("button", {       "class":`ftoken`,"title":a,"onclick":() => (anchorPaths = anchorPaths.filter((x) => x !== a)),});
                 { svelteHTML.createElement("span", { "class":`ftoken-t`,});a.split('/').pop()?.replace(/\.md$/, ''); }
                 { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});}
               }
            }
           }
           { svelteHTML.createElement("label", { "class":`row`,});
             { svelteHTML.createElement("span", {});  }
             { svelteHTML.createElement("input", {          "type":`range`,"min":`0`,"max":`3`,"step":`1`,"bind:value":hops,});/*Ωignore_startΩ*/() => hops = __sveltets_2_any(null);/*Ωignore_endΩ*/}
             { svelteHTML.createElement("span", { "class":`val`,});hops; }
           }
        }
         { svelteHTML.createElement("div", { "class":`typeahead`,});
           { svelteHTML.createElement("input", {            "dir":`auto`,"class":`filter`,"type":`text`,"placeholder":`Anchor on a note…`,"aria-label":`Anchor on a note`,"bind:value":anchorQuery,});/*Ωignore_startΩ*/() => anchorQuery = __sveltets_2_any(null);/*Ωignore_endΩ*/}
          if(anchorHits.length){
             { svelteHTML.createElement("ul", { "class":`hits`,});
                 for(let h of __sveltets_2_ensureArray(anchorHits)){h.path;
                 { svelteHTML.createElement("li", {});
                   { svelteHTML.createElement("button", {   "onclick":() => addAnchor(h.path),"title":h.path,});h.title; }
                 }
              }
             }
          }
         }

        
        if(serviceFacets.length > 1){
           { svelteHTML.createElement("div", { "class":`sec sub`,});
            
            if(selServices.length){
               { svelteHTML.createElement("button", {   "class":`link`,"onclick":() => (selServices = []),});  }
            }
           }
          if(serviceFacets.length > 8){
             { svelteHTML.createElement("input", {            "dir":`auto`,"class":`filter`,"type":`text`,"placeholder":`Find service…`,"aria-label":`Find service`,"bind:value":svcQuery,});/*Ωignore_startΩ*/() => svcQuery = __sveltets_2_any(null);/*Ωignore_endΩ*/}
          }
           { svelteHTML.createElement("div", { "class":`facets`,});
               for(let f of __sveltets_2_ensureArray(svcShown)){f.label;
               { svelteHTML.createElement("label", { "class":`chk`,});
                 { svelteHTML.createElement("input", {       "type":`checkbox`,"checked":selServices.includes(f.label),"onchange":() => (selServices = toggleIn(selServices, f.label)),});}
                 { svelteHTML.createElement("span", {   "class":`fl`,"title":f.label,});f.label; }
                 { svelteHTML.createElement("span", { "class":`fc`,});f.count.toLocaleString(); }
               }
            }
           }
        }

        
        if(typeFacets.length > 1){
           { svelteHTML.createElement("div", { "class":`sec sub`,});
            
            if(selTypes.length){
               { svelteHTML.createElement("button", {   "class":`link`,"onclick":() => (selTypes = []),});  }
            }
           }
           { svelteHTML.createElement("div", { "class":`facets`,});
               for(let f of __sveltets_2_ensureArray(typeFacets)){f.label;
               { svelteHTML.createElement("label", { "class":`chk`,});
                 { svelteHTML.createElement("input", {       "type":`checkbox`,"checked":selTypes.includes(f.label),"onchange":() => (selTypes = toggleIn(selTypes, f.label)),});}
                 { svelteHTML.createElement("span", {   "class":`fl`,"title":f.label,});f.label; }
                 { svelteHTML.createElement("span", { "class":`fc`,});f.count.toLocaleString(); }
               }
            }
           }
        }

        
        if(tagFacets.length){
           { svelteHTML.createElement("div", { "class":`sec sub`,});
            
            if(selTags.length){
               { svelteHTML.createElement("button", {   "class":`link`,"onclick":() => (selTags = []),});  }
            }
           }
          if(tagFacets.length > 8){
             { svelteHTML.createElement("input", {            "dir":`auto`,"class":`filter`,"type":`text`,"placeholder":`Find tag…`,"aria-label":`Find tag`,"bind:value":tagQuery,});/*Ωignore_startΩ*/() => tagQuery = __sveltets_2_any(null);/*Ωignore_endΩ*/}
          }
           { svelteHTML.createElement("div", { "class":`facets`,});
               for(let f of __sveltets_2_ensureArray(tagShown)){f.label;
               { svelteHTML.createElement("label", { "class":`chk`,});
                 { svelteHTML.createElement("input", {       "type":`checkbox`,"checked":selTags.includes(f.label),"onchange":() => (selTags = toggleIn(selTags, f.label)),});}
                 { svelteHTML.createElement("span", {   "class":`fl`,"title":f.label,}); f.label; }
                 { svelteHTML.createElement("span", { "class":`fc`,});f.count.toLocaleString(); }
               }
            }
           }
        }

         { svelteHTML.createElement("div", { "class":`sec`,});  }
         { svelteHTML.createElement("label", { "class":`row`,});
           { svelteHTML.createElement("span", {});  }
           { svelteHTML.createElement("select", {     "value":rollup,"onchange":(e) => {
              rollup = e.currentTarget.value as 'none' | 'service' | 'type';
              expandedGroups = [];
            },});
             { svelteHTML.createElement("option", { "value":`none`,});   }
             { svelteHTML.createElement("option", { "value":`service`,});  }
             { svelteHTML.createElement("option", { "value":`type`,});  }
           }
         }
        if(rollup !== 'none'){
           { svelteHTML.createElement("div", { "class":`hint`,});          }
          if(expandedGroups.length){
             { svelteHTML.createElement("div", { "class":`chips`,});
                 for(let g of __sveltets_2_ensureArray(expandedGroups)){g;
                 { svelteHTML.createElement("button", {     "class":`ftoken`,"title":`Collapse ${g}`,"onclick":() => toggleGroup(g),}); { svelteHTML.createElement("span", { "class":`ftoken-t`,});g; }  { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});} }
              }
             }
          }
        }
         { svelteHTML.createElement("label", { "class":`row`,});
           { svelteHTML.createElement("span", {});  }
           { svelteHTML.createElement("select", {     "value":colorBy,"onchange":(e) => (colorBy = e.currentTarget.value as 'service' | 'type'),});
             { svelteHTML.createElement("option", { "value":`service`,});  }
             { svelteHTML.createElement("option", { "value":`type`,});  }
           }
         }

         { svelteHTML.createElement("div", { "class":`sec`,});  }
         { svelteHTML.createElement("label", { "class":`chk`,}); { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":showTags,});/*Ωignore_startΩ*/() => showTags = __sveltets_2_any(null);/*Ωignore_endΩ*/}   }
         { svelteHTML.createElement("label", { "class":`chk`,}); { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":showOrphans,});/*Ωignore_startΩ*/() => showOrphans = __sveltets_2_any(null);/*Ωignore_endΩ*/}  }
         { svelteHTML.createElement("label", { "class":`chk`,}); { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":showGhosts,});/*Ωignore_startΩ*/() => showGhosts = __sveltets_2_any(null);/*Ωignore_endΩ*/}  }
         { svelteHTML.createElement("label", { "class":`chk`,}); { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":showReserved,});/*Ωignore_startΩ*/() => showReserved = __sveltets_2_any(null);/*Ωignore_endΩ*/}   }
        if(local){
           { svelteHTML.createElement("label", { "class":`row`,});
             { svelteHTML.createElement("span", {});  }
             { svelteHTML.createElement("input", {          "type":`range`,"min":`1`,"max":`3`,"step":`1`,"bind:value":depth,});/*Ωignore_startΩ*/() => depth = __sveltets_2_any(null);/*Ωignore_endΩ*/}
             { svelteHTML.createElement("span", { "class":`val`,});depth; }
           }
        }

         { svelteHTML.createElement("div", { "class":`sec`,});  }
         { svelteHTML.createElement("label", { "class":`row`,});
           { svelteHTML.createElement("span", {});  }
           { svelteHTML.createElement("input", {          "type":`range`,"min":`0`,"max":`1`,"step":`0.01`,"bind:value":fCenter,});/*Ωignore_startΩ*/() => fCenter = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           { svelteHTML.createElement("span", { "class":`val`,});fCenter.toFixed(2); }
         }
         { svelteHTML.createElement("label", { "class":`row`,});
           { svelteHTML.createElement("span", {});  }
           { svelteHTML.createElement("input", {          "type":`range`,"min":`0`,"max":`2`,"step":`0.05`,"bind:value":fRepel,});/*Ωignore_startΩ*/() => fRepel = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           { svelteHTML.createElement("span", { "class":`val`,});fRepel.toFixed(2); }
         }
         { svelteHTML.createElement("label", { "class":`row`,});
           { svelteHTML.createElement("span", {});  }
           { svelteHTML.createElement("input", {          "type":`range`,"min":`0`,"max":`2`,"step":`0.05`,"bind:value":fLink,});/*Ωignore_startΩ*/() => fLink = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           { svelteHTML.createElement("span", { "class":`val`,});fLink.toFixed(2); }
         }
         { svelteHTML.createElement("label", { "class":`row`,});
           { svelteHTML.createElement("span", {});  }
           { svelteHTML.createElement("input", {          "type":`range`,"min":`10`,"max":`300`,"step":`5`,"bind:value":fDist,});/*Ωignore_startΩ*/() => fDist = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           { svelteHTML.createElement("span", { "class":`val`,});fDist; }
         }

         { svelteHTML.createElement("div", { "class":`sec`,});  }
         { svelteHTML.createElement("label", { "class":`row`,});
           { svelteHTML.createElement("span", {});  }
           { svelteHTML.createElement("input", {          "type":`range`,"min":`0.3`,"max":`3`,"step":`0.1`,"bind:value":nodeScale,});/*Ωignore_startΩ*/() => nodeScale = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           { svelteHTML.createElement("span", { "class":`val`,});nodeScale.toFixed(1); }
         }
         { svelteHTML.createElement("label", { "class":`row`,});
           { svelteHTML.createElement("span", {});  }
           { svelteHTML.createElement("input", {          "type":`range`,"min":`0.2`,"max":`3`,"step":`0.1`,"bind:value":linkWidth,});/*Ωignore_startΩ*/() => linkWidth = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           { svelteHTML.createElement("span", { "class":`val`,});linkWidth.toFixed(1); }
         }
         { svelteHTML.createElement("label", { "class":`row`,});
           { svelteHTML.createElement("span", {});  }
           { svelteHTML.createElement("input", {          "type":`range`,"min":`0.2`,"max":`4`,"step":`0.1`,"bind:value":labelZoom,});/*Ωignore_startΩ*/() => labelZoom = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           { svelteHTML.createElement("span", { "class":`val`,});labelZoom.toFixed(1); }
         }
       }
    }
   }

  
  if(hoverIdx >= 0 && !draggingNode && hoverTip.title){
     { svelteHTML.createElement("div", {   "class":`tooltip`,"style":`left:${hoverTip.x}px; top:${hoverTip.y}px`,});
      hoverTip.title;if(hoverTip.meta){ { svelteHTML.createElement("span", { "class":`tmeta`,});hoverTip.meta; }}
     }
  }
 }


};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const GraphView__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type GraphView__SvelteComponent_ = ReturnType<typeof GraphView__SvelteComponent_>;
/*Ωignore_endΩ*/export default GraphView__SvelteComponent_;