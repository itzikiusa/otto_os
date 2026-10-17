/**
 * Storm Royale islands: a deterministic heightfield, named points of interest
 * built from Kenney kits, colliders for every solid prop, loot spots and
 * chests. Pure data (no three.js) so the host simulation, guests and tests
 * all derive the identical world from the map id.
 */
import { clamp, fbm, hashString, lerp, makeNoise, Rng, smoothstep } from '../common/math.ts';
import type { ShooterMapId } from './catalog.ts';

export const HALF = 160;
export const GRID = 2;
export const SEA_LEVEL = 0;
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
};
/** Trees collide only with their trunk. */
const TRUNK = new Set(['nat-tree_default', 'nat-tree_oak', 'nat-tree_detailed', 'nat-tree_pineTallA', 'nat-tree_pineRoundC', 'nat-tree_palmTall', 'nat-tree_palmBend', 'nat-tree_palmDetailedShort', 'surv-tree', 'surv-tree-tall', 'surv-tree-autumn', 'house-tree-large', 'nat-cactus_tall']);

export interface Prop { key: string; x: number; y: number; z: number; yaw: number; scale: number }
export interface Box { minX: number; minY: number; minZ: number; maxX: number; maxY: number; maxZ: number }
export interface Cylinder { x: number; z: number; r: number; y0: number; y1: number }
export interface Poi { name: string; x: number; z: number; r: number; kind: 'town' | 'port' | 'farm' | 'camp' | 'hill' }
export interface ShooterMap {
  id: ShooterMapId; theme: 'isle' | 'mesa';
  heights: Float32Array; size: number;
  props: Prop[]; boxes: Box[]; cylinders: Cylinder[]; pois: Poi[];
  lootSpots: { x: number; z: number }[]; chests: { x: number; z: number; yaw: number }[];
}

const POIS: Record<ShooterMapId, Poi[]> = {
  isle: [
    { name: 'Harbor Town', x: -58, z: 38, r: 34, kind: 'town' },
    { name: 'Container Port', x: 66, z: -58, r: 30, kind: 'port' },
    { name: 'Pine Farm', x: 62, z: 64, r: 26, kind: 'farm' },
    { name: 'Lookout Hill', x: -52, z: -70, r: 20, kind: 'hill' },
    { name: 'Center Camp', x: 6, z: -4, r: 20, kind: 'camp' },
  ],
  mesa: [
    { name: 'Dust Town', x: -44, z: 30, r: 34, kind: 'town' },
    { name: 'Oil Yard', x: 58, z: -50, r: 30, kind: 'port' },
    { name: 'Canyon Camp', x: 54, z: 62, r: 24, kind: 'farm' },
    { name: 'Mesa Top', x: -60, z: -62, r: 22, kind: 'hill' },
    { name: 'Crossroads', x: 4, z: 0, r: 18, kind: 'camp' },
  ],
};

const cache = new Map<ShooterMapId, ShooterMap>();

export function mapFor(id: string): ShooterMap {
  const mapId: ShooterMapId = id === 'mesa' ? 'mesa' : 'isle';
  const hit = cache.get(mapId);
  if (hit) return hit;
  const theme = mapId;
  const noise = makeNoise(hashString(mapId));
  const pois = POIS[mapId];
  const poiHeight = (p: Poi) => (theme === 'isle' ? (p.kind === 'hill' ? 13 : p.kind === 'port' ? 1.6 : 4 + fbm(noise, p.x * 0.01, p.z * 0.01) * 2) : p.kind === 'hill' ? 18 : 6);
  const raw = (x: number, z: number): number => {
    let h: number;
    if (theme === 'isle') {
      const wobble = 1 + fbm(noise, x * 0.008 + 3, z * 0.008, 3) * 0.18;
      const d = Math.hypot(x, z) / (138 * wobble);
      const land = smoothstep(1.02, 0.78, d);
      h = land * (2.4 + (fbm(noise, x * 0.011, z * 0.011, 4) * 0.5 + 0.5) * 12) - (1 - land) * 7;
    } else {
      const base = (fbm(noise, x * 0.009, z * 0.009, 4) * 0.5 + 0.5) * 26;
      const step = 6, q = Math.floor(base / step) * step, f = (base - q) / step;
      h = q + smoothstep(0.75, 1, f) * step + 2;
      h += smoothstep(125, 160, Math.max(Math.abs(x), Math.abs(z))) * 30;
    }
    for (const p of pois) {
      const k = smoothstep(p.r * 1.25, p.r * 0.75, Math.hypot(x - p.x, z - p.z));
      h = lerp(h, poiHeight(p), k);
    }
    return h;
  };
  const size = HALF * 2 / GRID + 1;
  const heights = new Float32Array(size * size);
  for (let iz = 0; iz < size; iz++) for (let ix = 0; ix < size; ix++) heights[iz * size + ix] = raw(-HALF + ix * GRID, -HALF + iz * GRID);
  const map: ShooterMap = { id: mapId, theme, heights, size, props: [], boxes: [], cylinders: [], pois, lootSpots: [], chests: [] };
  populate(map);
  cache.set(mapId, map);
  return map;
}

/** Bilinear terrain height. */
export function terrainHeight(map: ShooterMap, x: number, z: number): number {
  const fx = clamp((x + HALF) / GRID, 0, map.size - 1.001), fz = clamp((z + HALF) / GRID, 0, map.size - 1.001);
  const ix = Math.floor(fx), iz = Math.floor(fz), tx = fx - ix, tz = fz - iz;
  const h = map.heights, s = map.size;
  const a = h[iz * s + ix], b = h[iz * s + ix + 1], c = h[(iz + 1) * s + ix], d = h[(iz + 1) * s + ix + 1];
  return lerp(lerp(a, b, tx), lerp(c, d, tx), tz);
}

function populate(map: ShooterMap): void {
  const rng = new Rng(hashString(map.id) ^ 0xa11ce);
  const ground = (x: number, z: number) => terrainHeight(map, x, z);
  const free = (x: number, z: number, r: number) => {
    if (Math.abs(x) > HALF - 8 || Math.abs(z) > HALF - 8) return false;
    if (map.theme === 'isle' && ground(x, z) < SEA_LEVEL + 0.6) return false;
    for (const b of map.boxes) if (x > b.minX - r && x < b.maxX + r && z > b.minZ - r && z < b.maxZ + r) return false;
    for (const c of map.cylinders) if (Math.hypot(x - c.x, z - c.z) < c.r + r) return false;
    return true;
  };
  const place = (key: string, x: number, z: number, yaw: number, scale: number, solid = true, y?: number): boolean => {
    const gy = y ?? ground(x, z);
    const size = PROP_SIZE[key];
    if (solid && size) {
      const quarter = Math.round(yaw / (Math.PI / 2));
      const swap = Math.abs(quarter) % 2 === 1;
      const w = (swap ? size[2] : size[0]) * scale / 2, d = (swap ? size[0] : size[2]) * scale / 2;
      if (!free(x, z, 0.2)) return false;
      map.boxes.push({ minX: x - w, minY: gy - 1, minZ: z - d, maxX: x + w, maxY: gy + size[1] * scale, maxZ: z + d });
      yaw = quarter * (Math.PI / 2);
    } else if (solid && TRUNK.has(key)) {
      if (!free(x, z, 0.8)) return false;
      map.cylinders.push({ x, z, r: 0.45 * scale / 6, y0: gy - 1, y1: gy + 7 * scale / 6 });
    }
    map.props.push({ key, x, y: gy, z, yaw, scale });
    return true;
  };
  const lootNear = (x: number, z: number) => map.lootSpots.push({ x, z });
  const isle = map.theme === 'isle';
  const houses = ['house-building-type-a', 'house-building-type-c', 'house-building-type-d', 'house-building-type-f', 'house-building-type-h', 'house-building-type-k', 'house-building-type-m', 'house-building-type-o', 'house-building-type-r', 'house-building-type-t'];
  const cars = ['car-sedan', 'car-suv', 'car-police', 'car-delivery'];
  for (const poi of map.pois) {
    if (poi.kind === 'town') {
      for (let gx = -1; gx <= 1; gx++) for (let gz = -1; gz <= 1; gz++) {
        if (gx === 0 && gz === 0) continue;
        const x = poi.x + gx * 22, z = poi.z + gz * 22;
        const yaw = gz < 0 ? 0 : gz > 0 ? Math.PI : gx < 0 ? Math.PI / 2 : -Math.PI / 2;
        place(rng.pick(houses), x, z, yaw, 8.5);
        // Loot by the front door, a car on the street.
        lootNear(x + Math.sin(yaw) * 7, z + Math.cos(yaw) * 7);
        if (rng.chance(0.6)) place(rng.pick(cars), x + Math.sin(yaw) * 11 + Math.cos(yaw) * 5, z + Math.cos(yaw) * 11 - Math.sin(yaw) * 5, yaw + Math.PI / 2, 1.8);
        if (rng.chance(0.5)) place('house-fence-1x3', x - Math.sin(yaw) * 6.5, z - Math.cos(yaw) * 6.5, yaw, 4, true);
      }
      map.chests.push({ x: poi.x + 3, z: poi.z + 2, yaw: 0.3 });
      lootNear(poi.x - 3, poi.z - 3);
      for (let i = 0; i < 6; i++) place(rng.pick(['house-tree-large', 'nat-tree_oak']), poi.x + rng.range(-32, 32), poi.z + rng.range(-32, 32), rng.range(0, 6.28), 6);
    } else if (poi.kind === 'port') {
      place('ind-building-a', poi.x - 14, poi.z - 12, 0, 9);
      place(isle ? 'ind-building-k' : 'ind-chimney-large', poi.x + 16, poi.z - 14, 0, 9);
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
          map.props.push({ key: rng.pick(['ind-shipping-container-a', 'ind-shipping-container-b', 'ind-shipping-container-c']), x, y: gy, z, yaw, scale: 8 });
        }
        if (i % 2 === 0) lootNear(x + 4.5, z);
      }
      place('ind-detail-tank', poi.x + 8, poi.z - 26, 0, 8);
      map.chests.push({ x: poi.x + 2, z: poi.z - 2, yaw: 0 }, { x: poi.x - 22, z: poi.z + 18, yaw: 1.2 });
    } else if (poi.kind === 'farm') {
      for (let i = 0; i < 4; i++) {
        const a = (i / 4) * Math.PI * 2 + 0.4;
        const x = poi.x + Math.cos(a) * 12, z = poi.z + Math.sin(a) * 12;
        place(rng.chance(0.5) ? 'surv-tent-canvas' : 'surv-tent', x, z, -a + Math.PI / 2, 8);
        lootNear(poi.x + Math.cos(a) * 6, poi.z + Math.sin(a) * 6);
      }
      for (let i = 0; i < 10; i++) place(rng.pick(['surv-barrel', 'surv-box', 'surv-box-large', 'surv-workbench']), poi.x + rng.range(-20, 20), poi.z + rng.range(-20, 20), rng.chance(0.5) ? 0 : Math.PI / 2, 5);
      for (let i = 0; i < 8; i++) place('surv-fence-fortified', poi.x - 22 + i * 2.4 * 2, poi.z - 22, 0, 4.8);
      place('surv-tree-log', poi.x + 5, poi.z - 8, 0, 6);
      map.chests.push({ x: poi.x, z: poi.z, yaw: 0.8 });
    } else if (poi.kind === 'camp') {
      for (let i = 0; i < 4; i++) {
        const a = (i / 4) * Math.PI * 2;
        place('surv-structure-metal', poi.x + Math.cos(a) * 9, poi.z + Math.sin(a) * 9, 0, 7);
      }
      for (let i = 0; i < 8; i++) place(rng.pick(['surv-box', 'surv-box-large', 'surv-barrel', 'surv-metal-panel-screws']), poi.x + rng.range(-16, 16), poi.z + rng.range(-16, 16), rng.chance(0.5) ? 0 : Math.PI / 2, 5);
      lootNear(poi.x, poi.z); lootNear(poi.x + 5, poi.z - 5); lootNear(poi.x - 6, poi.z + 4);
      map.chests.push({ x: poi.x + 1, z: poi.z + 6, yaw: 2 });
    } else {
      place('ind-water-tower', poi.x, poi.z, 0, 7);
      for (let i = 0; i < 7; i++) place(rng.pick(isle ? ['nat-rock_tallB', 'nat-rock_largeC', 'surv-rock-b'] : ['nat-rock_tallB', 'surv-rock-sand-b', 'nat-stone_tallB']), poi.x + rng.range(-18, 18), poi.z + rng.range(-18, 18), rng.range(0, 6.28), rng.range(5, 8));
      lootNear(poi.x + 4, poi.z + 4); lootNear(poi.x - 5, poi.z - 2);
      map.chests.push({ x: poi.x - 3, z: poi.z + 6, yaw: 1 });
    }
  }
  // Wilderness: trees, rocks, cover and scattered loot / chests.
  const trees = isle ? ['nat-tree_default', 'nat-tree_oak', 'nat-tree_detailed', 'nat-tree_pineTallA', 'nat-tree_pineRoundC', 'surv-tree', 'surv-tree-tall'] : ['nat-cactus_tall', 'surv-tree-autumn', 'nat-cactus_tall'];
  const rocks = isle ? ['nat-rock_largeA', 'nat-rock_largeC', 'nat-rock_tallB', 'surv-rock-a', 'surv-rock-b', 'surv-rock-c'] : ['nat-rock_tallB', 'nat-rock_tallE', 'surv-rock-sand-a', 'surv-rock-sand-b', 'nat-stone_tallB'];
  const nearPoi = (x: number, z: number) => map.pois.some(p => Math.hypot(x - p.x, z - p.z) < p.r + 4);
  for (let n = 0; n < (isle ? 340 : 160); n++) {
    const x = rng.range(-HALF + 10, HALF - 10), z = rng.range(-HALF + 10, HALF - 10);
    if (nearPoi(x, z)) continue;
    const beach = isle && ground(x, z) < 1.6;
    place(beach ? rng.pick(['nat-tree_palmTall', 'nat-tree_palmBend', 'nat-tree_palmDetailedShort']) : rng.pick(trees), x, z, rng.range(0, 6.28), rng.range(5.5, 8));
  }
  for (let n = 0; n < (isle ? 110 : 190); n++) {
    const x = rng.range(-HALF + 10, HALF - 10), z = rng.range(-HALF + 10, HALF - 10);
    if (nearPoi(x, z)) continue;
    place(rng.pick(rocks), x, z, rng.range(0, 6.28), rng.range(4, 9));
  }
  for (let n = 0; n < 260; n++) {
    const x = rng.range(-HALF + 6, HALF - 6), z = rng.range(-HALF + 6, HALF - 6);
    if (isle && ground(x, z) < 0.8) continue;
    place(isle ? rng.pick(['nat-plant_bushLarge', 'nat-plant_bush', 'nat-grass_large', 'surv-grass-large', 'nat-flower_redA', 'nat-flower_yellowB']) : rng.pick(['nat-cactus_short', 'surv-grass-large']), x, z, rng.range(0, 6.28), rng.range(4, 7), false);
  }
  for (let n = 0; n < 40; n++) {
    const x = rng.range(-HALF + 20, HALF - 20), z = rng.range(-HALF + 20, HALF - 20);
    if (!nearPoi(x, z) && free(x, z, 2) && (!isle || ground(x, z) > 1)) {
      if (n % 3 === 0) place(rng.pick(['surv-box-large', 'surv-barrel', 'nat-log_large']), x + 2, z, 0, 5);
      lootNear(x, z);
    }
  }
  for (let n = 0; n < 6; n++) {
    const x = rng.range(-110, 110), z = rng.range(-110, 110);
    if (!nearPoi(x, z) && free(x, z, 2) && (!isle || ground(x, z) > 1)) map.chests.push({ x, z, yaw: rng.range(0, 6.28) });
  }
  for (const c of map.chests) {
    const gy = ground(c.x, c.z);
    map.boxes.push({ minX: c.x - 0.7, minY: gy - 1, minZ: c.z - 0.7, maxX: c.x + 0.7, maxY: gy + 0.8, maxZ: c.z + 0.7 });
  }
}
