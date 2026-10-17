import { test } from 'node:test';
import assert from 'node:assert/strict';
import { applyDamage, damageAt, rollRarity, rollWeapon, WEAPON_IDS, WEAPONS, type Rarity, type WeaponId } from '../src/modules/rooms/games/shooter/weapons.ts';
import { GRID, HALF, mapFor, terrainHeight, type Box, type Cylinder, type ShooterMap } from '../src/modules/rooms/games/shooter/map.ts';
import { groundAt, RADIUS, raycast, resolve, STEP, visible, worldFor, type Body } from '../src/modules/rooms/games/shooter/collide.ts';
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

test('mapFor: cached, falls back to isle, both maps distinct', () => {
  assert.equal(mapFor('isle'), mapFor('isle'));
  assert.equal(mapFor('volcano').id, 'isle');
  assert.equal(mapFor('mesa').id, 'mesa');
  assert.notDeepEqual(mapFor('isle').heights.slice(0, 500), mapFor('mesa').heights.slice(0, 500));
});

test('mapFor is deterministic: a fresh module instance generates the identical world', async () => {
  // A query string forces a separate module instance with an empty cache.
  const spec = new URL('../src/modules/rooms/games/shooter/map.ts?fresh=1', import.meta.url).href;
  const other = (await import(spec)) as typeof import('../src/modules/rooms/games/shooter/map.ts');
  for (const id of ['isle', 'mesa']) {
    const a = mapFor(id), b = other.mapFor(id);
    assert.notEqual(a, b);
    assert.deepEqual(b.heights, a.heights);
    assert.deepEqual(b.props, a.props);
    assert.deepEqual(b.boxes, a.boxes);
    assert.deepEqual(b.cylinders, a.cylinders);
    assert.deepEqual(b.lootSpots, a.lootSpots);
    assert.deepEqual(b.chests, a.chests);
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
    assert.equal(m.pois.length, 5);
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

// ---------------------------------------------------------------- collide

/** A flat test island (height 0) with one 2×3×2 crate and one tree trunk. */
function flatMap(id: string, boxes: Box[], cylinders: Cylinder[], height: (x: number, z: number) => number = () => 0): ShooterMap {
  const size = HALF * 2 / GRID + 1;
  const heights = new Float32Array(size * size);
  for (let iz = 0; iz < size; iz++) for (let ix = 0; ix < size; ix++) heights[iz * size + ix] = height(-HALF + ix * GRID, -HALF + iz * GRID);
  return { id: id as ShooterMapId, theme: 'mesa', heights, size, props: [], boxes, cylinders, pois: [], lootSpots: [], chests: [] };
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
