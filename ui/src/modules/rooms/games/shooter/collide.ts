/**
 * Collision for Storm Royale: a uniform grid over box and cylinder colliders,
 * capsule-vs-world movement with step-up/standing on props, and raycasts
 * against props + terrain. Allocation-free in the hot paths.
 */
import { HALF, terrainHeight, type Box, type Cylinder, type ShooterMap } from './map.ts';

const CELL = 12;
const EMPTY: number[] = [];
const N = Math.ceil((HALF * 2) / CELL);
export const RADIUS = 0.42;
export const HEIGHT = 1.8;
export const STEP = 0.6;

export interface World { map: ShooterMap; boxCells: number[][]; cylCells: number[][]; stamp: Uint32Array; cylStamp: Uint32Array; mark: number }

const cell = (v: number) => Math.max(0, Math.min(N - 1, Math.floor((v + HALF) / CELL)));

const worlds = new Map<string, World>();
export function worldFor(map: ShooterMap): World {
  const hit = worlds.get(map.id);
  if (hit) return hit;
  const boxCells: number[][] = Array.from({ length: N * N }, () => []);
  const cylCells: number[][] = Array.from({ length: N * N }, () => []);
  map.boxes.forEach((b, i) => {
    for (let cz = cell(b.minZ); cz <= cell(b.maxZ); cz++) for (let cx = cell(b.minX); cx <= cell(b.maxX); cx++) boxCells[cz * N + cx].push(i);
  });
  map.cylinders.forEach((c, i) => {
    for (let cz = cell(c.z - c.r); cz <= cell(c.z + c.r); cz++) for (let cx = cell(c.x - c.r); cx <= cell(c.x + c.r); cx++) cylCells[cz * N + cx].push(i);
  });
  const world: World = { map, boxCells, cylCells, stamp: new Uint32Array(map.boxes.length), cylStamp: new Uint32Array(map.cylinders.length), mark: 0 };
  worlds.set(map.id, world);
  return world;
}

/** Highest standable surface under (x, z) at or below `feet + STEP`. */
export function groundAt(w: World, x: number, z: number, feet: number): number {
  let g = terrainHeight(w.map, x, z);
  const list = w.boxCells[cell(z) * N + cell(x)];
  for (let k = 0; k < list.length; k++) {
    const b = w.map.boxes[list[k]];
    if (x > b.minX - RADIUS * 0.5 && x < b.maxX + RADIUS * 0.5 && z > b.minZ - RADIUS * 0.5 && z < b.maxZ + RADIUS * 0.5 && b.maxY <= feet + STEP && b.maxY > g) g = b.maxY;
  }
  return g;
}

export interface Body { x: number; y: number; z: number; vx: number; vy: number; vz: number; grounded: boolean }

/** Pushes a standing capsule out of boxes and trunks (XZ only). */
export function resolve(w: World, p: Body): boolean {
  let hit = false;
  const cx = cell(p.x), cz = cell(p.z);
  w.mark++;
  for (let dz = -1; dz <= 1; dz++) for (let dx = -1; dx <= 1; dx++) {
    const gx = cx + dx, gz = cz + dz;
    if (gx < 0 || gz < 0 || gx >= N || gz >= N) continue;
    const boxes = w.boxCells[gz * N + gx];
    for (let k = 0; k < boxes.length; k++) {
      const i = boxes[k];
      if (w.stamp[i] === w.mark) continue;
      w.stamp[i] = w.mark;
      const b = w.map.boxes[i];
      if (p.y + HEIGHT <= b.minY || p.y >= b.maxY - STEP * 0.5) continue;
      const nx = Math.max(b.minX, Math.min(p.x, b.maxX)), nz = Math.max(b.minZ, Math.min(p.z, b.maxZ));
      let ox = p.x - nx, oz = p.z - nz;
      const d2 = ox * ox + oz * oz;
      if (d2 >= RADIUS * RADIUS) continue;
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
      hit = true;
    }
    const cyls = w.cylCells[gz * N + gx];
    for (let k = 0; k < cyls.length; k++) {
      const i = cyls[k];
      if (w.cylStamp[i] === w.mark) continue;
      w.cylStamp[i] = w.mark;
      const c = w.map.cylinders[i];
      if (p.y + HEIGHT <= c.y0 || p.y >= c.y1) continue;
      let ox = p.x - c.x, oz = p.z - c.z;
      const d = Math.hypot(ox, oz), min = c.r + RADIUS;
      if (d >= min || d < 1e-6) continue;
      ox /= d; oz /= d;
      p.x = c.x + ox * min; p.z = c.z + oz * min;
      hit = true;
    }
  }
  p.x = Math.max(-HALF + 2, Math.min(HALF - 2, p.x));
  p.z = Math.max(-HALF + 2, Math.min(HALF - 2, p.z));
  return hit;
}

export interface Hit { t: number; nx: number; ny: number; nz: number; kind: 'none' | 'terrain' | 'prop' }
const _hit: Hit = { t: 0, nx: 0, ny: 1, nz: 0, kind: 'none' };

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

/** First world hit along a normalized ray within `max` metres (`trunks: false` ignores tree trunks, e.g. for the camera). */
export function raycast(w: World, ox: number, oy: number, oz: number, dx: number, dy: number, dz: number, max: number, trunks = true): Hit {
  let best = max, kind: Hit['kind'] = 'none';
  // Props: walk grid cells along the ray (2D DDA would be tighter; cells are coarse).
  w.mark++;
  const steps = Math.ceil(max / (CELL * 0.5));
  for (let s = 0; s <= steps; s++) {
    const t = Math.min(max, s * CELL * 0.5);
    if (t > best) break;
    const px = ox + dx * t, pz = oz + dz * t;
    if (Math.abs(px) > HALF + CELL || Math.abs(pz) > HALF + CELL) break;
    const cx = cell(px), cz = cell(pz);
    for (let ddz = -1; ddz <= 1; ddz++) for (let ddx = -1; ddx <= 1; ddx++) {
      const gx = cx + ddx, gz = cz + ddz;
      if (gx < 0 || gz < 0 || gx >= N || gz >= N) continue;
      const boxes = w.boxCells[gz * N + gx];
      for (let k = 0; k < boxes.length; k++) {
        const i = boxes[k];
        if (w.stamp[i] === w.mark) continue;
        w.stamp[i] = w.mark;
        const h = rayBox(ox, oy, oz, dx, dy, dz, w.map.boxes[i], best);
        if (h >= 0 && h < best) { best = h; kind = 'prop'; }
      }
      const cyls = trunks ? w.cylCells[gz * N + gx] : EMPTY;
      for (let k = 0; k < cyls.length; k++) {
        const i = cyls[k];
        if (w.cylStamp[i] === w.mark) continue;
        w.cylStamp[i] = w.mark;
        const h = rayCyl(ox, oz, oy, dx, dz, dy, w.map.cylinders[i], best);
        if (h >= 0 && h < best) { best = h; kind = 'prop'; }
      }
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
      best = hi; kind = 'terrain';
      break;
    }
    prev = t;
  }
  _hit.t = best; _hit.kind = kind;
  return _hit;
}

/** Line of sight between two points (eyes). */
export function visible(w: World, ax: number, ay: number, az: number, bx: number, by: number, bz: number): boolean {
  const dx = bx - ax, dy = by - ay, dz = bz - az, d = Math.hypot(dx, dy, dz);
  if (d < 1e-3) return true;
  return raycast(w, ax, ay, az, dx / d, dy / d, dz / d, d).kind === 'none';
}
