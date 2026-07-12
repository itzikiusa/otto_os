<script lang="ts">
  // Vault v3 graph — Obsidian-style force-directed view of the whole vault, or
  // (local mode) the depth-N neighborhood of the open note. Layout runs in a
  // Web Worker (graph.worker.ts — Barnes-Hut over typed arrays, positions
  // ping-pong back as transferred double-buffers); this component owns the
  // Canvas2D renderer, camera, picking and the controls panel. Built to stay
  // usable at 100k nodes / 1-2M edges: ONE batched path per edge style,
  // deterministic index-stride edge sampling past a draw budget, viewport
  // culling on nodes AND edges, a uniform spatial grid for hover/click/drag
  // hit tests (never O(n) per mousemove), degree-capped label budget, and a
  // single dirty-flag rAF loop that only repaints when something changed.
  import { vaultGraph, type VaultGraphQuery } from '../../lib/api/vault';
  import type { VaultGraphPayload } from '../../lib/api/types';
  import { vault } from './vault.svelte';
  import type { GraphWorkerIn, GraphWorkerOut } from './graph.worker';

  let { local = false }: { local?: boolean } = $props();

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

  // ── Payload data (plain lets — big arrays must NOT become $state proxies) ──
  let paths: string[] = [];
  let titles: string[] = [];
  let flagsArr = new Uint8Array(0);
  let groupsArr = new Uint16Array(0);
  let groupLabels: string[] = [];
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
  let hoverTip = $state({ x: 0, y: 0, title: '' });
  let draggingNode = $state(false); // suppresses the tooltip while dragging
  let dataRev = $state(0); // bumped per adopted payload (re-runs dependent effects)
  let panelOpen = $state(true);

  // Filters (all re-fetch — the server prunes, not the client).
  let filter = $state('');
  let showTags = $state(true);
  let showOrphans = $state(true);
  let showGhosts = $state(true);
  // Reserved scaffolding (index.md / log.md) is navigation, not knowledge —
  // off by default so the graph shows real notes; the toggle brings them back.
  let showReserved = $state(false);
  let groupBy = $state<'folder' | 'type'>('folder');
  let depth = $state(2); // local mode only

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
  let didFit = false; // auto zoom-to-fit once, on the first tick of a payload
  let ticks = 0;

  // Theme colors read from the app's CSS variables (dark fallbacks).
  let theme = {
    text: '#f2f2f5',
    dim: '#98989f',
    accent: '#0a84ff',
    surface: '#2a2a30',
    dark: true,
  };
  let groupColors: string[] = [];

  function readTheme(): void {
    const cs = getComputedStyle(document.documentElement);
    const v = (name: string, fb: string) => cs.getPropertyValue(name).trim() || fb;
    theme = {
      text: v('--text', '#f2f2f5'),
      dim: v('--text-dim', '#98989f'),
      accent: v('--accent', '#0a84ff'),
      surface: v('--surface', '#1c1c1e'),
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
      dirty = true;
    };
  }

  // ── Data adoption ───────────────────────────────────────────────────────
  function adopt(p: VaultGraphPayload): void {
    paths = p.paths;
    titles = p.titles;
    flagsArr = Uint8Array.from(p.flags);
    groupsArr = Uint16Array.from(p.groups);
    groupLabels = p.group_labels;
    edges = Uint32Array.from(p.edges);
    n = paths.length;
    E = edges.length >> 1;
    nodeCount = n;
    edgeCount = E;
    truncated = p.truncated;

    // Degrees → world radii (clamped sqrt curve) + CSR adjacency for hover
    // highlighting (neighbors of one node without scanning 2M edges).
    deg = new Uint32Array(n);
    for (let e = 0; e < E; e++) {
      deg[edges[e * 2]]++;
      deg[edges[e * 2 + 1]]++;
    }
    radii = new Float32Array(n);
    for (let i = 0; i < n; i++) {
      radii[i] = Math.min(12, Math.max(1.5, 1.5 + Math.sqrt(deg[i]) * 0.6));
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
    didFit = false;
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
      wpost({ t: 'init', n, edges: we, groups: wg }, transfer);
      wpost({ t: 'params', center: fCenter, repel: fRepel, link: fLink, dist: fDist });
    }
    dataRev++;
    dirty = true;
  }

  // ── Fetch (own data via vaultGraph; every filter toggle re-fetches) ──────
  let reqSeq = 0;
  $effect(() => {
    const v = vault.current;
    const wsId = vault.wsId;
    const path = local ? vault.notePath : null;
    const q: VaultGraphQuery = {
      tags: showTags,
      orphans: showOrphans,
      ghosts: showGhosts,
      reserved: showReserved,
      group_by: groupBy,
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
      adopt({ paths: [], titles: [], groups: [], group_labels: [], flags: [], edges: [], truncated: false });
      return;
    }
    const seq = ++reqSeq;
    loading = true;
    errorMsg = '';
    vaultGraph(wsId, v.id, q)
      .then((p) => {
        if (seq === reqSeq) adopt(p);
      })
      .catch((e: unknown) => {
        if (seq === reqSeq) errorMsg = e instanceof Error ? e.message : String(e);
      })
      .finally(() => {
        if (seq === reqSeq) loading = false;
      });
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
    dirty = true;
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
    dirty = true;
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
      hoverTip = { ...hoverTip, title: titles[i] ?? '' };
    }
    dirty = true;
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
    dirty = true;
  }

  // ── Rendering (one dirty-flag rAF loop) ──────────────────────────────────
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
    ctx.lineWidth = Math.max(0.4, linkWidth * Math.min(1.5, k));
    ctx.strokeStyle = theme.dim;
    ctx.globalAlpha = eAlpha;
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
    const ghostFill = theme.dark ? '#77777f' : '#9a9aa2';
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
      ctx.font = '10px -apple-system, BlinkMacSystemFont, sans-serif';
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
      dirty = true;
      return;
    }
    // Idle move: hover pick (grid — never O(n)) + tooltip anchor.
    const w = toWorld(p.x, p.y);
    setHover(pick(w.x, w.y));
    if (hoverIdx >= 0) hoverTip = { x: p.x + 14, y: p.y + 10, title: titles[hoverIdx] ?? '' };
  }

  function onPointerUp(e: PointerEvent): void {
    if (dragMode === 'none') return; // capture-less up (e.g. re-entry) — ignore
    if (dragMode === 'node' && !movedFar) {
      // A stationary press on a node is a click: open real notes (plain notes
      // and reserved files); ghost/tag nodes have nothing to open.
      const f = flagsArr[dragI];
      if (!(f & F_GHOST) && !(f & F_TAG)) void vault.open(paths[dragI]);
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
    dirty = true;
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
      dirty = true;
    });
    ro.observe(rootEl);

    // Theme/scheme flips re-read the CSS vars + rebuild the group palette.
    const mo = new MutationObserver(() => {
      readTheme();
      dirty = true;
    });
    mo.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme', 'data-scheme'] });

    // Wheel must be non-passive (we preventDefault to own the zoom gesture).
    canvasEl.addEventListener('wheel', onWheel, { passive: false });
    canvasEl.addEventListener('pointerdown', onPointerDown);
    canvasEl.addEventListener('pointermove', onPointerMove);
    canvasEl.addEventListener('pointerup', onPointerUp);
    canvasEl.addEventListener('pointerleave', onPointerLeave);
    canvasEl.addEventListener('dblclick', onDblClick);

    const loop = (): void => {
      if (dirty) {
        dirty = false;
        draw();
      }
      raf = requestAnimationFrame(loop);
    };
    raf = requestAnimationFrame(loop);

    return () => {
      cancelAnimationFrame(raf);
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
</script>

<div class="graph-root" bind:this={rootEl}>
  <canvas bind:this={canvasEl} class:grabbable={hoverIdx < 0}></canvas>

  {#if !vault.current}
    <div class="empty">No vault selected</div>
  {:else if !loading && nodeCount === 0}
    <div class="empty">
      {#if errorMsg}Graph failed: {errorMsg}
      {:else if local && !vault.notePath}Open a note to see its local graph
      {:else}Nothing to graph — the vault has no notes yet{/if}
    </div>
  {/if}

  <!-- Status strip: counts, truncation warning, sim state, stop/resume. -->
  <div class="statusbar">
    <span class="counts">{nodeCount.toLocaleString()} nodes · {edgeCount.toLocaleString()} links</span>
    {#if truncated}
      <span class="chip warn" title="Edge budget hit — some links were omitted by the server">truncated</span>
    {/if}
    {#if loading}
      <span class="chip">loading…</span>
    {:else if alphaLive > 0.02 && !stopped}
      <span class="chip sim">simulating…</span>
    {/if}
    {#if nodeCount > 0}
      <button class="mini" onclick={toggleSim}>{stopped ? 'Resume' : 'Stop'} layout</button>
    {/if}
  </div>

  <!-- Controls panel (collapsible, top-right). -->
  <div class="panel">
    <button class="panel-head" onclick={() => (panelOpen = !panelOpen)} aria-expanded={panelOpen}>
      <span>Graph</span>
      <span class="tri">{panelOpen ? '▾' : '▸'}</span>
    </button>
    {#if panelOpen}
      <div class="panel-body">
        <input class="filter" type="text" placeholder="Filter titles…" bind:value={filter} />

        <div class="sec">Filters</div>
        <label class="chk"><input type="checkbox" bind:checked={showTags} /> Tags</label>
        <label class="chk"><input type="checkbox" bind:checked={showOrphans} /> Orphans</label>
        <label class="chk"><input type="checkbox" bind:checked={showGhosts} /> Unresolved</label>
        <label class="chk"><input type="checkbox" bind:checked={showReserved} /> Reserved files</label>
        <label class="row">
          <span>Group by</span>
          <select
            value={groupBy}
            onchange={(e) => (groupBy = e.currentTarget.value as 'folder' | 'type')}
          >
            <option value="folder">folder</option>
            <option value="type">type</option>
          </select>
        </label>
        {#if local}
          <label class="row">
            <span>Depth</span>
            <input type="range" min="1" max="3" step="1" bind:value={depth} />
            <span class="val">{depth}</span>
          </label>
        {/if}

        <div class="sec">Forces</div>
        <label class="row">
          <span>Center</span>
          <input type="range" min="0" max="1" step="0.01" bind:value={fCenter} />
          <span class="val">{fCenter.toFixed(2)}</span>
        </label>
        <label class="row">
          <span>Repel</span>
          <input type="range" min="0" max="2" step="0.05" bind:value={fRepel} />
          <span class="val">{fRepel.toFixed(2)}</span>
        </label>
        <label class="row">
          <span>Link</span>
          <input type="range" min="0" max="2" step="0.05" bind:value={fLink} />
          <span class="val">{fLink.toFixed(2)}</span>
        </label>
        <label class="row">
          <span>Distance</span>
          <input type="range" min="10" max="300" step="5" bind:value={fDist} />
          <span class="val">{fDist}</span>
        </label>

        <div class="sec">Display</div>
        <label class="row">
          <span>Node size</span>
          <input type="range" min="0.3" max="3" step="0.1" bind:value={nodeScale} />
          <span class="val">{nodeScale.toFixed(1)}</span>
        </label>
        <label class="row">
          <span>Link width</span>
          <input type="range" min="0.2" max="3" step="0.1" bind:value={linkWidth} />
          <span class="val">{linkWidth.toFixed(1)}</span>
        </label>
        <label class="row">
          <span>Text fade</span>
          <input type="range" min="0.2" max="4" step="0.1" bind:value={labelZoom} />
          <span class="val">{labelZoom.toFixed(1)}</span>
        </label>
      </div>
    {/if}
  </div>

  <!-- Hover tooltip (title). -->
  {#if hoverIdx >= 0 && !draggingNode && hoverTip.title}
    <div class="tooltip" style="left:{hoverTip.x}px; top:{hoverTip.y}px">{hoverTip.title}</div>
  {/if}
</div>

<style>
  .graph-root {
    position: relative;
    width: 100%;
    height: 100%;
    overflow: hidden;
    background: var(--bg, #1e1e23);
  }
  canvas {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    touch-action: none; /* we own pan/zoom */
    cursor: pointer;
  }
  canvas.grabbable {
    cursor: grab;
  }

  .empty {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--text-dim, #98989f);
    font-size: 13px;
    pointer-events: none;
    padding: 20px;
    text-align: center;
  }

  .statusbar {
    position: absolute;
    left: 10px;
    bottom: 10px;
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 11px;
    color: var(--text-dim, #98989f);
    background: color-mix(in srgb, var(--surface, #1c1c1e) 82%, transparent);
    border: 1px solid var(--border, rgba(255, 255, 255, 0.1));
    border-radius: var(--radius-m, 8px);
    padding: 4px 8px;
    backdrop-filter: blur(8px);
  }
  .chip {
    padding: 1px 6px;
    border-radius: 999px;
    background: var(--surface-2, #323238);
    font-size: 10px;
  }
  .chip.warn {
    color: var(--status-warn, #e0a000);
    background: var(--status-warn-soft, rgba(224, 160, 0, 0.16));
  }
  .chip.sim {
    color: var(--accent, #0a84ff);
  }
  .mini {
    font-size: 10px;
    padding: 2px 7px;
    border: 1px solid var(--border, rgba(255, 255, 255, 0.1));
    border-radius: var(--radius-s, 5px);
    background: var(--surface-2, #323238);
    color: var(--text, #f2f2f5);
    cursor: pointer;
  }
  .mini:hover {
    border-color: var(--accent, #0a84ff);
  }

  .panel {
    position: absolute;
    top: 8px;
    right: 8px;
    width: 210px;
    background: color-mix(in srgb, var(--surface, #1c1c1e) 92%, transparent);
    border: 1px solid var(--border, rgba(255, 255, 255, 0.1));
    border-radius: var(--radius-m, 8px);
    box-shadow: var(--shadow, 0 8px 30px rgba(0, 0, 0, 0.45));
    backdrop-filter: blur(10px);
    font-size: 11px;
    color: var(--text, #f2f2f5);
    /* Never taller than the view — the body scrolls (floating-UI rule). */
    max-height: calc(100% - 16px);
    display: flex;
    flex-direction: column;
  }
  .panel-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    width: 100%;
    padding: 6px 10px;
    background: none;
    border: none;
    color: var(--text, #f2f2f5);
    font-size: 11px;
    font-weight: 600;
    cursor: pointer;
  }
  .tri {
    color: var(--text-dim, #98989f);
  }
  .panel-body {
    padding: 0 10px 10px;
    display: flex;
    flex-direction: column;
    gap: 4px;
    overflow-y: auto;
  }
  .filter {
    width: 100%;
    padding: 4px 6px;
    font-size: 11px;
    background: var(--surface-2, #323238);
    border: 1px solid var(--border, rgba(255, 255, 255, 0.1));
    border-radius: var(--radius-s, 5px);
    color: var(--text, #f2f2f5);
  }
  .sec {
    margin-top: 6px;
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-dim, #98989f);
  }
  .chk {
    display: flex;
    align-items: center;
    gap: 6px;
    cursor: pointer;
  }
  .chk input {
    margin: 0;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .row > span:first-child {
    flex: 0 0 62px;
    color: var(--text-dim, #98989f);
  }
  .row input[type='range'] {
    flex: 1;
    min-width: 0;
  }
  .row select {
    flex: 1;
    font-size: 11px;
    background: var(--surface-2, #323238);
    border: 1px solid var(--border, rgba(255, 255, 255, 0.1));
    border-radius: var(--radius-s, 5px);
    color: var(--text, #f2f2f5);
    padding: 2px 4px;
  }
  .val {
    flex: 0 0 30px;
    text-align: right;
    color: var(--text-dim, #98989f);
    font-variant-numeric: tabular-nums;
  }

  .tooltip {
    position: absolute;
    max-width: 260px;
    padding: 3px 8px;
    font-size: 11px;
    color: var(--text, #f2f2f5);
    background: color-mix(in srgb, var(--surface, #1c1c1e) 94%, transparent);
    border: 1px solid var(--border, rgba(255, 255, 255, 0.1));
    border-radius: var(--radius-s, 5px);
    box-shadow: var(--shadow, 0 8px 30px rgba(0, 0, 0, 0.45));
    pointer-events: none;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    z-index: 2;
  }
</style>
