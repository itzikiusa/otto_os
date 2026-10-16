/**
 * Kart tracks: closed centripetal Catmull-Rom centre lines sampled every ~2 m.
 * Pure data + geometry (no three.js) so the simulation and tests share it.
 * Coordinates: metres, Y up. Distance `s` runs from the start line (s = 0).
 */
import { clamp, fbm, hashString, makeNoise, smoothstep } from '../common/math.ts';

export type TrackId = 'meadow' | 'downtown' | 'canyon' | 'frost';
export type EdgeKind = 'grass' | 'wall';
export interface TrackDef {
  id: TrackId; name: string; blurb: string;
  /** [x, z, y] control points, start line at the first point. */
  points: [number, number, number][];
  width: number;
  edge: EdgeKind;
  /** Grass tracks: distance beyond the road edge where a racer is rescued. */
  outOfBounds: number;
  /** Fractions of a lap carrying a row of item boxes. */
  itemRows: number[];
  /** Dash panels: lap fraction + lateral position (-1..1 of half width). */
  boostPads: { at: number; lane: number }[];
  /** Full-width jump ramps (lap fraction). */
  ramps: number[];
  /** Low-grip ranges (lap fractions). */
  ice: [number, number][];
  hills: number;
  theme: 'meadow' | 'city' | 'canyon' | 'frost';
}

export const TRACK_DEFS: Record<TrackId, TrackDef> = {
  meadow: {
    id: 'meadow', name: 'Sunny Meadow', blurb: 'Rolling farmland, a lakeside sweep and a hilltop jump. A friendly first Grand Prix.',
    points: [[0, 0, 0], [100, 0, 0], [150, 20, 1], [170, 60, 3], [150, 100, 5], [110, 110, 6], [80, 140, 7], [90, 185, 8], [140, 200, 7], [200, 190, 5], [250, 210, 4], [270, 260, 4], [230, 300, 3], [150, 300, 2], [60, 290, 3], [-10, 260, 5], [-40, 210, 7], [-20, 160, 8], [-50, 110, 6], [-100, 100, 4], [-130, 60, 2], [-100, 15, 0], [-50, 0, 0]],
    width: 17, edge: 'grass', outOfBounds: 26, itemRows: [0.16, 0.47, 0.76], boostPads: [{ at: 0.31, lane: 0.35 }, { at: 0.62, lane: -0.35 }, { at: 0.9, lane: 0 }],
    ramps: [0.555], ice: [], hills: 9, theme: 'meadow',
  },
  downtown: {
    id: 'downtown', name: 'Downtown Dash', blurb: 'Tight city blocks between towers, barrier-lined corners and a boulevard boost lane.',
    points: [[0, 0, 0], [120, 0, 0], [150, 15, 0], [160, 45, 0], [160, 90, 1], [140, 115, 2], [100, 120, 3], [80, 140, 4], [80, 190, 5], [100, 215, 5], [150, 220, 4], [175, 245, 3], [170, 290, 2], [130, 305, 1], [40, 305, 0], [0, 290, 0], [-20, 260, 0], [-20, 200, 1], [-50, 180, 2], [-90, 175, 2], [-110, 150, 1], [-110, 60, 0], [-90, 20, 0], [-50, 0, 0]],
    width: 16, edge: 'wall', outOfBounds: 0, itemRows: [0.14, 0.43, 0.7], boostPads: [{ at: 0.05, lane: -0.4 }, { at: 0.36, lane: 0.3 }, { at: 0.58, lane: 0 }, { at: 0.86, lane: 0.4 }],
    ramps: [0.27], ice: [], hills: 0, theme: 'city',
  },
  canyon: {
    id: 'canyon', name: 'Canyon Run', blurb: 'Red-rock switchbacks, steep climbs and a big jump over the dry riverbed.',
    points: [[0, 0, 0], [90, -10, 2], [140, 10, 5], [160, 50, 8], [130, 90, 10], [80, 100, 11], [60, 130, 12], [80, 170, 13], [140, 175, 15], [190, 150, 17], [230, 170, 18], [240, 220, 16], [200, 260, 13], [130, 270, 10], [70, 250, 9], [20, 270, 10], [-40, 260, 11], [-70, 220, 9], [-60, 170, 7], [-100, 140, 6], [-110, 90, 4], [-80, 40, 2], [-50, 10, 0]],
    width: 16, edge: 'wall', outOfBounds: 0, itemRows: [0.18, 0.5, 0.77], boostPads: [{ at: 0.33, lane: 0 }, { at: 0.64, lane: 0.4 }, { at: 0.92, lane: -0.3 }],
    ramps: [0.41, 0.84], ice: [], hills: 22, theme: 'canyon',
  },
  frost: {
    id: 'frost', name: 'Frostbite Pass', blurb: 'Snowy pines, an icy lake crossing and a mountain-pass descent. Watch your grip.',
    points: [[0, 0, 0], [110, 0, 1], [160, -30, 3], [220, -30, 5], [260, 0, 7], [270, 50, 9], [240, 90, 11], [190, 100, 12], [160, 130, 12], [170, 180, 13], [220, 200, 14], [250, 240, 12], [220, 280, 9], [150, 290, 6], [80, 270, 4], [40, 230, 3], [0, 240, 3], [-50, 250, 4], [-90, 220, 5], [-90, 170, 4], [-60, 130, 3], [-80, 90, 2], [-90, 40, 1], [-50, 5, 0]],
    width: 17, edge: 'grass', outOfBounds: 24, itemRows: [0.15, 0.44, 0.73], boostPads: [{ at: 0.28, lane: -0.3 }, { at: 0.57, lane: 0.3 }, { at: 0.88, lane: 0 }],
    ramps: [0.66], ice: [[0.47, 0.55]], hills: 16, theme: 'frost',
  },
};
export const TRACK_IDS = Object.keys(TRACK_DEFS) as TrackId[];

export interface Track {
  def: TrackDef;
  count: number;
  length: number;
  /** Per sample: position, unit tangent (xz), cumulative distance. */
  px: Float64Array; py: Float64Array; pz: Float64Array;
  tx: Float64Array; tz: Float64Array; dist: Float64Array;
  /** Signed curvature per sample (rad/m, + = left turn). */
  curve: Float64Array;
  halfWidth: number;
  ground: (x: number, z: number, near: Near) => number;
}
export interface Near { index: number; t: number; s: number; lateral: number; y: number; tx: number; tz: number; distance: number }

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
export function rampHeight(def: TrackDef, length: number, s: number): number {
  for (const at of def.ramps) {
    const d = at * length - s;
    if (d >= 0 && d < RAMP_LENGTH) return RAMP_HEIGHT * (1 - d / RAMP_LENGTH);
  }
  return 0;
}

const cache = new Map<TrackId, Track>();
export function trackFor(id: string): Track {
  const def = TRACK_DEFS[(id in TRACK_DEFS ? id : 'meadow') as TrackId];
  const hit = cache.get(def.id);
  if (hit) return hit;
  // Dense pass, then resample at an even 2 m spacing.
  const pts = def.points.map(([x, z, y]) => [x, y, z]);
  const dense: number[][] = [];
  const out = [0, 0, 0];
  for (let i = 0; i < pts.length; i++) {
    const p0 = pts[(i - 1 + pts.length) % pts.length], p1 = pts[i], p2 = pts[(i + 1) % pts.length], p3 = pts[(i + 2) % pts.length];
    for (let k = 0; k < 60; k++) { catmull(p0, p1, p2, p3, k / 60, out); dense.push([out[0], out[1], out[2]]); }
  }
  const cum = [0];
  for (let i = 1; i <= dense.length; i++) {
    const a = dense[i - 1], b = dense[i % dense.length];
    cum.push(cum[i - 1] + Math.hypot(b[0] - a[0], b[2] - a[2]));
  }
  const length = cum[dense.length];
  const count = Math.round(length / 2);
  const px = new Float64Array(count), py = new Float64Array(count), pz = new Float64Array(count);
  const tx = new Float64Array(count), tz = new Float64Array(count), dist = new Float64Array(count), curve = new Float64Array(count);
  let j = 0;
  for (let i = 0; i < count; i++) {
    const s = (i / count) * length;
    while (cum[j + 1] < s) j++;
    const a = dense[j], b = dense[(j + 1) % dense.length];
    const f = (s - cum[j]) / Math.max(1e-6, cum[j + 1] - cum[j]);
    px[i] = a[0] + (b[0] - a[0]) * f; py[i] = a[1] + (b[1] - a[1]) * f; pz[i] = a[2] + (b[2] - a[2]) * f;
    dist[i] = s;
  }
  for (let i = 0; i < count; i++) {
    const n = (i + 1) % count, p = (i - 1 + count) % count;
    const dx = px[n] - px[p], dz = pz[n] - pz[p], l = Math.hypot(dx, dz) || 1;
    tx[i] = dx / l; tz[i] = dz / l;
  }
  for (let i = 0; i < count; i++) {
    const n = (i + 3) % count, p = (i - 3 + count) % count;
    const a0 = Math.atan2(tx[p], tz[p]), a1 = Math.atan2(tx[n], tz[n]);
    let d = a1 - a0;
    while (d > Math.PI) d -= Math.PI * 2;
    while (d < -Math.PI) d += Math.PI * 2;
    curve[i] = d / (6 * (length / count));
  }
  const halfWidth = def.width / 2;
  const noise = makeNoise(hashString(def.id));
  const track: Track = {
    def, count, length, px, py, pz, tx, tz, dist, curve, halfWidth,
    // Off-road ground: road height near the tarmac, rising into themed hills.
    ground: (x, z, near) => {
      const off = Math.abs(near.lateral) - halfWidth;
      if (off <= 0) return near.y + rampHeight(def, length, near.s);
      if (off <= 1) return near.y;
      const rise = smoothstep(1, 70, off);
      const hill = (fbm(noise, x * 0.012, z * 0.012, 4) * 0.5 + 0.5) * def.hills;
      const wall = def.edge === 'wall' ? smoothstep(3, 14, off) * (def.theme === 'city' ? 0 : 10 + def.hills * 0.6) : 0;
      return near.y + rise * hill + wall;
    },
  };
  cache.set(def.id, track);
  return track;
}

const _near: Near = { index: 0, t: 0, s: 0, lateral: 0, y: 0, tx: 0, tz: 1, distance: 0 };
/**
 * Nearest centre-line point. With `hint >= 0` only a ±window of samples is
 * searched (racers move < 2 samples per tick); otherwise the whole loop.
 * Writes into `out` (allocation-free in the hot loop).
 */
export function nearest(track: Track, x: number, z: number, hint = -1, out: Near = _near, window = 24): Near {
  const n = track.count;
  let best = Infinity, bi = 0, bt = 0;
  const from = hint >= 0 ? hint - window : 0, to = hint >= 0 ? hint + window : n - 1;
  for (let k = from; k <= to; k++) {
    const i = ((k % n) + n) % n, j = (i + 1) % n;
    const ax = track.px[i], az = track.pz[i], bx = track.px[j] - ax, bz = track.pz[j] - az;
    const l2 = bx * bx + bz * bz || 1;
    const t = clamp(((x - ax) * bx + (z - az) * bz) / l2, 0, 1);
    const dx = x - (ax + bx * t), dz = z - (az + bz * t), d = dx * dx + dz * dz;
    if (d < best) { best = d; bi = i; bt = t; }
  }
  const j = (bi + 1) % n;
  const cx = track.px[bi] + (track.px[j] - track.px[bi]) * bt, cz = track.pz[bi] + (track.pz[j] - track.pz[bi]) * bt;
  const tx = track.tx[bi], tz = track.tz[bi];
  out.index = bi; out.t = bt;
  out.s = track.dist[bi] + (j === 0 ? track.length - track.dist[bi] : track.dist[j] - track.dist[bi]) * bt;
  // Facing (tx, tz) with yaw 0 = +Z, the left-hand side is (tz, -tx); lateral > 0 = left of travel.
  out.lateral = (x - cx) * tz - (z - cz) * tx;
  out.y = track.py[bi] + (track.py[j] - track.py[bi]) * bt;
  out.tx = tx; out.tz = tz; out.distance = Math.sqrt(best);
  return out;
}

/** Point at distance `s` plus lateral offset; writes [x, y, z, yaw]. */
export function pointAt(track: Track, s: number, lateral: number, out: number[] = [0, 0, 0, 0]): number[] {
  const L = track.length;
  s = ((s % L) + L) % L;
  const f = (s / L) * track.count;
  const i = Math.floor(f) % track.count, j = (i + 1) % track.count, t = f - Math.floor(f);
  const x = track.px[i] + (track.px[j] - track.px[i]) * t, z = track.pz[i] + (track.pz[j] - track.pz[i]) * t;
  const tx = track.tx[i], tz = track.tz[i];
  out[0] = x + tz * lateral; out[2] = z - tx * lateral;
  out[1] = track.py[i] + (track.py[j] - track.py[i]) * t;
  out[3] = Math.atan2(tx, tz);
  return out;
}
export const sampleAt = (track: Track, s: number): number => {
  const L = track.length;
  return Math.floor((((s % L) + L) % L) / L * track.count) % track.count;
};
