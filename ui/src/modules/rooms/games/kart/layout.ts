/**
 * Kart course layout: where every solid piece of the world goes — barrier
 * walls and rails, bridge deck sides, embankment / ramp / half-pipe skirts,
 * abutments, tunnel tubes, gantries, drawbridge towers, bridge pillars, the
 * fork signs, the themed props (buildings, rocks, trees, grandstands…) and the
 * landscape height. Pure data (no three.js) so the renderer builds meshes from
 * exactly what the tests sweep: every route's road, forks and merges included,
 * must stay clear of all of it (a branch never drives into a wall).
 *
 * `RoadIndex` buckets every route's centre-line segments into a coarse grid,
 * so "is this point on some road, at a height a kart would hit?" costs a few
 * segment checks instead of a scan of every path.
 */
import { clamp, fbm, hashString, makeNoise, Rng, smoothstep } from '../common/math.ts';
import {
  circ, F_BRIDGE, F_DRAW, F_GAP, F_GLASS, F_PIPE, F_RAILS, F_TUNNEL, FORK_ZONE, LAUNCH_LENGTH, nearest, newNear, pipeOn, pointAt, PIPE_ENTRY, PIPE_HEIGHT, RAIL_SHOULDER,
  rampOn, RAMP_LENGTH, sampleAt, type Near, type Path, type Track, type TrackId,
} from './tracks.ts';

export const KART_HEIGHT = 2.2;
/** Barrier walls / rails stand this far out from the road edge (inner face), this thick. */
export const BARRIER_OFFSET = 1.4;
export const BARRIER_THICK = 0.6;
/** Tunnel tubes arch out this far beyond the road edge. */
export const TUBE_PAD = 1.6;
/** Edge furniture stops where it would stand within this distance of another route's tarmac (wide fork mouths). */
export const MOUTH_PAD = 2.4;
/** Solid props keep at least this far off every road's tarmac. */
export const PROP_PAD = 2.0;

// ── Road index ─────────────────────────────────────────────────────────────
const CELL = 12;
const key = (ix: number, iz: number) => (ix + 32768) * 65536 + (iz + 32768);
/** Result of a road query: which path (-1 none), its lateral offset and surface height at the point. */
export interface RoadHit { seg: number; index: number; lateral: number; y: number; edge: number }

export class RoadIndex {
  readonly track: Track;
  private cells = new Map<number, number[]>();
  private reach: number;
  constructor(track: Track) {
    this.track = track;
    let widest = 0;
    for (const p of track.paths) widest = Math.max(widest, p.halfWidth);
    this.reach = widest + RAIL_SHOULDER + MOUTH_PAD + 2;
    for (const p of track.paths) {
      const last = p.closed ? p.count : p.count - 1;
      for (let i = 0; i < last; i++) {
        const j = (i + 1) % p.count, r = p.halfWidth + RAIL_SHOULDER + MOUTH_PAD + 2;
        const x0 = Math.floor((Math.min(p.px[i], p.px[j]) - r) / CELL), x1 = Math.floor((Math.max(p.px[i], p.px[j]) + r) / CELL);
        const z0 = Math.floor((Math.min(p.pz[i], p.pz[j]) - r) / CELL), z1 = Math.floor((Math.max(p.pz[i], p.pz[j]) + r) / CELL);
        for (let ix = x0; ix <= x1; ix++) for (let iz = z0; iz <= z1; iz++) {
          const k = key(ix, iz);
          let list = this.cells.get(k);
          if (!list) { list = []; this.cells.set(k, list); }
          list.push(p.index * 65536 + i);
        }
      }
    }
  }
  /**
   * The road (any route except `skip`) whose tarmac, widened by `pad`, covers
   * (x, z) at a height where a kart on it would touch a solid spanning
   * [y0, y1]; writes it into `out` and returns true. Overpasses well above or
   * roads well below the solid do not count.
   */
  hit(x: number, z: number, y0: number, y1: number, pad: number, skip = -1, out: RoadHit = _hit): boolean {
    const list = this.cells.get(key(Math.floor(x / CELL), Math.floor(z / CELL)));
    if (!list) return false;
    const paths = this.track.paths;
    let best = Infinity;
    for (const code of list) {
      const p = paths[Math.floor(code / 65536)];
      if (p.index === skip) continue;
      const i = code % 65536, j = (i + 1) % p.count;
      const ax = p.px[i], az = p.pz[i], bx = p.px[j] - ax, bz = p.pz[j] - az;
      const l2 = bx * bx + bz * bz || 1;
      const t = clamp(((x - ax) * bx + (z - az) * bz) / l2, 0, 1);
      const dx = x - (ax + bx * t), dz = z - (az + bz * t), d = Math.sqrt(dx * dx + dz * dz);
      const edge = d - p.halfWidth;
      if (edge > pad || edge >= best) continue;
      // Ignore the far end of a segment that only grazes the point sideways (its projection clamps).
      const lateral = dx * p.tz[i] - dz * p.tx[i];
      const y = p.py[i] + (p.py[j] - p.py[i]) * t - clamp(lateral, -p.halfWidth, p.halfWidth) * p.bank[i] + rampOn(p, p.dist[i]);
      // A kart on that road spans [y, y + KART_HEIGHT].
      if (y >= y1 || y + KART_HEIGHT <= y0) continue;
      // Over a gap or an open drawbridge there is no road to drive into the solid from.
      if (p.flags[i] & F_GAP) continue;
      best = edge;
      out.seg = p.index; out.index = i; out.lateral = lateral; out.y = y; out.edge = edge;
    }
    return best < Infinity;
  }
  /** Edge distance in plan from (x, z) to the nearest tarmac of any route but `skip` within reach (negative on it; Infinity beyond reach). */
  gap(x: number, z: number, skip = -1): number {
    const list = this.cells.get(key(Math.floor(x / CELL), Math.floor(z / CELL)));
    if (!list) return Infinity;
    const paths = this.track.paths;
    let best = Infinity;
    for (const code of list) {
      const p = paths[Math.floor(code / 65536)];
      if (p.index === skip) continue;
      const i = code % 65536, j = (i + 1) % p.count;
      const ax = p.px[i], az = p.pz[i], bx = p.px[j] - ax, bz = p.pz[j] - az;
      const l2 = bx * bx + bz * bz || 1;
      const t = clamp(((x - ax) * bx + (z - az) * bz) / l2, 0, 1);
      const d = Math.hypot(x - (ax + bx * t), z - (az + bz * t)) - p.halfWidth;
      if (d < best) best = d;
    }
    return best <= this.reach ? best : Infinity;
  }
}
const _hit: RoadHit = { seg: -1, index: 0, lateral: 0, y: 0, edge: 0 };
const indexCache = new Map<TrackId, RoadIndex>();
export function roadIndex(track: Track): RoadIndex {
  let idx = indexCache.get(track.def.id);
  if (!idx) { idx = new RoadIndex(track); indexCache.set(track.def.id, idx); }
  return idx;
}

// ── Landscape ──────────────────────────────────────────────────────────────
const _tn: Near = newNear();
const _levels = new Int32Array(6), _levelDist = new Float64Array(6);
/**
 * Landscape height at (x, z) over every route: the lowest road bed when the
 * point is on a road (under an overpass the land meets the lower road),
 * otherwise each nearby road's own off-road ground blended by closeness, so
 * the land near a road is what the simulation drives on and slopes between
 * roads at different heights.
 */
export function terrainHeight(track: Track, x: number, z: number): number {
  // `cap`: beside a road at ground level the land may rise no faster than 1:1 from a metre past its
  // edge, whatever another road's ground says (a bridge crossing overhead, a neighbouring route), so
  // the 4 m landscape grid never folds up across the tarmac.
  let road = Infinity, sum = 0, weight = 0, far = 0, farD = Infinity, cap = Infinity;
  const levels = _levels, dists = _levelDist;
  for (const p of track.paths) {
    // Nearest sample per stretch within reach: stacked or neighbouring stretches of one
    // path (spirals, a road passing under itself, hairpins) are separate candidates.
    let bd = Infinity, bi = 0, nl = 0;
    const reach = (p.halfWidth + 40) ** 2;
    for (let i = 0; i < p.count; i += 5) {
      const d = (p.px[i] - x) ** 2 + (p.pz[i] - z) ** 2;
      if (d < bd) { bd = d; bi = i; }
      if (d > reach) continue;
      let slot = -1;
      for (let k = 0; k < nl; k++) {
        const gap = Math.abs(i - levels[k]);
        if (Math.abs(p.py[i] - p.py[levels[k]]) < 4 && Math.min(gap, p.closed ? p.count - gap : gap) < 40) { slot = k; break; }
      }
      if (slot < 0) { if (nl < levels.length) { levels[nl] = i; dists[nl] = d; nl++; } }
      else if (d < dists[slot]) { levels[slot] = i; dists[slot] = d; }
    }
    if (nl === 0) { levels[0] = bi; nl = 1; }
    for (let k = 0; k < nl; k++) {
      const n = nearest(p, x, z, levels[k], _tn, 8);
      const g = track.terrain(x, z, n);
      const off = Math.abs(n.lateral) - p.halfWidth;
      const ground = !(p.flags[n.index] & (F_BRIDGE | F_GAP));
      if (off <= 1 && ground) road = Math.min(road, g);
      if (ground && off < 8) cap = Math.min(cap, n.y - Math.sign(n.lateral) * p.halfWidth * p.bank[n.index] - 0.35 + Math.max(0, off - 1));
      // Under a deck the land stays below its slab (a boardwalk over a cove never sinks into a hillside).
      else if (!ground && off < 8) cap = Math.min(cap, n.y - 1.6 + Math.max(0, off - RAIL_SHOULDER - 0.5));
      if (n.distance < p.halfWidth + 90) { const w = 1 / Math.max(1, off) ** 3; sum += g * w; weight += w; }
      if (n.distance < farD) { farD = n.distance; far = g; }
    }
  }
  return Math.min(cap, road < Infinity ? road : weight > 0 ? sum / weight : far);
}

/** Far-field shaping per theme (mountains rise, lakes and seas fall away) beyond ~60 m from every road. */
const FAR: Record<string, number> = { meadow: -10, city: 0, canyon: 34, frost: 30, lagoon: -14, volcano: -16, dusk: 0 };
const noiseCache = new Map<TrackId, (x: number, z: number) => number>();
/** Distance in plan from (x, z) to the nearest road edge of any route (coarse: every third sample). */
export function roadDistance(track: Track, x: number, z: number): number {
  let best = Infinity;
  for (const p of track.paths) for (let i = 0; i < p.count; i += 3) {
    const e = Math.sqrt((p.px[i] - x) ** 2 + (p.pz[i] - z) ** 2) - p.halfWidth;
    if (e < best) best = e;
  }
  return best;
}
/**
 * Height of the rendered landscape mesh at a grid vertex: the terrain plus the
 * far-field shaping, measured from the nearest road of ANY route (a branch far
 * from the main loop is never buried under a far-field mountain).
 */
export function worldHeight(track: Track, x: number, z: number): number {
  let noise = noiseCache.get(track.def.id);
  if (!noise) { noise = makeNoise(hashString(track.def.id) ^ 0x51ed); noiseCache.set(track.def.id, noise); }
  const off = roadDistance(track, x, z);
  return terrainHeight(track, x, z) + smoothstep(60, 240, off) * FAR[track.def.theme] * (0.55 + 0.45 * fbm(noise, x * 0.006, z * 0.006, 3));
}
/** The landscape mesh grid: origin and spacing (the renderer and the terrain audit share it). */
export function terrainGrid(track: Track): { x0: number; z0: number; nx: number; nz: number; step: number } {
  let minX = Infinity, maxX = -Infinity, minZ = Infinity, maxZ = -Infinity;
  for (const p of track.paths) for (let i = 0; i < p.count; i++) {
    minX = Math.min(minX, p.px[i]); maxX = Math.max(maxX, p.px[i]);
    minZ = Math.min(minZ, p.pz[i]); maxZ = Math.max(maxZ, p.pz[i]);
  }
  const margin = 260, step = 4;
  return { x0: minX - margin, z0: minZ - margin, nx: Math.ceil((maxX - minX + margin * 2) / step) + 1, nz: Math.ceil((maxZ - minZ + margin * 2) / step) + 1, step };
}

// ── Edge furniture ─────────────────────────────────────────────────────────
const _pt = [0, 0, 0, 0];
const _h: RoadHit = { seg: -1, index: 0, lateral: 0, y: 0, edge: 0 };
/**
 * Per sample and side (1 = build): edge furniture (curbs, walls, rails, skirts)
 * stops wherever it would stand on or right beside another route's tarmac —
 * the mouths of forks and merges stay wide open — plus a sample either side.
 */
const clearCache = new Map<string, { left: Uint8Array; right: Uint8Array }>();
export function edgeClear(track: Track, path: Path): { left: Uint8Array; right: Uint8Array } {
  const cacheKey = `${track.def.id}/${path.index}`;
  const hit = clearCache.get(cacheKey);
  if (hit) return hit;
  const idx = roadIndex(track), hw = path.halfWidth, n = path.count;
  const left = new Uint8Array(n).fill(1), right = new Uint8Array(n).fill(1);
  clearCache.set(cacheKey, { left, right });
  if (track.paths.length < 2) return { left, right };
  for (const [side, mask] of [[1, left], [-1, right]] as [number, Uint8Array][]) {
    const blocked = new Uint8Array(n);
    for (let i = 0; i < n; i++) {
      const y = path.py[i];
      for (const lat of [hw + 0.3, hw + BARRIER_OFFSET, hw + BARRIER_OFFSET + BARRIER_THICK, hw + TUBE_PAD]) {
        pointAt(path, path.dist[i], side * lat, _pt);
        if (idx.hit(_pt[0], _pt[2], y - 1.5 - side * lat * path.bank[i], y + 3.5 - side * lat * path.bank[i], MOUTH_PAD, path.index, _h)) { blocked[i] = 1; break; }
      }
    }
    for (let i = 0; i < n; i++) {
      if (!blocked[i]) continue;
      for (let k = -2; k <= 2; k++) {
        const j = path.closed ? (i + k + n) % n : i + k;
        if (j >= 0 && j < n) mask[j] = 0;
      }
    }
  }
  return { left, right };
}

export type BarrierKind = 'wall' | 'rail' | 'deck' | 'pipe';
/** A wall-like ribbon along a path at `lateral` (thickness outward on `side`), heights relative to the road, where `mask` is 1. */
export interface Barrier { kind: BarrierKind; side: number; lateral: number; top: number; bottom: number; thick: number; mask: Uint8Array }
const masked = (n: number, f: (i: number) => boolean): Uint8Array => { const m = new Uint8Array(n); for (let i = 0; i < n; i++) m[i] = f(i) ? 1 : 0; return m; };
/** Every barrier of a path: walls on walled roads, rails on railed decks, deck sides under bridges, rails along half-pipe copings. */
export function barriers(track: Track, path: Path): Barrier[] {
  const hw = path.halfWidth, n = path.count, out: Barrier[] = [];
  const flag = (i: number, f: number) => (path.flags[i] & f) !== 0;
  const paved = (i: number) => !flag(i, F_GAP | F_DRAW | F_PIPE);
  const { left, right } = edgeClear(track, path);
  const clear = (side: number, i: number) => (side > 0 ? left[i] : right[i]) === 1;
  for (const side of [-1, 1]) {
    if (path.edge === 'grass') {
      const mask = masked(n, i => flag(i, F_RAILS) && !flag(i, F_TUNNEL) && paved(i) && clear(side, i));
      if (mask.some(v => v)) out.push({ kind: 'rail', side, lateral: side * (hw + BARRIER_OFFSET), top: 1.0, bottom: -0.4, thick: BARRIER_THICK, mask });
    } else {
      const h = track.def.theme === 'city' || track.def.theme === 'dusk' ? 1.0 : 1.3;
      out.push({ kind: 'wall', side, lateral: side * (hw + BARRIER_OFFSET), top: h, bottom: -0.4, thick: BARRIER_THICK, mask: masked(n, i => paved(i) && !flag(i, F_TUNNEL) && clear(side, i)) });
    }
    // Bridge deck sides (the slab edge under the rails).
    const deck = masked(n, i => flag(i, F_BRIDGE) && !flag(i, F_GAP | F_DRAW) && clear(side, i));
    if (deck.some(v => v)) out.push({ kind: 'deck', side, lateral: side * (hw + RAIL_SHOULDER), top: 0.05, bottom: -1.2, thick: 0.01, mask: deck });
    // Rails along each half-pipe's coping once its walls are up.
    for (const [f, t] of path.features.halfpipes) {
      const a = f * path.length, b = t * path.length;
      out.push({ kind: 'pipe', side, lateral: side * (hw + RAIL_SHOULDER - 0.2), top: PIPE_HEIGHT + 1.1, bottom: PIPE_HEIGHT, thick: 0.4, mask: masked(n, i => path.dist[i] >= a + PIPE_ENTRY && path.dist[i] <= b) });
    }
  }
  return out;
}

export type SkirtKind = 'embankment' | 'ramp' | 'pipe';
/** A vertical ribbon at `lateral` from `bottom[i]` to `top[i]` (relative to the centre line, banking applied) where `mask` is 1. */
export interface Skirt { kind: SkirtKind; lateral: number; mask: Uint8Array; top: Float32Array; bottom: Float32Array; glide?: boolean }
/**
 * Solid faces beside the road: embankments where the land falls away before
 * bridges and jumps, the sides under kicker and launch ramps, and the outer
 * skirts of half-pipes — never across another route's mouth.
 */
const skirtCache = new Map<string, Skirt[]>();
export function skirts(track: Track, path: Path): Skirt[] {
  const cacheKey = `${track.def.id}/${path.index}`;
  const cached = skirtCache.get(cacheKey);
  if (cached) return cached;
  const hw = path.halfWidth, n = path.count, out: Skirt[] = [];
  skirtCache.set(cacheKey, out);
  const flag = (i: number, f: number) => (path.flags[i] & f) !== 0;
  const { left, right } = edgeClear(track, path);
  const clear = (side: number, i: number) => (side > 0 ? left[i] : right[i]) === 1;
  const solid = (i: number) => path.drop[i] > 0.05 && !flag(i, F_BRIDGE | F_GAP | F_TUNNEL);
  for (const side of [-1, 1]) {
    const mask = masked(n, i => solid(i) && clear(side, i));
    if (mask.some(v => v)) {
      const top = new Float32Array(n), bottom = new Float32Array(n);
      for (let i = 0; i < n; i++) {
        if (!mask[i]) continue;
        const lat = side * (hw + (flag(i, F_RAILS) ? RAIL_SHOULDER : 0.6) + 2.5);
        pointAt(path, path.dist[i], lat, _pt);
        const land = terrainHeight(track, _pt[0], _pt[2]) - (path.py[i] - lat * path.bank[i]);
        bottom[i] = Math.min(-0.5, land - 0.8);
        top[i] = 0.04 + (flag(i, F_PIPE) ? 0 : rampOn(path, path.dist[i]));
      }
      out.push({ kind: 'embankment', lateral: side * (hw + 0.6), mask, top, bottom });
    }
    // Ramp sides: kicker ramps and launch ramps.
    const ramps: [number, number, boolean][] = [
      ...path.features.ramps.map((at): [number, number, boolean] => [at * path.length - RAMP_LENGTH, at * path.length, false]),
      ...path.features.launches.map((l): [number, number, boolean] => [l.at * path.length - LAUNCH_LENGTH, l.at * path.length, l.glide === true]),
    ];
    for (const [s0, s1, glide] of ramps) {
      const rmask = masked(n, i => path.dist[i] >= s0 - 2 && path.dist[i] <= s1 + 0.5 && clear(side, i));
      const top = new Float32Array(n), bottom = new Float32Array(n).fill(-0.6);
      for (let i = 0; i < n; i++) top[i] = 0.08 + rampOn(path, Math.min(path.dist[i], s1 - 1e-6));
      out.push({ kind: 'ramp', lateral: side * (hw + 0.02), mask: rmask, top, bottom, glide });
    }
    for (const [f, t] of path.features.halfpipes) {
      const a = f * path.length, b = t * path.length;
      const top = new Float32Array(n), bottom = new Float32Array(n).fill(-1.2);
      for (let i = 0; i < n; i++) top[i] = pipeOn(path, path.dist[i], hw) + 0.07;
      out.push({ kind: 'pipe', lateral: side * (hw + RAIL_SHOULDER), mask: masked(n, i => path.dist[i] >= a && path.dist[i] <= b && clear(side, i)), top, bottom });
    }
  }
  return out;
}

/** Whether a road surface (lanes and shoulder) lies under (x, z) more than 2 m below `y`. */
export function roadBelow(track: Track, x: number, z: number, y: number): boolean {
  for (const p of track.paths) for (let i = 0; i < p.count; i += 2) {
    if (p.py[i] > y - 2) continue;
    const dx = p.px[i] - x, dz = p.pz[i] - z, r = p.halfWidth + 1.4;
    if (dx * dx + dz * dz < r * r) return true;
  }
  return false;
}
/**
 * Abutment faces across the road wherever a deck or a gap begins or ends,
 * flat [ax, az, bx, bz, floor, top] per face: from below the land (or 3 m
 * above a road passing underneath) up to the road surface.
 */
export function abutments(track: Track, path: Path): number[] {
  const hw = path.halfWidth, n = path.count, last = path.closed ? n : n - 1, out: number[] = [], idx = roadIndex(track);
  const flag = (i: number, f: number) => (path.flags[i] & f) !== 0;
  const deck = (i: number) => flag(i, F_BRIDGE | F_GAP);
  for (let i = 1; i < last; i++) {
    const a = (i - 1 + n) % n, b = i % n;
    if (deck(a) === deck(b)) continue;
    const k = deck(a) ? b : a;
    const roadY = path.py[k], top = roadY + 0.05 + rampOn(path, path.dist[k]);
    const floor = roadBelow(track, path.px[k], path.pz[k], roadY) ? roadY - 3 : roadY - 18;
    const w = hw + (flag(k, F_RAILS) ? RAIL_SHOULDER : 0.6), lx = path.tz[k] * w, lz = -path.tx[k] * w;
    // No face where it would stand across another route's tarmac (a deck that starts inside a fork mouth).
    let across = false;
    for (let f = 0; f <= 1 && !across; f += 0.1) across = idx.hit(path.px[k] - lx + 2 * lx * f, path.pz[k] - lz + 2 * lz * f, floor, top - 0.15, 0.3, path.index, _h);
    if (across) continue;
    out.push(path.px[k] - lx, path.pz[k] - lz, path.px[k] + lx, path.pz[k] + lz, floor, top);
  }
  return out;
}

/** Tunnel tube mask of a path (rock or glass): the arch spans ±(halfWidth + TUBE_PAD) up to TUNNEL_HEIGHT. */
export function tubeMask(path: Path, glass: boolean): Uint8Array {
  return masked(path.count, i => (path.flags[i] & F_TUNNEL) !== 0 && ((path.flags[i] & F_GLASS) !== 0) === glass);
}

// ── Point furniture: gantries, towers, pillars, fork signs ─────────────────
/** Overhead banner across the road; `posts` are [x, y, z] feet (none when it hangs from a tunnel roof). */
export interface Gantry { path: number; s: number; x: number; y: number; z: number; yaw: number; width: number; text: string; color: string; inside: boolean; posts: number[][] }
export const GANTRY_POST = 1.2;
/** Half-size and height of a gantry post. */
export const POST_HALF = 0.25, POST_HEIGHT = 7.2;
function gantryAt(track: Track, path: Path, s: number, text: string, color: string): Gantry | null {
  // Never inside a tunnel (the banner would cut through its roof): back up to open road first,
  // or, in a long tunnel, hang the banner from the roof instead. Posts never stand on another road.
  const idx = roadIndex(track), hw = path.halfWidth;
  const roofed = (at: number) => [-3, 0, 3].some(d => (path.flags[sampleAt(path, path.closed ? circ(at + d, path.length) : at + d)] & F_TUNNEL) !== 0);
  const postsClear = (at: number) => [-1, 1].every(side => {
    pointAt(path, at, side * (hw + GANTRY_POST), _pt);
    return !idx.hit(_pt[0], _pt[2], _pt[1] - 1, _pt[1] + POST_HEIGHT, PROP_PAD, path.index, _h) && idx.gap(_pt[0], _pt[2], path.index) > POST_HALF + 0.6;
  });
  let at = s;
  for (let back = 0; back <= 120 && roofed(at); back += 4) at -= 4;
  if (!roofed(at)) {
    // Step back (then forward) until both posts stand clear of every other route.
    let found = NaN;
    for (let k = 0; k <= 30 && found !== found; k++) for (const d of [-k * 4, k * 4]) {
      const c = at + d;
      if (found === found || roofed(c) || (!path.closed && (c < 4 || c > path.length - 4))) continue;
      if (postsClear(c)) found = c;
    }
    if (found !== found) return null;
    at = found;
  }
  if (path.closed) at = circ(at, path.length);
  if (!path.closed && (at < 4 || at > path.length - 4)) return null;
  const inside = roofed(at);
  pointAt(path, at, 0, _pt);
  const g: Gantry = { path: path.index, s: at, x: _pt[0], y: _pt[1], z: _pt[2], yaw: _pt[3], width: hw * 1.4, text, color, inside, posts: [] };
  if (!inside) for (const side of [-1, 1]) { pointAt(path, at, side * (hw + GANTRY_POST), _pt); g.posts.push([_pt[0], _pt[1], _pt[2]]); }
  return g;
}
/** Sign text for a branch: its name and what kind of route it is. */
export function routeLabel(path: Path): string {
  return path.boost ? `${path.name} · mushroom!` : path.shortcut ? `${path.name} · shortcut` : path.name;
}
/** Every gantry of the track: jump and glider banners, drawbridge and half-pipe warnings, and a fork arrow ~70 m before each branch. */
export function gantries(track: Track): Gantry[] {
  const out: Gantry[] = [];
  const push = (g: Gantry | null) => { if (g) out.push(g); };
  for (const path of track.paths) {
    for (const l of path.features.launches) push(gantryAt(track, path, l.at * path.length - LAUNCH_LENGTH - 6, l.glide ? 'GLIDER' : 'JUMP', l.glide ? '#38b6ff' : '#ffcc00'));
    for (const d of path.features.drawbridges) push(gantryAt(track, path, d.from * path.length - 40, 'DRAWBRIDGE', '#ff9a2e'));
    for (const [a] of path.features.halfpipes) push(gantryAt(track, path, a * path.length - 12, 'HALF-PIPE', '#9ff6ff'));
    if (path.index > 0) {
      // On the main road before the fork: an arrow naming the branch, on the side it leaves.
      pointAt(path, Math.min(FORK_ZONE, path.length / 3), 0, _pt);
      const n = nearest(track, _pt[0], _pt[2], sampleAt(track, path.from + FORK_ZONE), _tn, 30);
      const text = n.lateral > 0 ? `◀ ${routeLabel(path)}` : `${routeLabel(path)} ▶`;
      push(gantryAt(track, track, path.from - 70, text, path.boost ? '#ff9a2e' : path.shortcut ? '#ffcc00' : '#ffffff'));
    }
  }
  return out;
}

/** The route sign in the gore where a branch splits off, facing the oncoming racers: post foot and sign yaw. */
export interface ForkSign { path: number; x: number; y: number; z: number; yaw: number; text: string; color: string }
export function forkSign(track: Track, path: Path): ForkSign | null {
  if (path.index === 0) return null;
  const idx = roadIndex(track);
  for (let s = 24; s < Math.min(160, path.length / 2); s += 4) {
    pointAt(path, s, 0, _pt);
    const n = nearest(track, _pt[0], _pt[2], sampleAt(track, path.from + s), _tn, 30);
    if (n.distance < path.halfWidth + track.halfWidth + 5) continue;
    const bx = _pt[0], bz = _pt[2], yaw = _pt[3];
    const mx = bx - n.lateral * n.tz, mz = bz + n.lateral * n.tx; // closest main-road point
    const x = (bx + mx) / 2, z = (bz + mz) / 2;
    // In the gore, clear of every road (another branch may run alongside).
    if (idx.gap(x, z) < 2) continue;
    return { path: path.index, x, y: terrainHeight(track, x, z), z, yaw, text: routeLabel(path), color: path.shortcut ? '#ffcc00' : '#ffffff' };
  }
  return null;
}

/** Bascule towers either side of each drawbridge hinge: [x, y, z] of each tower's centre line at road level. */
export const TOWER_HALF = 1.2, TOWER_HEIGHT = 9;
export function towers(path: Path): number[][] {
  const out: number[][] = [], W = path.halfWidth + RAIL_SHOULDER;
  for (const d of path.features.drawbridges) {
    const a = d.from * path.length, b = d.to * path.length;
    for (const [s, flip] of [[a, false], [b, true]] as [number, boolean][]) for (const side of [-1, 1]) {
      pointAt(path, s + (flip ? 2 : -2), side * (W + 1.6), _pt);
      out.push([_pt[0], _pt[1], _pt[2]]);
    }
  }
  return out;
}

/** Bridge pillars just outside the deck, down to the land (never onto a road below): flat [x, base, z, height]. */
export function pillars(track: Track, path: Path): number[] {
  const hw = path.halfWidth, spots: number[] = [];
  const bridge = (i: number) => (path.flags[i] & F_BRIDGE) !== 0 && !(path.flags[i] & (F_GAP | F_DRAW));
  const idx = roadIndex(track);
  for (let i = 0; i < path.count; i += 8) {
    if (!bridge(i)) continue;
    for (const side of [-1, 1]) {
      const lat = side * (hw + RAIL_SHOULDER + 0.6);
      pointAt(path, path.dist[i], lat, _pt);
      const top = path.py[i] - lat * path.bank[i] - 1.2;
      const base = terrainHeight(track, _pt[0], _pt[2]);
      if (top - base < 2.5 || roadBelow(track, _pt[0], _pt[2], top)) continue;
      if (idx.hit(_pt[0], _pt[2], base - 1, top, PROP_PAD, path.index, _h)) continue;
      spots.push(_pt[0], base - 1, _pt[2], top - base + 1);
    }
  }
  return spots;
}

// ── Props ──────────────────────────────────────────────────────────────────
/**
 * Footprints of the kit models placed beside the road, in model units:
 * [half width (x), half depth (z), height]. Measured from the kit's bounds
 * (a unit test checks them against the shipped kart-kit.glb).
 */
export const PROP_BOXES: Record<string, [number, number, number]> = {
  'race-grandStand': [0.5, 0.5, 0.9], 'race-grandStandCovered': [0.5, 0.51, 1.19], 'race-tentLong': [1, 0.5, 0.7], 'race-pitsGarage': [0.5, 0.55, 0.7], 'race-pitsOffice': [0.5, 0.52, 0.51],
  'race-flagCheckers': [0.1, 0.02, 1.25], 'race-flagRed': [0.1, 0.02, 1.25], 'race-flagGreen': [0.1, 0.02, 1.25], 'race-billboard': [0.5, 0.24, 1], 'race-billboardLow': [0.5, 0.24, 0.69],
  'race-lightPostLarge': [0.19, 0.05, 0.8], 'race-treeLarge': [0.18, 0.21, 1.51],
  'nat-tree_default': [0.38, 0.33, 1.71], 'nat-tree_oak': [0.32, 0.37, 1.23], 'nat-tree_detailed': [0.42, 0.38, 1.33], 'nat-tree_fat': [0.38, 0.33, 1.15],
  'nat-tree_palmTall': [0.52, 0.52, 1.36], 'nat-tree_palmBend': [0.47, 0.51, 1.38], 'nat-cactus_tall': [0.2, 0.17, 0.75], 'nat-cactus_short': [0.17, 0.15, 0.53],
  'nat-rock_largeA': [0.39, 0.51, 0.26], 'nat-rock_largeC': [0.53, 0.51, 0.32], 'nat-rock_tallB': [0.38, 0.39, 0.88], 'nat-rock_tallE': [0.23, 0.26, 0.44], 'nat-rock_tallG': [0.21, 0.25, 0.78],
  'nat-cliff_large_rock': [0.5, 0.21, 1], 'nat-cliff_block_rock': [0.5, 0.5, 1], 'nat-fence_simple': [0.5, 0.04, 0.35], 'nat-log_stack': [0.21, 0.35, 0.35], 'nat-tent_detailedOpen': [0.44, 0.34, 0.56],
  'nat-stump_old': [0.18, 0.19, 0.27], 'nat-plant_bushLarge': [0.19, 0.17, 0.24], 'nat-plant_bush': [0.2, 0.2, 0.24], 'nat-flower_redA': [0.08, 0.09, 0.29], 'nat-flower_yellowB': [0.11, 0.12, 0.16],
  'nat-flower_purpleA': [0.08, 0.09, 0.24], 'nat-mushroom_redGroup': [0.14, 0.13, 0.25],
  'plat-tree-pine-snow': [0.47, 0.47, 2], 'plat-tree-snow': [0.55, 0.56, 1.93], 'plat-tree-pine-snow-small': [0.33, 0.33, 1.4], 'plat-rocks': [0.33, 0.33, 0.4], 'plat-stones': [0.36, 0.4, 0.05],
  'plat-block-snow-large': [1.04, 1.04, 1],
  'city-building-a': [0.44, 0.47, 1.29], 'city-building-b': [0.49, 0.47, 1.29], 'city-building-c': [0.44, 0.55, 0.89], 'city-building-d': [0.42, 0.45, 1.29], 'city-building-e': [0.82, 0.5, 0.89],
  'city-building-f': [0.42, 0.51, 1.69], 'city-building-g': [0.49, 0.46, 1.69], 'city-building-h': [0.44, 0.5, 1.29], 'city-building-i': [0.62, 0.65, 1.68], 'city-building-j': [1.04, 0.67, 1.69],
  'city-building-k': [1.04, 0.47, 1.47], 'city-building-l': [0.68, 0.7, 2.27], 'city-building-skyscraper-a': [0.68, 0.68, 2.88], 'city-building-skyscraper-b': [0.68, 0.68, 4.48],
  'city-building-skyscraper-c': [0.64, 0.69, 4.08], 'city-building-skyscraper-d': [0.64, 0.69, 5.47], 'city-detail-awning': [0.2, 0.08, 0.4], 'city-detail-parasol-a': [0.17, 0.2, 0.45],
  'house-tree-large': [0.11, 0.12, 0.77], 'house-tree-small': [0.11, 0.12, 0.57],
};
/** Ground cover a kart brushes through (never casts shadows, may sit closer to the road). */
export const DECOR = /grass|flower|bush|plant_|stones|mushroom/;
/** One prop: kit model `key` with its footprint centred on (x, z), base at y. `sy` stretches it vertically. */
export interface PropPlacement { key: string; x: number; y: number; z: number; yaw: number; scale: number; sy: number }

/** Whether a prop's footprint (rotated by yaw, scaled, widened by `pad`) stands on any road at a height a kart would hit. */
export function propBlocksRoad(track: Track, p: PropPlacement, pad: number): boolean {
  const box = PROP_BOXES[p.key];
  if (!box) return false;
  const idx = roadIndex(track);
  const hx = box[0] * p.scale, hz = box[1] * p.scale, top = p.y + box[2] * p.sy;
  // Quick reject: the whole footprint (plus pad) is beyond reach of every road.
  const radius = Math.hypot(hx, hz);
  const c = Math.cos(p.yaw), s = Math.sin(p.yaw);
  const nx = Math.max(1, Math.ceil((hx * 2) / 2.5)), nz = Math.max(1, Math.ceil((hz * 2) / 2.5));
  if (idx.gap(p.x, p.z) > radius + pad + 1) return false;
  for (let a = 0; a <= nx; a++) for (let b = 0; b <= nz; b++) {
    const lx = -hx + (2 * hx * a) / nx, lz = -hz + (2 * hz * b) / nz;
    // Local x / z rotated by yaw about Y (three.js convention).
    const x = p.x + lx * c + lz * s, z = p.z - lx * s + lz * c;
    if (idx.hit(x, z, p.y - 0.5, top, pad, -1, _h)) return true;
  }
  return false;
}

const propCache = new Map<TrackId, PropPlacement[]>();
/**
 * Every themed prop of the track (deterministic per track): grandstands and
 * pits at the start, billboards and flags on corners, then the theme's
 * scenery. A placement whose footprint would stand on any route's road —
 * including a branch running beside the main loop — is dropped.
 */
export function propLayout(track: Track): PropPlacement[] {
  const hit = propCache.get(track.def.id);
  if (hit) return hit;
  const out: PropPlacement[] = [];
  const rng = new Rng(hashString(track.def.id) ^ 0x2f1);
  const hw = track.halfWidth, liquid = track.liquid;
  const place = (p: PropPlacement): void => {
    if (propBlocksRoad(track, p, DECOR.test(p.key) ? 0.5 : PROP_PAD)) return;
    out.push(p);
  };
  /** Place along a path at lateral distance (beyond the road edge), on dry land clear of every road. */
  const beside = (path: Path, key: string, s: number, side: number, dist: number, scale: number, faceRoad = true, jitterYaw = 0) => {
    pointAt(path, s, side * (path.halfWidth + dist), _pt);
    const x = _pt[0], z = _pt[2], roadYaw = _pt[3];
    const y = terrainHeight(track, x, z);
    if (liquid !== null && y < liquid + 0.4) return;
    const yaw = (faceRoad ? roadYaw + (side > 0 ? -Math.PI / 2 : Math.PI / 2) : rng.range(0, Math.PI * 2)) + rng.range(-jitterYaw, jitterYaw);
    place({ key, x, y: y - 0.05, z, yaw, scale, sy: scale });
  };
  const scatter = (keys: string[], count: number, minD: number, maxD: number, scale: [number, number]) => {
    for (let n = 0; n < count; n++) {
      const path = track.paths.length > 1 && rng.chance(0.15) ? track.paths[1 + rng.int(track.paths.length - 1)] : track;
      const s = rng.range(0, path.length), side = rng.chance(0.5) ? 1 : -1, d = rng.range(minD, maxD);
      beside(path, rng.pick(keys), s, side, d, rng.range(scale[0], scale[1]), false);
    }
  };
  // Solid structures stay beyond the reachable area: outside the walls, or
  // past the rescue line on grass tracks (karts never drive through them).
  const gap = track.def.edge === 'grass' ? track.def.outOfBounds + 2 : 3;
  const t = track.def.theme;
  const urban = t === 'city' || t === 'dusk';
  // Start area: grandstands, banner towers, flags, tents.
  for (let s = -70; s <= 70; s += 12) {
    beside(track, urban ? 'race-grandStandCovered' : 'race-grandStand', s, 1, gap + 6, 11);
    if (Math.abs(s) > 20) beside(track, 'race-tentLong', s, -1, gap + 3, 6);
  }
  beside(track, 'race-pitsGarage', -24, -1, gap + 5, 10); beside(track, 'race-pitsOffice', 0, -1, gap + 5, 10); beside(track, 'race-pitsGarage', 24, -1, gap + 5, 10);
  for (let s = -90; s < 110; s += 30) { beside(track, 'race-flagCheckers', s, -1, gap, 5); beside(track, 'race-flagRed', s + 15, 1, gap, 5); }
  // The finish arch spans the road on purpose (its feet stand outside the edges).
  pointAt(track, 2, 0, _pt);
  out.push({ key: 'toy-gate-finish', x: _pt[0], y: _pt[1], z: _pt[2], yaw: _pt[3], scale: (hw * 2 + 6) / 1.55, sy: 9 });
  // Billboards and flags on corners.
  for (const path of track.paths) for (let i = 0; i < path.count; i += 22) {
    if (Math.abs(path.curve[i]) < 0.02 || path.drop[i] > 0) continue;
    const side = path.curve[i] > 0 ? -1 : 1; // outside of the bend
    beside(path, rng.chance(0.5) ? 'race-billboard' : 'race-billboardLow', path.dist[i], side, gap + 4, 9);
    beside(path, rng.pick(['race-flagRed', 'race-flagGreen', 'race-flagCheckers']), path.dist[i] + 8, side, gap, 4);
  }
  if (t === 'meadow') {
    scatter(['nat-tree_default', 'nat-tree_oak', 'nat-tree_detailed', 'nat-tree_fat', 'race-treeLarge'], 520, gap, 150, [6, 9]);
    scatter(['nat-plant_bushLarge', 'nat-plant_bush', 'nat-flower_redA', 'nat-flower_yellowB', 'nat-flower_purpleA', 'nat-mushroom_redGroup'], 600, 3, 40, [5, 8]);
    scatter(['nat-tent_detailedOpen', 'nat-log_stack', 'nat-stump_old'], 50, gap, 50, [6, 8]);
    for (let i = 0; i < track.count; i += 7) if (Math.abs(track.curve[i]) > 0.018) beside(track, 'nat-fence_simple', track.dist[i], track.curve[i] > 0 ? -1 : 1, gap - 1, 5);
  } else if (urban) {
    const blocks = ['city-building-a', 'city-building-b', 'city-building-c', 'city-building-d', 'city-building-e', 'city-building-f', 'city-building-g', 'city-building-h', 'city-building-i', 'city-building-j', 'city-building-k', 'city-building-l'];
    for (const path of track.paths) for (let s = 0; s < path.length; s += 15) for (const side of [-1, 1]) {
      if (path.index === 0 && Math.abs(((s + 70) % path.length) - 70) < 80 && side > 0) continue; // grandstands
      const i = sampleAt(path, s);
      if (t === 'dusk' && (path.drop[i] > 0 || path.py[i] > 4)) continue; // not beside the freeway decks
      beside(path, rng.pick(blocks), s, side, rng.range(12, 16), rng.range(10, 12));
      if (rng.chance(0.25)) beside(path, 'race-lightPostLarge', s + 7, side, 2.4, 7);
    }
    scatter(['city-building-skyscraper-a', 'city-building-skyscraper-b', 'city-building-skyscraper-c', 'city-building-skyscraper-d'], 140, 40, 200, [12, 15]);
    scatter(['house-tree-large', 'house-tree-small', 'city-detail-parasol-a'], 160, 4, 10, [9, 12]);
    if (t === 'dusk') scatter(['race-lightPostLarge', 'city-detail-awning'], 90, 3, 9, [6, 8]);
  } else if (t === 'canyon') {
    for (let s = 0; s < track.length; s += 9) for (const side of [-1, 1]) {
      if (rng.chance(0.75)) beside(track, rng.pick(['nat-cliff_block_rock', 'nat-cliff_large_rock', 'nat-rock_tallB']), s, side, rng.range(5, 9), rng.range(9, 14), true, 0.4);
    }
    scatter(['nat-rock_tallB', 'nat-rock_tallE', 'nat-rock_tallG', 'nat-cliff_block_rock'], 260, 20, 180, [12, 26]);
    scatter(['nat-cactus_tall', 'nat-cactus_short', 'nat-rock_largeA', 'nat-rock_largeC'], 260, 6, 70, [5, 9]);
    scatter(['nat-tree_palmTall', 'nat-tree_palmBend'], 40, 10, 60, [6, 8]);
  } else if (t === 'frost') {
    scatter(['plat-tree-pine-snow', 'plat-tree-snow', 'plat-tree-pine-snow-small'], 620, gap, 170, [4, 7]);
    scatter(['plat-rocks', 'plat-block-snow-large', 'nat-rock_largeA'], 120, gap, 90, [4, 8]);
    scatter(['plat-stones'], 90, 4, gap - 2, [3, 5]);
  } else if (t === 'lagoon') {
    scatter(['nat-tree_palmTall', 'nat-tree_palmBend', 'nat-tree_palmTall'], 520, gap, 150, [6, 9]);
    scatter(['nat-plant_bushLarge', 'nat-plant_bush', 'nat-flower_redA', 'nat-flower_yellowB', 'nat-flower_purpleA'], 520, 3, 40, [5, 8]);
    scatter(['nat-rock_largeA', 'nat-rock_largeC', 'nat-cliff_block_rock'], 120, gap, 90, [6, 12]);
    scatter(['city-detail-parasol-a', 'nat-tent_detailedOpen', 'nat-log_stack'], 70, gap, 40, [6, 9]);
  } else {
    // Volcano: basalt spires along the road, boulder fields, dead stumps.
    for (let s = 0; s < track.length; s += 12) for (const side of [-1, 1]) {
      if (rng.chance(0.45)) beside(track, rng.pick(['nat-cliff_block_rock', 'nat-cliff_large_rock', 'nat-rock_tallE']), s, side, rng.range(gap, gap + 8), rng.range(8, 14), true, 0.5);
    }
    scatter(['nat-rock_tallB', 'nat-rock_tallE', 'nat-rock_tallG', 'nat-cliff_large_rock'], 300, 20, 180, [12, 28]);
    scatter(['nat-rock_largeA', 'nat-rock_largeC', 'plat-rocks', 'nat-stump_old', 'nat-log_stack'], 260, 5, 70, [5, 9]);
  }
  propCache.set(track.def.id, out);
  return out;
}
