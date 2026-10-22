/**
 * Storm Royale worlds: a deterministic heightfield, named points of interest
 * built from Kenney kits plus procedural multi-storey buildings (stairs, upper
 * floors, roof access), a lake (and a river to the sea on Palm Isle),
 * colliders for every solid prop, loot spots (indoors too) and chests. Dead
 * City (`maps/deadcity.ts`) reuses the same architecture for a ruined
 * downtown. Every map also exports mode anchors (team bases, flags, the
 * zombies arena, a street nav graph; `maps/anchors.ts`). Pure data (no
 * three.js) so the host simulation, guests and tests all derive the identical
 * world from the map id.
 */
import { clamp, fbm, hashString, lerp, makeNoise, Rng, smoothstep } from '../common/math.ts';
import type { ShooterMapId } from './catalog.ts';
import { buildAnchors, type ModeAnchors } from './maps/anchors.ts';
import { deadCityHeight, populateDeadCity, DEAD_CITY_HINTS, DEAD_CITY_POIS } from './maps/deadcity.ts';

export type { AnchorHints, ModeAnchors, Flag, Spawn, Barricade, BuyWall, NavGraph, ZombieArena } from './maps/anchors.ts';
export { navNext, nearestNavNode } from './maps/anchors.ts';

export const HALF = 240;
export const GRID = 2;
export const SEA_LEVEL = 0;
/** The isle sea / river surface (the render plane sits here too). */
export const SEA_SURFACE = SEA_LEVEL - 0.25;
/** Model footprints in kit units [width x, height y, depth z] (see manifest.json). */
export const PROP_SIZE: Record<string, [number, number, number]> = {
  'house-building-type-a': [1.3, 0.83, 1.03], 'house-building-type-c': [1.29, 1.03, 1.03], 'house-building-type-d': [1.76, 1.24, 1.03],
  'house-building-type-f': [1.43, 1.14, 1.41], 'house-building-type-h': [1.3, 0.74, 0.92], 'house-building-type-k': [0.92, 1.15, 1.02],
  'house-building-type-m': [1.43, 0.74, 1.43], 'house-building-type-o': [1.27, 1.14, 1.03], 'house-building-type-r': [1.03, 1.14, 1.02],
  'house-building-type-t': [1.31, 1.16, 1.41], 'house-fence-1x3': [1.28, 0.27, 0.44],
  'ind-building-a': [2.08, 1.47, 1.24], 'ind-building-h': [1.32, 0.73, 1.31], 'ind-building-i': [1.03, 0.73, 1.3], 'ind-building-j': [1.03, 0.86, 1.3],
  'ind-building-k': [1.3, 0.77, 0.91], 'ind-building-o': [0.88, 0.92, 1.24], 'ind-shipping-container-a': [0.37, 0.35, 0.82],
  'ind-shipping-container-b': [0.37, 0.35, 0.82], 'ind-shipping-container-c': [0.37, 0.35, 0.82], 'ind-detail-tank': [0.85, 0.41, 0.52],
  'ind-water-tower': [0.85, 2.14, 0.83], 'ind-chimney-large': [1.08, 1.7, 1.08],
  'surv-barrel': [0.24, 0.34, 0.24], 'surv-box-large': [0.25, 0.25, 0.5], 'surv-box': [0.25, 0.25, 0.25], 'surv-fence-fortified': [0.5, 0.52, 0.05],
  'surv-metal-panel-screws': [0.5, 0.5, 0.06], 'surv-rock-a': [0.55, 0.39, 0.62], 'surv-rock-b': [0.83, 0.42, 0.72], 'surv-rock-c': [0.78, 0.51, 0.57],
  'surv-rock-sand-a': [0.55, 0.39, 0.48], 'surv-rock-sand-b': [0.8, 0.45, 0.79], 'surv-tent-canvas': [0.56, 0.49, 0.56], 'surv-tent': [0.56, 0.49, 0.56],
  'surv-structure-metal': [0.54, 0.5, 0.54], 'surv-workbench': [0.33, 0.29, 0.3], 'surv-tree-log': [0.25, 0.28, 1.0],
  'nat-rock_largeA': [0.78, 0.26, 1.02], 'nat-rock_largeC': [1.06, 0.32, 1.02], 'nat-rock_tallB': [0.76, 0.88, 0.77], 'nat-rock_tallE': [0.46, 0.44, 0.53],
  'nat-stone_tallB': [0.76, 0.88, 0.77], 'nat-log_large': [1.0, 0.42, 0.55],
  'car-sedan': [1.5, 1.3, 2.55], 'car-suv': [1.5, 1.3, 2.7], 'car-police': [1.5, 1.3, 3.1], 'car-delivery': [1.5, 1.65, 3.25],
  'surv-fence': [0.5, 0.52, 0.04], 'surv-campfire-pit': [0.28, 0.11, 0.27], 'house-planter': [0.4, 0.18, 0.3], 'house-fence-low': [1.28, 0.17, 0.84],
};
/** Trees collide only with their trunk. */
const TRUNK = new Set(['nat-tree_default', 'nat-tree_oak', 'nat-tree_detailed', 'nat-tree_pineTallA', 'nat-tree_pineRoundC', 'nat-tree_palmTall', 'nat-tree_palmBend', 'nat-tree_palmDetailedShort', 'surv-tree', 'surv-tree-tall', 'surv-tree-autumn', 'house-tree-large', 'nat-cactus_tall']);
/** What the pickaxe harvests from a prop: 0 wood, 1 brick (stone), 2 metal. */
export function harvestOf(key: string): number {
  if (/rock|stone|chimney/.test(key)) return 1;
  if (/^car-|^ind-|barrel|metal|container|tank/.test(key)) return 2;
  return 0;
}

/** `burnt`: drawn charred (Dead City wrecks). */
export interface Prop { key: string; x: number; y: number; z: number; yaw: number; scale: number; burnt?: true }
export interface Box { minX: number; minY: number; minZ: number; maxX: number; maxY: number; maxZ: number }
export interface Cylinder { x: number; z: number; r: number; y0: number; y1: number }
export type PoiKind = 'town' | 'port' | 'farm' | 'camp' | 'hill' | 'factory' | 'forest' | 'lake'
  | 'tower' | 'hospital' | 'mall' | 'subway' | 'checkpoint' | 'park';
export interface Poi { name: string; x: number; z: number; r: number; kind: PoiKind }
/**
 * A piece of procedural architecture: also a collider in `boxes`. `mat` picks
 * the render palette (BLOCK_PALETTE) and what the pickaxe harvests from it.
 */
export interface Block extends Box { mat: number }
/**
 * Render palette for blocks: plaster, floor boards, roof, stairs, metal sheet,
 * concrete, brick trim, dock planks, barn red, then the Dead City set: rubble,
 * charred, sandbags, asphalt slab, hospital tile, dark glass, rust, hazard
 * paint, planter soil and olive drab.
 */
export const BLOCK_PALETTE = [
  0xe9e1cf, 0xa8774a, 0x7a4a3a, 0x9c6b42, 0x8d98a3, 0xb9b6ad, 0xb5574a, 0x8a6a48, 0xa8483a,
  0x8f8a82, 0x3b3532, 0xb3a27a, 0x505357, 0xdfe4e2, 0x34444f, 0x8a5a3c, 0xd2ae38, 0x5e6b3a, 0x59613f,
] as const;
/** Block material names (index = `Block.mat`), for the renderer's texture sets. */
export const BLOCK_KINDS = [
  'plaster', 'wood', 'roof', 'wood', 'metal', 'concrete', 'brick', 'wood', 'brick',
  'rubble', 'charred', 'sandbag', 'asphalt', 'tile', 'glass', 'rust', 'hazard', 'planter', 'metal',
] as const;
const BLOCK_HARVEST = [1, 0, 0, 0, 2, 1, 1, 0, 0, 1, 0, 0, 1, 1, 2, 2, 2, 0, 2];
/** Named block materials (indices into BLOCK_PALETTE). */
export const MAT = { plaster: 0, floor: 1, roof: 2, stair: 3, metal: 4, concrete: 5, brick: 6, deck: 7, barn: 8, rubble: 9, charred: 10, sandbag: 11, asphalt: 12, tile: 13, glass: 14, rust: 15, hazard: 16, planter: 17, olive: 18 } as const;
export interface Water { x: number; z: number; r: number; y: number }
export interface Spot { x: number; z: number; y?: number }
/**
 * A walk-in building's stair layout, so bots can climb to upper floors and the
 * roof (see `climbStep`): the local frame (centre, quarter turn, size), the
 * ground-floor height, storey count and the shared flight geometry.
 */
export interface Climb { cx: number; cz: number; q: number; w: number; d: number; base: number; floors: number; z0: number; run: number; sw: number; roof: number; box: Box; doors: number }
/**
 * A ground-floor door or window of a walk-in building (Zombies boards these
 * up): centre of the hole at its sill, outward facing (forward = sin/cos
 * yaw, like fighters), width and height.
 */
export interface Opening { x: number; y: number; z: number; yaw: number; w: number; h: number; door: boolean }
/** A fire the renderer keeps burning (wrecks, barrels, ruins); `s` scales it. */
export interface Fire { x: number; y: number; z: number; s: number }
/** A paved street segment (render + minimap): centre line from (x0, z0) to (x1, z1), full width `w`. */
export interface Road { x0: number; z0: number; x1: number; z1: number; w: number }
export type ShooterTheme = 'isle' | 'mesa' | 'city';
export interface ShooterMap {
  id: ShooterMapId; theme: ShooterTheme;
  heights: Float32Array; size: number;
  props: Prop[]; boxes: Box[]; cylinders: Cylinder[]; pois: Poi[];
  /** Harvest material per box (0 wood, 1 brick, 2 metal, -1 nothing), aligned with `boxes`. */
  boxMats: number[];
  /** The prop drawn for each tree trunk, aligned with `cylinders` (felled trees hide it). */
  trunkProps: number[];
  blocks: Block[]; waters: Water[]; river: [number, number][];
  lootSpots: Spot[]; chests: { x: number; z: number; yaw: number; y?: number }[];
  /** Walk-in buildings bots can climb, and the fixed launch pads (set pieces). */
  climbs: Climb[]; pads: { x: number; z: number; y: number }[];
  /** Ground-floor doors and windows, fires, streets (Dead City) and the anchors every mode spawns from. */
  openings: Opening[]; fires: Fire[]; roads: Road[];
  anchors: ModeAnchors;
}

const POIS: Record<Exclude<ShooterMapId, 'deadcity'>, Poi[]> = {
  isle: [
    { name: 'Harbor Town', x: -110, z: 80, r: 48, kind: 'town' },
    { name: 'Rustworks', x: 115, z: -115, r: 42, kind: 'factory' },
    { name: 'Container Port', x: 160, z: 70, r: 32, kind: 'port' },
    { name: 'Whispering Woods', x: -70, z: -130, r: 46, kind: 'forest' },
    { name: 'Lookout Hill', x: -165, z: -30, r: 26, kind: 'hill' },
    { name: 'Pine Farm', x: 70, z: 140, r: 30, kind: 'farm' },
    { name: 'Loot Lake', x: 62, z: -12, r: 32, kind: 'lake' },
    { name: 'Center Camp', x: -14, z: 20, r: 20, kind: 'camp' },
  ],
  mesa: [
    { name: 'Dust Town', x: -100, z: 70, r: 48, kind: 'town' },
    { name: 'Oil Works', x: 110, z: -110, r: 42, kind: 'factory' },
    { name: 'Canyon Camp', x: 110, z: 120, r: 28, kind: 'farm' },
    { name: 'Mesa Top', x: -140, z: -120, r: 26, kind: 'hill' },
    { name: 'Cactus Grove', x: -30, z: 150, r: 40, kind: 'forest' },
    { name: 'Oasis', x: 46, z: 10, r: 30, kind: 'lake' },
    { name: 'Crossroads', x: -40, z: -40, r: 20, kind: 'camp' },
  ],
};
/** Isle river: from Loot Lake east to the sea. */
const RIVER: [number, number][] = [[62, -12], [96, 4], [132, -14], [172, 2], [205, -8], [240, -2]];
const LAKE_BED = { isle: -2.8, mesa: 1 };
const MESA_LAKE_SURFACE = 3.6;

const cache = new Map<ShooterMapId, ShooterMap>();

/** Distance from (x, z) to a polyline. */
function polyDistance(path: [number, number][], x: number, z: number): number {
  let best = Infinity;
  for (let i = 0; i + 1 < path.length; i++) {
    const [ax, az] = path[i], [bx, bz] = path[i + 1];
    const dx = bx - ax, dz = bz - az, l2 = dx * dx + dz * dz;
    const t = clamp(((x - ax) * dx + (z - az) * dz) / l2, 0, 1);
    best = Math.min(best, Math.hypot(x - (ax + dx * t), z - (az + dz * t)));
  }
  return best;
}

export function mapFor(id: string): ShooterMap {
  const mapId: ShooterMapId = id === 'mesa' ? 'mesa' : id === 'deadcity' ? 'deadcity' : 'isle';
  const hit = cache.get(mapId);
  if (hit) return hit;
  if (mapId === 'deadcity') {
    const map = emptyMap(mapId, 'city', deadCityHeight, DEAD_CITY_POIS);
    populateDeadCity(map);
    map.anchors = buildAnchors(map, DEAD_CITY_HINTS);
    cache.set(mapId, map);
    return map;
  }
  const theme = mapId;
  const noise = makeNoise(hashString(mapId));
  const pois = POIS[mapId];
  const isle = theme === 'isle';
  const poiHeight = (p: Poi) => {
    if (p.kind === 'lake') return LAKE_BED[theme];
    if (isle) return p.kind === 'hill' ? 19 : p.kind === 'port' ? 1.6 : p.kind === 'factory' ? 5 : 4 + fbm(noise, p.x * 0.01, p.z * 0.01) * 2;
    return p.kind === 'hill' ? 24 : 6;
  };
  const raw = (x: number, z: number): number => {
    let h: number;
    if (isle) {
      const wobble = 1 + fbm(noise, x * 0.006 + 3, z * 0.006, 3) * 0.18;
      const d = Math.hypot(x, z) / (210 * wobble);
      const land = smoothstep(1.02, 0.78, d);
      h = land * (2.4 + (fbm(noise, x * 0.008, z * 0.008, 4) * 0.5 + 0.5) * 15) - (1 - land) * 7;
    } else {
      const base = (fbm(noise, x * 0.007, z * 0.007, 4) * 0.5 + 0.5) * 28;
      const step = 6, q = Math.floor(base / step) * step, f = (base - q) / step;
      h = q + smoothstep(0.75, 1, f) * step + 2;
      h += smoothstep(HALF - 40, HALF, Math.max(Math.abs(x), Math.abs(z))) * 34;
    }
    for (const p of pois) {
      // Forests keep most of their natural roll; everything else is levelled to build on.
      const k = smoothstep(p.r * 1.25, p.r * 0.75, Math.hypot(x - p.x, z - p.z)) * (p.kind === 'forest' ? 0.55 : 1);
      h = lerp(h, poiHeight(p), k);
    }
    if (isle) {
      // The river cuts a channel below the sea surface, with sloped banks.
      const d = polyDistance(RIVER, x, z);
      h = lerp(h, -1.8, smoothstep(10, 4.5, d));
    }
    return h;
  };
  const lake = pois.find(p => p.kind === 'lake')!;
  const map = emptyMap(mapId, theme, raw, pois);
  if (!isle) map.waters.push({ x: lake.x, z: lake.z, r: lake.r * 1.2, y: MESA_LAKE_SURFACE });
  if (isle) map.river = RIVER;
  populate(map);
  map.anchors = buildAnchors(map);
  cache.set(mapId, map);
  return map;
}

/** A map with its heightfield sampled from `raw` and nothing placed yet. */
function emptyMap(id: ShooterMapId, theme: ShooterTheme, raw: (x: number, z: number) => number, pois: Poi[]): ShooterMap {
  const size = HALF * 2 / GRID + 1;
  const heights = new Float32Array(size * size);
  for (let iz = 0; iz < size; iz++) for (let ix = 0; ix < size; ix++) heights[iz * size + ix] = raw(-HALF + ix * GRID, -HALF + iz * GRID);
  return {
    id, theme, heights, size, props: [], boxes: [], cylinders: [], pois, boxMats: [], trunkProps: [], blocks: [],
    waters: [], river: [], lootSpots: [], chests: [], climbs: [], pads: [], openings: [], fires: [], roads: [],
    anchors: { spawns: [], teamSpawns: [[], []], flags: [], zombies: { cx: 0, cz: 0, r: 0, start: [], spawns: [], barricades: [], buyWalls: [], extraction: { x: 0, y: 0, z: 0, r: 0 } }, nav: { nodes: [], links: [] } },
  };
}

/** Bilinear terrain height. */
export function terrainHeight(map: ShooterMap, x: number, z: number): number {
  const fx = clamp((x + HALF) / GRID, 0, map.size - 1.001), fz = clamp((z + HALF) / GRID, 0, map.size - 1.001);
  const ix = Math.floor(fx), iz = Math.floor(fz), tx = fx - ix, tz = fz - iz;
  const h = map.heights, s = map.size;
  const a = h[iz * s + ix], b = h[iz * s + ix + 1], c = h[(iz + 1) * s + ix], d = h[(iz + 1) * s + ix + 1];
  return lerp(lerp(a, b, tx), lerp(c, d, tx), tz);
}

/** Water surface at (x, z), or -Infinity where there is none (the isle sea covers everything low). */
export function waterLevel(map: ShooterMap, x: number, z: number): number {
  if (map.theme === 'isle') return SEA_SURFACE;
  for (const w of map.waters) if ((x - w.x) * (x - w.x) + (z - w.z) * (z - w.z) < w.r * w.r) return w.y;
  return -Infinity;
}
/** Dry land worth walking to (not under water). */
export const dryLand = (map: ShooterMap, x: number, z: number): boolean => terrainHeight(map, x, z) > waterLevel(map, x, z) + 0.8;

// ------------------------------------------------------------ architecture

const WT = 0.3;          // wall thickness
const SLAB = 0.3;        // floor slab thickness
export const STOREY = 3.6;
const RISE = STOREY / 10; // ten 0.36 m steps per flight (below the 0.6 m step-up)
const RUN = 0.45;

/**
 * Builds axis-aligned architecture in a local frame (front = +z) turned by a
 * quarter `q` and placed at (cx, cz). Every rectangle becomes a Block.
 */
export class Frame {
  private map: ShooterMap; private cx: number; private cz: number; private q: number;
  constructor(map: ShooterMap, cx: number, cz: number, q: number) { this.map = map; this.cx = cx; this.cz = cz; this.q = q; }
  world(lx: number, lz: number): [number, number] {
    switch (this.q & 3) {
      case 1: return [this.cx + lz, this.cz - lx];
      case 2: return [this.cx - lx, this.cz - lz];
      case 3: return [this.cx - lz, this.cz + lx];
      default: return [this.cx + lx, this.cz + lz];
    }
  }
  block(lx0: number, lz0: number, lx1: number, lz1: number, y0: number, y1: number, mat: number): void {
    if (lx1 - lx0 < 0.05 || lz1 - lz0 < 0.05 || y1 - y0 < 0.05) return;
    const [ax, az] = this.world(lx0, lz0), [bx, bz] = this.world(lx1, lz1);
    const b: Block = { minX: Math.min(ax, bx), minY: y0, minZ: Math.min(az, bz), maxX: Math.max(ax, bx), maxY: y1, maxZ: Math.max(az, bz), mat };
    this.map.blocks.push(b);
    this.map.boxes.push(b);
    this.map.boxMats.push(BLOCK_HARVEST[mat] ?? 0);
  }
  /** A wall along x (at lz0..lz1) from lx0 to lx1 with optional openings [centre, width, bottom, top]. */
  wallX(lx0: number, lx1: number, lz0: number, lz1: number, y0: number, y1: number, mat: number, holes: [number, number, number, number][] = []): void {
    let x = lx0;
    for (const [c, w, b, t] of holes.sort((a, b2) => a[0] - b2[0])) {
      this.block(x, lz0, c - w / 2, lz1, y0, y1, mat);
      this.block(c - w / 2, lz0, c + w / 2, lz1, y0, y0 + b, mat);
      this.block(c - w / 2, lz0, c + w / 2, lz1, y0 + t, y1, mat);
      x = c + w / 2;
    }
    this.block(x, lz0, lx1, lz1, y0, y1, mat);
  }
  /** A wall along z (at lx0..lx1). */
  wallZ(lz0: number, lz1: number, lx0: number, lx1: number, y0: number, y1: number, mat: number, holes: [number, number, number, number][] = []): void {
    let z = lz0;
    for (const [c, w, b, t] of holes.sort((a, b2) => a[0] - b2[0])) {
      this.block(lx0, z, lx1, c - w / 2, y0, y1, mat);
      this.block(lx0, c - w / 2, lx1, c + w / 2, y0, y0 + b, mat);
      this.block(lx0, c - w / 2, lx1, c + w / 2, y0 + t, y1, mat);
      z = c + w / 2;
    }
    this.block(lx0, z, lx1, lz1, y0, y1, mat);
  }
  /** A straight flight along z from `lz` (dir ±1), `n` steps from y0 rising `rise` each. */
  stairs(lx0: number, lx1: number, lz: number, dir: 1 | -1, y0: number, n: number, rise: number, mat: number): void {
    for (let k = 0; k < n; k++) {
      const a = lz + dir * k * RUN, b = lz + dir * (k + 1) * RUN;
      this.block(lx0, Math.min(a, b), lx1, Math.max(a, b), y0 - SLAB, y0 + (k + 1) * rise, mat);
    }
  }
}

export interface BuildingStyle { wall: number; floor: number; roof: number; stair: number; trim: number }
export const STYLES: Record<string, BuildingStyle> = {
  house: { wall: 0, floor: 1, roof: 2, stair: 3, trim: 6 },
  brick: { wall: 6, floor: 1, roof: 5, stair: 3, trim: 5 },
  barn: { wall: 8, floor: 1, roof: 2, stair: 3, trim: 7 },
  metal: { wall: 4, floor: 5, roof: 4, stair: 4, trim: 5 },
  // Dead City: concrete office blocks, the tiled hospital, the mall's glass and steel.
  office: { wall: 5, floor: 12, roof: 5, stair: 5, trim: 9 },
  tenement: { wall: 6, floor: 1, roof: 10, stair: 3, trim: 9 },
  hospital: { wall: 13, floor: 5, roof: 5, stair: 5, trim: 5 },
  mall: { wall: 14, floor: 5, roof: 4, stair: 4, trim: 5 },
};
/**
 * Ruined walk-ins (Dead City): random upper-storey windows blown out into
 * walk-through breaches (plus the `breaches` asked for, `[storey, side]` with
 * side 0 front, 1 back, 2 left, 3 right), a broken parapet with jagged stubs,
 * and now and then a fire on an upper floor.
 */
export interface BuildingOptions { ruin?: boolean; breaches?: [number, number][] }
const _wrapYaw = (a: number): number => Math.atan2(Math.sin(a), Math.cos(a));

/**
 * A walk-in building: foundation, door front and back, windows on every
 * side, switchback stairs to each upper floor and onto a parapet roof. Adds
 * a loot spot on every floor; returns the roof height.
 */
export function building(map: ShooterMap, cx: number, cz: number, q: number, w: number, d: number, floors: number, style: BuildingStyle, rng: Rng, opts: BuildingOptions = {}): number {
  const f = new Frame(map, cx, cz, q);
  let lo = Infinity, hi = -Infinity;
  for (const [lx, lz] of [[-w / 2, -d / 2], [w / 2, -d / 2], [-w / 2, d / 2], [w / 2, d / 2], [0, 0]]) {
    const [x, z] = f.world(lx, lz);
    const h = terrainHeight(map, x, z);
    lo = Math.min(lo, h); hi = Math.max(hi, h);
  }
  const base = hi + 0.25;
  f.block(-w / 2, -d / 2, w / 2, d / 2, lo - 1, base, style.trim);
  // Door steps where the foundation stands proud of the ground; a door more than the steps (plus a step-up) above the ground is out of reach.
  let doors = 0;
  for (const side of [1, -1]) {
    const [x, z] = f.world(0, side * (d / 2 + 0.6));
    const drop = base - terrainHeight(map, x, z);
    if (drop <= 2.5) doors |= side > 0 ? 1 : 2;
    for (let k = 1; drop - k * 0.4 > 0.15 && k < 6; k++) f.block(-1.2, side > 0 ? d / 2 + (k - 1) * 0.5 : -d / 2 - k * 0.5, 1.2, side > 0 ? d / 2 + k * 0.5 : -d / 2 - (k - 1) * 0.5, lo - 1, base - k * 0.4, style.trim);
  }
  const sw = 1.3; // stair width
  const run = 10 * RUN;
  // Each flight starts a one-metre landing in from the back wall and climbs toward the front.
  const z0 = -d / 2 + WT + 1.0;
  for (let s = 0; s <= floors; s++) {
    const y = base + s * STOREY;
    const roof = s === floors;
    // The flight that arrives at this level runs on the left (even storeys) or right side.
    const below = s - 1;
    const holeX0 = below % 2 === 0 ? -w / 2 + WT : w / 2 - WT - sw, holeX1 = holeX0 + sw;
    if (s > 0) {
      // Slab with a stairwell hole over the flight from the storey below.
      const mat = roof ? style.roof : style.floor;
      f.block(-w / 2, -d / 2, holeX0, d / 2, y - SLAB, y, mat);
      f.block(holeX1, -d / 2, w / 2, d / 2, y - SLAB, y, mat);
      f.block(holeX0, -d / 2, holeX1, z0, y - SLAB, y, mat);
      f.block(holeX0, z0 + run, holeX1, d / 2, y - SLAB, y, mat);
    }
    if (roof) {
      // Parapet: waist-high cover around the roof (ruins: broken runs and jagged column stubs).
      if (opts.ruin) {
        const cut = (a: number, b: number): [number, number] => { const k = rng.next(); return k < 0.3 ? [a, lerp(a, b, rng.range(0.25, 0.6))] : k < 0.55 ? [lerp(a, b, rng.range(0.35, 0.7)), b] : [a, b]; };
        let [a, b] = cut(-w / 2, w / 2); f.wallX(a, b, d / 2 - WT, d / 2, y, y + 1.1, style.trim);
        [a, b] = cut(-w / 2, w / 2); f.wallX(a, b, -d / 2, -d / 2 + WT, y, y + 1.1, style.trim);
        [a, b] = cut(-d / 2 + WT, d / 2 - WT); f.wallZ(a, b, -w / 2, -w / 2 + WT, y, y + 1.1, style.trim);
        [a, b] = cut(-d / 2 + WT, d / 2 - WT); f.wallZ(a, b, w / 2 - WT, w / 2, y, y + 1.1, style.trim);
        for (const [sx, sz] of [[-1, -1], [1, -1], [1, 1], [-1, 1]]) {
          if (rng.chance(0.35)) continue;
          f.block(sx * w / 2 - (sx > 0 ? 0.7 : 0), sz * d / 2 - (sz > 0 ? 0.7 : 0), sx * w / 2 + (sx < 0 ? 0.7 : 0), sz * d / 2 + (sz < 0 ? 0.7 : 0), y, y + rng.range(1.4, 4.2), style.trim);
        }
        if (rng.chance(0.3)) {
          const [fx, fz] = f.world(rng.range(-w / 4, w / 4), rng.range(0, d / 4));
          map.fires.push({ x: fx, y, z: fz, s: rng.range(0.8, 1.3) });
        }
      } else {
        f.wallX(-w / 2, w / 2, d / 2 - WT, d / 2, y, y + 1.1, style.trim);
        f.wallX(-w / 2, w / 2, -d / 2, -d / 2 + WT, y, y + 1.1, style.trim);
        f.wallZ(-d / 2 + WT, d / 2 - WT, -w / 2, -w / 2 + WT, y, y + 1.1, style.trim);
        f.wallZ(-d / 2 + WT, d / 2 - WT, w / 2 - WT, w / 2, y, y + 1.1, style.trim);
      }
      break;
    }
    const top = y + STOREY - SLAB;
    const win = (c: number): [number, number, number, number] => [c, 1.6, 1.0, 2.3];
    const door = (c: number): [number, number, number, number] => [c, 1.7, 0, 2.5];
    // A blown-out window: floor to near the ceiling, wide enough to walk through.
    const breach = (c: number): [number, number, number, number] => [c, 3.0, 0, 2.9];
    const holes: [number, number, number, number][][] = s === 0
      ? [[door(w * 0.15), win(-w * 0.3)], [door(-w * 0.15), win(w * 0.3)], [win(d * 0.2)], [win(-d * 0.2)]]
      : [[win(-w * 0.25), win(w * 0.25)], [win(0)], [win(d * 0.2)], [win(-d * 0.2)]];
    if (s > 0) for (let side = 0; side < 4; side++) {
      const asked = opts.breaches?.some(([fl, sd]) => fl === s && sd === side) ?? false;
      if (asked || (opts.ruin && rng.chance(0.3))) holes[side] = holes[side].map(h => breach(h[0]));
    }
    if (s === 0) {
      // Ground-floor doors and windows, outward-facing, for the zombies' boards.
      const sides: [number, number][] = [[0, 0], [Math.PI, 0], [-Math.PI / 2, 0], [Math.PI / 2, 0]];
      holes.forEach((list, side) => {
        for (const [c, hw, b, t] of list) {
          const [lx, lz] = side === 0 ? [c, d / 2] : side === 1 ? [c, -d / 2] : side === 2 ? [-w / 2, c] : [w / 2, c];
          const [x, z] = f.world(lx, lz);
          map.openings.push({ x, y: y + b, z, yaw: _wrapYaw(sides[side][0] + q * Math.PI / 2), w: hw, h: t - b, door: b === 0 });
        }
      });
    }
    f.wallX(-w / 2, w / 2, d / 2 - WT, d / 2, y, top, style.wall, holes[0]);
    f.wallX(-w / 2, w / 2, -d / 2, -d / 2 + WT, y, top, style.wall, holes[1]);
    f.wallZ(-d / 2 + WT, d / 2 - WT, -w / 2, -w / 2 + WT, y, top, style.wall, holes[2]);
    f.wallZ(-d / 2 + WT, d / 2 - WT, w / 2 - WT, w / 2, y, top, style.wall, holes[3]);
    // This storey's flight (left on even storeys, right on odd), back to front.
    const sx0 = s % 2 === 0 ? -w / 2 + WT : w / 2 - WT - sw;
    f.stairs(sx0, sx0 + sw, z0, 1, y, 10, RISE, style.stair);
    // Loot and the odd chest go on the side away from this storey's own flight.
    const [lx, lz] = f.world(s % 2 === 0 ? w * 0.15 : -w * 0.15, 0);
    map.lootSpots.push({ x: lx, z: lz, y });
    if (s > 0 && rng.chance(0.35)) {
      const [chx, chz] = f.world(s % 2 === 0 ? w * 0.25 : -w * 0.25, d * 0.25);
      map.chests.push({ x: chx, z: chz, yaw: q * Math.PI / 2, y });
    }
  }
  const [ax, az] = f.world(-w / 2, -d / 2), [bx, bz] = f.world(w / 2, d / 2);
  map.climbs.push({
    cx, cz, q, w, d, base, floors, z0, run, sw, roof: base + floors * STOREY,
    box: { minX: Math.min(ax, bx), minY: lo, minZ: Math.min(az, bz), maxX: Math.max(ax, bx), maxY: base + floors * STOREY + 1.1, maxZ: Math.max(az, bz) }, doors,
  });
  return base + floors * STOREY;
}

/** World (x, z) to a climb's local frame (front = +z), inverting Frame.world. */
function toLocal(c: Climb, x: number, z: number): [number, number] {
  const dx = x - c.cx, dz = z - c.cz;
  switch (c.q & 3) {
    case 1: return [-dz, dx];
    case 2: return [-dx, -dz];
    case 3: return [dz, -dx];
    default: return [dx, dz];
  }
}
function toWorld(c: Climb, lx: number, lz: number, out: [number, number]): [number, number] {
  switch (c.q & 3) {
    case 1: out[0] = c.cx + lz; out[1] = c.cz - lx; break;
    case 2: out[0] = c.cx - lx; out[1] = c.cz - lz; break;
    case 3: out[0] = c.cx - lz; out[1] = c.cz + lx; break;
    default: out[0] = c.cx + lx; out[1] = c.cz + lz;
  }
  return out;
}
/** Is (x, z) inside the building's footprint (plus `pad` metres)? */
export const inClimb = (c: Climb, x: number, z: number, pad = 0): boolean =>
  x > c.box.minX - pad && x < c.box.maxX + pad && z > c.box.minZ - pad && z < c.box.maxZ + pad;

const _step: [number, number] = [0, 0];
/**
 * Where a survivor at (x, y, z) should walk next to reach a goal at
 * (goalX, goalY, goalZ) through this building: round the corner to a door,
 * through it, along the back wall to the flight that goes up from this storey
 * and up it (switchback stairs, see `building`), or down through the stairwell
 * hole it arrived by and out of the door on the goal's side. Null when no
 * help is needed (on the goal's floor, or outside with the goal outside too):
 * walk straight to the goal. A goal height of NaN means "on the ground".
 * Allocation-free: the returned pair is reused.
 */
export function climbStep(c: Climb, x: number, y: number, z: number, goalX: number, goalY: number, goalZ: number): [number, number] | null {
  const [lx, lz] = toLocal(c, x, z);
  const hw = c.w / 2, hd = c.d / 2, door = c.w * 0.15;
  const goalIn = inClimb(c, goalX, goalZ);
  if (!(Math.abs(lx) < hw && Math.abs(lz) < hd && y > c.base - 0.4)) {
    if (!goalIn) return null;
    // Outside: the nearer door that can be reached (front +z, back -z); round the corners to it first.
    let side = lz >= 0 ? 1 : -1;
    if (!(c.doors & (side > 0 ? 1 : 2))) side = -side;
    if (!(c.doors & (side > 0 ? 1 : 2))) return null;
    const sx = lx >= 0 ? 1 : -1, dx = side * door, face = lz * side;
    if (face < -hd + 0.2 && Math.abs(lx) <= hw + 0.3) return toWorld(c, sx * (hw + 1.6), -side * (hd + 1.8), _step);
    if (Math.abs(lx) > hw + 0.3 && face < hd + 1.2) return toWorld(c, sx * (hw + 1.6), side * (hd + 1.8), _step);
    if (Math.abs(lx - dx) > 0.7 || face < hd) return toWorld(c, dx, side * (hd + 1.6), _step);
    return toWorld(c, dx, side * (hd - 1.4), _step);
  }
  // A goal outside the building (or on the ground) is reached from the ground floor.
  const want = goalIn && Number.isFinite(goalY) ? goalY : c.base;
  // Storey under the feet (on a flight it is the one the flight leaves from).
  const storey = Math.max(0, Math.min(c.floors, Math.floor((y - c.base + 0.4) / STOREY)));
  const floorY = c.base + storey * STOREY;
  const flightX = (s: number) => (s % 2 === 0 ? -hw + 0.3 : hw - 0.3 - c.sw) + c.sw / 2;
  const fx = flightX(storey), top = c.z0 + c.run + 0.7, back = c.z0 - 0.45;
  const onFlight = storey < c.floors && Math.abs(lx - fx) < c.sw / 2 + 0.25 && lz > c.z0 - 0.3 && lz < c.z0 + c.run + 0.3 && y > floorY + 0.25;
  if (onFlight) return toWorld(c, fx, want > floorY + 1.2 ? top : back, _step);
  if (Math.abs(want - floorY) < 1.2) {
    if (goalIn || storey > 0) return null;
    // Ground floor, goal outside: out of the door on the goal's side (or the one that is usable).
    let side = toLocal(c, goalX, goalZ)[1] >= 0 ? 1 : -1;
    if (!(c.doors & (side > 0 ? 1 : 2)) && c.doors) side = -side;
    // Beside a flight: step out to the centre line first so the way to the door does not run up the stairs.
    if (Math.abs(lx) > 1.2 && lz < c.z0 + c.run + 0.3) return toWorld(c, 0, lz, _step);
    return toWorld(c, side * door, side * (hd + 2.2), _step);
  }
  if (want > floorY) {
    if (storey >= c.floors) return null;
    // Centre line to the back wall, along it to the flight, then up.
    if (lz > c.z0 + 0.2) return toWorld(c, 0, Math.abs(lx) > 1.2 ? lz : back, _step);
    return toWorld(c, fx, Math.abs(lx - fx) > 0.5 ? back : top, _step);
  }
  // Down: drop through the hole over the flight that arrives at this storey.
  return toWorld(c, flightX(storey - 1), c.z0 + c.run * 0.5, _step);
}

/** Big open hall: wide doors, high windows, a catwalk along the back reached by stairs. */
export function factory(map: ShooterMap, cx: number, cz: number, q: number, w: number, d: number, style: BuildingStyle): number {
  const f = new Frame(map, cx, cz, q);
  let lo = Infinity, hi = -Infinity;
  for (const [lx, lz] of [[-w / 2, -d / 2], [w / 2, -d / 2], [-w / 2, d / 2], [w / 2, d / 2]]) {
    const [x, z] = f.world(lx, lz);
    const h = terrainHeight(map, x, z);
    lo = Math.min(lo, h); hi = Math.max(hi, h);
  }
  const y = hi + 0.2, H = 9, cat = 4.5, catD = 4;
  f.block(-w / 2, -d / 2, w / 2, d / 2, lo - 1, y, style.trim);
  const big = (c: number): [number, number, number, number] => [c, 5, 0, 5.2];
  const high = (c: number): [number, number, number, number] => [c, 3, 5.6, 7.6];
  for (const [lx, lz, yaw, hw] of [[-w * 0.22, d / 2, 0, 5], [w * 0.22, d / 2, 0, 5], [0, -d / 2, Math.PI, 5], [-w / 2, d * 0.15, -Math.PI / 2, 3.4]]) {
    const [x, z] = f.world(lx, lz);
    map.openings.push({ x, y, z, yaw: _wrapYaw(yaw + q * Math.PI / 2), w: hw, h: hw === 5 ? 5.2 : 3.4, door: true });
  }
  f.wallX(-w / 2, w / 2, d / 2 - WT, d / 2, y, y + H, style.wall, [big(-w * 0.22), big(w * 0.22)]);
  f.wallX(-w / 2, w / 2, -d / 2, -d / 2 + WT, y, y + H, style.wall, [big(0), high(-w * 0.32), high(w * 0.32)]);
  f.wallZ(-d / 2 + WT, d / 2 - WT, -w / 2, -w / 2 + WT, y, y + H, style.wall, [[d * 0.15, 3.4, 0, 3.4], high(-d * 0.25)]);
  f.wallZ(-d / 2 + WT, d / 2 - WT, w / 2 - WT, w / 2, y, y + H, style.wall, [high(0)]);
  // Roof with a skylight strip.
  f.block(-w / 2, -d / 2, -2, d / 2, y + H, y + H + SLAB, style.roof);
  f.block(2, -d / 2, w / 2, d / 2, y + H, y + H + SLAB, style.roof);
  // Catwalk along the back wall plus a railing, and the stair up to it.
  const zc = -d / 2 + WT + catD;
  f.block(-w / 2 + WT, -d / 2 + WT, w / 2 - WT, zc, y + cat - SLAB, y + cat, style.floor);
  f.wallX(-w / 2 + WT + 1.5, w / 2 - WT, zc - 0.15, zc, y + cat, y + cat + 1.0, style.trim);
  const n = Math.round(cat / 0.375);
  f.stairs(-w / 2 + WT, -w / 2 + WT + 1.5, zc + n * RUN, -1, y, n, cat / n, style.stair);
  for (const lx of [-w * 0.3, 0, w * 0.3]) { const [x, z] = f.world(lx, -d / 2 + 2.2); map.lootSpots.push({ x, z, y: y + cat }); }
  const [chx, chz] = f.world(w * 0.36, -d / 2 + 2.2);
  map.chests.push({ x: chx, z: chz, yaw: q * Math.PI / 2, y: y + cat });
  for (const lx of [-w * 0.25, w * 0.25]) { const [x, z] = f.world(lx, d * 0.2); map.lootSpots.push({ x, z, y }); }
  return y;
}

/** A plank pier from the shore out over the water (segments follow the ground on land). */
function dock(map: ShooterMap, x0: number, z0: number, dx: number, dz: number, length: number, water: number): void {
  // Short segments so the climb from the water end back to the shore stays a step.
  for (let s = 0; s < length; s += 0.75) {
    const x = x0 + dx * s, z = z0 + dz * s;
    const deck = Math.max(water + 0.55, terrainHeight(map, x, z) + 0.15);
    const hx = Math.abs(dx) > 0.5 ? 0.4 : 1.4, hz = Math.abs(dx) > 0.5 ? 1.4 : 0.4;
    const b: Block = { minX: x - hx, minY: Math.min(deck - 0.3, terrainHeight(map, x, z) - 1), minZ: z - hz, maxX: x + hx, maxY: deck, maxZ: z + hz, mat: 7 };
    map.blocks.push(b); map.boxes.push(b); map.boxMats.push(0);
  }
}

/** A footbridge over the river: decks interpolate between the banks in short steps. */
function bridge(map: ShooterMap, x: number, zc: number, half: number): void {
  const za = zc - half, zb = zc + half;
  const ha = terrainHeight(map, x, za), hb = terrainHeight(map, x, zb);
  for (let z = za; z < zb; z += 1) {
    const t = (z + 0.5 - za) / (zb - za);
    const deck = Math.max(SEA_SURFACE + 1.2, lerp(ha, hb, t) + 0.1);
    const b: Block = { minX: x - 2, minY: deck - 0.35, minZ: z, maxX: x + 2, maxY: deck, maxZ: z + 1, mat: 7 };
    map.blocks.push(b); map.boxes.push(b); map.boxMats.push(0);
    for (const side of [-1, 1]) {
      const rail: Block = { minX: x + side * 2 - 0.12, minY: deck, minZ: z, maxX: x + side * 2 + 0.12, maxY: deck + 1, maxZ: z + 1, mat: 3 };
      map.blocks.push(rail); map.boxes.push(rail); map.boxMats.push(0);
    }
  }
}

/**
 * Prop placement for a map: `free` checks the ground is dry, inside the play
 * area and clear of colliders; `place` adds a kit prop with its collider (a
 * box squared to the nearest quarter turn, or a trunk for trees). `y` places
 * the prop on a floor (indoors), skipping the clearance check against it;
 * `burnt` draws it charred.
 */
export function makePlacer(map: ShooterMap) {
  const ground = (x: number, z: number) => terrainHeight(map, x, z);
  const wet = (x: number, z: number) => ground(x, z) < waterLevel(map, x, z) + 0.6;
  const free = (x: number, z: number, r: number) => {
    if (Math.abs(x) > HALF - 8 || Math.abs(z) > HALF - 8) return false;
    if (wet(x, z)) return false;
    for (const b of map.boxes) if (x > b.minX - r && x < b.maxX + r && z > b.minZ - r && z < b.maxZ + r) return false;
    for (const c of map.cylinders) if (Math.hypot(x - c.x, z - c.z) < c.r + r) return false;
    return true;
  };
  const place = (key: string, x: number, z: number, yaw: number, scale: number, solid = true, y?: number, burnt = false): boolean => {
    const gy = y ?? ground(x, z);
    const size = PROP_SIZE[key];
    if (solid && size) {
      const quarter = Math.round(yaw / (Math.PI / 2));
      const swap = Math.abs(quarter) % 2 === 1;
      const w = (swap ? size[2] : size[0]) * scale / 2, d = (swap ? size[0] : size[2]) * scale / 2;
      if (y === undefined && !free(x, z, 0.2)) return false;
      map.boxes.push({ minX: x - w, minY: gy - 1, minZ: z - d, maxX: x + w, maxY: gy + size[1] * scale, maxZ: z + d });
      map.boxMats.push(harvestOf(key));
      yaw = quarter * (Math.PI / 2);
    } else if (solid && TRUNK.has(key)) {
      if (!free(x, z, 0.8)) return false;
      map.cylinders.push({ x, z, r: 0.45 * scale / 6, y0: gy - 1, y1: gy + 7 * scale / 6 });
      map.trunkProps.push(map.props.length);
    }
    const prop: Prop = { key, x, y: gy, z, yaw, scale };
    if (burnt) prop.burnt = true;
    map.props.push(prop);
    return true;
  };
  return { ground, wet, free, place };
}

function populate(map: ShooterMap): void {
  const rng = new Rng(hashString(map.id) ^ 0xa11ce);
  const isle = map.theme === 'isle';
  const { ground, wet, free, place } = makePlacer(map);
  const lootNear = (x: number, z: number) => map.lootSpots.push({ x, z });
  const houses = ['house-building-type-a', 'house-building-type-c', 'house-building-type-d', 'house-building-type-f', 'house-building-type-h', 'house-building-type-k', 'house-building-type-m', 'house-building-type-o', 'house-building-type-r', 'house-building-type-t'];
  const cars = ['car-sedan', 'car-suv', 'car-police', 'car-delivery'];
  const forestTrees = isle ? ['nat-tree_pineTallA', 'nat-tree_pineRoundC', 'nat-tree_oak', 'nat-tree_detailed', 'surv-tree-tall'] : ['nat-cactus_tall', 'surv-tree-autumn', 'nat-cactus_tall', 'surv-tree'];
  // Architecture first so props keep clear of it.
  if (isle) { bridge(map, 108, 0, 12); bridge(map, 188, -3, 12); }
  for (const poi of map.pois) {
    if (poi.kind === 'town') {
      // Town hall in the square, walk-in houses on the corners, kit houses between.
      building(map, poi.x, poi.z - 4, 0, 14, 11, 3, STYLES.brick, rng);
      map.chests.push({ x: poi.x + 4, z: poi.z + 6.5, yaw: 0.3 });
      for (let gx = -1; gx <= 1; gx++) for (let gz = -1; gz <= 1; gz++) {
        if (gx === 0 && gz === 0) continue;
        const x = poi.x + gx * 30, z = poi.z + gz * 30;
        const yaw = gz < 0 ? 0 : gz > 0 ? Math.PI : gx < 0 ? Math.PI / 2 : -Math.PI / 2;
        const q = gz < 0 ? 0 : gz > 0 ? 2 : gx < 0 ? 1 : 3;
        if (gx !== 0 && gz !== 0) building(map, x, z, q, 12, 10, rng.chance(0.5) ? 2 : 3, rng.chance(0.5) ? STYLES.house : STYLES.brick, rng);
        else place(rng.pick(houses), x, z, yaw, 8.5);
        lootNear(x + Math.sin(yaw) * 8, z + Math.cos(yaw) * 8);
        if (rng.chance(0.6)) place(rng.pick(cars), x + Math.sin(yaw) * 12 + Math.cos(yaw) * 5, z + Math.cos(yaw) * 12 - Math.sin(yaw) * 5, yaw + Math.PI / 2, 1.8);
        if (rng.chance(0.5)) place('house-fence-1x3', x - Math.sin(yaw) * 7.5 + Math.cos(yaw) * 7, z - Math.cos(yaw) * 7.5 - Math.sin(yaw) * 7, yaw, 4, true);
      }
      lootNear(poi.x - 9, poi.z + 9);
      for (let i = 0; i < 10; i++) place(rng.pick(['house-tree-large', 'nat-tree_oak']), poi.x + rng.range(-44, 44), poi.z + rng.range(-44, 44), rng.range(0, 6.28), 6);
    } else if (poi.kind === 'factory') {
      const floor = factory(map, poi.x, poi.z, 0, 34, 22, STYLES.metal);
      place('ind-chimney-large', poi.x + 24, poi.z - 6, 0, 9);
      place('ind-water-tower', poi.x - 26, poi.z + 18, 0, 7);
      place('ind-detail-tank', poi.x + 22, poi.z + 18, 0, 8);
      building(map, poi.x - 24, poi.z - 22, 1, 10, 9, 2, STYLES.brick, rng);
      for (let i = 0; i < 8; i++) {
        const x = poi.x - 14 + (i % 4) * 9 + rng.range(-1, 1), z = poi.z + 20 + Math.floor(i / 4) * 9;
        place(rng.pick(['ind-shipping-container-a', 'ind-shipping-container-b', 'ind-shipping-container-c']), x, z, rng.chance(0.5) ? 0 : Math.PI / 2, 8);
        if (i % 3 === 0) lootNear(x + 4.5, z);
      }
      // Crates and barrels on the shop floor for cover (clear of the stair and the doors).
      for (let i = 0; i < 7; i++) place(rng.pick(['surv-barrel', 'surv-box-large', 'surv-box']), poi.x - 9 + i * 3.6 + rng.range(-0.6, 0.6), poi.z + rng.range(-2, 3), 0, 5, true, floor);
    } else if (poi.kind === 'port') {
      place('ind-building-a', poi.x - 14, poi.z - 12, 0, 9);
      place('ind-building-k', poi.x + 16, poi.z - 14, 0, 9);
      place('ind-water-tower', poi.x + 18, poi.z + 16, 0, 7);
      for (let i = 0; i < 9; i++) {
        const x = poi.x - 14 + (i % 3) * 9 + rng.range(-1, 1), z = poi.z + 4 + Math.floor(i / 3) * 9;
        const key = rng.pick(['ind-shipping-container-a', 'ind-shipping-container-b', 'ind-shipping-container-c']);
        const yaw = rng.chance(0.5) ? 0 : Math.PI / 2;
        if (place(key, x, z, yaw, 8) && rng.chance(0.4)) {
          // Stacked container: a second box on top of the first.
          const gy = ground(x, z) + 0.35 * 8;
          const size = PROP_SIZE[key];
          const swap = yaw !== 0;
          const w = (swap ? size[2] : size[0]) * 4, d = (swap ? size[0] : size[2]) * 4;
          map.boxes.push({ minX: x - w, minY: gy, minZ: z - d, maxX: x + w, maxY: gy + size[1] * 8, maxZ: z + d });
          map.boxMats.push(2);
          map.props.push({ key: rng.pick(['ind-shipping-container-a', 'ind-shipping-container-b', 'ind-shipping-container-c']), x, y: gy, z, yaw, scale: 8 });
        }
        if (i % 2 === 0) lootNear(x + 4.5, z);
      }
      place('ind-detail-tank', poi.x + 8, poi.z - 26, 0, 8);
      map.chests.push({ x: poi.x + 2, z: poi.z - 2, yaw: 0 }, { x: poi.x - 22, z: poi.z + 18, yaw: 1.2 });
    } else if (poi.kind === 'farm') {
      building(map, poi.x - 6, poi.z - 14, 0, 14, 10, 2, STYLES.barn, rng);
      for (let i = 0; i < 4; i++) {
        const a = (i / 4) * Math.PI * 2 + 0.4;
        const x = poi.x + Math.cos(a) * 16, z = poi.z + 6 + Math.sin(a) * 12;
        place(rng.chance(0.5) ? 'surv-tent-canvas' : 'surv-tent', x, z, -a + Math.PI / 2, 8);
        lootNear(poi.x + Math.cos(a) * 8, poi.z + 6 + Math.sin(a) * 6);
      }
      for (let i = 0; i < 10; i++) place(rng.pick(['surv-barrel', 'surv-box', 'surv-box-large', 'surv-workbench']), poi.x + rng.range(-24, 24), poi.z + rng.range(-4, 24), rng.chance(0.5) ? 0 : Math.PI / 2, 5);
      for (let i = 0; i < 10; i++) place('surv-fence-fortified', poi.x - 26 + i * 2.4 * 2, poi.z + 26, 0, 4.8);
      place('surv-tree-log', poi.x + 12, poi.z + 2, 0, 6);
      map.chests.push({ x: poi.x + 4, z: poi.z + 4, yaw: 0.8 });
    } else if (poi.kind === 'camp') {
      for (let i = 0; i < 4; i++) {
        const a = (i / 4) * Math.PI * 2;
        place('surv-structure-metal', poi.x + Math.cos(a) * 9, poi.z + Math.sin(a) * 9, 0, 7);
      }
      for (let i = 0; i < 8; i++) place(rng.pick(['surv-box', 'surv-box-large', 'surv-barrel', 'surv-metal-panel-screws']), poi.x + rng.range(-16, 16), poi.z + rng.range(-16, 16), rng.chance(0.5) ? 0 : Math.PI / 2, 5);
      lootNear(poi.x, poi.z); lootNear(poi.x + 5, poi.z - 5); lootNear(poi.x - 6, poi.z + 4);
      map.chests.push({ x: poi.x + 1, z: poi.z + 6, yaw: 2 });
    } else if (poi.kind === 'forest') {
      // A cabin in a clearing, a ring of dense woods with logs and stumps.
      building(map, poi.x, poi.z, rng.int(4), 9, 8, 1, STYLES.house, rng);
      map.chests.push({ x: poi.x + 7, z: poi.z + 7, yaw: 0.5 });
      for (let n = 0; n < 150; n++) {
        const a = rng.range(0, Math.PI * 2), r = rng.range(12, poi.r);
        place(rng.pick(forestTrees), poi.x + Math.cos(a) * r, poi.z + Math.sin(a) * r, rng.range(0, 6.28), rng.range(5.5, 8.5));
      }
      for (let n = 0; n < 10; n++) {
        const a = rng.range(0, Math.PI * 2), r = rng.range(10, poi.r * 0.9);
        const x = poi.x + Math.cos(a) * r, z = poi.z + Math.sin(a) * r;
        if (place(rng.pick(['nat-log_large', 'surv-tree-log', 'nat-stump_old']), x, z, rng.chance(0.5) ? 0 : Math.PI / 2, 5) && n % 2 === 0) lootNear(x + 2.5, z);
      }
      for (let n = 0; n < 30; n++) place(isle ? rng.pick(['nat-plant_bushLarge', 'nat-plant_bush']) : 'nat-cactus_short', poi.x + rng.range(-poi.r, poi.r), poi.z + rng.range(-poi.r, poi.r), rng.range(0, 6.28), rng.range(4, 7), false);
    } else if (poi.kind === 'lake') {
      // Piers into the water and a two-storey lake house on the north shore.
      const water = isle ? SEA_SURFACE : map.waters[0].y;
      dock(map, poi.x, poi.z + poi.r * 1.15, 0, -1, 14, water);
      dock(map, poi.x - poi.r * 1.15, poi.z, 1, 0, 12, water);
      lootNear(poi.x, poi.z + poi.r * 1.15 - 12);
      lootNear(poi.x - poi.r * 1.15 + 10, poi.z);
      building(map, poi.x + 4, poi.z + poi.r * 1.45, 2, 11, 9, 2, STYLES.house, rng);
      map.chests.push({ x: poi.x, z: poi.z + poi.r * 1.15 - 13, yaw: 0, y: water + 0.55 });
      for (let i = 0; i < 16; i++) {
        const a = rng.range(0, Math.PI * 2), r = poi.r * rng.range(1.25, 1.55);
        place(isle ? rng.pick(['nat-tree_oak', 'nat-tree_default', 'nat-plant_bushLarge']) : rng.pick(['nat-tree_palmTall', 'nat-tree_palmBend']), poi.x + Math.cos(a) * r, poi.z + Math.sin(a) * r, rng.range(0, 6.28), rng.range(5, 7.5));
      }
    } else {
      // Hill: a three-storey lookout on the summit, water tower and boulders.
      building(map, poi.x, poi.z, 0, 8, 8, 3, STYLES.brick, rng);
      place('ind-water-tower', poi.x + 12, poi.z - 8, 0, 7);
      for (let i = 0; i < 9; i++) place(rng.pick(isle ? ['nat-rock_tallB', 'nat-rock_largeC', 'surv-rock-b'] : ['nat-rock_tallB', 'surv-rock-sand-b', 'nat-stone_tallB']), poi.x + rng.range(-24, 24), poi.z + rng.range(-24, 24), rng.range(0, 6.28), rng.range(5, 8));
      lootNear(poi.x + 8, poi.z + 8); lootNear(poi.x - 9, poi.z - 4);
      map.chests.push({ x: poi.x - 6, z: poi.z + 9, yaw: 1 });
    }
  }
  // Wilderness: trees, rocks, cover and scattered loot / chests.
  const trees = isle ? ['nat-tree_default', 'nat-tree_oak', 'nat-tree_detailed', 'nat-tree_pineTallA', 'nat-tree_pineRoundC', 'surv-tree', 'surv-tree-tall'] : ['nat-cactus_tall', 'surv-tree-autumn', 'nat-cactus_tall'];
  const rocks = isle ? ['nat-rock_largeA', 'nat-rock_largeC', 'nat-rock_tallB', 'surv-rock-a', 'surv-rock-b', 'surv-rock-c'] : ['nat-rock_tallB', 'nat-rock_tallE', 'surv-rock-sand-a', 'surv-rock-sand-b', 'nat-stone_tallB'];
  const nearPoi = (x: number, z: number) => map.pois.some(p => Math.hypot(x - p.x, z - p.z) < p.r * (p.kind === 'lake' ? 1.6 : 1) + 4);
  const nearRiver = (x: number, z: number) => isle && polyDistance(map.river, x, z) < 12;
  for (let n = 0; n < (isle ? 640 : 300); n++) {
    const x = rng.range(-HALF + 10, HALF - 10), z = rng.range(-HALF + 10, HALF - 10);
    if (nearPoi(x, z) || nearRiver(x, z)) continue;
    const beach = isle && ground(x, z) < 1.6;
    place(beach ? rng.pick(['nat-tree_palmTall', 'nat-tree_palmBend', 'nat-tree_palmDetailedShort']) : rng.pick(trees), x, z, rng.range(0, 6.28), rng.range(5.5, 8));
  }
  for (let n = 0; n < (isle ? 220 : 380); n++) {
    const x = rng.range(-HALF + 10, HALF - 10), z = rng.range(-HALF + 10, HALF - 10);
    if (nearPoi(x, z) || nearRiver(x, z)) continue;
    place(rng.pick(rocks), x, z, rng.range(0, 6.28), rng.range(4, 9));
  }
  for (let n = 0; n < 480; n++) {
    const x = rng.range(-HALF + 6, HALF - 6), z = rng.range(-HALF + 6, HALF - 6);
    if (wet(x, z)) continue;
    place(isle ? rng.pick(['nat-plant_bushLarge', 'nat-plant_bush', 'nat-grass_large', 'surv-grass-large', 'nat-flower_redA', 'nat-flower_yellowB']) : rng.pick(['nat-cactus_short', 'surv-grass-large']), x, z, rng.range(0, 6.28), rng.range(4, 7), false);
  }
  for (let n = 0; n < 80; n++) {
    const x = rng.range(-HALF + 24, HALF - 24), z = rng.range(-HALF + 24, HALF - 24);
    if (!nearPoi(x, z) && free(x, z, 2)) {
      if (n % 3 === 0) place(rng.pick(['surv-box-large', 'surv-barrel', 'nat-log_large']), x + 2, z, 0, 5);
      lootNear(x, z);
    }
  }
  for (let n = 0; n < 12; n++) {
    const x = rng.range(-170, 170), z = rng.range(-170, 170);
    if (!nearPoi(x, z) && free(x, z, 2)) map.chests.push({ x, z, yaw: rng.range(0, 6.28) });
  }
  // Launch pad set pieces: a pad on the camp / crossroads square and at the foot of each hill.
  for (const poi of map.pois) {
    if (poi.kind !== 'camp' && poi.kind !== 'hill' && poi.kind !== 'town') continue;
    for (let k = 0; k < 12; k++) {
      const a = k * 2.1 + 0.4, r = poi.kind === 'hill' ? poi.r * 1.35 : poi.kind === 'town' ? 12 + k : 5 + k * 0.6;
      const x = poi.x + Math.cos(a) * r, z = poi.z + Math.sin(a) * r;
      if (!free(x, z, 1.6) || map.chests.some(c => Math.hypot(c.x - x, c.z - z) < 3)) continue;
      map.pads.push({ x, z, y: ground(x, z) });
      break;
    }
  }
  for (const c of map.chests) {
    const gy = c.y ?? ground(c.x, c.z);
    map.boxes.push({ minX: c.x - 0.7, minY: gy - 1, minZ: c.z - 0.7, maxX: c.x + 0.7, maxY: gy + 0.8, maxZ: c.z + 0.7 });
    map.boxMats.push(-1);
  }
}

/** Floor height a chest stands on. */
export const chestY = (map: ShooterMap, c: { x: number; z: number; y?: number }): number => c.y ?? terrainHeight(map, c.x, c.z);
