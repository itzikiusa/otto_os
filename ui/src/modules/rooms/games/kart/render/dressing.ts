/**
 * Kart track dressing: where the purely visual furniture goes — the painted
 * start grid, tyre marks through the corners, chevron boards on the outside
 * of hairpins, sponsor banners on the barrier walls, extra trackside props
 * (floodlight posts on the main straight, grandstands at the big corners) and
 * the firework launch spots by the finish. Pure data (no three.js) so the
 * unit tests can hold it to the same rule as layout.ts: nothing solid stands
 * on any route's road, forks and merges included, and nothing lands on top of
 * a layout prop.
 */
import { clamp, hashString, Rng, smoothstep } from '../../common/math.ts';
import { F_BRIDGE, F_DRAW, F_GAP, F_PIPE, F_TUNNEL, FORK_ZONE, pointAt, type Path, type Track } from '../tracks.ts';
import { BARRIER_OFFSET, BARRIER_THICK, barriers, PROP_BOXES, PROP_PAD, propBlocksRoad, propLayout, roadIndex, terrainHeight, type PropPlacement, type RoadHit } from '../layout.ts';

const _pt = [0, 0, 0, 0];
const _h: RoadHit = { seg: -1, index: 0, lateral: 0, y: 0, edge: 0 };

// ── Start grid ─────────────────────────────────────────────────────────────
/** Painted grid boxes: one per possible racer (the simulation seats at most this many). */
export const GRID_SLOTS = 8;
/**
 * The painted grid slots behind the start line: flat [x, y, z, yaw] per slot,
 * at exactly the spots createRace seats the racers (two staggered columns,
 * 7.5 m rows) so every kart starts inside its box.
 */
export function gridSlots(track: Track): number[] {
  const out: number[] = [];
  for (let i = 0; i < GRID_SLOTS; i++) {
    const row = Math.floor(i / 2), side = i % 2 ? -1 : 1;
    const s = -9 - row * 7.5 - (i % 2) * 3.2;
    pointAt(track, s, side * track.halfWidth * 0.42, _pt);
    out.push(_pt[0], _pt[1], _pt[2], _pt[3]);
  }
  return out;
}

// ── Road paint ─────────────────────────────────────────────────────────────
/**
 * Fade for a branch's own markings where it still shares tarmac with the road
 * it forks from or merges into (1 = full strength). The main loop paints
 * everywhere; a branch fades its tyre grooves in over its first FORK_ZONE
 * metres and out over its last, so the two routes' paint never crosses in the
 * mouth.
 */
export function branchFade(path: Path, i: number): number {
  if (path.index === 0) return 1;
  const s = path.dist[i];
  return smoothstep(FORK_ZONE * 0.25, FORK_ZONE, s) * smoothstep(FORK_ZONE * 0.25, FORK_ZONE, path.length - s);
}
/**
 * Tyre marks per sample (0..1): dark drift arcs through every real corner,
 * strongest at the apex, none where there is no plain tarmac (gaps,
 * drawbridge leaves, half-pipes), faded in fork mouths.
 */
export function skidAlpha(path: Path): Float32Array {
  const n = path.count, raw = new Float32Array(n), out = new Float32Array(n);
  for (let i = 0; i < n; i++) raw[i] = smoothstep(0.013, 0.042, Math.abs(path.curve[i]));
  // Smear along the road (±6 samples): marks start before the turn-in and run out past the exit.
  for (let i = 0; i < n; i++) {
    let m = 0;
    for (let k = -6; k <= 6; k++) {
      const j = path.closed ? (i + k + n) % n : clamp(i + k, 0, n - 1);
      m = Math.max(m, raw[j] * (1 - Math.abs(k) / 7));
    }
    out[i] = (path.flags[i] & (F_GAP | F_DRAW | F_PIPE)) ? 0 : m * branchFade(path, i);
  }
  return out;
}

// ── Barrier banners ────────────────────────────────────────────────────────
/** A sponsor banner hung on the inner face of a barrier wall where `mask` is 1. */
export interface Banner { side: number; lateral: number; top: number; mask: Uint8Array }
/**
 * Banners along the walls of walled routes: the whole home straight (±120 m
 * of the line) and every long straight elsewhere, only where the wall itself
 * stands (so never across a fork mouth or over a gap).
 */
export function wallBanners(track: Track, path: Path): Banner[] {
  const out: Banner[] = [];
  for (const b of barriers(track, path)) {
    if (b.kind !== 'wall') continue;
    const n = path.count, mask = new Uint8Array(n);
    for (let i = 0; i < n; i++) {
      if (!b.mask[i]) continue;
      const s = path.dist[i];
      const home = path.index === 0 && (s < 120 || s > path.length - 120);
      // A straight: no real curvature for ±8 samples.
      let straight = true;
      for (let k = -8; k <= 8 && straight; k++) {
        const j = path.closed ? (i + k + n) % n : clamp(i + k, 0, n - 1);
        if (Math.abs(path.curve[j]) > 0.006) straight = false;
      }
      mask[i] = home || straight ? 1 : 0;
    }
    if (mask.some(v => v)) out.push({ side: b.side, lateral: b.lateral, top: b.top, mask });
  }
  return out;
}

// ── Solid trackside furniture ──────────────────────────────────────────────
/** Chevron board footprint (metres): half width, half depth, height (posts plus face). */
export const CHEVRON_HALF = 1.5, CHEVRON_DEPTH = 0.15, CHEVRON_HEIGHT = 3.2;
/** A chevron board facing the oncoming racers; `dir` is the way the arrows point (+1 left, -1 right). */
export interface Chevron { x: number; y: number; z: number; yaw: number; dir: number }

/** Footprint circles of every layout prop near (x, z): [x, z, radius]. Cached per track. */
const footCache = new Map<string, Map<number, number[]>>();
const FOOT_CELL = 16;
function footprints(track: Track): Map<number, number[]> {
  let grid = footCache.get(track.def.id);
  if (grid) return grid;
  grid = new Map();
  for (const p of propLayout(track)) {
    const box = PROP_BOXES[p.key];
    const r = box ? Math.hypot(box[0], box[1]) * p.scale : p.key === 'toy-gate-finish' ? p.scale : 2;
    const k = Math.floor(p.x / FOOT_CELL) * 100003 + Math.floor(p.z / FOOT_CELL);
    let list = grid.get(k);
    if (!list) { list = []; grid.set(k, list); }
    list.push(p.x, p.z, r);
  }
  footCache.set(track.def.id, grid);
  return grid;
}
/** Whether a footprint of radius `r` at (x, z) overlaps any layout prop (props up to 40 m wide are searched). */
export function overlapsProps(track: Track, x: number, z: number, r: number): boolean {
  const grid = footprints(track);
  const cx = Math.floor(x / FOOT_CELL), cz = Math.floor(z / FOOT_CELL);
  for (let dx = -3; dx <= 3; dx++) for (let dz = -3; dz <= 3; dz++) {
    const list = grid.get((cx + dx) * 100003 + cz + dz);
    if (!list) continue;
    for (let k = 0; k < list.length; k += 3) if (Math.hypot(list[k] - x, list[k + 1] - z) < list[k + 2] + r) return true;
  }
  return false;
}
/** Whether a box footprint (half sizes hx, hz, rotated by yaw, standing y0..y1) stands on any road widened by `pad`. */
export function footprintBlocksRoad(track: Track, x: number, z: number, yaw: number, hx: number, hz: number, y0: number, y1: number, pad: number): boolean {
  const idx = roadIndex(track);
  const c = Math.cos(yaw), s = Math.sin(yaw);
  const nx = Math.max(1, Math.ceil((hx * 2) / 1.5)), nz = Math.max(1, Math.ceil((hz * 2) / 1.5));
  for (let a = 0; a <= nx; a++) for (let b = 0; b <= nz; b++) {
    const lx = -hx + (2 * hx * a) / nx, lz = -hz + (2 * hz * b) / nz;
    if (idx.hit(x + lx * c + lz * s, z - lx * s + lz * c, y0, y1, pad, -1, _h)) return true;
  }
  return false;
}
/** Dry land a solid may stand on (not under the lagoon or in the lava). */
const dry = (track: Track, y: number) => track.liquid === null || y > track.liquid + 0.4;

const chevronCache = new Map<string, Chevron[]>();
/**
 * Chevron boards around the outside of every sharp bend, one every ~12 m
 * through the turn, arrows pointing the way the road goes: just behind the
 * barrier on walled routes, just beyond the rescue line on open ones. Any
 * board that would stand on a road (another route may pass behind the wall)
 * or on a layout prop is dropped.
 */
export function chevronBoards(track: Track): Chevron[] {
  const hit = chevronCache.get(track.def.id);
  if (hit) return hit;
  const out: Chevron[] = [];
  chevronCache.set(track.def.id, out);
  for (const path of track.paths) {
    const behind = path.edge === 'wall' ? BARRIER_OFFSET + BARRIER_THICK + PROP_PAD + 0.6 : path.outOfBounds + PROP_PAD + 1;
    let last = -Infinity;
    for (let i = 0; i < path.count; i++) {
      const c = path.curve[i];
      if (Math.abs(c) < 0.028 || path.dist[i] - last < 12 || (path.flags[i] & (F_GAP | F_BRIDGE | F_TUNNEL | F_DRAW | F_PIPE)) || path.drop[i] > 0) continue;
      const out_ = c > 0 ? -1 : 1; // outside of the bend
      pointAt(path, path.dist[i], out_ * (path.halfWidth + behind), _pt);
      const x = _pt[0], z = _pt[2];
      // Face the racers coming into the bend: the board's normal points back down the road, tilted toward the inside.
      const yaw = _pt[3] + Math.PI;
      const y = terrainHeight(track, x, z);
      if (!dry(track, y) || Math.abs(y - path.py[i]) > 4) continue;
      if (footprintBlocksRoad(track, x, z, yaw, CHEVRON_HALF, CHEVRON_DEPTH, y - 0.5, y + CHEVRON_HEIGHT, PROP_PAD)) continue;
      if (overlapsProps(track, x, z, CHEVRON_HALF + 0.5)) continue;
      out.push({ x, y, z, yaw, dir: c > 0 ? 1 : -1 });
      last = path.dist[i];
    }
  }
  return out;
}

const extraCache = new Map<string, PropPlacement[]>();
/**
 * Extra kit props layered over layout's scenery: floodlight posts lining the
 * home straight and grandstands on the outside of the three sharpest bends of
 * the main loop. Every placement passes layout's own road test
 * (propBlocksRoad, PROP_PAD) and keeps off every layout prop and off each other.
 */
export function extraProps(track: Track): PropPlacement[] {
  const hit = extraCache.get(track.def.id);
  if (hit) return hit;
  const out: PropPlacement[] = [];
  extraCache.set(track.def.id, out);
  const rng = new Rng(hashString(track.def.id) ^ 0x7a11);
  const urban = track.def.theme === 'city' || track.def.theme === 'dusk';
  const gap = track.def.edge === 'grass' ? track.def.outOfBounds + 2 : 3;
  const taken: number[] = [];
  const free = (x: number, z: number, r: number) => {
    for (let k = 0; k < taken.length; k += 3) if (Math.hypot(taken[k] - x, taken[k + 1] - z) < taken[k + 2] + r) return false;
    return !overlapsProps(track, x, z, r);
  };
  const place = (key: string, s: number, side: number, dist: number, scale: number, path: Path = track) => {
    pointAt(path, s, side * (path.halfWidth + dist), _pt);
    const x = _pt[0], z = _pt[2], y = terrainHeight(track, x, z);
    if (!dry(track, y)) return;
    const p: PropPlacement = { key, x, y: y - 0.05, z, yaw: _pt[3] + (side > 0 ? -Math.PI / 2 : Math.PI / 2), scale, sy: scale };
    const box = PROP_BOXES[key];
    const r = Math.hypot(box[0], box[1]) * scale;
    if (propBlocksRoad(track, p, PROP_PAD) || !free(x, z, r)) return;
    taken.push(x, z, r);
    out.push(p);
  };
  // Floodlights both sides of the home straight.
  for (let s = -150; s <= 150; s += 24) for (const side of [-1, 1]) place('race-lightPostLarge', s + (side > 0 ? 12 : 0), side, gap + 1.5, 8);
  // Grandstands on the outside of the sharpest bends (at least 120 m apart, clear of the start area).
  const bends: [number, number][] = [];
  for (let i = 0; i < track.count; i += 4) {
    const s = track.dist[i];
    if (s < 160 || s > track.length - 160 || track.drop[i] > 0 || (track.flags[i] & (F_TUNNEL | F_BRIDGE))) continue;
    bends.push([Math.abs(track.curve[i]), i]);
  }
  bends.sort((a, b) => b[0] - a[0]);
  const chosen: number[] = [];
  for (const [c, i] of bends) {
    if (chosen.length >= 3 || c < 0.02) break;
    if (chosen.some(j => Math.abs(track.dist[j] - track.dist[i]) < 120)) continue;
    chosen.push(i);
    const side = track.curve[i] > 0 ? -1 : 1;
    for (const ds of [-12, 0, 12]) place(urban ? 'race-grandStandCovered' : 'race-grandStand', track.dist[i] + ds, side, gap + 8 + rng.range(0, 1), 11);
  }
  return out;
}

// ── Celebrations ───────────────────────────────────────────────────────────
/** Where the finish fireworks burst: flat [x, y, z] high over both sides of the line. */
export function fireworkSpots(track: Track): number[] {
  const out: number[] = [];
  for (const [s, side, h] of [[-10, 1, 34], [6, -1, 40], [20, 1, 46], [-24, -1, 38], [34, -1, 30], [-40, 1, 44]] as const) {
    pointAt(track, s, side * (track.halfWidth + 18), _pt);
    out.push(_pt[0], _pt[1] + h, _pt[2]);
  }
  return out;
}

/** Whether sample `i` of `path` carries plain tarmac a curb or line can sit on. */
export function paintable(path: Path, i: number): boolean {
  return !(path.flags[i] & (F_GAP | F_DRAW | F_PIPE));
}
