/**
 * Dead City: a ruined, burning downtown on a six-by-six street grid. Two
 * collapsed skyscrapers (the Spire's fallen upper floors lie across Second
 * Avenue as a climbable rubble ramp into its fourth storey), Mercy Hospital,
 * Eastgate Mall with its burnt-out car park, the sunken Line 3 subway entrance
 * with a derailed carriage on the platform, Checkpoint Bravo (barriers,
 * sandbag nests, guard towers, tents, trucks), overgrown Ashwood Park with
 * its fountain, the Ironside Depot and a downtown of blown-out tenements.
 * Streets carry wrecks, jersey barriers, lamp posts, craters and debris; a
 * rubble berm rings the city. Built with map.ts's architecture and placer, so
 * colliders, loot, chests, climbs, openings and fires work like every map.
 */
import { clamp, fbm, hashString, makeNoise, Rng, smoothstep } from '../../common/math.ts';
import { building, factory, Frame, HALF, makePlacer, MAT, STOREY, STYLES, type Poi, type ShooterMap } from '../map.ts';
import type { AnchorHints } from './anchors.ts';

/** Street centre lines (both axes); blocks sit between them. */
export const STREETS = [-180, -110, -40, 30, 100, 170];
const BLOCKS = [-145, -75, -5, 65, 135];
const ROAD_W = 12;
/** Asphalt plus sidewalks either side of a street centre line. */
const KERB = ROAD_W / 2 + 3;
const RIM = 205;

export const DEAD_CITY_POIS: Poi[] = [
  { name: 'The Spire', x: -5, z: -5, r: 34, kind: 'tower' },
  { name: 'Broken Tower', x: 135, z: -145, r: 30, kind: 'tower' },
  { name: 'Mercy Hospital', x: -145, z: -145, r: 32, kind: 'hospital' },
  { name: 'Eastgate Mall', x: 135, z: 65, r: 36, kind: 'mall' },
  { name: 'Line 3 Subway', x: -6, z: 133, r: 24, kind: 'subway' },
  { name: 'Checkpoint Bravo', x: -140, z: 135, r: 34, kind: 'checkpoint' },
  { name: 'Ashwood Park', x: -75, z: -75, r: 30, kind: 'park' },
  { name: 'Old Downtown', x: -75, z: 65, r: 30, kind: 'town' },
  { name: 'Ironside Depot', x: 65, z: -145, r: 30, kind: 'factory' },
];
/** Bases at the checkpoint and the depot yard, flags on three crossings, zombies around the hospital, the chopper in the park. */
export const DEAD_CITY_HINTS: AnchorHints = {
  bases: [[-165, 165], [165, -165]],
  flags: [{ label: 'Checkpoint Road', x: -110, z: 100 }, { label: 'Spire Crossing', x: -40, z: -40 }, { label: 'Depot Junction', x: 100, z: -110 }],
  arena: { x: -120, z: -120, r: 56 },
  extraction: [-75, -82],
};

/** The subway trench: entrance at (x, z) running +z, a 12.5 m stair down to a platform 4.8 m below the street. */
const SUBWAY = { x: -6, z: 118, half: 4, ramp: 12.5, len: 30, depth: 4.8 };
/** Bomb craters [x, z, radius, depth]. */
const CRATERS: [number, number, number, number][] = [[30, -40, 6, 1.6], [-40, -112, 7, 2.2], [100, 30, 6, 1.8], [170, 100, 6, 2], [-110, -40, 6, 1.7], [-80, 170, 5, 1.4]];

const noise = makeNoise(hashString('deadcity'));

/** Street-flat downtown with a rubble berm at the rim, bomb craters and the subway cut. */
export function deadCityHeight(x: number, z: number): number {
  let h = fbm(noise, x * 0.012, z * 0.012, 3) * 0.35;
  const edge = Math.max(Math.abs(x), Math.abs(z));
  const berm = smoothstep(RIM - 12, HALF - 4, edge);
  h += berm * (14 + fbm(noise, x * 0.03 + 7, z * 0.03, 3) * 6);
  for (const [cx, cz, r, depth] of CRATERS) {
    const d = Math.hypot(x - cx, z - cz) / r;
    if (d < 1.4) h += d < 1 ? -depth * (1 - d * d) : 0.35 * (1 - (d - 1) / 0.4);
  }
  const lx = x - SUBWAY.x, lz = z - SUBWAY.z;
  if (Math.abs(lx) <= SUBWAY.half + 0.01 && lz >= 0 && lz <= SUBWAY.len) h = Math.min(h, -clamp(lz / SUBWAY.ramp, 0, 1) * SUBWAY.depth);
  return h;
}

export function populateDeadCity(map: ShooterMap): void {
  const rng = new Rng(hashString(map.id) ^ 0xdead);
  const { ground, free, place } = makePlacer(map);
  const loot = (x: number, z: number, y?: number) => map.lootSpots.push(y === undefined ? { x, z } : { x, z, y });
  const fire = (x: number, z: number, s = 1, y = ground(x, z)) => map.fires.push({ x, y, z, s });
  const cars = ['car-sedan', 'car-suv', 'car-police', 'car-delivery'];
  const block = (x0: number, z0: number, x1: number, z1: number, y0: number, y1: number, mat: number) => new Frame(map, 0, 0, 0).block(x0, z0, x1, z1, y0, y1, mat);
  /** A wreck (burnt unless `clean`), maybe still burning, with loot beside it now and then. */
  const wreck = (x: number, z: number, yaw: number, clean = false) => {
    const key = rng.pick(cars);
    if (!place(key, x, z, yaw, 1.8, true, undefined, !clean)) return false;
    if (!clean && rng.chance(0.3)) fire(x, z, 0.9, ground(x, z) + 1.4);
    if (rng.chance(0.25)) loot(x + Math.cos(yaw) * 3, z - Math.sin(yaw) * 3);
    return true;
  };
  /** Ziggurat of rubble: stacked squares 0.45 m apart (walkable), lumps on top for a broken outline. */
  const mound = (cx: number, cz: number, r: number, layers: number) => {
    const gy = ground(cx, cz);
    for (let l = 0; l < layers; l++) {
      const half = r * (1 - l / layers), ox = rng.range(-0.8, 0.8), oz = rng.range(-0.8, 0.8);
      block(cx + ox - half, cz + oz - half, cx + ox + half, cz + oz + half, gy - 1, gy + (l + 1) * 0.45, MAT.rubble);
    }
    for (let k = 0; k < Math.round(r * 1.2); k++) {
      const a = rng.range(0, Math.PI * 2), d = rng.range(0.3, 1) * r, s = rng.range(0.5, 1.4);
      const x = cx + Math.cos(a) * d, z = cz + Math.sin(a) * d, top = gy + (1 - d / r) * layers * 0.45;
      block(x - s, z - s * 0.7, x + s, z + s * 0.7, top - 0.6, top + rng.range(0.2, 0.42), rng.chance(0.2) ? MAT.charred : MAT.rubble);
    }
  };
  /** A jersey barrier run along x (`alongX`) or z, `n` pieces with small gaps. */
  const barriers = (x: number, z: number, alongX: boolean, n: number, mat: number = MAT.concrete) => {
    for (let k = 0; k < n; k++) {
      const o = (k - (n - 1) / 2) * 3.5, bx = alongX ? x + o : x, bz = alongX ? z : z + o, gy = ground(bx, bz);
      if (!free(bx, bz, 0.4)) continue;
      if (alongX) block(bx - 1.6, bz - 0.3, bx + 1.6, bz + 0.3, gy - 0.6, gy + 0.85, mat);
      else block(bx - 0.3, bz - 1.6, bx + 0.3, bz + 1.6, gy - 0.6, gy + 0.85, mat);
    }
  };
  /** A U of sandbags (waist-high cover) opening toward `yaw`'s back. */
  const nest = (x: number, z: number) => {
    const gy = ground(x, z);
    block(x - 2.4, z - 2.4, x + 2.4, z - 1.8, gy - 0.5, gy + 1.05, MAT.sandbag);
    block(x - 2.4, z - 1.8, x - 1.8, z + 1.6, gy - 0.5, gy + 1.05, MAT.sandbag);
    block(x + 1.8, z - 1.8, x + 2.4, z + 1.6, gy - 0.5, gy + 1.05, MAT.sandbag);
    loot(x, z);
  };

  // ---------------------------------------------------------------- streets
  for (const s of STREETS) {
    map.roads.push({ x0: s, z0: -RIM, x1: s, z1: RIM, w: ROAD_W });
    map.roads.push({ x0: -RIM, z0: s, x1: RIM, z1: s, w: ROAD_W });
  }

  // ---------------------------------------------------------------- landmarks (before the street clutter so it keeps clear of them)
  // The Spire: a five-storey stump; its fallen upper floors make a stepped rubble ramp east into storey three.
  {
    const p = DEAD_CITY_POIS[0], w = 20;
    const roof = building(map, p.x, p.z, 0, w, w, 5, STYLES.office, rng, { ruin: true, breaches: [[3, 3], [2, 3]] });
    const base = roof - 5 * STOREY, top = base + 3 * STOREY;
    const x0 = p.x + w / 2, zc = p.z - w * 0.2;
    const n = Math.ceil(top / 0.42);
    for (let k = 0; k < n; k++) {
      const x = x0 + k * 1.6, h = top - k * (top / n), gy = ground(x + 0.8, zc);
      if (h <= gy + 0.1) break;
      block(x, zc - 4.5, x + 1.6, zc + 4.5, gy - 1, h, k % 7 === 3 ? MAT.charred : MAT.concrete);
      // Twisted steel and broken facade along both edges of the fallen section.
      if (k % 3 === 0) for (const side of [-1, 1]) block(x, zc + side * 4.5 - (side > 0 ? 0 : 0.35), x + 1.6, zc + side * 4.5 + (side > 0 ? 0.35 : 0), gy - 1, h + rng.range(0.6, 1.6), side > 0 ? MAT.rust : MAT.glass);
      if (k % 9 === 4) loot(x + 0.8, zc, h);
    }
    map.chests.push({ x: x0 + 2.4, z: zc + 2.5, yaw: 0, y: top - top / n });
    mound(p.x - 18, p.z + 16, 7, 6);
    mound(p.x + 8, p.z - 20, 6, 5);
    for (let k = 0; k < 4; k++) fire(p.x + rng.range(-14, 14), p.z + rng.range(14, 18), rng.range(1, 1.6));
  }
  // Broken Tower: a three-storey stump beside the mound its top half became.
  {
    const p = DEAD_CITY_POIS[1];
    building(map, p.x - 6, p.z - 4, 1, 18, 18, 3, STYLES.office, rng, { ruin: true });
    mound(p.x + 14, p.z + 15, 9, 8);
    mound(p.x - 16, p.z + 18, 5, 4);
    fire(p.x + 14, p.z + 15, 1.8, ground(p.x + 14, p.z + 15) + 3.6);
    map.chests.push({ x: p.x + 18, z: p.z - 6, yaw: 0.4 });
  }
  // Mercy Hospital: three tiled storeys, ambulances, a triage camp and sandbags at the doors.
  {
    const p = DEAD_CITY_POIS[2];
    building(map, p.x, p.z - 6, 0, 34, 20, 3, STYLES.hospital, rng, { ruin: true });
    for (const dx of [-10, 6]) place('car-delivery', p.x + dx, p.z + 12, Math.PI / 2, 1.8);
    for (let i = 0; i < 3; i++) place('surv-tent-canvas', p.x - 14 + i * 8, p.z + 20, 0, 8);
    nest(p.x + 14, p.z + 16);
    for (let i = 0; i < 6; i++) place(rng.pick(['surv-box-large', 'surv-box', 'surv-barrel']), p.x + rng.range(-16, 16), p.z + rng.range(14, 24), 0, 5);
    loot(p.x - 6, p.z + 17); loot(p.x + 2, p.z + 22);
    map.chests.push({ x: p.x - 18, z: p.z + 14, yaw: 0 });
  }
  // Eastgate Mall: a glass-and-steel hall with a mezzanine and kiosks.
  {
    const p = DEAD_CITY_POIS[3];
    const floor = factory(map, p.x, p.z - 4, 0, 44, 28, STYLES.mall);
    for (const [kx, kz] of [[-12, 4], [0, 6], [12, 4], [-6, -2], [6, -2]]) {
      const x = p.x + kx, z = p.z - 4 + kz;
      block(x - 1.6, z - 1, x + 1.6, z + 1, floor - 0.2, floor + 1.1, rng.chance(0.5) ? MAT.glass : MAT.floor);
      loot(x, z + 2, floor);
    }
    fire(p.x + 10, p.z - 4, 1.2, floor);
  }
  // Mall car park (the next block south): rows of burnt cars and lamp posts.
  for (let row = 0; row < 3; row++) for (let k = 0; k < 6; k++) {
    if (rng.chance(0.3)) continue;
    wreck(115 + k * 7.5, 120 + row * 14 + rng.range(-0.5, 0.5), rng.chance(0.5) ? 0 : Math.PI);
  }
  // Line 3 Subway: walls and stairs down the cut, a canopy, the platform and a derailed carriage.
  {
    const f = new Frame(map, SUBWAY.x, SUBWAY.z, 0), d = -SUBWAY.depth, hw = SUBWAY.half;
    f.block(-hw - 0.6, -0.4, -hw, SUBWAY.len, d - 1.2, 1.0, MAT.concrete);
    f.block(hw, -0.4, hw + 0.6, SUBWAY.len, d - 1.2, 1.0, MAT.concrete);
    f.block(-hw - 0.6, SUBWAY.len, hw + 0.6, SUBWAY.len + 0.6, d - 1.2, 1.0, MAT.charred);
    const steps = Math.ceil(SUBWAY.ramp / 0.9);
    for (let k = 0; k < steps; k++) f.block(-hw, k * 0.9, hw, (k + 1) * 0.9, d - 1.2, -k * (SUBWAY.depth / steps), MAT.concrete);
    f.block(-hw, steps * 0.9, hw, SUBWAY.len, d - 1.2, d + 0.05, MAT.asphalt);
    // Canopy over the entrance.
    for (const sx of [-1, 1]) for (const lz of [-0.4, 5.5]) f.block(sx * hw + (sx > 0 ? 0 : -0.6), lz, sx * hw + (sx > 0 ? 0.6 : 0), lz + 0.4, 1.0, 3.3, MAT.metal);
    f.block(-hw - 0.8, -0.8, hw + 0.8, 6.2, 3.3, 3.55, MAT.metal);
    // The carriage: floor, roof, a side with two doors and windows facing the walkway, closed ends.
    const y0 = d + 0.05, cx0 = 0.4, cx1 = 3.6, cz0 = 14, cz1 = 28.5, ch = 2.5;
    f.block(cx0, cz0, cx1, cz1, y0 - 0.2, y0 + 0.5, MAT.metal);
    f.block(cx0, cz0, cx1, cz1, y0 + 0.5 + ch, y0 + 0.75 + ch, MAT.metal);
    f.wallZ(cz0, cz1, cx0, cx0 + 0.15, y0 + 0.5, y0 + 0.5 + ch, MAT.rust, [[17.5, 1.4, 0, 2.1], [25, 1.4, 0, 2.1], [21.2, 2.2, 0.9, 1.9]]);
    f.wallZ(cz0, cz1, cx1 - 0.15, cx1, y0 + 0.5, y0 + 0.5 + ch, MAT.rust, [[17.5, 2, 0.9, 1.9], [24, 2, 0.9, 1.9]]);
    f.wallX(cx0, cx1, cz0, cz0 + 0.15, y0 + 0.5, y0 + 0.5 + ch, MAT.rust);
    f.wallX(cx0, cx1, cz1 - 0.15, cz1, y0 + 0.5, y0 + 0.5 + ch, MAT.rust);
    const [lx, lz] = f.world(2, 21);
    loot(lx, lz, y0 + 0.5);
    const [kx, kz] = f.world(-2.2, 24);
    map.chests.push({ x: kx, z: kz, yaw: 0, y: y0 });
    const [px, pz] = f.world(-2, 16);
    loot(px, pz, y0);
    const [ex, ez] = f.world(0, -6);
    map.pads.push({ x: ex + 8, z: ez, y: ground(ex + 8, ez) });
  }
  // Checkpoint Bravo: guard towers, sandbag nests, tents, a container wall, trucks and a barrier chicane across the avenue.
  {
    const p = DEAD_CITY_POIS[5];
    for (const [tx, tz] of [[p.x + 18, p.z - 16], [p.x - 20, p.z + 14]]) {
      const f = new Frame(map, tx, tz, 0), gy = ground(tx, tz), deck = gy + 4.3;
      for (const [sx, sz] of [[-1, -1], [1, -1], [1, 1], [-1, 1]]) f.block(sx * 1.6 - (sx > 0 ? 0.3 : 0), sz * 1.6 - (sz > 0 ? 0.3 : 0), sx * 1.6 + (sx < 0 ? 0.3 : 0), sz * 1.6 + (sz < 0 ? 0.3 : 0), gy - 0.5, deck, MAT.olive);
      f.block(-1.6, -1.6, 1.6, 1.6, deck - 0.3, deck, MAT.floor);
      f.stairs(-0.6, 0.6, 1.6 + 12 * 0.45, -1, gy, 12, (deck - gy) / 12, MAT.stair);
      f.block(-1.6, -1.6, 1.6, -1.25, deck, deck + 1.0, MAT.sandbag);
      f.block(-1.6, -1.25, -1.25, 1.6, deck, deck + 1.0, MAT.sandbag);
      f.block(1.25, -1.25, 1.6, 1.6, deck, deck + 1.0, MAT.sandbag);
      loot(tx, tz, deck);
    }
    nest(p.x - 4, p.z - 18); nest(p.x + 10, p.z + 16);
    for (let i = 0; i < 3; i++) place('surv-tent-canvas', p.x - 12 + i * 9, p.z, 0, 8);
    for (let i = 0; i < 4; i++) place(rng.pick(['ind-shipping-container-a', 'ind-shipping-container-b', 'ind-shipping-container-c']), p.x - 22, p.z - 12 + i * 7, Math.PI / 2, 8);
    place('car-delivery', p.x + 4, p.z + 6, 0, 2.1); place('car-delivery', p.x + 12, p.z + 4, Math.PI, 2.1);
    for (let i = 0; i < 6; i++) place(rng.pick(['surv-box-large', 'surv-box', 'surv-metal-panel-screws']), p.x + rng.range(-12, 12), p.z + rng.range(-10, 10), 0, 5);
    for (let k = 0; k < 4; k++) barriers(STREETS[1] + (k % 2 ? 3 : -3), p.z - 12 + k * 8, true, 1, k % 2 ? MAT.hazard : MAT.concrete);
    barriers(p.x, STREETS[4] + 2.5, true, 5, MAT.concrete);
    loot(p.x + 2, p.z - 6); loot(p.x - 8, p.z + 8);
    map.chests.push({ x: p.x + 16, z: p.z + 2, yaw: 0 }, { x: p.x - 14, z: p.z - 4, yaw: Math.PI / 2 });
    map.pads.push({ x: p.x + 2, z: p.z + 22, y: ground(p.x + 2, p.z + 22) });
  }
  // Ashwood Park: overgrown woods round a dry fountain, a gazebo, benches and hedges.
  {
    const p = DEAD_CITY_POIS[6], gy = ground(p.x, p.z);
    block(p.x - 4, p.z - 4, p.x + 4, p.z - 3.5, gy - 0.5, gy + 0.55, MAT.concrete);
    block(p.x - 4, p.z + 3.5, p.x + 4, p.z + 4, gy - 0.5, gy + 0.55, MAT.concrete);
    block(p.x - 4, p.z - 3.5, p.x - 3.5, p.z + 3.5, gy - 0.5, gy + 0.55, MAT.concrete);
    block(p.x + 3.5, p.z - 3.5, p.x + 4, p.z + 3.5, gy - 0.5, gy + 0.55, MAT.concrete);
    block(p.x - 0.5, p.z - 0.5, p.x + 0.5, p.z + 0.5, gy - 0.5, gy + 1.8, MAT.concrete);
    map.waters.push({ x: p.x, z: p.z, r: 3.4, y: gy + 0.3 });
    building(map, p.x + 17, p.z - 15, 0, 7, 7, 1, STYLES.house, rng, { ruin: true });
    for (let k = 0; k < 8; k++) {
      const a = (k / 8) * Math.PI * 2, x = p.x + Math.cos(a) * 9, z = p.z + Math.sin(a) * 9;
      if (k % 2 === 0) block(x - 0.9, z - 0.3, x + 0.9, z + 0.3, ground(x, z) - 0.3, ground(x, z) + 0.5, MAT.floor);
    }
    for (let n = 0; n < 70; n++) {
      const a = rng.range(0, Math.PI * 2), r = rng.range(11, p.r + 4);
      place(rng.pick(['nat-tree_oak', 'nat-tree_default', 'surv-tree-autumn', 'house-tree-large', 'nat-tree_detailed']), p.x + Math.cos(a) * r, p.z + Math.sin(a) * r, rng.range(0, 6.28), rng.range(5.5, 8));
    }
    for (let n = 0; n < 90; n++) place(rng.pick(['nat-plant_bushLarge', 'nat-plant_bush', 'nat-grass_large', 'surv-grass-large']), p.x + rng.range(-p.r, p.r), p.z + rng.range(-p.r, p.r), rng.range(0, 6.28), rng.range(4, 7.5), false);
    loot(p.x + 6, p.z + 6); loot(p.x - 8, p.z - 5); loot(p.x + 14, p.z + 12);
    map.chests.push({ x: p.x - 6, z: p.z + 7, yaw: 1 });
    map.pads.push({ x: p.x + 7, z: p.z - 7, y: gy });
  }
  // Old Downtown: four blown-out tenements and offices facing the streets.
  {
    const p = DEAD_CITY_POIS[7];
    for (const [dx, dz] of [[-12, -12], [12, -12], [-12, 12], [12, 12]]) {
      building(map, p.x + dx, p.z + dz, dz > 0 ? 0 : 2, 18, 16, 3 + rng.int(4), rng.pick([STYLES.office, STYLES.tenement, STYLES.brick]), rng, { ruin: true });
    }
    fire(p.x, p.z, 1.4);
    loot(p.x, p.z + 2);
  }
  // Ironside Depot: a steel shed, its container yard, chimney and water tower.
  {
    const p = DEAD_CITY_POIS[8];
    factory(map, p.x, p.z + 4, 0, 34, 22, STYLES.metal);
    place('ind-chimney-large', p.x + 22, p.z - 14, 0, 9);
    place('ind-water-tower', p.x - 22, p.z - 16, 0, 7);
    for (let i = 0; i < 6; i++) {
      const x = p.x - 15 + (i % 3) * 10, z = p.z - 16 + Math.floor(i / 3) * 6;
      place(rng.pick(['ind-shipping-container-a', 'ind-shipping-container-b', 'ind-shipping-container-c']), x, z, Math.PI / 2, 8);
      if (i % 2 === 0) loot(x + 4, z);
    }
  }

  // ---------------------------------------------------------------- the rest of the blocks
  const landmark = (x: number, z: number) => DEAD_CITY_POIS.some(p => Math.abs(p.x - x) < 20 && Math.abs(p.z - z) < 20);
  for (const bx of BLOCKS) for (const bz of BLOCKS) {
    if (landmark(bx, bz) || (bx === 135 && bz === 135)) continue;
    const kind = rng.next();
    if (kind < 0.55) {
      // Four ruins round a courtyard, fronts on the streets (one may be a rubble heap instead).
      for (const [sx, sz] of [[-1, -1], [1, -1], [-1, 1], [1, 1]]) {
        const x = bx + sx * 12.5, z = bz + sz * 12, w = rng.range(14, 18), d = rng.range(12, 15);
        if (!free(x, z, Math.max(w, d) / 2)) continue;
        if (rng.chance(0.15)) { mound(x, z, rng.range(4, 6), 3 + rng.int(3)); continue; }
        building(map, x, z, sz > 0 ? 0 : 2, w, d, 2 + rng.int(5), rng.pick([STYLES.office, STYLES.tenement, STYLES.brick]), rng, { ruin: true });
      }
    } else if (kind < 0.85) {
      // One big office block and a rubble heap where its neighbour stood.
      const w = rng.range(24, 30), d = rng.range(18, 22), ox = rng.chance(0.5) ? -5 : 5;
      if (free(bx + ox, bz, w / 2 + 1)) building(map, bx + ox, bz, rng.int(4), w, d, 4 + rng.int(4), STYLES.office, rng, { ruin: true });
      const mx = bx - ox * 3.6, mz = bz + rng.range(-14, 14);
      if (free(mx, mz, 4)) mound(mx, mz, rng.range(4, 6), 4 + rng.int(3));
    } else {
      // A burnt-out lot: wrecks, barrels on fire, rubble and a shack.
      for (let k = 0; k < 5; k++) wreck(bx + rng.range(-18, 18), bz + rng.range(-18, 18), rng.chance(0.5) ? 0 : Math.PI / 2);
      for (let k = 0; k < 3; k++) {
        const x = bx + rng.range(-16, 16), z = bz + rng.range(-16, 16);
        if (place('surv-barrel', x, z, 0, 5) && k === 0) fire(x, z, 0.6, ground(x, z) + 1.7);
      }
      if (free(bx, bz, 5)) mound(bx + rng.range(-6, 6), bz + rng.range(-6, 6), rng.range(4, 6), 3 + rng.int(3));
      if (free(bx + 12, bz - 12, 4)) building(map, bx + 12, bz - 12, rng.int(4), 8, 7, 1, STYLES.tenement, rng, { ruin: true });
      loot(bx, bz + 8); loot(bx - 10, bz - 4);
      if (rng.chance(0.5)) map.chests.push({ x: bx - 8, z: bz + 12, yaw: rng.range(0, 6.28) });
    }
  }

  // ---------------------------------------------------------------- street clutter
  const onRoad = (x: number, z: number) => STREETS.some(s => Math.abs(x - s) < KERB) || STREETS.some(s => Math.abs(z - s) < KERB);
  for (const s of STREETS) for (let t = -RIM + 10; t < RIM - 10; t += 6) {
    for (const vertical of [true, false]) {
      const along = (o: number) => (vertical ? [s + o, t] : [t, s + o]) as [number, number];
      const yaw = vertical ? 0 : Math.PI / 2;
      const r = rng.next();
      if (r < 0.07) { const [x, z] = along(rng.chance(0.5) ? -3 : 3); wreck(x, z, yaw + (rng.chance(0.5) ? 0 : Math.PI)); }
      else if (r < 0.085) { const [x, z] = along(rng.range(-4, 4)); barriers(x, z, !vertical, 2 + rng.int(2)); }
      else if (r < 0.1) {
        // Heaved asphalt and a slab of fallen facade.
        const [x, z] = along(rng.range(-4, 4)), gy = ground(x, z);
        if (free(x, z, 1.5)) block(x - rng.range(1, 2.2), z - rng.range(1, 2.2), x + rng.range(1, 2.2), z + rng.range(1, 2.2), gy - 0.6, gy + rng.range(0.3, 1.2), rng.chance(0.5) ? MAT.asphalt : MAT.rubble);
      } else if (r < 0.115) { const [x, z] = along(rng.chance(0.5) ? -7 : 7); place(rng.pick(['surv-barrel', 'surv-box-large', 'surv-box']), x, z, 0, 5); }
      // Lamp posts every 24 m on alternate sides of the pavement.
      if (Math.round(t / 6) % 4 === 0 && !STREETS.some(c => Math.abs(t - c) < KERB + 1)) {
        const [x, z] = along(Math.round(t / 24) % 2 ? KERB - 1 : -KERB + 1), gy = ground(x, z);
        if (free(x, z, 0.4)) block(x - 0.12, z - 0.12, x + 0.12, z + 0.12, gy - 0.5, gy + rng.range(2.5, 6), MAT.metal);
      }
    }
  }
  for (const [cx, cz, r] of CRATERS) for (let k = 0; k < 5; k++) {
    const a = rng.range(0, Math.PI * 2), x = cx + Math.cos(a) * r * 1.25, z = cz + Math.sin(a) * r * 1.25, gy = ground(x, z);
    if (free(x, z, 1)) block(x - 1.4, z - 1, x + 1.4, z + 1, gy - 0.6, gy + rng.range(0.35, 0.8), MAT.asphalt);
  }
  // Overgrowth: street trees, weeds through the cracks, and shrubs on the berm.
  for (let n = 0; n < 420; n++) {
    const x = rng.range(-RIM, RIM), z = rng.range(-RIM, RIM);
    if (Math.abs(x) > RIM - 14 || Math.abs(z) > RIM - 14) { place(rng.pick(['surv-tree', 'nat-tree_default', 'surv-tree-autumn']), x, z, rng.range(0, 6.28), rng.range(5, 7.5)); continue; }
    const kerb = STREETS.some(s => Math.abs(Math.abs(x - s) - (KERB - 1.2)) < 0.8) || STREETS.some(s => Math.abs(Math.abs(z - s) - (KERB - 1.2)) < 0.8);
    if (kerb) place(rng.pick(['house-tree-large', 'nat-tree_oak']), x, z, rng.range(0, 6.28), rng.range(5, 6.5));
  }
  for (let n = 0; n < 700; n++) {
    const x = rng.range(-HALF + 6, HALF - 6), z = rng.range(-HALF + 6, HALF - 6);
    if (!free(x, z, 0.2)) continue;
    place(rng.pick(['surv-grass-large', 'nat-grass_large', 'nat-plant_bush', 'nat-grass_large', 'nat-flower_yellowB']), x, z, rng.range(0, 6.28), rng.range(3.5, 6.5), false);
  }
  for (let n = 0; n < 60; n++) {
    const x = rng.range(-RIM + 6, RIM - 6), z = rng.range(-RIM + 6, RIM - 6);
    if (onRoad(x, z) && free(x, z, 2)) loot(x, z);
  }
  // Chests are solid like everywhere else.
  for (const c of map.chests) {
    const gy = c.y ?? ground(c.x, c.z);
    map.boxes.push({ minX: c.x - 0.7, minY: gy - 1, minZ: c.z - 0.7, maxX: c.x + 0.7, maxY: gy + 0.8, maxZ: c.z + 0.7 });
    map.boxMats.push(-1);
  }
}
