/**
 * Player structures for Storm Royale: walls, floors and ramps on a 4 m world
 * grid (heights snap to 0.8 m so a wall always covers a standing survivor).
 * Pure geometry + grid rules shared by the host sim, guests and tests:
 * colliders, ramp surfaces, the placement planner, structural support and a
 * one-number wire packing. No three.js.
 */
import { HALF, type Box } from './map.ts';
import { MATERIALS, type Material } from './weapons.ts';

export const TILE = 4;
/** Wall / ramp rise; floors stack this far apart. */
export const LEVEL = 3.2;
/** Height snap: a quarter level (levels are counted in these steps). */
export const LEVEL_SUBS = 4;
export const SUB = LEVEL / LEVEL_SUBS;
export const LEVEL_MIN = -16;
export const LEVEL_COUNT = 128;
export const TILES = (HALF * 2) / TILE;
export const MAX_BUILDS = 300;
export type BuildKind = 0 | 1 | 2;
export const BUILD_KIND_NAMES = ['Wall', 'Floor', 'Ramp'] as const;
/** Wall edits: solid, doorway, window. */
export const WALL_VARIANTS = 3;
const THICK = 0.24;

export interface Build {
  id: number; kind: BuildKind; mat: Material;
  /** Tile, height (in SUB units) and orientation. Walls: dir 0 = the tile's +z edge, 1 = its +x edge. Ramps: dir is the ascent (0 +z, 1 +x, 2 -z, 3 -x). */
  gx: number; gz: number; level: number; dir: number; variant: number;
  hp: number; owner: number; grow: number;
  /** Solid colliders (a ramp has none: it is a surface, see rampHeight). */
  boxes: Box[];
  aabb: Box;
}
export interface BuildSlot { kind: BuildKind; gx: number; gz: number; level: number; dir: number }

export const tileOf = (v: number): number => Math.max(0, Math.min(TILES - 1, Math.floor((v + HALF) / TILE)));
export const tileX = (g: number): number => -HALF + g * TILE;
export const baseY = (level: number): number => level * SUB;
export const maxHp = (b: { mat: Material }): number => MATERIALS[b.mat].hp;

const box = (minX: number, minY: number, minZ: number, maxX: number, maxY: number, maxZ: number): Box => ({ minX, minY, minZ, maxX, maxY, maxZ });

/** Colliders + bounds for a slot; walls split around door / window openings. */
export function buildGeometry(b: Pick<Build, 'kind' | 'gx' | 'gz' | 'level' | 'dir' | 'variant'>): { boxes: Box[]; aabb: Box } {
  const x0 = tileX(b.gx), z0 = tileX(b.gz), y = baseY(b.level);
  if (b.kind === 1) { const f = box(x0, y - 0.2, z0, x0 + TILE, y, z0 + TILE); return { boxes: [f], aabb: f }; }
  if (b.kind === 2) return { boxes: [], aabb: box(x0, y, z0, x0 + TILE, y + LEVEL, z0 + TILE) };
  // Wall along x (dir 0, at z0 + TILE) or along z (dir 1, at x0 + TILE): (a, b) span the edge, c is across it.
  const along = b.dir === 0;
  const c = along ? z0 + TILE : x0 + TILE, a0 = along ? x0 : z0, a1 = a0 + TILE, mid = a0 + TILE / 2;
  const piece = (s0: number, s1: number, y0: number, y1: number) => (along ? box(s0, y0, c - THICK / 2, s1, y1, c + THICK / 2) : box(c - THICK / 2, y0, s0, c + THICK / 2, y1, s1));
  const top = y + LEVEL;
  const boxes = b.variant === 1 ? [piece(a0, mid - 0.8, y, top), piece(mid + 0.8, a1, y, top), piece(mid - 0.8, mid + 0.8, y + 2.4, top)]
    : b.variant === 2 ? [piece(a0, mid - 0.8, y, top), piece(mid + 0.8, a1, y, top), piece(mid - 0.8, mid + 0.8, y, y + 1.1), piece(mid - 0.8, mid + 0.8, y + 2.3, top)]
      : [piece(a0, a1, y, top)];
  return { boxes, aabb: piece(a0, a1, y, top) };
}

export function makeBuild(id: number, slot: BuildSlot, mat: Material, owner: number, variant = 0): Build {
  const geo = buildGeometry({ ...slot, variant });
  const m = MATERIALS[mat];
  return { id, ...slot, mat, variant, owner, hp: Math.round(m.hp * m.start), grow: m.time, ...geo };
}
/** Re-derives colliders after an edit. */
export function refreshGeometry(b: Build): void { const g = buildGeometry(b); b.boxes = g.boxes; b.aabb = g.aabb; }

/** Progress (0..1) up a ramp at (x, z), or -1 outside its tile. */
function rampProgress(b: Build, x: number, z: number): number {
  const x0 = tileX(b.gx), z0 = tileX(b.gz);
  if (x < x0 || x > x0 + TILE || z < z0 || z > z0 + TILE) return -1;
  switch (b.dir) {
    case 1: return (x - x0) / TILE;
    case 2: return (z0 + TILE - z) / TILE;
    case 3: return (x0 + TILE - x) / TILE;
    default: return (z - z0) / TILE;
  }
}
/** Ramp surface height at (x, z), or -Infinity off the ramp. */
export function rampHeight(b: Build, x: number, z: number): number {
  const u = rampProgress(b, x, z);
  return u < 0 ? -Infinity : baseY(b.level) + LEVEL * u;
}
/** Ray vs a ramp's sloped surface; t or -1. */
export function rayRamp(b: Build, ox: number, oy: number, oz: number, dx: number, dy: number, dz: number, max: number): number {
  const x0 = tileX(b.gx), z0 = tileX(b.gz), k = LEVEL / TILE;
  // s(t) = s0 + ds * t is the distance up the slope along the ascent axis.
  const [s0, ds] = b.dir === 1 ? [ox - x0, dx] : b.dir === 2 ? [z0 + TILE - oz, -dz] : b.dir === 3 ? [x0 + TILE - ox, -dx] : [oz - z0, dz];
  const den = dy - k * ds;
  if (Math.abs(den) < 1e-9) return -1;
  const t = (baseY(b.level) + k * s0 - oy) / den;
  if (t < 0 || t > max) return -1;
  const x = ox + dx * t, z = oz + dz * t;
  return x >= x0 - 1e-6 && x <= x0 + TILE + 1e-6 && z >= z0 - 1e-6 && z <= z0 + TILE + 1e-6 ? t : -1;
}

/** Cardinal facing (0 +z, 1 +x, 2 -z, 3 -x) for a yaw (forward = (sin yaw, cos yaw)). */
export const cardinal = (yaw: number): number => ((Math.round(yaw / (Math.PI / 2)) % 4) + 4) % 4;

/**
 * Where a piece goes for a survivor at (x, y, z) looking (yaw, pitch):
 * walls on the facing edge of the tile they stand in, floors and ramps in
 * the tile ahead (floors go overhead when looking up). A ramp ahead of a
 * ramp the survivor is climbing continues from its top (ramp rush).
 */
export function planBuild(kind: BuildKind, x: number, y: number, z: number, yaw: number, pitch: number, builds: readonly Build[]): BuildSlot {
  const face = cardinal(yaw);
  const fx = [0, 1, 0, -1][face], fz = [1, 0, -1, 0][face];
  let level = Math.floor((y + 0.05) / SUB);
  if (kind === 0) {
    let gx = tileOf(x), gz = tileOf(z), dir = face;
    // Canonical edge: -z / -x edges belong to the neighbouring tile.
    if (dir === 2) { gz -= 1; dir = 0; } else if (dir === 3) { gx -= 1; dir = 1; }
    return { kind, gx: Math.max(0, gx), gz: Math.max(0, gz), level, dir };
  }
  const reach = kind === 1 ? TILE * 0.75 : TILE * 0.55;
  const gx = tileOf(x + fx * reach), gz = tileOf(z + fz * reach);
  if (kind === 1) return { kind, gx, gz, level: pitch > 0.5 ? level + LEVEL_SUBS : level, dir: 0 };
  // Ramp rush: continue from the top of the ramp behind the target tile.
  for (const b of builds) {
    if (b.kind !== 2 || b.dir !== face || b.gx !== gx - fx || b.gz !== gz - fz) continue;
    const top = b.level + LEVEL_SUBS;
    if (top * SUB >= y - 0.6 && top * SUB <= y + LEVEL) { level = top; break; }
  }
  return { kind, gx, gz, level, dir: face };
}

const overlap = (a: Box, b: Box, e: number) => a.minX < b.maxX - e && a.maxX > b.minX + e && a.minY < b.maxY - e && a.maxY > b.minY + e && a.minZ < b.maxZ - e && a.maxZ > b.minZ + e;
/** Two structures hold each other up when their bounds touch. */
export const touching = (a: Box, b: Box) => a.minX <= b.maxX + 0.06 && a.maxX >= b.minX - 0.06 && a.minY <= b.maxY + 0.06 && a.maxY >= b.minY - 0.06 && a.minZ <= b.maxZ + 0.06 && a.maxZ >= b.minZ - 0.06;

/** Is the slot inside the world and free of a same-kind piece? */
export function slotFree(slot: BuildSlot, builds: readonly Build[]): boolean {
  if (slot.gx < 0 || slot.gz < 0 || slot.gx >= TILES || slot.gz >= TILES || slot.level < LEVEL_MIN || slot.level >= LEVEL_MIN + LEVEL_COUNT) return false;
  const { aabb } = buildGeometry({ ...slot, variant: 0 });
  for (const b of builds) if (b.kind === slot.kind && b.dir === slot.dir && overlap(aabb, b.aabb, 0.05)) return false;
  return true;
}

/** Sample points on a structure's footprint used to test whether it rests on the world. */
function footprint(b: Pick<Build, 'kind' | 'gx' | 'gz' | 'dir'>, out: [number, number][]): [number, number][] {
  out.length = 0;
  const x0 = tileX(b.gx), z0 = tileX(b.gz);
  if (b.kind === 0) {
    for (const s of [0.3, TILE / 2, TILE - 0.3]) out.push(b.dir === 0 ? [x0 + s, z0 + TILE] : [x0 + TILE, z0 + s]);
  } else for (const [a, c] of [[0.3, 0.3], [TILE - 0.3, 0.3], [0.3, TILE - 0.3], [TILE - 0.3, TILE - 0.3], [TILE / 2, TILE / 2]]) out.push([x0 + a, z0 + c]);
  return out;
}
const _pts: [number, number][] = [];
/** Rests on terrain or a static prop (`ground` is collide.groundAt bound to the world, without builds). */
export function grounded(b: Pick<Build, 'kind' | 'gx' | 'gz' | 'dir' | 'level'>, ground: (x: number, z: number, feet: number) => number): boolean {
  const y = baseY(b.level);
  for (const [x, z] of footprint(b, _pts)) if (ground(x, z, y + 0.6) >= y - 0.6) return true;
  return false;
}
/** Can a piece go in this slot: free, and touching the ground or another structure. */
export function canPlace(slot: BuildSlot, builds: readonly Build[], ground: (x: number, z: number, feet: number) => number): boolean {
  if (!slotFree(slot, builds)) return false;
  if (grounded(slot, ground)) return true;
  const { aabb } = buildGeometry({ ...slot, variant: 0 });
  return builds.some(b => touching(aabb, b.aabb));
}

/**
 * Structural integrity: everything not connected (through touching pieces)
 * to a grounded piece collapses. Returns the indices that fall, ascending.
 */
export function unsupported(builds: readonly Build[], ground: (x: number, z: number, feet: number) => number): number[] {
  const n = builds.length;
  const held = new Uint8Array(n);
  const queue: number[] = [];
  for (let i = 0; i < n; i++) if (grounded(builds[i], ground)) { held[i] = 1; queue.push(i); }
  while (queue.length) {
    const a = builds[queue.pop()!];
    for (let j = 0; j < n; j++) if (!held[j] && touching(a.aabb, builds[j].aabb)) { held[j] = 1; queue.push(j); }
  }
  const out: number[] = [];
  for (let i = 0; i < n; i++) if (!held[i]) out.push(i);
  return out;
}

// ------------------------------------------------------------ wire packing

/** One integer per structure (< 2^31): tile, height, orientation, kind, material, edit. */
export function packBuild(b: Pick<Build, 'gx' | 'gz' | 'level' | 'dir' | 'kind' | 'mat' | 'variant'>): number {
  return (((((b.gx * TILES + b.gz) * LEVEL_COUNT + (b.level - LEVEL_MIN)) * 4 + b.dir) * 3 + b.kind) * 3 + b.mat) * WALL_VARIANTS + b.variant;
}
export function unpackBuild(code: number): Pick<Build, 'gx' | 'gz' | 'level' | 'dir' | 'kind' | 'mat' | 'variant'> | null {
  if (!Number.isInteger(code) || code < 0 || code >= TILES * TILES * LEVEL_COUNT * 4 * 3 * 3 * WALL_VARIANTS) return null;
  let c = code;
  const variant = c % WALL_VARIANTS; c = Math.floor(c / WALL_VARIANTS);
  const mat = (c % 3) as Material; c = Math.floor(c / 3);
  const kind = (c % 3) as BuildKind; c = Math.floor(c / 3);
  const dir = c % 4; c = Math.floor(c / 4);
  const level = (c % LEVEL_COUNT) + LEVEL_MIN; c = Math.floor(c / LEVEL_COUNT);
  const gz = c % TILES, gx = Math.floor(c / TILES);
  if (kind === 0 && dir > 1) return null;
  if (kind === 1 && dir !== 0) return null;
  if (kind !== 0 && variant !== 0) return null;
  return { gx, gz, level, dir, kind, mat, variant };
}
