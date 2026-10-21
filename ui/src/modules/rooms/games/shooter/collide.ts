/**
 * Collision for Storm Royale: a uniform grid over the static box and
 * cylinder colliders, plus the (few hundred, linearly scanned) player
 * structures passed in per call (trunks harvested away are masked out, see
 * `syncFelled`). Capsule-vs-world movement with step-up,
 * standing on props / floors / ramps and head-bumping ceilings, and a
 * cell-walking (DDA) raycast against props, structures and terrain.
 * Allocation-free in the hot paths.
 */
import { HALF, terrainHeight, type Box, type Cylinder, type ShooterMap } from './map.ts';
import { rampHeight, rayRamp, type Build } from './builds.ts';

const CELL = 12;
const EMPTY: number[] = [];
const N = Math.ceil((HALF * 2) / CELL);
export const RADIUS = 0.42;
export const HEIGHT = 1.8;
export const STEP = 0.6;
/** A box whose underside is this close below the head is a ceiling: bump down instead of sideways. */
const BUMP = 0.35;

/**
 * `felled` marks tree trunks that were harvested away (no collision until they
 * regrow); `felledFrom` is the list it was filled from (see `syncFelled`).
 */
export interface World { map: ShooterMap; boxCells: number[][]; cylCells: number[][]; stamp: Uint32Array; cylStamp: Uint32Array; mark: number; felled: Uint8Array; felledFrom: readonly number[] | null }

const cell = (v: number) => Math.max(0, Math.min(N - 1, Math.floor((v + HALF) / CELL)));

const worlds = new Map<string, World>();
export function worldFor(map: ShooterMap): World {
  const hit = worlds.get(map.id);
  if (hit && hit.map === map) return hit;
  const boxCells: number[][] = Array.from({ length: N * N }, () => []);
  const cylCells: number[][] = Array.from({ length: N * N }, () => []);
  map.boxes.forEach((b, i) => {
    for (let cz = cell(b.minZ); cz <= cell(b.maxZ); cz++) for (let cx = cell(b.minX); cx <= cell(b.maxX); cx++) boxCells[cz * N + cx].push(i);
  });
  map.cylinders.forEach((c, i) => {
    for (let cz = cell(c.z - c.r); cz <= cell(c.z + c.r); cz++) for (let cx = cell(c.x - c.r); cx <= cell(c.x + c.r); cx++) cylCells[cz * N + cx].push(i);
  });
  const world: World = { map, boxCells, cylCells, stamp: new Uint32Array(map.boxes.length), cylStamp: new Uint32Array(map.cylinders.length), mark: 0, felled: new Uint8Array(map.cylinders.length), felledFrom: null };
  worlds.set(map.id, world);
  return world;
}

/** Points the world's trunk mask at a match's felled-tree list (cheap no-op while the list is unchanged). */
export function syncFelled(w: World, list: readonly number[]): void {
  if (w.felledFrom === list) return;
  w.felled.fill(0);
  for (let k = 0; k < list.length; k++) if (list[k] >= 0 && list[k] < w.felled.length) w.felled[list[k]] = 1;
  w.felledFrom = list;
}

type Builds = readonly Build[] | null | undefined;

const standOn = (b: Box, x: number, z: number, feet: number, g: number) =>
  x > b.minX - RADIUS * 0.5 && x < b.maxX + RADIUS * 0.5 && z > b.minZ - RADIUS * 0.5 && z < b.maxZ + RADIUS * 0.5 && b.maxY <= feet + STEP && b.maxY > g;

/** Highest standable surface under (x, z) at or below `feet + STEP` (terrain, props, then structures). */
export function groundAt(w: World, x: number, z: number, feet: number, builds?: Builds): number {
  let g = terrainHeight(w.map, x, z);
  const list = w.boxCells[cell(z) * N + cell(x)];
  for (let k = 0; k < list.length; k++) {
    const b = w.map.boxes[list[k]];
    if (standOn(b, x, z, feet, g)) g = b.maxY;
  }
  if (builds) for (let i = 0; i < builds.length; i++) {
    const s = builds[i];
    if (s.kind === 2) { const h = rampHeight(s, x, z); if (h <= feet + STEP && h > g) g = h; continue; }
    for (let k = 0; k < s.boxes.length; k++) if (standOn(s.boxes[k], x, z, feet, g)) g = s.boxes[k].maxY;
  }
  return g;
}

export interface Body { x: number; y: number; z: number; vx: number; vy: number; vz: number; grounded: boolean }

/** Pushes the capsule out of one box: sideways, or down when the head just bumped its underside. */
function pushBox(p: Body, b: Box): boolean {
  // Tops within a step of the feet are walked onto (groundAt lifts the body), not pushed against.
  if (p.y + HEIGHT <= b.minY || p.y >= b.maxY - STEP) return false;
  const nx = Math.max(b.minX, Math.min(p.x, b.maxX)), nz = Math.max(b.minZ, Math.min(p.z, b.maxZ));
  let ox = p.x - nx, oz = p.z - nz;
  const d2 = ox * ox + oz * oz;
  if (d2 >= RADIUS * RADIUS) return false;
  if (p.y < b.minY && p.y + HEIGHT - b.minY < BUMP && d2 < 1e-8) {
    p.y = b.minY - HEIGHT;
    if (p.vy > 0) p.vy = 0;
    return true;
  }
  if (d2 < 1e-8) {
    // Centre inside the box: leave by the nearest face.
    const l = p.x - b.minX, r = b.maxX - p.x, f = p.z - b.minZ, bk = b.maxZ - p.z, m = Math.min(l, r, f, bk);
    if (m === l) p.x = b.minX - RADIUS; else if (m === r) p.x = b.maxX + RADIUS; else if (m === f) p.z = b.minZ - RADIUS; else p.z = b.maxZ + RADIUS;
  } else {
    const d = Math.sqrt(d2), push = RADIUS - d;
    ox /= d; oz /= d;
    p.x += ox * push; p.z += oz * push;
    const vn = p.vx * ox + p.vz * oz;
    if (vn < 0) { p.vx -= ox * vn; p.vz -= oz * vn; }
  }
  return true;
}

/** Pushes a standing capsule out of boxes, trunks and structures (XZ, or down under a ceiling). */
export function resolve(w: World, p: Body, builds?: Builds): boolean {
  let hit = false;
  w.mark++;
  // Boxes live in every cell they overlap, so only the cells under the capsule matter.
  for (let gz = cell(p.z - RADIUS); gz <= cell(p.z + RADIUS); gz++) for (let gx = cell(p.x - RADIUS); gx <= cell(p.x + RADIUS); gx++) {
    const boxes = w.boxCells[gz * N + gx];
    for (let k = 0; k < boxes.length; k++) {
      const i = boxes[k];
      if (w.stamp[i] === w.mark) continue;
      w.stamp[i] = w.mark;
      if (pushBox(p, w.map.boxes[i])) hit = true;
    }
    const cyls = w.cylCells[gz * N + gx];
    for (let k = 0; k < cyls.length; k++) {
      const i = cyls[k];
      if (w.cylStamp[i] === w.mark) continue;
      w.cylStamp[i] = w.mark;
      const c = w.map.cylinders[i];
      if (w.felled[i] || p.y + HEIGHT <= c.y0 || p.y >= c.y1) continue;
      let ox = p.x - c.x, oz = p.z - c.z;
      const d = Math.hypot(ox, oz), min = c.r + RADIUS;
      if (d >= min || d < 1e-6) continue;
      ox /= d; oz /= d;
      p.x = c.x + ox * min; p.z = c.z + oz * min;
      hit = true;
    }
  }
  if (builds) for (let i = 0; i < builds.length; i++) {
    const s = builds[i], a = s.aabb;
    if (s.kind === 2 || p.x < a.minX - RADIUS || p.x > a.maxX + RADIUS || p.z < a.minZ - RADIUS || p.z > a.maxZ + RADIUS) continue;
    for (let k = 0; k < s.boxes.length; k++) if (pushBox(p, s.boxes[k])) hit = true;
  }
  p.x = Math.max(-HALF + 2, Math.min(HALF - 2, p.x));
  p.z = Math.max(-HALF + 2, Math.min(HALF - 2, p.z));
  return hit;
}

export interface Hit {
  t: number; nx: number; ny: number; nz: number;
  kind: 'none' | 'terrain' | 'prop' | 'build';
  /** Box index (prop), cylinder index (prop with `cyl`), or structure index (build). */
  index: number; cyl: boolean;
}
const _hit: Hit = { t: 0, nx: 0, ny: 1, nz: 0, kind: 'none', index: -1, cyl: false };

function rayBox(ox: number, oy: number, oz: number, dx: number, dy: number, dz: number, b: Box, max: number): number {
  let t0 = 0, t1 = max;
  for (let a = 0; a < 3; a++) {
    const o = a === 0 ? ox : a === 1 ? oy : oz, d = a === 0 ? dx : a === 1 ? dy : dz;
    const lo = a === 0 ? b.minX : a === 1 ? b.minY : b.minZ, hi = a === 0 ? b.maxX : a === 1 ? b.maxY : b.maxZ;
    if (Math.abs(d) < 1e-9) { if (o < lo || o > hi) return -1; continue; }
    let ta = (lo - o) / d, tb = (hi - o) / d;
    if (ta > tb) { const s = ta; ta = tb; tb = s; }
    if (ta > t0) t0 = ta;
    if (tb < t1) t1 = tb;
    if (t0 > t1) return -1;
  }
  return t0;
}
function rayCyl(ox: number, oz: number, oy: number, dx: number, dz: number, dy: number, c: Cylinder, max: number): number {
  const fx = ox - c.x, fz = oz - c.z;
  const a = dx * dx + dz * dz;
  if (a < 1e-9) return -1;
  const b = 2 * (fx * dx + fz * dz), cc = fx * fx + fz * fz - c.r * c.r;
  const disc = b * b - 4 * a * cc;
  if (disc < 0) return -1;
  const t = (-b - Math.sqrt(disc)) / (2 * a);
  if (t < 0 || t > max) return -1;
  const y = oy + dy * t;
  return y >= c.y0 && y <= c.y1 ? t : -1;
}

/**
 * First hit along a normalized ray within `max` metres. `trunks: false`
 * ignores tree trunks (e.g. for the camera); `builds` adds structures.
 */
export function raycast(w: World, ox: number, oy: number, oz: number, dx: number, dy: number, dz: number, max: number, trunks = true, builds?: Builds): Hit {
  let best = max, kind: Hit['kind'] = 'none', index = -1, isCyl = false;
  w.mark++;
  // Walk the grid cells the ray crosses (Amanatides–Woo), nearest first.
  const gx0 = (ox + HALF) / CELL, gz0 = (oz + HALF) / CELL;
  let cx = Math.floor(gx0), cz = Math.floor(gz0);
  const sx = dx > 0 ? 1 : -1, sz = dz > 0 ? 1 : -1;
  const adx = Math.abs(dx), adz = Math.abs(dz);
  const deltaX = adx < 1e-9 ? Infinity : CELL / adx, deltaZ = adz < 1e-9 ? Infinity : CELL / adz;
  let nextX = adx < 1e-9 ? Infinity : ((dx > 0 ? cx + 1 - gx0 : gx0 - cx) * CELL) / adx;
  let nextZ = adz < 1e-9 ? Infinity : ((dz > 0 ? cz + 1 - gz0 : gz0 - cz) * CELL) / adz;
  for (let guard = 0; guard < 4 * N + 4; guard++) {
    if (cx >= 0 && cz >= 0 && cx < N && cz < N) {
      const boxes = w.boxCells[cz * N + cx];
      for (let k = 0; k < boxes.length; k++) {
        const i = boxes[k];
        if (w.stamp[i] === w.mark) continue;
        w.stamp[i] = w.mark;
        const h = rayBox(ox, oy, oz, dx, dy, dz, w.map.boxes[i], best);
        if (h >= 0 && h < best) { best = h; kind = 'prop'; index = i; isCyl = false; }
      }
      const cyls = trunks ? w.cylCells[cz * N + cx] : EMPTY;
      for (let k = 0; k < cyls.length; k++) {
        const i = cyls[k];
        if (w.cylStamp[i] === w.mark || w.felled[i]) continue;
        w.cylStamp[i] = w.mark;
        const h = rayCyl(ox, oz, oy, dx, dz, dy, w.map.cylinders[i], best);
        if (h >= 0 && h < best) { best = h; kind = 'prop'; index = i; isCyl = true; }
      }
    }
    const t = Math.min(nextX, nextZ);
    if (t > best) break;
    if (nextX < nextZ) { nextX += deltaX; cx += sx; } else { nextZ += deltaZ; cz += sz; }
  }
  if (builds) for (let i = 0; i < builds.length; i++) {
    const s = builds[i];
    if (rayBox(ox, oy, oz, dx, dy, dz, s.aabb, best) < 0) continue;
    if (s.kind === 2) {
      const h = rayRamp(s, ox, oy, oz, dx, dy, dz, best);
      if (h >= 0 && h < best) { best = h; kind = 'build'; index = i; isCyl = false; }
      continue;
    }
    for (let k = 0; k < s.boxes.length; k++) {
      const h = rayBox(ox, oy, oz, dx, dy, dz, s.boxes[k], best);
      if (h >= 0 && h < best) { best = h; kind = 'build'; index = i; isCyl = false; }
    }
  }
  // Terrain: march then bisect.
  let prev = 0;
  for (let t = 0.5; t < best; t += 1.5) {
    const y = oy + dy * t;
    if (y < terrainHeight(w.map, ox + dx * t, oz + dz * t)) {
      let lo = prev, hi = t;
      for (let i = 0; i < 6; i++) {
        const mid = (lo + hi) / 2;
        if (oy + dy * mid < terrainHeight(w.map, ox + dx * mid, oz + dz * mid)) hi = mid; else lo = mid;
      }
      best = hi; kind = 'terrain'; index = -1; isCyl = false;
      break;
    }
    prev = t;
  }
  _hit.t = best; _hit.kind = kind; _hit.index = index; _hit.cyl = isCyl;
  return _hit;
}

/** Line of sight between two points (eyes), through props, structures and terrain. */
export function visible(w: World, ax: number, ay: number, az: number, bx: number, by: number, bz: number, builds?: Builds): boolean {
  const dx = bx - ax, dy = by - ay, dz = bz - az, d = Math.hypot(dx, dy, dz);
  if (d < 1e-3) return true;
  return raycast(w, ax, ay, az, dx / d, dy / d, dz / d, d, true, builds).kind === 'none';
}
