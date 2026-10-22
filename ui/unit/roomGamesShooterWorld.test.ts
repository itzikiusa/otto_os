import { test } from 'node:test';
import assert from 'node:assert/strict';
import { applyDamage, damageAt, rollRarity, rollWeapon, WEAPON_IDS, WEAPONS, type Rarity, type WeaponId } from '../src/modules/rooms/games/shooter/weapons.ts';
import { dryLand, GRID, HALF, harvestOf, inClimb, mapFor, navNext, SEA_SURFACE, terrainHeight, waterLevel, type Box, type Cylinder, type ShooterMap } from '../src/modules/rooms/games/shooter/map.ts';
import { groundAt, RADIUS, raycast, resolve, STEP, syncFelled, visible, worldFor, type Body } from '../src/modules/rooms/games/shooter/collide.ts';
import type { ShooterMapId } from '../src/modules/rooms/games/shooter/catalog.ts';

// ---------------------------------------------------------------- weapons

test('damageAt: full damage up close, linear falloff to minFalloff at range', () => {
  const rifle = WEAPONS.rifle;
  assert.equal(damageAt(rifle, 0, 0, false), 29);
  assert.equal(damageAt(rifle, 0, rifle.falloff[0], false), 29);
  assert.equal(damageAt(rifle, 0, (rifle.falloff[0] + rifle.falloff[1]) / 2, false), Math.round(29 * (1 - 0.5 * (1 - rifle.minFalloff))));
  assert.equal(damageAt(rifle, 0, rifle.falloff[1], false), Math.round(29 * rifle.minFalloff));
  assert.equal(damageAt(rifle, 0, 10_000, false), Math.round(29 * rifle.minFalloff));
  for (const id of WEAPON_IDS) {
    const def = WEAPONS[id];
    let prev = Infinity;
    for (let d = 0; d <= def.range * 1.5; d += 2) {
      const dmg = damageAt(def, 0, d, false);
      assert.ok(dmg <= prev && dmg > 0, `${id} at ${d}`);
      prev = dmg;
    }
  }
  // Rockets have no falloff (near == far).
  assert.equal(damageAt(WEAPONS.rocket, 0, 150, false), 95);
  assert.equal(damageAt(WEAPONS.rocket, 0, 0, false), 95);
});

test('damageAt: rarity and headshot multipliers', () => {
  const rifle = WEAPONS.rifle;
  const byRarity = ([0, 1, 2, 3, 4] as Rarity[]).map(r => damageAt(rifle, r, 0, false));
  assert.deepEqual(byRarity, [29, 31, 32, 34, 36]);
  assert.equal(damageAt(rifle, 0, 0, true), Math.round(29 * rifle.head));
  assert.equal(damageAt(WEAPONS.sniper, 4, 0, true), Math.round(98 * 1.25 * 2.5));
  for (const id of WEAPON_IDS) assert.ok(damageAt(WEAPONS[id], 0, 5, true) >= damageAt(WEAPONS[id], 0, 5, false));
});

test('applyDamage: shield absorbs first, hp never below zero', () => {
  assert.deepEqual(applyDamage(100, 50, 30), { hp: 100, shield: 20, absorbed: 30 });
  assert.deepEqual(applyDamage(100, 20, 50), { hp: 70, shield: 0, absorbed: 20 });
  assert.deepEqual(applyDamage(100, 0, 40), { hp: 60, shield: 0, absorbed: 0 });
  assert.deepEqual(applyDamage(10, 0, 50), { hp: 0, shield: 0, absorbed: 0 });
  assert.deepEqual(applyDamage(30, 50, 200), { hp: 0, shield: 0, absorbed: 50 });
  assert.deepEqual(applyDamage(80, 40, 0), { hp: 80, shield: 40, absorbed: 0 });
});

test('rollRarity: range, extremes and chest bonus', () => {
  assert.equal(rollRarity(0), 0);
  assert.equal(rollRarity(0.999999), 4);
  const avg = (bonus: number) => {
    let sum = 0;
    for (let i = 0; i < 1000; i++) {
      const r = rollRarity(i / 1000, bonus);
      assert.ok(Number.isInteger(r) && r >= 0 && r <= 4);
      sum += r;
    }
    return sum / 1000;
  };
  const plain = avg(0), chest = avg(1.2);
  assert.ok(chest > plain + 0.3, `chest avg ${chest} vs ${plain}`);
  // Commons are the most likely plain roll (40%).
  let commons = 0;
  for (let i = 0; i < 1000; i++) if (rollRarity(i / 1000) === 0) commons++;
  assert.ok(Math.abs(commons - 400) <= 2);
});

test('rollWeapon: covers the table proportionally', () => {
  assert.equal(rollWeapon(0), 'pistol');
  assert.equal(rollWeapon(0.999999), 'rocket');
  const counts: Partial<Record<WeaponId, number>> = {};
  for (let i = 0; i < 1000; i++) { const w = rollWeapon(i / 1000); counts[w] = (counts[w] ?? 0) + 1; }
  assert.deepEqual(Object.keys(counts).sort(), [...WEAPON_IDS].sort());
  // Weights 24/20/24/18/8/6 of 100.
  assert.ok(Math.abs((counts.sniper ?? 0) - 80) <= 2 && Math.abs((counts.rifle ?? 0) - 240) <= 2);
});

// ---------------------------------------------------------------- map

test('mapFor: cached, falls back to isle, every map distinct', () => {
  assert.equal(mapFor('isle'), mapFor('isle'));
  assert.equal(mapFor('volcano').id, 'isle');
  assert.equal(mapFor('mesa').id, 'mesa');
  assert.equal(mapFor('deadcity').id, 'deadcity');
  assert.equal(mapFor('deadcity').theme, 'city');
  assert.notDeepEqual(mapFor('isle').heights.slice(0, 500), mapFor('mesa').heights.slice(0, 500));
  assert.notDeepEqual(mapFor('deadcity').heights.slice(20000, 20500), mapFor('mesa').heights.slice(20000, 20500));
});

test('mapFor is deterministic: a fresh module instance generates the identical world', async () => {
  // A query string forces a separate module instance with an empty cache.
  const spec = new URL('../src/modules/rooms/games/shooter/map.ts?fresh=1', import.meta.url).href;
  const other = (await import(spec)) as typeof import('../src/modules/rooms/games/shooter/map.ts');
  for (const id of ['isle', 'mesa', 'deadcity']) {
    const a = mapFor(id), b = other.mapFor(id);
    assert.notEqual(a, b);
    assert.deepEqual(b.heights, a.heights);
    assert.deepEqual(b.props, a.props);
    assert.deepEqual(b.boxes, a.boxes);
    assert.deepEqual(b.cylinders, a.cylinders);
    assert.deepEqual(b.lootSpots, a.lootSpots);
    assert.deepEqual(b.chests, a.chests);
    assert.deepEqual(b.openings, a.openings);
    assert.deepEqual(b.fires, a.fires);
    assert.deepEqual(b.anchors, a.anchors);
  }
});

test('terrainHeight: exact on grid nodes, bilinear between, clamped outside', () => {
  for (const id of ['isle', 'mesa']) {
    const m = mapFor(id);
    assert.equal(m.size, HALF * 2 / GRID + 1);
    assert.equal(m.heights.length, m.size * m.size);
    for (const [ix, iz] of [[10, 20], [80, 80], [150, 33]]) {
      const x = -HALF + ix * GRID, z = -HALF + iz * GRID;
      const h = (i: number, j: number) => m.heights[j * m.size + i];
      assert.ok(Math.abs(terrainHeight(m, x, z) - h(ix, iz)) < 1e-5, `${id} node ${ix},${iz}`);
      const centre = (h(ix, iz) + h(ix + 1, iz) + h(ix, iz + 1) + h(ix + 1, iz + 1)) / 4;
      assert.ok(Math.abs(terrainHeight(m, x + GRID / 2, z + GRID / 2) - centre) < 1e-4, `${id} cell centre`);
      const edge = (h(ix, iz) * 0.75 + h(ix + 1, iz) * 0.25);
      assert.ok(Math.abs(terrainHeight(m, x + GRID / 4, z) - edge) < 1e-4, `${id} along x`);
    }
    for (const [x, z] of [[-1e6, 0], [1e6, 1e6], [0, -500], [0, 9999]]) assert.ok(Number.isFinite(terrainHeight(m, x, z)));
  }
  // Isle: sea at the rim, land in the middle.
  const isle = mapFor('isle');
  assert.ok(terrainHeight(isle, HALF - 2, HALF - 2) < 0);
  assert.ok(terrainHeight(isle, 6, -4) > 1);
});

test('both maps have colliders, props, loot spots and chests inside the bounds', () => {
  for (const id of ['isle', 'mesa']) {
    const m = mapFor(id);
    assert.ok(m.props.length > 100, `${id} props ${m.props.length}`);
    assert.ok(m.boxes.length > 50 && m.cylinders.length > 20, `${id} colliders`);
    assert.ok(m.lootSpots.length >= 20, `${id} loot spots ${m.lootSpots.length}`);
    assert.ok(m.chests.length >= 5, `${id} chests ${m.chests.length}`);
    assert.ok(m.pois.length >= 7, `${id} pois ${m.pois.length}`);
    for (const kind of ['town', 'factory', 'forest', 'hill', 'lake', 'farm', 'camp'] as const) assert.ok(m.pois.some(p => p.kind === kind), `${id} has a ${kind}`);
    assert.equal(new Set(m.pois.map(p => p.name)).size, m.pois.length, 'unique names');
    assert.equal(m.boxMats.length, m.boxes.length, 'harvest material per box');
    assert.ok(m.boxMats.every(v => v >= -1 && v <= 2));
    // Walk-in architecture: hundreds of blocks (walls, slabs, steps), indoor loot on upper floors, chests indoors.
    assert.ok(m.blocks.length > 400, `${id} blocks ${m.blocks.length}`);
    for (const b of m.blocks) assert.ok(m.boxes.includes(b), 'every block collides');
    const indoor = m.lootSpots.filter(l => l.y !== undefined);
    assert.ok(indoor.length >= 15, `${id} indoor loot ${indoor.length}`);
    assert.ok(indoor.some(l => l.y! > terrainHeight(m, l.x, l.z) + 6), 'loot two floors up');
    assert.ok(m.chests.some(c => c.y !== undefined), 'a chest indoors');
    for (const p of m.props) {
      assert.ok(Math.abs(p.x) <= HALF && Math.abs(p.z) <= HALF, `${id} prop ${p.key} at ${p.x},${p.z}`);
      assert.ok([p.x, p.y, p.z, p.yaw, p.scale].every(Number.isFinite) && p.scale > 0);
    }
    for (const b of m.boxes) assert.ok(b.minX < b.maxX && b.minY < b.maxY && b.minZ < b.maxZ && Math.abs(b.minX) <= HALF && Math.abs(b.maxZ) <= HALF);
    for (const c of m.cylinders) assert.ok(c.r > 0 && c.y0 < c.y1);
    for (const s of [...m.lootSpots, ...m.chests]) assert.ok(Math.abs(s.x) < HALF && Math.abs(s.z) < HALF);
    // Every chest is solid.
    for (const c of m.chests) assert.ok(m.boxes.some(b => c.x > b.minX && c.x < b.maxX && c.z > b.minZ && c.z < b.maxZ), `${id} chest collider`);
  }
});

test('water: the isle sea, river and lake sit below the surface; the mesa oasis holds water, dry land elsewhere', () => {
  const isle = mapFor('isle'), mesa = mapFor('mesa');
  const lake = isle.pois.find(p => p.kind === 'lake')!;
  assert.ok(terrainHeight(isle, lake.x, lake.z) < SEA_SURFACE - 1, 'Loot Lake is deep');
  for (let i = 1; i < isle.river.length - 1; i++) {
    const [x, z] = isle.river[i];
    assert.ok(terrainHeight(isle, x, z) < SEA_SURFACE, `river at ${x},${z}`);
    assert.equal(dryLand(isle, x, z), false);
  }
  assert.equal(waterLevel(isle, 0, 0), SEA_SURFACE);
  const oasis = mesa.pois.find(p => p.kind === 'lake')!;
  assert.equal(mesa.waters.length, 1);
  assert.ok(terrainHeight(mesa, oasis.x, oasis.z) < mesa.waters[0].y - 1, 'oasis is deep');
  assert.equal(waterLevel(mesa, oasis.x, oasis.z), mesa.waters[0].y);
  assert.equal(waterLevel(mesa, -150, 150), -Infinity);
  assert.equal(dryLand(mesa, oasis.x, oasis.z), false);
  const town = mesa.pois.find(p => p.kind === 'town')!;
  assert.equal(dryLand(mesa, town.x + 20, town.z + 20), true);
  // Footbridges span the isle river.
  assert.ok(isle.blocks.filter(b => b.mat === 7).length > 30, 'decks');
});

test('harvestOf: trees and boxes give wood, rocks brick, vehicles and metal metal', () => {
  assert.equal(harvestOf('surv-box-large'), 0);
  assert.equal(harvestOf('house-building-type-a'), 0);
  assert.equal(harvestOf('nat-rock_tallB'), 1);
  assert.equal(harvestOf('surv-rock-sand-a'), 1);
  assert.equal(harvestOf('car-police'), 2);
  assert.equal(harvestOf('ind-shipping-container-a'), 2);
  assert.equal(harvestOf('surv-barrel'), 2);
});

// ---------------------------------------------------------------- collide

/** A flat test island (height 0) with one 2×3×2 crate and one tree trunk. */
function flatMap(id: string, boxes: Box[], cylinders: Cylinder[], height: (x: number, z: number) => number = () => 0): ShooterMap {
  const size = HALF * 2 / GRID + 1;
  const heights = new Float32Array(size * size);
  for (let iz = 0; iz < size; iz++) for (let ix = 0; ix < size; ix++) heights[iz * size + ix] = height(-HALF + ix * GRID, -HALF + iz * GRID);
  return {
    id: id as ShooterMapId, theme: 'mesa', heights, size, props: [], boxes, cylinders, pois: [], boxMats: boxes.map(() => 0), trunkProps: cylinders.map(() => -1), blocks: [], waters: [], river: [], lootSpots: [], chests: [], climbs: [], pads: [],
    openings: [], fires: [], roads: [], anchors: { spawns: [], teamSpawns: [[], []], flags: [], zombies: { cx: 0, cz: 0, r: 0, start: [], spawns: [], barricades: [], buyWalls: [], extraction: { x: 0, y: 0, z: 0, r: 0 } }, nav: { nodes: [], links: [] } },
  };
}
const crate: Box = { minX: 10, minY: -1, minZ: -1, maxX: 12, maxY: 3, maxZ: 1 };
const trunk: Cylinder = { x: 0, z: 20, r: 1, y0: -1, y1: 7 };
const flat = worldFor(flatMap('unit-flat', [crate], [trunk]));
const body = (x: number, y: number, z: number, vx = 0, vz = 0): Body => ({ x, y, z, vx, vy: 0, vz, grounded: true });

test('worldFor caches per map id', () => {
  assert.equal(worldFor(flat.map), flat);
  assert.equal(worldFor(mapFor('isle')), worldFor(mapFor('isle')));
});

test('raycast hits a box, a trunk and the terrain; misses into the sky', () => {
  const box = raycast(flat, 0, 1.5, 0, 1, 0, 0, 50);
  assert.equal(box.kind, 'prop');
  assert.ok(Math.abs(box.t - 10) < 1e-9);
  const tree = raycast(flat, 0, 1, 0, 0, 0, 1, 50);
  assert.equal(tree.kind, 'prop');
  assert.ok(Math.abs(tree.t - 19) < 1e-9);
  // Over the crate's top: no hit.
  assert.equal(raycast(flat, 0, 3.5, 0, 1, 0, 0, 50).kind, 'none');
  // Out of range: no hit, t = max.
  const short = raycast(flat, 0, 1.5, 0, 1, 0, 0, 8);
  assert.equal(short.kind, 'none');
  assert.equal(short.t, 8);
  // Down into flat ground from 5 m: t = 5 / 0.8 (bisection precision).
  const ground = raycast(flat, -30, 5, -30, 0.6, -0.8, 0, 50);
  assert.equal(ground.kind, 'terrain');
  assert.ok(Math.abs(ground.t - 6.25) < 0.05, `terrain t ${ground.t}`);
  assert.equal(raycast(flat, -30, 5, -30, 0, 1, 0, 50).kind, 'none');
});

test('raycast hits a hillside on real terrain', () => {
  const slope = worldFor(flatMap('unit-slope', [], [], x => Math.max(0, x - 20)));
  const hit = raycast(slope, 0, 2, 0, 1, 0, 0, 100);
  assert.equal(hit.kind, 'terrain');
  assert.ok(Math.abs(hit.t - 22) < 0.1, `slope t ${hit.t}`);
});

test('visible: blocked by the crate, clear around it', () => {
  assert.equal(visible(flat, 0, 1.5, 0, 20, 1.5, 0), false);
  assert.equal(visible(flat, 0, 1.5, 5, 20, 1.5, 5), true);
  assert.equal(visible(flat, 0, 1.5, 0, 0, 1.5, 30), false);
  assert.equal(visible(flat, 3, 1, 3, 3, 1, 3), true);
});

test('resolve pushes a body out of a box and cancels velocity into it', () => {
  const p = body(10 - RADIUS / 2, 0, 0, 3, 0);
  assert.equal(resolve(flat, p), true);
  assert.ok(Math.abs(p.x - (10 - RADIUS)) < 1e-9);
  assert.equal(p.vx, 0);
  // Centre inside the box leaves through the nearest face.
  const inside = body(11.8, 0, 0.1);
  resolve(flat, inside);
  assert.ok(Math.abs(inside.x - (12 + RADIUS)) < 1e-9 && inside.z === 0.1);
  // Standing on top (feet above maxY - STEP/2): not pushed.
  const top = body(11, 3, 0);
  assert.equal(resolve(flat, top), false);
  assert.equal(top.x, 11);
  // Trunks push out radially.
  const t = body(0.5, 0, 20);
  assert.equal(resolve(flat, t), true);
  assert.ok(Math.abs(Math.hypot(t.x - trunk.x, t.z - trunk.z) - (trunk.r + RADIUS)) < 1e-9);
  // Free space: untouched. Map edge: clamped.
  const free = body(-40, 0, -40);
  assert.equal(resolve(flat, free), false);
  assert.deepEqual([free.x, free.z], [-40, -40]);
  const edge = body(HALF + 10, 0, -HALF - 10);
  resolve(flat, edge);
  assert.deepEqual([edge.x, edge.z], [HALF - 2, -HALF + 2]);
});

test('groundAt stands on a crate top only when stepping up is possible', () => {
  assert.equal(groundAt(flat, 11, 0, 3), 3);
  assert.equal(groundAt(flat, 11, 0, 3 - STEP + 0.01), 3);
  assert.equal(groundAt(flat, 11, 0, 0), 0);
  assert.equal(groundAt(flat, -20, 0, 50), 0);
  // Real map: a chest lid is standable from just below its top.
  const isle = mapFor('isle'), world = worldFor(isle);
  const chest = isle.chests[0], gy = terrainHeight(isle, chest.x, chest.z);
  assert.ok(groundAt(world, chest.x, chest.z, gy + 0.8) >= gy + 0.8 - 1e-6);
});

test('syncFelled: a felled trunk stops blocking rays and bodies until the list brings it back', () => {
  const ray = () => raycast(flat, 0, 1.5, 10, 0, 0, 1, 20);
  assert.equal(ray().kind, 'prop');
  const felled = [0];
  syncFelled(flat, felled);
  assert.equal(ray().kind, 'none');
  const p = body(0, 0, 20.3); // standing where the trunk was
  assert.equal(resolve(flat, p), false);
  assert.equal(p.z, 20.3);
  // The same list is a no-op; a new (empty) list restores the trunk.
  flat.felled[0] = 0;
  syncFelled(flat, felled);
  assert.equal(flat.felled[0], 0, 'unchanged list: mask not rebuilt');
  syncFelled(flat, []);
  assert.equal(ray().kind, 'prop');
  assert.equal(resolve(flat, p), true);
  // Out-of-range indices are ignored.
  syncFelled(flat, [5, -1]);
  assert.equal(ray().kind, 'prop');
});

// ---------------------------------------------------------------- Dead City and mode anchors

const MAPS = ['isle', 'mesa', 'deadcity'] as const;
/** Is a body of radius `r` standing at (x, z) on the terrain inside something it cannot step onto? */
const blocked = (m: ShooterMap, x: number, z: number, r: number): boolean => {
  const feet = terrainHeight(m, x, z);
  return m.boxes.some(b => b.maxY > feet + STEP && b.minY < feet + 1.8 && x > b.minX - r && x < b.maxX + r && z > b.minZ - r && z < b.maxZ + r)
    || m.cylinders.some(c => Math.hypot(x - c.x, z - c.z) < c.r + r);
};

test('Dead City: a ruined downtown with every landmark, streets, fires, wrecks and walk-in ruins', () => {
  const m = mapFor('deadcity');
  for (const kind of ['tower', 'hospital', 'mall', 'subway', 'checkpoint', 'park', 'town', 'factory'] as const) assert.ok(m.pois.some(p => p.kind === kind), `has a ${kind}`);
  assert.equal(m.pois.filter(p => p.kind === 'tower').length, 2, 'two collapsed skyscrapers');
  assert.equal(new Set(m.pois.map(p => p.name)).size, m.pois.length, 'unique names');
  assert.ok(m.roads.length >= 12 && m.roads.every(r => r.w > 8), `${m.roads.length} streets`);
  assert.ok(m.blocks.length > 2000, `blocks ${m.blocks.length}`);
  assert.ok(m.climbs.length >= 15, `walk-ins ${m.climbs.length}`);
  assert.ok(m.climbs.some(c => c.floors >= 5), 'a tall ruin');
  assert.ok(m.lootSpots.length >= 120 && m.chests.length >= 15, `loot ${m.lootSpots.length}, chests ${m.chests.length}`);
  assert.ok(m.fires.length >= 20 && m.fires.every(f => [f.x, f.y, f.z, f.s].every(Number.isFinite) && f.s > 0), `fires ${m.fires.length}`);
  assert.ok(m.props.filter(p => p.burnt).length >= 20 && m.props.filter(p => p.burnt).every(p => /^car-/.test(p.key)), 'burnt wrecks are cars');
  assert.ok(m.props.some(p => /tree/.test(p.key)) && m.props.some(p => /grass|bush/.test(p.key)), 'overgrowth');
  assert.ok(m.pads.length >= 3, 'launch pads');
  // Every material in the palette shows up somewhere: rubble, charred, sandbags, glass, hazard…
  for (const mat of [9, 10, 11, 12, 13, 14, 15, 16]) assert.ok(m.blocks.some(b => b.mat === mat), `material ${mat} used`);
  for (const b of m.blocks) assert.ok(m.boxes.includes(b), 'every block collides');
  assert.equal(m.boxMats.length, m.boxes.length);
  for (const p of m.props) assert.ok(Math.abs(p.x) <= HALF && Math.abs(p.z) <= HALF && p.scale > 0);
  for (const c of m.chests) assert.ok(m.boxes.some(b => c.x > b.minX && c.x < b.maxX && c.z > b.minZ && c.z < b.maxZ), 'chest collider');
  // Dry streets; the only water is the park fountain.
  const park = m.pois.find(p => p.kind === 'park')!;
  assert.equal(m.waters.length, 1);
  assert.ok(Math.hypot(m.waters[0].x - park.x, m.waters[0].z - park.z) < 1);
  assert.equal(waterLevel(m, 30, 100), -Infinity);
  assert.equal(dryLand(m, 30, 100), true);
  // A rubble berm rings the city.
  assert.ok(terrainHeight(m, HALF - 6, 0) > 10 && terrainHeight(m, 0, -HALF + 6) > 10, 'berm');
  assert.ok(Math.abs(terrainHeight(m, 30, 60)) < 1, 'street level');
});

test('Dead City: the subway is cut below the street with stairs down, and the Spire ramp climbs into storey three', () => {
  const m = mapFor('deadcity'), world = worldFor(m);
  const sub = m.pois.find(p => p.kind === 'subway')!;
  assert.ok(terrainHeight(m, -6, 140) < -4, 'platform level');
  // Walk the stairs: the floor under the feet never steps more than a step-up.
  let feet = groundAt(world, -6, 117, 0.2);
  for (let z = 117; z <= 140; z += 0.3) {
    const next = groundAt(world, -6, z, feet + 0.05);
    assert.ok(Math.abs(next - feet) < STEP, `stairs at z ${z.toFixed(1)}: ${feet.toFixed(2)} -> ${next.toFixed(2)}`);
    feet = next;
  }
  assert.ok(feet < -4, `down on the platform (${feet.toFixed(2)})`);
  assert.ok(m.lootSpots.some(l => l.y !== undefined && l.y < -3) && m.chests.some(c => c.y !== undefined && c.y < -3), 'loot and a chest underground');
  assert.ok(Math.hypot(sub.x + 6, sub.z - 133) < 1);
  // The Spire: the stepped ramp's top meets a breach in the stump at storey three.
  const spire = m.pois.find(p => p.name === 'The Spire')!;
  const stump = m.climbs.find(c => Math.abs(c.cx - spire.x) < 1 && Math.abs(c.cz - spire.z) < 1)!;
  assert.ok(stump && stump.floors === 5);
  const floor3 = stump.base + 3 * 3.6, edge = stump.box.maxX;
  let y = groundAt(world, 46, stump.cz - 4, 0.3);
  for (let x = 46; x > edge + 0.3; x -= 0.4) {
    const next = groundAt(world, x, stump.cz - 4, y + 0.05);
    assert.ok(next - y < STEP + 1e-6, `ramp at x ${x.toFixed(1)}: ${y.toFixed(2)} -> ${next.toFixed(2)}`);
    y = next;
  }
  assert.ok(Math.abs(y - floor3) < 0.5, `ramp top ${y.toFixed(2)} vs storey three ${floor3.toFixed(2)}`);
  // ...and the wall there is open from the floor up.
  assert.equal(raycast(world, edge + 1, floor3 + 1.2, stump.cz - 4, -1, 0, 0, 3).kind, 'none', 'breach');
});

test('anchors: spread free-for-all spawns, two facing team bases, three flags between them, on open dry ground', () => {
  for (const id of MAPS) {
    const m = mapFor(id), a = m.anchors;
    assert.ok(a.spawns.length >= 16, `${id} spawns ${a.spawns.length}`);
    for (const s of [...a.spawns, ...a.teamSpawns[0], ...a.teamSpawns[1]]) {
      assert.ok(!blocked(m, s.x, s.z, 0.8), `${id} spawn at ${s.x.toFixed(1)},${s.z.toFixed(1)} is clear`);
      assert.ok(dryLand(m, s.x, s.z), `${id} spawn on dry land`);
      assert.ok(Math.abs(s.y - terrainHeight(m, s.x, s.z)) < 1e-6 && Number.isFinite(s.yaw));
    }
    let closest = Infinity;
    for (let i = 0; i < a.spawns.length; i++) for (let j = i + 1; j < a.spawns.length; j++) closest = Math.min(closest, Math.hypot(a.spawns[i].x - a.spawns[j].x, a.spawns[i].z - a.spawns[j].z));
    assert.ok(closest > 30, `${id} spawns spread (${closest.toFixed(0)} m)`);
    assert.equal(a.teamSpawns[0].length, 8); assert.equal(a.teamSpawns[1].length, 8);
    const ca = centre(a.teamSpawns[0]), cb = centre(a.teamSpawns[1]);
    assert.ok(Math.hypot(ca.x - cb.x, ca.z - cb.z) > 150, `${id} bases apart`);
    // Each team faces the other base.
    for (const s of a.teamSpawns[0]) assert.ok(Math.sin(s.yaw) * (cb.x - s.x) + Math.cos(s.yaw) * (cb.z - s.z) > 0, `${id} team A faces B`);
    assert.deepEqual(a.flags.map(f => f.name), ['A', 'B', 'C']);
    for (const f of a.flags) assert.ok(!blocked(m, f.x, f.z, 2) && f.r > 3 && f.label.length > 0, `${id} flag ${f.name} clear`);
    for (let i = 0; i < 3; i++) for (let j = i + 1; j < 3; j++) assert.ok(Math.hypot(a.flags[i].x - a.flags[j].x, a.flags[i].z - a.flags[j].z) > 40, `${id} flags apart`);
    // A is nearer the first base than C is.
    assert.ok(Math.hypot(a.flags[0].x - ca.x, a.flags[0].z - ca.z) < Math.hypot(a.flags[2].x - ca.x, a.flags[2].z - ca.z), `${id} A sits on team A's side`);
  }
});
const centre = (l: { x: number; z: number }[]) => ({ x: l.reduce((s, p) => s + p.x, 0) / l.length, z: l.reduce((s, p) => s + p.z, 0) / l.length });

test('anchors: zombies get boardable windows and doors, wall-buys on outer walls, an extraction pad and a spawn ring', () => {
  for (const id of MAPS) {
    const m = mapFor(id), z = m.anchors.zombies, world = worldFor(m);
    assert.ok(z.r > 30 && Number.isFinite(z.cx + z.cz), `${id} arena`);
    assert.ok(z.barricades.length >= 8, `${id} barricades ${z.barricades.length}`);
    for (const b of z.barricades) {
      assert.ok(Math.hypot(b.x - z.cx, b.z - z.cz) < z.r && b.w > 1 && b.h > 1, `${id} barricade in the arena`);
      // The hole is open: a ray through its middle along its facing passes, but just beside it there is wall.
      const fx = Math.sin(b.yaw), fz = Math.cos(b.yaw), y = b.y + b.h / 2;
      assert.equal(raycast(world, b.x + fx * 0.8, y, b.z + fz * 0.8, -fx, 0, -fz, 1.4).kind, 'none', `${id} opening at ${b.x.toFixed(1)},${b.z.toFixed(1)} is open`);
      const sx = b.x + fz * (b.w / 2 + 0.4), sz = b.z - fx * (b.w / 2 + 0.4);
      assert.notEqual(raycast(world, sx + fx * 0.8, y, sz + fz * 0.8, -fx, 0, -fz, 1.4).kind, 'none', `${id} wall beside the opening`);
    }
    assert.ok(z.buyWalls.length >= 4, `${id} wall-buys ${z.buyWalls.length}`);
    assert.equal(new Set(z.buyWalls.map(w => w.weapon)).size, z.buyWalls.length, 'different guns');
    for (const w of z.buyWalls) {
      assert.ok(w.weapon in WEAPONS && w.cost >= 500, `${id} ${w.weapon}`);
      // The chalk outline is on a wall: facing outward, the wall is right behind it.
      const fx = Math.sin(w.yaw), fz = Math.cos(w.yaw);
      const hit = raycast(world, w.x + fx * 0.5, w.y, w.z + fz * 0.5, -fx, 0, -fz, 1);
      assert.ok(hit.kind === 'prop' && hit.t < 0.7, `${id} wall-buy ${w.weapon} on a wall`);
    }
    const e = z.extraction;
    assert.ok(e.r >= 3 && !blocked(m, e.x, e.z, e.r - 1), `${id} extraction pad clear`);
    assert.equal(z.spawns.length, 16);
    assert.equal(z.start.length, 4);
    for (const s of [...z.spawns, ...z.start]) assert.ok(!blocked(m, s.x, s.z, 0.6), `${id} zombie / survivor spawn clear`);
  }
});

test('anchors: the street nav graph is connected, its links walkable, and navNext leads across the map', () => {
  for (const id of MAPS) {
    const m = mapFor(id), { nodes, links } = m.anchors.nav;
    assert.ok(nodes.length > 300, `${id} nodes ${nodes.length}`);
    links.forEach((l, i) => { for (const j of l) assert.ok(links[j].includes(i), 'links both ways'); });
    // One big connected component.
    const seen = new Uint8Array(nodes.length), queue = [0];
    seen[0] = 1;
    while (queue.length) { const i = queue.pop()!; for (const j of links[i]) if (!seen[j]) { seen[j] = 1; queue.push(j); } }
    const reach = seen.reduce((s, v) => s + v, 0) / nodes.length;
    assert.ok(reach > 0.85, `${id}: ${(reach * 100).toFixed(0)}% of nodes connected`);
    // Every link is walkable (sampled): nothing tall in the way, no water.
    for (let i = 0; i < nodes.length; i += 3) for (const j of links[i]) {
      if (j < i) continue;
      for (let k = 1; k < 8; k++) {
        const x = nodes[i].x + (nodes[j].x - nodes[i].x) * (k / 8), z = nodes[i].z + (nodes[j].z - nodes[i].z) * (k / 8);
        assert.ok(!blocked(m, x, z, 0.3) && dryLand(m, x, z), `${id} link ${i}-${j} clear at ${x.toFixed(1)},${z.toFixed(1)}`);
      }
    }
    // Follow navNext from team base A to team base B.
    const from = centre(m.anchors.teamSpawns[0]), to = centre(m.anchors.teamSpawns[1]);
    let x = from.x, z = from.z, steps = 0;
    while (Math.hypot(to.x - x, to.z - z) > 24 && steps < 200) {
      const next = navNext(m, x, z, to.x, to.z);
      if (!next) break;
      assert.ok(Number.isFinite(next[0] + next[1]));
      [x, z] = [next[0], next[1]];
      steps++;
    }
    assert.ok(Math.hypot(to.x - x, to.z - z) <= 30, `${id}: reached the far base in ${steps} hops (${Math.hypot(to.x - x, to.z - z).toFixed(0)} m short)`);
    assert.equal(navNext(m, to.x, to.z, to.x, to.z), null, 'no hop needed at the goal');
    assert.equal(navNext({ ...m, anchors: { ...m.anchors, nav: { nodes: [], links: [] } } }, 0, 0, 100, 100), null, 'no graph, no route');
  }
});

test('openings: every ground-floor door and window of a walk-in is recorded, facing out of its building', () => {
  for (const id of MAPS) {
    const m = mapFor(id);
    assert.ok(m.openings.length >= m.climbs.length * 6, `${id} openings ${m.openings.length}`);
    assert.ok(m.openings.some(o => o.door) && m.openings.some(o => !o.door));
    for (const o of m.openings) {
      const c = m.climbs.find(k => inClimb(k, o.x, o.z, 0.6));
      if (!c) continue; // factory halls are not climbs
      // One step outward leaves the footprint.
      assert.ok(!inClimb(c, o.x + Math.sin(o.yaw) * 1.2, o.z + Math.cos(o.yaw) * 1.2), `${id} opening at ${o.x.toFixed(1)},${o.z.toFixed(1)} faces out`);
    }
  }
});
