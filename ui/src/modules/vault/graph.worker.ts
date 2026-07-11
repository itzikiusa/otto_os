// graph.worker.ts — Vault v3 graph layout worker (force simulation off-thread).
//
// The view (GraphView.svelte) sends 'init' once per payload with the flat edge
// list TRANSFERRED in, then live 'params' / 'pin' / 'drag' / 'release' /
// 'reheat' / 'stop' / 'resume'. Positions stream back as {t:'tick'} carrying a
// TRANSFERRED Float32Array [x0,y0,x1,y1,…]; the view returns every buffer via
// {t:'buf'}, so two buffers ping-pong forever and steady state allocates
// NOTHING per frame. Physics: Barnes-Hut repulsion over a compact array-backed
// quadtree (no object-per-node allocations), springs along edges, weak
// centering gravity, velocity-Verlet-ish integration with alpha cooling.
// Deterministic (mulberry32, fixed seed) so a given payload always settles
// into the same shape. Plain TS — no DOM APIs.

export type GraphWorkerIn =
  | { t: 'init'; n: number; edges: Uint32Array<ArrayBuffer>; groups?: Uint16Array<ArrayBuffer> }
  | { t: 'params'; center: number; repel: number; link: number; dist: number }
  | { t: 'pin'; i: number; x: number; y: number }
  | { t: 'drag'; i: number; x: number; y: number }
  | { t: 'release'; i: number }
  | { t: 'reheat' }
  | { t: 'stop' }
  | { t: 'resume' }
  | { t: 'buf'; buf: ArrayBuffer };

export type GraphWorkerOut = { t: 'tick'; pos: Float32Array<ArrayBuffer>; alpha: number };

// ── Tunables ────────────────────────────────────────────────────────────────
const ALPHA_MIN = 0.003; // simulation stops below this
const ALPHA_DECAY = 0.995; // alpha *= per step
const DAMP = 0.85; // velocity damping per step
const VMAX2 = 40 * 40; // max velocity magnitude² clamp
const REPEL_BASE = 40; // charge scale (× repel param × source mass)
const CENTER_BASE = 0.03; // gravity scale (× center param)
const MIN_D2 = 1; // repulsion distance² floor (no singularities)
const MAX_DEPTH = 24; // quadtree subdivision cap (coincident points)
const BIG_N = 50_000; // past this: theta 1.5 + chunked edge passes
const EDGE_CHUNK = 600_000; // edges processed per step on big graphs
const CLUSTER_N = 30_000; // past this: group-cluster seeding (needs groups)
const POST_MS = 33; // ≥33ms between tick posts (≤30/sec)

/** mulberry32 — tiny deterministic PRNG; fixed seed → reproducible layouts. */
function mulberry32(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

// ── Simulation state ────────────────────────────────────────────────────────
let n = 0;
let edges = new Uint32Array(0); // flat [a,b,a,b,…] node indices
let E = 0; // edge count (edges.length / 2)
let px = new Float32Array(0);
let py = new Float32Array(0);
let vx = new Float32Array(0);
let vy = new Float32Array(0);
let deg = new Uint32Array(0);
let mass = new Float32Array(0); // 1 + sqrt(degree)
let pinned = new Uint8Array(0);
let alpha = 0;
let stopped = false;
let edgeCursor = 0; // round-robin cursor for chunked spring passes
let params = { center: 0.1, repel: 1, link: 1, dist: 60 };
let rnd = mulberry32(0x1234abcd);
let pool: Float32Array<ArrayBuffer>[] = []; // returned tick buffers awaiting reuse
let lastPost = 0;
let timer: ReturnType<typeof setTimeout> | null = null;

// ── Quadtree (compact, array-backed — zero object allocations) ─────────────
// qtChild holds 4 slots per node: -1 = empty, ≤-2 = leaf point (-(i+2)),
// ≥0 = child node index. Mass/center-of-mass are aggregated bottom-up after
// the build (children always allocate AFTER their parent, so a reverse index
// sweep sees every child before its parent). Arrays grow geometrically and
// are reused across steps.
let qtChild = new Int32Array(4 * 1024);
let qtMass = new Float32Array(1024);
let qtCx = new Float32Array(1024);
let qtCy = new Float32Array(1024);
let qtCount = 0;
let bx = 0, by = 0, bs = 1; // tree bounds (square: origin + side)
// Explicit traversal stack (node index + that node's cell size).
let stackN = new Int32Array(4096);
let stackS = new Float32Array(4096);

function growTree(min: number): void {
  let cap = qtMass.length;
  while (cap < min) cap *= 2;
  if (cap === qtMass.length) return;
  const c = new Int32Array(cap * 4); c.set(qtChild); qtChild = c;
  const m = new Float32Array(cap); m.set(qtMass); qtMass = m;
  const x = new Float32Array(cap); x.set(qtCx); qtCx = x;
  const y = new Float32Array(cap); y.set(qtCy); qtCy = y;
}

function allocNode(): number {
  if (qtCount + 1 > qtMass.length) growTree(qtCount + 1);
  const k = qtCount++;
  qtChild.fill(-1, k * 4, k * 4 + 4);
  return k;
}

function buildTree(): void {
  // Square bounds over current positions.
  let minx = Infinity, miny = Infinity, maxx = -Infinity, maxy = -Infinity;
  for (let i = 0; i < n; i++) {
    const x = px[i], y = py[i];
    if (x < minx) minx = x;
    if (x > maxx) maxx = x;
    if (y < miny) miny = y;
    if (y > maxy) maxy = y;
  }
  bx = minx;
  by = miny;
  bs = Math.max(maxx - minx, maxy - miny, 1) + 1e-3;

  qtCount = 0;
  allocNode(); // root = 0
  for (let i = 0; i < n; i++) {
    // Iterative insert; descend by quadrant, subdividing occupied leaves.
    let node = 0, x0 = bx, y0 = by, size = bs, depth = 0;
    const x = px[i], y = py[i];
    for (;;) {
      const half = size / 2, mx = x0 + half, my = y0 + half;
      const q = (x >= mx ? 1 : 0) + (y >= my ? 2 : 0);
      const slot = node * 4 + q;
      const c = qtChild[slot];
      if (c === -1) {
        qtChild[slot] = -(i + 2);
        break;
      }
      if (c <= -2) {
        const j = -c - 2;
        // Coincident points (or a pathological cluster) would subdivide
        // forever — past the depth cap, drop the structural insert. The point
        // still feels every force; it merely stops CONTRIBUTING repulsion
        // this step (vanishingly rare with the deterministic random seeding).
        if (depth >= MAX_DEPTH) break;
        // Split: push the resident leaf one level down, then re-run the loop
        // for i inside the new cell (may split repeatedly if they share
        // quadrants).
        const k = allocNode();
        qtChild[slot] = k;
        if (q & 1) x0 = mx;
        if (q & 2) y0 = my;
        size = half;
        depth++;
        const qh = size / 2;
        const qj = (px[j] >= x0 + qh ? 1 : 0) + (py[j] >= y0 + qh ? 2 : 0);
        qtChild[k * 4 + qj] = -(j + 2);
        node = k;
        continue;
      }
      node = c;
      if (q & 1) x0 = mx;
      if (q & 2) y0 = my;
      size = half;
      depth++;
    }
  }

  // Bottom-up mass + center-of-mass (reverse order: children before parents).
  for (let k = qtCount - 1; k >= 0; k--) {
    let m = 0, sx = 0, sy = 0;
    for (let q = 0; q < 4; q++) {
      const c = qtChild[k * 4 + q];
      if (c === -1) continue;
      if (c <= -2) {
        const j = -c - 2;
        const w = mass[j];
        m += w; sx += px[j] * w; sy += py[j] * w;
      } else {
        const w = qtMass[c];
        m += w; sx += qtCx[c] * w; sy += qtCy[c] * w;
      }
    }
    qtMass[k] = m;
    qtCx[k] = m > 0 ? sx / m : 0;
    qtCy[k] = m > 0 ? sy / m : 0;
  }
}

/** Barnes-Hut repulsion: accept an aggregate when size² < θ²·d² else descend.
 *  d3-style force form: v += Δ·(-k·m/d²)·α — no sqrt in the hot loop. */
function repulsion(theta2: number): void {
  const k = REPEL_BASE * params.repel;
  if (k <= 0) return;
  const ka = k * alpha;
  for (let i = 0; i < n; i++) {
    const x = px[i], y = py[i];
    let fx = 0, fy = 0;
    let top = 0;
    stackN[top] = 0;
    stackS[top++] = bs;
    while (top > 0) {
      const node = stackN[--top];
      const size = stackS[top];
      const m = qtMass[node];
      if (m <= 0) continue;
      const dx = qtCx[node] - x, dy = qtCy[node] - y;
      const d2 = dx * dx + dy * dy;
      if (size * size < theta2 * d2) {
        // Far enough — take the aggregate. (An aggregate CONTAINING i is by
        // construction near, so it always fails this test and expands.)
        const dd = d2 < MIN_D2 ? MIN_D2 : d2;
        const w = -ka * m / dd;
        fx += dx * w;
        fy += dy * w;
        continue;
      }
      // Near — expand: apply leaves pairwise, push child nodes.
      for (let q = 0; q < 4; q++) {
        const c = qtChild[node * 4 + q];
        if (c === -1) continue;
        if (c <= -2) {
          const j = -c - 2;
          if (j === i) continue;
          let ddx = px[j] - x, ddy = py[j] - y;
          let dd2 = ddx * ddx + ddy * ddy;
          if (dd2 < 1e-4) {
            // Exactly overlapping — deterministic jiggle so they separate.
            ddx = rnd() - 0.5;
            ddy = rnd() - 0.5;
            dd2 = ddx * ddx + ddy * ddy;
          }
          if (dd2 < MIN_D2) dd2 = MIN_D2;
          const w = -ka * mass[j] / dd2;
          fx += ddx * w;
          fy += ddy * w;
        } else {
          if (top + 1 > stackN.length) {
            const sn = new Int32Array(stackN.length * 2); sn.set(stackN); stackN = sn;
            const ss = new Float32Array(stackS.length * 2); ss.set(stackS); stackS = ss;
          }
          stackN[top] = c;
          stackS[top++] = size / 2;
        }
      }
    }
    vx[i] += fx;
    vy[i] += fy;
  }
}

/** Springs along edges toward params.dist. On big graphs only EDGE_CHUNK
 *  edges run per step (round-robin cursor) so a step stays bounded; the force
 *  is scaled up (capped ×3) to compensate for the reduced visit frequency. */
function springs(): void {
  const kL = params.link;
  if (kL <= 0 || E === 0) return;
  const chunk = n > BIG_N ? Math.min(E, EDGE_CHUNK) : E;
  const comp = Math.min(3, E / chunk);
  const rest = params.dist;
  const s0 = kL * alpha * comp;
  for (let c = 0; c < chunk; c++) {
    if (edgeCursor >= E) edgeCursor = 0;
    const p = edgeCursor++ * 2;
    const a = edges[p], b = edges[p + 1];
    // Anticipate current velocities (d3-link style) — steadier convergence.
    let dx = px[b] + vx[b] - px[a] - vx[a];
    let dy = py[b] + vy[b] - py[a] - vy[a];
    let l = Math.sqrt(dx * dx + dy * dy);
    if (l < 1e-3) {
      dx = rnd() - 0.5;
      dy = rnd() - 0.5;
      l = Math.sqrt(dx * dx + dy * dy);
    }
    // Hub-normalized strength (1/min degree) keeps high-degree stars from
    // collapsing into a point; degree-weighted bias moves the lighter end.
    const da = deg[a] || 1, db = deg[b] || 1;
    const f = ((l - rest) / l) * (s0 / Math.min(da, db));
    const bias = da / (da + db);
    dx *= f;
    dy *= f;
    vx[b] -= dx * bias;
    vy[b] -= dy * bias;
    vx[a] += dx * (1 - bias);
    vy[a] += dy * (1 - bias);
  }
}

/** Weak centering gravity + damping + velocity clamp + position update.
 *  Pinned nodes hold their exact dragged position (velocity zeroed). */
function integrate(): void {
  const g = params.center * CENTER_BASE * alpha;
  for (let i = 0; i < n; i++) {
    if (pinned[i]) {
      vx[i] = 0;
      vy[i] = 0;
      continue;
    }
    let ix = (vx[i] - px[i] * g) * DAMP;
    let iy = (vy[i] - py[i] * g) * DAMP;
    const v2 = ix * ix + iy * iy;
    if (v2 > VMAX2) {
      const s = Math.sqrt(VMAX2 / v2);
      ix *= s;
      iy *= s;
    }
    vx[i] = ix;
    vy[i] = iy;
    px[i] += ix;
    py[i] += iy;
  }
}

function step(): void {
  // θ=1.2 for quality; 1.5 past 50k nodes so a step stays under ~50ms.
  const theta2 = n > BIG_N ? 1.5 * 1.5 : 1.2 * 1.2;
  buildTree();
  repulsion(theta2);
  springs();
  integrate();
  alpha *= ALPHA_DECAY;
}

// ── Posting (double-buffered, transferred) ──────────────────────────────────
// `self` is typed as Window under the app's DOM lib; the (message, {transfer})
// overload matches DedicatedWorkerGlobalScope.postMessage at runtime.
const wpost = self.postMessage as (msg: GraphWorkerOut, opts: { transfer: Transferable[] }) => void;

function postTick(force = false): void {
  if (n === 0) return;
  let buf = pool.pop();
  if (!buf) {
    if (!force) return; // both buffers in flight — skip, keep stepping
    buf = new Float32Array(n * 2); // rare: forced post (settle/drag) with none free
  }
  for (let i = 0; i < n; i++) {
    buf[i * 2] = px[i];
    buf[i * 2 + 1] = py[i];
  }
  wpost({ t: 'tick', pos: buf, alpha }, { transfer: [buf.buffer] });
  lastPost = Date.now();
}

// ── Step loop (setTimeout(0) so messages interleave between steps) ─────────
function loop(): void {
  timer = null;
  if (stopped || n === 0 || alpha < ALPHA_MIN) return;
  step();
  if (alpha < ALPHA_MIN) {
    postTick(true); // settled — the final frame must land
    return;
  }
  if (Date.now() - lastPost >= POST_MS) postTick();
  timer = setTimeout(loop, 0);
}

function kick(): void {
  if (!timer && !stopped && n > 0 && alpha >= ALPHA_MIN) timer = setTimeout(loop, 0);
}

// ── Seeding ─────────────────────────────────────────────────────────────────
function seed(groups?: Uint16Array): void {
  const R = Math.sqrt(Math.max(n, 1)) * 24 + 60; // radius scales so density stays ~constant
  if (n > CLUSTER_N && groups && groups.length === n) {
    // Group-cluster seeding: each group's center sits on a golden-angle
    // spiral ring, members jitter around it — related notes start together,
    // so the first visible frames already read as a clustered map and the
    // simulation converges far faster than from a uniform disc.
    let ng = 1;
    for (let i = 0; i < n; i++) if (groups[i] + 1 > ng) ng = groups[i] + 1;
    const golden = Math.PI * (3 - Math.sqrt(5));
    const cx = new Float32Array(ng), cy = new Float32Array(ng);
    for (let g = 0; g < ng; g++) {
      const a = g * golden;
      const r = R * 0.55 * Math.sqrt((g + 0.5) / ng);
      cx[g] = Math.cos(a) * r;
      cy[g] = Math.sin(a) * r;
    }
    const j = R * 0.18;
    for (let i = 0; i < n; i++) {
      const g = groups[i];
      px[i] = cx[g] + (rnd() + rnd() - 1) * j;
      py[i] = cy[g] + (rnd() + rnd() - 1) * j;
    }
  } else {
    // Uniform random disc (sqrt for even area density).
    for (let i = 0; i < n; i++) {
      const r = R * Math.sqrt(rnd());
      const a = rnd() * Math.PI * 2;
      px[i] = Math.cos(a) * r;
      py[i] = Math.sin(a) * r;
    }
  }
  vx.fill(0);
  vy.fill(0);
}

function initSim(m: { n: number; edges: Uint32Array<ArrayBuffer>; groups?: Uint16Array }): void {
  n = m.n;
  // Defensive compaction: drop any out-of-range pair so a bad index can't
  // poison the position arrays with NaN (contract says this never happens).
  const raw = m.edges;
  let w = 0;
  for (let r = 0; r + 1 < raw.length; r += 2) {
    const a = raw[r], b = raw[r + 1];
    if (a < n && b < n) {
      raw[w++] = a;
      raw[w++] = b;
    }
  }
  edges = raw.subarray(0, w);
  E = w >> 1;

  px = new Float32Array(n);
  py = new Float32Array(n);
  vx = new Float32Array(n);
  vy = new Float32Array(n);
  pinned = new Uint8Array(n);
  deg = new Uint32Array(n);
  mass = new Float32Array(n);
  for (let e = 0; e < E; e++) {
    deg[edges[e * 2]]++;
    deg[edges[e * 2 + 1]]++;
  }
  for (let i = 0; i < n; i++) mass[i] = 1 + Math.sqrt(deg[i]);

  rnd = mulberry32(0x1234abcd); // re-seed → identical payload = identical layout
  seed(m.groups);

  alpha = 1;
  stopped = false;
  edgeCursor = 0;
  lastPost = 0;
  pool = n > 0 ? [new Float32Array(n * 2), new Float32Array(n * 2)] : [];
  postTick(); // first frame immediately so the view can draw the seeds
  kick();
}

// ── Message pump ────────────────────────────────────────────────────────────
self.onmessage = (e: MessageEvent) => {
  const m = e.data as GraphWorkerIn;
  switch (m.t) {
    case 'init':
      initSim(m);
      break;
    case 'params':
      params = { center: m.center, repel: m.repel, link: m.link, dist: m.dist };
      alpha = Math.max(alpha, 0.3); // mild reheat so the change is visible
      kick();
      break;
    case 'pin':
    case 'drag':
      if (m.i >= 0 && m.i < n) {
        pinned[m.i] = 1;
        px[m.i] = m.x;
        py[m.i] = m.y;
        vx[m.i] = 0;
        vy[m.i] = 0;
        alpha = Math.max(alpha, 0.12); // keep neighbors following the drag
        kick();
        // When the loop is idle (settled/stopped) still echo the move —
        // throttled — so the view sees the node track the pointer.
        if (Date.now() - lastPost >= POST_MS) postTick(true);
      }
      break;
    case 'release':
      if (m.i >= 0 && m.i < n) {
        pinned[m.i] = 0;
        alpha = Math.max(alpha, 0.3);
        kick();
      }
      break;
    case 'reheat':
      alpha = Math.max(alpha, 0.5);
      kick();
      break;
    case 'stop':
      stopped = true;
      if (timer) {
        clearTimeout(timer);
        timer = null;
      }
      break;
    case 'resume':
      stopped = false;
      alpha = Math.max(alpha, 0.1);
      kick();
      break;
    case 'buf':
      // A tick buffer coming home. Ignore stale sizes (init happened since).
      if (m.buf.byteLength === n * 2 * 4 && pool.length < 2) pool.push(new Float32Array(m.buf));
      break;
  }
};
