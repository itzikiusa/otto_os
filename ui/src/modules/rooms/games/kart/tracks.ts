/**
 * Kart tracks: a closed main loop plus optional branches (forks that leave the
 * loop and rejoin it later), each a centripetal Catmull-Rom centre line sampled
 * every ~2 m. Pure data + geometry (no three.js) so the simulation, renderer and
 * tests share it. Coordinates: metres, Y up. Distance `s` runs from the start
 * line along the main loop; a branch has its own local distance and maps onto
 * main-loop progress linearly between its fork and merge, so laps, ranking and
 * wrong-way detection work on every route.
 *
 * Per-sample flags carry the course furniture the simulation needs in O(1):
 * bridges (the ground falls away beside the deck), gaps (no road: jump or
 * glide), tunnels, rails, ice, off-road patches, banking, drawbridges,
 * half-pipes and stretches that run under the water surface. Overpasses (a
 * stretch passing above another) are detected and railed automatically.
 * Everything that moves (rollers, crushers, drawbridge leaves) is a pure
 * function of the race clock, so guests animate it with no extra net data.
 */
import { clamp, fbm, hashString, makeNoise, smoothstep } from '../common/math.ts';
import type { TrackId, EdgeKind, ObstacleKind, Drawbridge, Features, TrackDef } from './tracks/types.ts';
import { TRACK_DEFS } from './tracks/defs.ts';
export * from './tracks/types.ts';
export { TRACK_DEFS, TRACK_IDS } from './tracks/defs.ts';

// ── Sample flags ────────────────────────────────────────────────────────────
export const F_BRIDGE = 1, F_RAILS = 2, F_GAP = 4, F_TUNNEL = 8, F_UNDER = 16, F_ICE = 32, F_OFFROAD = 64, F_BANK = 128, F_GLASS = 256, F_DRAW = 512, F_PIPE = 1024;
/** Below a bridge or gap the ground drops this far (no liquid). */
export const CHASM = 18;
/** Tunnel ceiling above the road. */
export const TUNNEL_HEIGHT = 7;
/** Railed roads (bridges, tunnels) carry a hard shoulder this wide out to the rail. */
export const RAIL_SHOULDER = 1.6;

export interface Path {
  /** 0 = main loop, k = branch k (1-based). */
  index: number; name: string; closed: boolean;
  count: number; length: number;
  /** Per sample: position, unit tangent (xz), distance along this path. */
  px: Float64Array; py: Float64Array; pz: Float64Array;
  tx: Float64Array; tz: Float64Array; dist: Float64Array;
  /** Signed curvature per sample (rad/m, + = left turn). */
  curve: Float64Array;
  /** Road cross slope per sample: height change per metre of lateral offset is `-bank` (+ raises the right-hand side). */
  bank: Float64Array;
  /** 0..1: how far the ground beside the road falls away (bridges, gaps), eased at the ends. */
  drop: Float32Array;
  flags: Uint16Array;
  halfWidth: number; edge: EdgeKind; outOfBounds: number;
  /** Main-loop distances where this path forks off and merges back (main loop: 0 and its length). */
  from: number; to: number;
  features: Features;
  shortcut: boolean;
  /** Pays off only with a boost item (see BranchDef.boost). */
  boost: boolean;
  cpu: [number, number, number];
}
export interface Obstacle { seg: number; s: number; kind: ObstacleKind; period: number; phase: number; lane: number }
export interface Track extends Path {
  def: TrackDef;
  /** paths[0] is the track itself (the main loop); then one per branch. */
  paths: Path[];
  liquid: number | null;
  lava: boolean;
  obstacles: Obstacle[];
  /** Height a racer drives on at (x, z), measured from its nearest centre-line point, at race time `time` (drawbridges). */
  ground: (x: number, z: number, near: Near, time?: number) => number;
  /** Landscape height (for the renderer): the road bed on the road, the land below bridge decks and gaps. */
  terrain: (x: number, z: number, near: Near) => number;
}
export interface Near { seg: number; index: number; t: number; s: number; lateral: number; y: number; tx: number; tz: number; distance: number }
export const newNear = (): Near => ({ seg: 0, index: 0, t: 0, s: 0, lateral: 0, y: 0, tx: 0, tz: 1, distance: 0 });

/** Centripetal Catmull-Rom point between p1 and p2. */
function catmull(p0: number[], p1: number[], p2: number[], p3: number[], t: number, out: number[]): void {
  const d = (a: number[], b: number[]) => Math.max(1e-4, Math.sqrt(Math.hypot(b[0] - a[0], b[1] - a[1], b[2] - a[2])));
  const t0 = 0, t1 = t0 + d(p0, p1), t2 = t1 + d(p1, p2), t3 = t2 + d(p2, p3);
  const u = t1 + (t2 - t1) * t;
  for (let k = 0; k < 3; k++) {
    const a1 = ((t1 - u) * p0[k] + (u - t0) * p1[k]) / (t1 - t0);
    const a2 = ((t2 - u) * p1[k] + (u - t1) * p2[k]) / (t2 - t1);
    const a3 = ((t3 - u) * p2[k] + (u - t2) * p3[k]) / (t3 - t2);
    const b1 = ((t2 - u) * a1 + (u - t0) * a2) / (t2 - t0);
    const b2 = ((t3 - u) * a2 + (u - t1) * a3) / (t3 - t1);
    out[k] = ((t2 - u) * b1 + (u - t1) * b2) / (t2 - t1);
  }
}

/** Jump ramps rise over the RAMP_LENGTH metres before their lip. */
export const RAMP_LENGTH = 8;
export const RAMP_HEIGHT = 1.4;
/** Launch ramps (and glider pads) are longer and taller. */
export const LAUNCH_LENGTH = 14;
export const LAUNCH_HEIGHT = 2.6;
export function rampHeight(def: TrackDef, length: number, s: number): number {
  for (const at of def.ramps) {
    const d = at * length - s;
    if (d >= 0 && d < RAMP_LENGTH) return RAMP_HEIGHT * (1 - d / RAMP_LENGTH);
  }
  return 0;
}
/** Kicker or launch ramp height on any path at local distance `s`. */
export function rampOn(path: Path, s: number): number {
  for (const at of path.features.ramps) {
    const d = at * path.length - s;
    if (d >= 0 && d < RAMP_LENGTH) return RAMP_HEIGHT * (1 - d / RAMP_LENGTH);
  }
  for (const l of path.features.launches) {
    const d = l.at * path.length - s;
    if (d >= 0 && d < LAUNCH_LENGTH) return LAUNCH_HEIGHT * (1 - d / LAUNCH_LENGTH);
  }
  return 0;
}

/** Drawbridge leaves tilt up to this angle (radians). */
export const DRAW_ANGLE = 0.3;
/** Drawbridge cycle at race time `time`: 0 = down … 1 = fully up (down 45 %, rising 15 %, up 25 %, lowering 15 % of the period). */
export function drawLift(d: Drawbridge, time: number): number {
  const u = (((time / d.period + (d.phase ?? 0)) % 1) + 1) % 1;
  return u < 0.45 ? 0 : u < 0.6 ? smoothstep(0.45, 0.6, u) : u < 0.85 ? 1 : 1 - smoothstep(0.85, 1, u);
}
/**
 * Height of a drawbridge leaf above the road at local distance `s` and time
 * `time`: 0 off the bridge or while it is down, NaN in the opening between the
 * raised tips (each leaf, rotating about its hinge, pulls back from the middle).
 */
export function drawOn(path: Path, s: number, time: number): number {
  for (const d of path.features.drawbridges) {
    const a = d.from * path.length, b = d.to * path.length;
    if (s < a || s > b) continue;
    const lift = drawLift(d, time);
    if (lift <= 0) return 0;
    const th = DRAW_ANGLE * lift, half = (b - a) / 2;
    const dist = s < a + half ? s - a : b - s;
    if (dist > half * Math.cos(th)) return NaN;
    return dist * Math.tan(th);
  }
  return 0;
}
/** Half-pipe wall height at the road edge (the shoulder out to the rail stays at this height). */
export const PIPE_HEIGHT = 3.4;
/** Half-pipes ease in over this many metres and end abruptly in the lip. */
export const PIPE_ENTRY = 14;
/** Half-pipe wall height above the road at local distance `s`, lateral offset `lateral`. */
export function pipeOn(path: Path, s: number, lateral: number): number {
  for (const [f, t] of path.features.halfpipes) {
    const a = f * path.length, b = t * path.length;
    if (s < a || s > b) continue;
    const w = clamp((Math.abs(lateral) / path.halfWidth - 0.35) / 0.65, 0, 1);
    return PIPE_HEIGHT * w * w * smoothstep(a, a + PIPE_ENTRY, s);
  }
  return 0;
}

interface Line { count: number; length: number; px: Float64Array; py: Float64Array; pz: Float64Array; tx: Float64Array; tz: Float64Array; dist: Float64Array; curve: Float64Array }
/**
 * Samples a centre line every ~2 m. `ctrl` is [x, y, z]; a closed line wraps,
 * an open one runs from ctrl[1] to ctrl[n-2] (the ends only guide the tangent).
 */
function sampleLine(ctrl: number[][], closed: boolean): Line {
  const m = ctrl.length;
  const dense: number[][] = [];
  const out = [0, 0, 0];
  const first = closed ? 0 : 1, last = closed ? m : m - 2;
  for (let i = first; i < last; i++) {
    const at = (k: number) => ctrl[closed ? (k + m) % m : clamp(k, 0, m - 1)];
    for (let k = 0; k < 60; k++) { catmull(at(i - 1), at(i), at(i + 1), at(i + 2), k / 60, out); dense.push([out[0], out[1], out[2]]); }
  }
  if (!closed) dense.push([...ctrl[m - 2]]);
  const segs = closed ? dense.length : dense.length - 1;
  const cum = [0];
  for (let i = 1; i <= segs; i++) {
    const a = dense[i - 1], b = dense[i % dense.length];
    cum.push(cum[i - 1] + Math.hypot(b[0] - a[0], b[2] - a[2]));
  }
  const length = cum[segs];
  const count = closed ? Math.round(length / 2) : Math.round(length / 2) + 1;
  const spacing = closed ? length / count : length / (count - 1);
  const px = new Float64Array(count), py = new Float64Array(count), pz = new Float64Array(count);
  const tx = new Float64Array(count), tz = new Float64Array(count), dist = new Float64Array(count), curve = new Float64Array(count);
  let j = 0;
  for (let i = 0; i < count; i++) {
    const s = Math.min(length, i * spacing);
    while (j < segs - 1 && cum[j + 1] < s) j++;
    const a = dense[j], b = dense[(j + 1) % dense.length];
    const f = clamp((s - cum[j]) / Math.max(1e-6, cum[j + 1] - cum[j]), 0, 1);
    px[i] = a[0] + (b[0] - a[0]) * f; py[i] = a[1] + (b[1] - a[1]) * f; pz[i] = a[2] + (b[2] - a[2]) * f;
    dist[i] = s;
  }
  const idx = (i: number) => (closed ? (i + count) % count : clamp(i, 0, count - 1));
  for (let i = 0; i < count; i++) {
    const n = idx(i + 1), p = idx(i - 1);
    const dx = px[n] - px[p], dz = pz[n] - pz[p], l = Math.hypot(dx, dz) || 1;
    tx[i] = dx / l; tz[i] = dz / l;
  }
  for (let i = 0; i < count; i++) {
    const n = idx(i + 3), p = idx(i - 3), span = closed ? 6 : n - p;
    const a0 = Math.atan2(tx[p], tz[p]), a1 = Math.atan2(tx[n], tz[n]);
    let d = a1 - a0;
    while (d > Math.PI) d -= Math.PI * 2;
    while (d < -Math.PI) d += Math.PI * 2;
    curve[i] = span > 0 ? d / (span * spacing) : 0;
  }
  return { count, length, px, py, pz, tx, tz, dist, curve };
}

const FEATURE_KEYS = ['itemRows', 'boostPads', 'ramps', 'ice', 'launches', 'offroad', 'bridges', 'gaps', 'tunnels', 'banked', 'obstacles', 'drawbridges', 'halfpipes'] as const;
function features(src: Partial<Features>): Features {
  const f = {} as Record<(typeof FEATURE_KEYS)[number], unknown>;
  for (const k of FEATURE_KEYS) f[k] = src[k] ?? [];
  return f as unknown as Features;
}

function makePath(index: number, name: string, line: Line, closed: boolean, opts: { width: number; edge: EdgeKind; outOfBounds: number; features: Features; from: number; to: number; cpu: [number, number, number]; shortcut: boolean; boost: boolean }): Path {
  const { count } = line;
  const flags = new Uint16Array(count);
  const f = opts.features;
  const inside = (frac: number, a: number, b: number) => frac >= a && frac <= b;
  for (let i = 0; i < count; i++) {
    const frac = line.dist[i] / line.length;
    let v = 0;
    for (const [a, b] of f.ice) if (inside(frac, a, b)) v |= F_ICE;
    for (const o of f.offroad) if (inside(frac, o.from, o.to)) v |= F_OFFROAD;
    for (const b of f.bridges) if (inside(frac, b.from, b.to)) v |= F_BRIDGE | (b.rails ? F_RAILS : 0);
    for (const [a, b] of f.gaps) if (inside(frac, a, b)) v |= F_GAP;
    for (const t of f.tunnels) if (inside(frac, t.from, t.to)) v |= F_TUNNEL | F_RAILS | (t.glass ? F_GLASS : 0);
    for (const [a, b] of f.banked) if (inside(frac, a, b)) v |= F_BANK;
    for (const d of f.drawbridges) if (inside(frac, d.from, d.to)) v |= F_BRIDGE | F_RAILS | F_DRAW;
    for (const [a, b] of f.halfpipes) if (inside(frac, a, b)) v |= F_PIPE | F_RAILS;
    flags[i] = v;
  }
  const bank = new Float64Array(count), drop = new Float32Array(count);
  return {
    index, name, closed, ...line, bank, drop, flags,
    halfWidth: opts.width / 2, edge: opts.edge, outOfBounds: opts.outOfBounds,
    from: opts.from, to: opts.to, features: f, shortcut: opts.shortcut, boost: opts.boost, cpu: opts.cpu,
  };
}

/** Smooths a per-sample array with a box filter of radius `r` (wrapping on closed paths). */
function blur(path: Path, src: Float64Array, r: number): Float64Array {
  const n = path.count, out = new Float64Array(n);
  for (let i = 0; i < n; i++) {
    let sum = 0, w = 0;
    for (let k = -r; k <= r; k++) {
      const j = path.closed ? (i + k + n) % n : i + k;
      if (j < 0 || j >= n) continue;
      sum += src[j]; w++;
    }
    out[i] = sum / w;
  }
  return out;
}

/** Marks stretches that pass above another stretch (overpasses, stacked spiral turns) as railed bridges. */
function markOverpasses(paths: Path[]): void {
  for (const p of paths) for (const q of paths) {
    // Roads overlapping (or nearly touching) in plan, one well above the other.
    const reach = p.halfWidth + q.halfWidth + 3;
    for (let i = 0; i < p.count; i++) {
      const x = p.px[i], z = p.pz[i], y = p.py[i];
      for (let j = 0; j < q.count; j++) {
        if (y - q.py[j] < 4.5) continue;
        const dx = q.px[j] - x;
        if (dx > reach || dx < -reach) continue;
        const dz = q.pz[j] - z;
        if (dz > reach || dz < -reach || dx * dx + dz * dz > reach * reach) continue;
        if (p === q) {
          const gap = Math.abs(i - j);
          if (Math.min(gap, p.closed ? p.count - gap : gap) < 30) continue;
        }
        p.flags[i] |= F_BRIDGE | F_RAILS;
        break;
      }
    }
  }
}

/**
 * Where a branch still shares tarmac with the main loop (the mouth of a fork
 * or merge), its road surface is the main road's: it may only climb or dive
 * once the two have pulled apart. Otherwise one strip runs under the other
 * and a racer swapping paths drops off (or into) a ledge.
 */
function matchForkHeights(main: Path, line: Line, from: number, to: number, halfWidth: number): void {
  const near = newNear(), touch = main.halfWidth + halfWidth;
  for (let i = 0; i < line.count; i++) {
    const s = line.dist[i];
    if (s > FORK_ZONE * 2.5 && s < line.length - FORK_ZONE * 2.5) continue;
    const hint = sampleAt(main, s < line.length / 2 ? from + s : to - (line.length - s));
    nearest(main, line.px[i], line.pz[i], hint, near, 40);
    const w = 1 - smoothstep(touch - 2, touch + 12, near.distance);
    if (w <= 0) continue;
    line.py[i] += (near.y - line.py[i]) * w;
  }
}

const cache = new Map<TrackId, Track>();
export function trackFor(id: string): Track {
  const def = TRACK_DEFS[(id in TRACK_DEFS ? id : 'meadow') as TrackId];
  const hit = cache.get(def.id);
  if (hit) return hit;
  const mainLine = sampleLine(def.points.map(([x, z, y]) => [x, y, z]), true);
  const main = makePath(0, def.name, mainLine, true, {
    width: def.width, edge: def.edge, outOfBounds: def.outOfBounds, features: features(def),
    from: 0, to: mainLine.length, cpu: [1, 1, 1], shortcut: false, boost: false,
  });
  const paths: Path[] = [main];
  const pt = [0, 0, 0, 0];
  for (const b of def.branches ?? []) {
    const F = b.from * main.length, T = b.to * main.length;
    const at = (s: number): number[] => { pointAt(main, s, 0, pt); return [pt[0], pt[1], pt[2]]; };
    const ctrl = [at(F - 16), at(F), ...b.points.map(([x, z, y]) => [x, y, z]), at(T), at(T + 16)];
    const line = sampleLine(ctrl, false);
    matchForkHeights(main, line, F, T, (b.width ?? def.width) / 2);
    paths.push(makePath(paths.length, b.name, line, false, {
      width: b.width ?? def.width, edge: b.edge ?? def.edge, outOfBounds: b.outOfBounds ?? def.outOfBounds, features: features(b),
      from: F, to: T, cpu: b.cpu, shortcut: b.shortcut === true, boost: b.boost === true,
    }));
  }
  const liquid = def.liquid ? def.liquid.level : null, lava = def.liquid?.kind === 'lava';
  markOverpasses(paths);
  for (const p of paths) {
    const raw = new Float64Array(p.count), dropRaw = new Float64Array(p.count);
    for (let i = 0; i < p.count; i++) {
      // ~15° on a 60 m radius, steeper on the spirals.
      if (p.flags[i] & F_BANK) raw[i] = clamp(p.curve[i] * 16, -0.42, 0.42);
      if (p.flags[i] & (F_BRIDGE | F_GAP)) dropRaw[i] = 1;
      if (liquid !== null && !lava && p.py[i] < liquid + 0.5) p.flags[i] |= F_UNDER;
    }
    p.bank.set(blur(p, raw, 6));
    // The ground eases down over ~12 m before a bridge or gap.
    for (let i = 0; i < p.count; i++) {
      let d = 0;
      for (let k = -6; k <= 6; k++) {
        const j = p.closed ? (i + k + p.count) % p.count : i + k;
        if (j >= 0 && j < p.count && dropRaw[j] > 0) d = Math.max(d, 1 - Math.abs(k) / 7);
      }
      p.drop[i] = d;
    }
  }
  const obstacles: Obstacle[] = [];
  for (const p of paths) for (const o of p.features.obstacles) obstacles.push({ seg: p.index, s: o.at * p.length, kind: o.kind, period: o.period, phase: o.phase ?? 0, lane: o.lane ?? 0 });
  const noise = makeNoise(hashString(def.id));
  const pools = def.pools ?? [];
  const poolSink = (x: number, z: number): number => {
    let sink = 0;
    for (const [cx, cz, r] of pools) {
      const d = Math.hypot(x - cx, z - cz);
      if (d < r) sink = Math.max(sink, 1 - smoothstep(r * 0.55, r, d));
    }
    return sink;
  };
  const track = main as Track;
  track.def = def; track.paths = paths; track.liquid = liquid; track.lava = lava; track.obstacles = obstacles;
  /**
   * Ground beside the road `off` metres beyond its edge: the road edge height,
   * rising into themed hills (walls on walled tracks, rock either side of a
   * tunnel); in pools it sinks under the surface, and beside bridges and gaps
   * it falls away to the water, lava or a ravine.
   */
  const beside = (x: number, z: number, near: Near, p: Path, off: number): number => {
    const flags = p.flags[near.index] ?? 0, drop = p.drop[near.index] ?? 0;
    // Land meets the road edge on that side (lower or higher on a banked turn).
    const edge = near.y - Math.sign(near.lateral) * p.halfWidth * (p.bank[near.index] ?? 0);
    let natural: number;
    if (off <= 1) natural = edge;
    else {
      const rise = smoothstep(1, 70, off);
      const hill = (fbm(noise, x * 0.012, z * 0.012, 4) * 0.5 + 0.5) * def.hills;
      const wall = p.edge === 'wall' ? smoothstep(3, 14, off) * (def.theme === 'city' || def.theme === 'dusk' ? 0 : 10 + def.hills * 0.6) : 0;
      const cut = flags & F_TUNNEL && !(flags & F_GLASS) ? smoothstep(1, 8, off) * (TUNNEL_HEIGHT + 2) : 0;
      natural = edge + rise * hill + wall + cut;
    }
    // Lagoons and lava lakes: the land sinks below the surface a few metres off the road.
    const sink = poolSink(x, z);
    const wet = liquid !== null ? Math.min(near.y - 4, liquid - 5) : near.y - CHASM;
    if (sink > 0 && off > 2) natural += (Math.min(natural, wet) - natural) * sink * smoothstep(2, 7, off);
    if (drop <= 0) return natural;
    // Beside a bridge or gap: down to the water/lava inside a pool, else a ravine.
    const low = sink > 0.5 ? wet : near.y - CHASM;
    return natural + (low - natural) * drop;
  };
  track.ground = (x, z, near, time = 0) => {
    const p = paths[near.seg] ?? main;
    const off = Math.abs(near.lateral) - p.halfWidth;
    const flags = p.flags[near.index] ?? 0;
    // On the road, and on a railed deck's shoulder out to the rail.
    if (off <= 0 || (off <= RAIL_SHOULDER && flags & F_RAILS && !(flags & F_GAP))) {
      if (flags & F_GAP) return near.y - CHASM * 3;
      let lift = off <= 0 ? rampOn(p, near.s) : 0;
      if (flags & F_DRAW) {
        const leaf = drawOn(p, near.s, time);
        if (leaf !== leaf) return near.y - CHASM * 3; // the opening between raised leaves
        lift += leaf;
      }
      if (flags & F_PIPE) lift += pipeOn(p, near.s, near.lateral);
      return near.y + lift - near.lateral * (p.bank[near.index] ?? 0);
    }
    return beside(x, z, near, p, off);
  };
  track.terrain = (x, z, near) => {
    const p = paths[near.seg] ?? main;
    const off = Math.abs(near.lateral) - p.halfWidth;
    // Under a bridge deck or over a gap the land is what lies beside it.
    if ((p.flags[near.index] ?? 0) & (F_BRIDGE | F_GAP)) return beside(x, z, near, p, Math.max(off, 2.5));
    if (off <= 1) return near.y - clamp(near.lateral, -p.halfWidth, p.halfWidth) * (p.bank[near.index] ?? 0) - 0.35;
    return beside(x, z, near, p, off);
  };
  cache.set(def.id, track);
  return track;
}

const _near: Near = newNear();
const _cand: Near = newNear();
/**
 * Nearest centre-line point of `path`. With `hint >= 0` only a ±window of
 * samples is searched (racers move < 2 samples per tick); otherwise the whole
 * path. A finite `y` prefers stretches at that height (overpasses, spirals).
 * Writes into `out` (allocation-free in the hot loop).
 */
export function nearest(path: Path, x: number, z: number, hint = -1, out: Near = _near, window = 24, y = NaN): Near {
  const n = path.count, closed = path.closed;
  let from: number, to: number;
  if (hint >= 0) {
    from = hint - window; to = hint + window;
    if (!closed) { from = Math.max(0, from); to = Math.min(n - 2, to); }
  } else { from = 0; to = closed ? n - 1 : n - 2; }
  let best = Infinity, bestXZ = Infinity, bi = 0, bt = 0;
  for (let k = from; k <= to; k++) {
    const i = closed ? ((k % n) + n) % n : k, j = closed ? (i + 1) % n : i + 1;
    const ax = path.px[i], az = path.pz[i], bx = path.px[j] - ax, bz = path.pz[j] - az;
    const l2 = bx * bx + bz * bz || 1;
    const t = clamp(((x - ax) * bx + (z - az) * bz) / l2, 0, 1);
    const dx = x - (ax + bx * t), dz = z - (az + bz * t), xz = dx * dx + dz * dz;
    let d = xz;
    if (y === y) {
      const dy = Math.abs(y - (path.py[i] + (path.py[j] - path.py[i]) * t));
      if (dy > 4) d += (dy - 4) * (dy - 4) * 4;
    }
    if (d < best) { best = d; bestXZ = xz; bi = i; bt = t; }
  }
  const j = closed ? (bi + 1) % n : Math.min(n - 1, bi + 1);
  const cx = path.px[bi] + (path.px[j] - path.px[bi]) * bt, cz = path.pz[bi] + (path.pz[j] - path.pz[bi]) * bt;
  const tx = path.tx[bi], tz = path.tz[bi];
  out.seg = path.index; out.index = bi; out.t = bt;
  out.s = path.dist[bi] + (j === 0 ? path.length - path.dist[bi] : path.dist[j] - path.dist[bi]) * bt;
  // Facing (tx, tz) with yaw 0 = +Z, the left-hand side is (tz, -tx); lateral > 0 = left of travel.
  out.lateral = (x - cx) * tz - (z - cz) * tx;
  out.y = path.py[bi] + (path.py[j] - path.py[bi]) * bt;
  out.tx = tx; out.tz = tz; out.distance = Math.sqrt(bestXZ);
  return out;
}

/**
 * Selection score: how far outside the road edge (0 anywhere on the tarmac, so
 * a racer stays on its path while two roads overlap at a fork) plus a penalty
 * for being far above/below the road.
 */
const score = (n: Near, y: number, hw: number): number => Math.max(0, n.distance - hw) + (y === y ? Math.max(0, Math.abs(y - n.y) - 4) * 2 : 0);
const copyNear = (a: Near, b: Near): Near => {
  b.seg = a.seg; b.index = a.index; b.t = a.t; b.s = a.s; b.lateral = a.lateral; b.y = a.y; b.tx = a.tx; b.tz = a.tz; b.distance = a.distance;
  return b;
};
/** Metres either side of a fork or merge where a racer may switch between paths. */
export const FORK_ZONE = 40;
/**
 * Where a racer is: nearest point on its current path `seg`, switching to a
 * branch (or back to the main loop) inside fork/merge zones when that centre
 * line is clearly closer. `out.seg` names the path.
 */
export function locate(track: Track, x: number, y: number, z: number, seg: number, hint: number, out: Near = _near): Near {
  const paths = track.paths;
  const path = paths[seg] ?? track;
  nearest(path, x, z, hint, out, hint < 0 ? 0 : 24, y);
  if (paths.length === 1) return out;
  let best = score(out, y, path.halfWidth);
  const consider = (cand: Near) => {
    const sc = score(cand, y, paths[cand.seg].halfWidth);
    if (sc + 0.3 < best) { best = sc; copyNear(cand, out); }
  };
  if (path.index === 0) {
    const s = out.s, L = track.length;
    for (let b = 1; b < paths.length; b++) {
      const bp = paths[b];
      const df = circ(s - bp.from, L), dt = circ(s - bp.to, L);
      if (df > -6 && df < FORK_ZONE) consider(nearest(bp, x, z, 10, _cand, 12, y));
      if (dt > -FORK_ZONE && dt < 6) consider(nearest(bp, x, z, bp.count - 12, _cand, 12, y));
    }
  } else {
    const s = out.s;
    // Past either end of a branch the racer is back on the main loop, whatever the scores say.
    if (s <= 0.05 || s >= path.length - 0.05) best = Infinity;
    if (s < FORK_ZONE) consider(nearest(track, x, z, sampleAt(track, path.from), _cand, 22, y));
    if (s > path.length - FORK_ZONE) consider(nearest(track, x, z, sampleAt(track, path.to), _cand, 22, y));
  }
  return out;
}
/** Circular difference into (-L/2, L/2]. */
export function circ(d: number, L: number): number {
  d %= L;
  if (d > L / 2) d -= L;
  else if (d <= -L / 2) d += L;
  return d;
}

/**
 * Lap progress (main-loop distance) of local distance `s` on path `seg`. On a
 * branch it runs metre for metre through the fork and merge zones (where a
 * racer may switch paths, so progress never jumps) and stretches or squeezes
 * linearly in between.
 */
export function progressOf(track: Track, seg: number, s: number): number {
  if (seg === 0) return s;
  const p = track.paths[seg];
  if (!p) return s;
  const L = p.length, span = p.to - p.from, z = Math.min(FORK_ZONE, L / 3, span / 3);
  s = clamp(s, 0, L);
  if (s <= z) return p.from + s;
  if (s >= L - z) return p.to - (L - s);
  return p.from + z + (s - z) / (L - 2 * z) * (span - 2 * z);
}

export interface Spot { seg: number; s: number }
/**
 * Walks `d` metres ahead along a route: on the main loop, every branch whose
 * bit is set in `route` (bit k-1 = branch k) is taken at its fork; a branch
 * always continues onto the main loop at its merge.
 */
export function advance(track: Track, seg: number, s: number, d: number, route: number, out: Spot): Spot {
  let rest = d;
  for (let guard = 0; guard < 4; guard++) {
    if (seg !== 0) {
      const p = track.paths[seg];
      if (s + rest <= p.length) { s += rest; rest = 0; break; }
      rest -= p.length - s; seg = 0; s = p.to;
      continue;
    }
    let fork = -1, gap = Infinity;
    for (let b = 1; b < track.paths.length; b++) {
      if (!(route & (1 << (b - 1)))) continue;
      const g = circ(track.paths[b].from - s, track.length);
      if (g > -FORK_ZONE && g <= rest && g < gap) { gap = g; fork = b; }
    }
    if (fork < 0) { s = (((s + rest) % track.length) + track.length) % track.length; rest = 0; break; }
    // (A racer still counted on the main loop may already be up to FORK_ZONE metres into the branch.)
    rest -= gap; seg = fork; s = 0;
  }
  out.seg = seg; out.s = s;
  return out;
}

/** Point at distance `s` plus lateral offset; writes [x, y, z, yaw]. Open paths clamp `s` to their ends. */
export function pointAt(path: Path, s: number, lateral: number, out: number[] = [0, 0, 0, 0]): number[] {
  const L = path.length, n = path.count;
  let i: number, j: number, t: number;
  if (path.closed) {
    s = ((s % L) + L) % L;
    const f = (s / L) * n;
    i = Math.floor(f) % n; j = (i + 1) % n; t = f - Math.floor(f);
  } else {
    const f = (clamp(s, 0, L) / L) * (n - 1);
    i = Math.min(n - 2, Math.floor(f)); j = i + 1; t = f - i;
  }
  const x = path.px[i] + (path.px[j] - path.px[i]) * t, z = path.pz[i] + (path.pz[j] - path.pz[i]) * t;
  const tx = path.tx[i], tz = path.tz[i];
  out[0] = x + tz * lateral; out[2] = z - tx * lateral;
  out[1] = path.py[i] + (path.py[j] - path.py[i]) * t;
  out[3] = Math.atan2(tx, tz);
  return out;
}
export const sampleAt = (path: Path, s: number): number => {
  const L = path.length;
  if (!path.closed) return clamp(Math.floor((clamp(s, 0, L) / L) * (path.count - 1)), 0, path.count - 1);
  return Math.floor((((s % L) + L) % L) / L * path.count) % path.count;
};
/** Run-up before a launch ramp: rescues never drop a racer here (it could not reach take-off speed). */
export const RUN_UP = 60;
/** Whether local distance `s` lies in the run-up, on the ramp or over the gap of one of the path's launches (or on a drawbridge and its run-up). */
export function inLaunchZone(path: Path, s: number): boolean {
  for (const l of path.features.launches) {
    const d = l.at * path.length - s;
    if (d > -16 && d < RUN_UP) return true;
  }
  for (const d of path.features.drawbridges) if (s > d.from * path.length - RUN_UP && s < d.to * path.length + 8) return true;
  return (path.flags[sampleAt(path, s)] & F_GAP) !== 0;
}
/** Whether lateral offset `lateral` at local distance `s` lies in one of the path's off-road patches. */
export function inOffroad(path: Path, s: number, lateral: number): boolean {
  const frac = s / path.length;
  for (const o of path.features.offroad) {
    if (frac < o.from || frac > o.to) continue;
    if (!o.lanes) return true;
    const lane = lateral / path.halfWidth;
    if (lane >= o.lanes[0] && lane <= o.lanes[1]) return true;
  }
  return false;
}

/** Roller radius / crusher half-size and lift. */
export const ROLLER_RADIUS = 1.7;
export const CRUSHER_HALF = 2.4;
/** Raised crushers clear a kart and stay under a tunnel roof. */
export const CRUSHER_RISE = 3.8;
/**
 * Obstacle pose at race time `time`: writes [x, y, z, k] where k is the
 * crusher's lift above the road (0 = slammed down) or the roller's lateral
 * position (for spin). Pure: the host and every guest agree from the clock.
 */
export function obstacleAt(track: Track, o: Obstacle, time: number, out: number[]): number[] {
  const path = track.paths[o.seg] ?? track;
  const u = ((time / o.period + o.phase) % 1 + 1) % 1;
  if (o.kind === 'roller') {
    const lat = Math.sin(u * Math.PI * 2) * (path.halfWidth - 2);
    pointAt(path, o.s, lat, out);
    out[1] += ROLLER_RADIUS - lat * (path.bank[sampleAt(path, o.s)] ?? 0);
    out[3] = lat;
    return out;
  }
  // Crusher: waits up, slams down fast, rests, rises slowly.
  const lift = u < 0.6 ? CRUSHER_RISE : u < 0.66 ? CRUSHER_RISE * (1 - ((u - 0.6) / 0.06) ** 2) : u < 0.82 ? 0 : CRUSHER_RISE * smoothstep(0.82, 1, u);
  pointAt(path, o.s, o.lane * path.halfWidth * 0.5, out);
  out[3] = lift;
  return out;
}

/** Coins per line, metres between them, and the spacing of lines on the main loop / on branches. */
export const COIN_LINE = 5;
const COIN_GAP = 4, COIN_EVERY_MAIN = 150, COIN_EVERY_BRANCH = 70;
const LANES = [-0.45, 0.45, 0, -0.6, 0.6];
const coinCache = new Map<TrackId, Float64Array>();
/**
 * Coin positions, flat [x, y, z] per coin: lines of five down the road, one
 * every ~150 m of the main loop and every ~70 m of a branch (the riskier
 * routes pay better), clear of the start grid, forks and merges, jumps,
 * drawbridges, half-pipes, item rows and obstacles.
 */
export function coinSpots(track: Track): Float64Array {
  const hit = coinCache.get(track.def.id);
  if (hit) return hit;
  const out: number[] = [];
  const near = newNear(), pt = [0, 0, 0, 0];
  let line = 0;
  for (const p of track.paths) {
    const every = p.index === 0 ? COIN_EVERY_MAIN : COIN_EVERY_BRANCH, span = (COIN_LINE - 1) * COIN_GAP;
    const clear = (s: number): boolean => {
      if (p.index === 0) {
        if (s < 40 || s > p.length - 30) return false;
        for (const b of track.paths) if (b.index > 0 && (Math.abs(circ(s - b.from, p.length)) < FORK_ZONE + 12 || Math.abs(circ(s - b.to, p.length)) < FORK_ZONE + 12)) return false;
      } else if (s < FORK_ZONE + 8 || s > p.length - FORK_ZONE - 8) return false;
      if (inLaunchZone(p, s) || p.flags[sampleAt(p, s)] & (F_GAP | F_DRAW | F_PIPE)) return false;
      if (p.features.itemRows.some(a => Math.abs(a * p.length - s) < 14)) return false;
      if (p.features.ramps.some(a => Math.abs(a * p.length - s) < 12)) return false;
      return !track.obstacles.some(o => o.seg === p.index && Math.abs(o.s - s) < 14);
    };
    for (let s0 = every * 0.6; s0 + span < p.length; s0 += every) {
      let s = -1;
      for (let k = 0; k < 4 && s < 0; k++) { const c = s0 + k * 18; if (clear(c) && clear(c + span)) s = c; }
      if (s < 0) continue;
      const lane = LANES[line++ % LANES.length] * p.halfWidth;
      for (let k = 0; k < COIN_LINE; k++) {
        pointAt(p, s + k * COIN_GAP, lane, pt);
        nearest(p, pt[0], pt[2], sampleAt(p, s + k * COIN_GAP), near, 6);
        out.push(pt[0], track.ground(pt[0], pt[2], near) + 1.0, pt[2]);
      }
    }
  }
  const spots = Float64Array.from(out);
  coinCache.set(track.def.id, spots);
  return spots;
}
