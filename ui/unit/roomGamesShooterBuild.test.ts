import { test } from 'node:test';
import assert from 'node:assert/strict';
import { HALF, mapFor, terrainHeight, waterLevel, type ShooterMap } from '../src/modules/rooms/games/shooter/map.ts';
import { groundAt, raycast, worldFor } from '../src/modules/rooms/games/shooter/collide.ts';
import {
  buildGeometry, canPlace, LEVEL, makeBuild, planBuild, rampHeight, rayRamp, SUB, TILE, tileOf, tileX, unsupported, type Build, type BuildSlot,
} from '../src/modules/rooms/games/shooter/builds.ts';
import { BUILD_COST, CONSUMABLES, MATERIALS, PICKAXE, WEAPONS } from '../src/modules/rooms/games/shooter/weapons.ts';
import {
  createMatch, EYE, heldItem, idleShooterInput, PICKAXE_KEY, SLOTS, STEP, SWIM_DEPTH, tickMatch,
  type Fighter, type ShooterEventType, type ShooterInput, type ShooterState,
} from '../src/modules/rooms/games/shooter/sim.ts';
import { shooterSeats, type HumanSeat } from '../src/modules/rooms/games/setup.ts';
import { coverFrom } from '../src/modules/rooms/games/shooter/bots.ts';

type Inputs = (ShooterInput | undefined)[];
const humans = (n: number): HumanSeat[] => Array.from({ length: n }, (_, i) => ({ slot: i, name: `P${i}`, pick: 'male-b' }));
const run = (s: ShooterState, seconds: number, inputs: Inputs | ((s: ShooterState) => Inputs) = []): void => {
  for (let i = 0, n = Math.round(seconds / STEP); i < n; i++) tickMatch(s, typeof inputs === 'function' ? inputs(s) : inputs);
};
const events = (s: ShooterState, type: ShooterEventType, from = 0) => s.events.filter(e => e.id > from && e.type === type);
const press = (k: Partial<ShooterInput>): ShooterInput => ({ ...idleShooterInput(), ...k });
const flatGround = () => 0;

/** First flat, prop-free spot on a map (same scan as the sim tests), centred on a build tile. */
function clearSpot(m: ShooterMap, clear = 16): { x: number; z: number } {
  for (let x = -120; x <= 120; x += 4) for (let z = -120; z <= 120; z += 4) {
    const cx = tileX(tileOf(x)) + TILE / 2, cz = tileX(tileOf(z)) + TILE / 2;
    const h0 = terrainHeight(m, cx, cz);
    if (h0 < waterLevel(m, cx, cz) + 1.5) continue;
    if (m.boxes.some(b => cx > b.minX - clear && cx < b.maxX + clear && cz > b.minZ - clear && cz < b.maxZ + clear)) continue;
    if (m.cylinders.some(c => Math.hypot(c.x - cx, c.z - cz) < c.r + clear)) continue;
    let flat = 0;
    for (let dx = -12; dx <= 12; dx += 2) for (let dz = -12; dz <= 12; dz += 2) flat = Math.max(flat, Math.abs(terrainHeight(m, cx + dx, cz + dz) - h0));
    if (flat < 0.4) return { x: cx, z: cz };
  }
  throw new Error(`no clear spot on ${m.id}`);
}
const MESA = mapFor('mesa');
const SPOT = clearSpot(MESA);
function put(s: ShooterState, p: Fighter, x: number, z: number, yaw = 0): void {
  const m = mapFor(s.map);
  p.x = x; p.z = z; p.y = groundAt(worldFor(m), x, z, terrainHeight(m, x, z) + 0.5, s.builds);
  p.vx = p.vy = p.vz = 0; p.mode = 'ground'; p.grounded = true; p.yaw = yaw;
}
/** Red Mesa, `n` humans on the ground (fighter 0 at SPOT, the rest 40 m away), no loot. */
function groundMatch(n: number): ShooterState {
  const s = createMatch({ map: 'mesa', difficulty: 'normal', seed: 5, seats: shooterSeats(humans(n), 0) });
  s.loot = [];
  s.players.forEach((p, i) => put(s, p, SPOT.x + (i === 0 ? 0 : 40 + i * 4), SPOT.z - (i === 0 ? 0 : 40)));
  tickMatch(s, []);
  return s;
}

// ------------------------------------------------------------ geometry

test('buildGeometry: walls split around doorways and windows, floors are slabs, ramps are surfaces', () => {
  const g = 60, x0 = tileX(g); // x0 = 0
  assert.equal(x0, 0);
  const wall = (variant: number, dir = 0) => buildGeometry({ kind: 0, gx: g, gz: g, level: 0, dir, variant });
  const solid = wall(0);
  assert.equal(solid.boxes.length, 1);
  assert.deepEqual([solid.aabb.minX, solid.aabb.maxX, solid.aabb.minY, solid.aabb.maxY], [0, TILE, 0, LEVEL]);
  assert.ok(solid.aabb.minZ < TILE && solid.aabb.maxZ > TILE, 'on the +z edge');
  const side = wall(0, 1);
  assert.ok(side.aabb.minX < TILE && side.aabb.maxX > TILE && side.aabb.minZ === 0 && side.aabb.maxZ === TILE, 'dir 1: the +x edge');
  const inside = (boxes: { minX: number; maxX: number; minY: number; maxY: number }[], x: number, y: number) => boxes.some(b => x > b.minX && x < b.maxX && y > b.minY && y < b.maxY);
  const door = wall(1), win = wall(2);
  assert.equal(door.boxes.length, 3);
  assert.equal(inside(door.boxes, 2, 0.5), false, 'doorway open at the bottom');
  assert.equal(inside(door.boxes, 2, 2.8), true, 'lintel');
  assert.equal(inside(door.boxes, 0.5, 1), true);
  assert.equal(win.boxes.length, 4);
  assert.equal(inside(win.boxes, 2, 0.5), true, 'sill');
  assert.equal(inside(win.boxes, 2, 1.7), false, 'window');
  assert.equal(inside(win.boxes, 2, 2.8), true, 'header');
  const floor = buildGeometry({ kind: 1, gx: g, gz: g, level: 5, dir: 0, variant: 0 });
  assert.equal(floor.boxes.length, 1);
  assert.equal(floor.boxes[0].maxY, 5 * SUB);
  const ramp = makeBuild(1, { kind: 2, gx: g, gz: g, level: 0, dir: 0 }, 0, 0);
  assert.equal(ramp.boxes.length, 0);
  assert.equal(rampHeight(ramp, 2, 0), 0);
  assert.ok(Math.abs(rampHeight(ramp, 2, 2) - LEVEL / 2) < 1e-9);
  assert.ok(Math.abs(rampHeight(ramp, 2, TILE) - LEVEL) < 1e-9);
  assert.equal(rampHeight(ramp, 2, TILE + 0.1), -Infinity);
  for (const [dir, x, z] of [[1, 3, 2], [2, 2, 1], [3, 1, 2]] as const) {
    const r = makeBuild(1, { kind: 2, gx: g, gz: g, level: 0, dir }, 0, 0);
    assert.ok(Math.abs(rampHeight(r, x, z) - LEVEL * 0.75) < 1e-9, `dir ${dir}`);
  }
});

test('rayRamp hits the slope from above and below, misses beside the tile', () => {
  const ramp = makeBuild(1, { kind: 2, gx: 60, gz: 60, level: 0, dir: 0 }, 0, 0);
  // Straight down onto the middle of the ramp from 10 m.
  assert.ok(Math.abs(rayRamp(ramp, 2, 10, 2, 0, -1, 0, 50) - (10 - LEVEL / 2)) < 1e-9);
  // Horizontally from under the high end: the underside.
  assert.ok(Math.abs(rayRamp(ramp, 2, 1.6, -5, 0, 0, 1, 50) - 7) < 1e-9);
  assert.equal(rayRamp(ramp, 6, 10, 2, 0, -1, 0, 50), -1, 'beside the tile');
  assert.equal(rayRamp(ramp, 2, 10, 2, 0, -1, 0, 5), -1, 'out of range');
});

test('planBuild: walls on the facing edge (canonical), floors and ramps ahead, floors overhead when looking up', () => {
  const x = 2, z = 2, y = 0.3; // centre of tile (60, 60)
  assert.deepEqual(planBuild(0, x, y, z, 0, 0, []), { kind: 0, gx: 60, gz: 60, level: 0, dir: 0 });
  assert.deepEqual(planBuild(0, x, y, z, Math.PI / 2, 0, []), { kind: 0, gx: 60, gz: 60, level: 0, dir: 1 });
  // Facing -z / -x: the wall belongs to the neighbouring tile's +z / +x edge.
  assert.deepEqual(planBuild(0, x, y, z, Math.PI, 0, []), { kind: 0, gx: 60, gz: 59, level: 0, dir: 0 });
  assert.deepEqual(planBuild(0, x, y, z, -Math.PI / 2, 0, []), { kind: 0, gx: 59, gz: 60, level: 0, dir: 1 });
  assert.deepEqual(planBuild(1, x, y, z, 0, 0, []), { kind: 1, gx: 60, gz: 61, level: 0, dir: 0 });
  assert.deepEqual(planBuild(1, x, y, z, 0, 0.8, []), { kind: 1, gx: 60, gz: 61, level: LEVEL / SUB, dir: 0 }, 'overhead');
  assert.deepEqual(planBuild(2, x, y, z, Math.PI / 2, 0, []), { kind: 2, gx: 61, gz: 60, level: 0, dir: 1 });
  // Heights snap down to 0.8 m so a wall always covers a standing survivor.
  assert.equal(planBuild(0, x, 2.3, z, 0, 0, []).level, 2);
  assert.equal(planBuild(0, x, 2.4, z, 0, 0, []).level, 3);
  // Ramp rush: halfway up a ramp, the next ramp starts at the top of this one.
  const first = makeBuild(1, { kind: 2, gx: 60, gz: 61, level: 0, dir: 0 }, 0, 0);
  assert.deepEqual(planBuild(2, 2, 2.6, 7.2, 0, 0, [first]), { kind: 2, gx: 60, gz: 62, level: LEVEL / SUB, dir: 0 });
});

test('canPlace needs ground or a touching piece and refuses same-kind overlaps; unsupported() finds what would fall', () => {
  const wall: BuildSlot = { kind: 0, gx: 60, gz: 60, level: 0, dir: 0 };
  const floating: BuildSlot = { kind: 1, gx: 60, gz: 61, level: 10, dir: 0 };
  const onWall: BuildSlot = { kind: 1, gx: 60, gz: 61, level: LEVEL / SUB, dir: 0 };
  assert.equal(canPlace(wall, [], flatGround), true);
  assert.equal(canPlace(floating, [], flatGround), false, 'nothing holds it up');
  const w = makeBuild(1, wall, 0, 0);
  assert.equal(canPlace(wall, [w], flatGround), false, 'occupied');
  assert.equal(canPlace({ ...wall, level: 1 }, [w], flatGround), false, 'overlapping wall');
  assert.equal(canPlace({ ...wall, level: 4 }, [w], flatGround), true, 'stacked wall');
  assert.equal(canPlace(onWall, [w], flatGround), true, 'floor resting on the wall top');
  assert.equal(canPlace({ kind: 1, gx: 60, gz: 60, level: 0, dir: 0 }, [w], flatGround), true, 'different kinds share a tile');
  const f = makeBuild(2, onWall, 0, 0);
  const far = makeBuild(3, { kind: 1, gx: 61, gz: 61, level: LEVEL / SUB, dir: 0 }, 0, 0);
  assert.deepEqual(unsupported([w, f, far], flatGround), [], 'chain: wall → floor → floor');
  assert.deepEqual(unsupported([f, far], flatGround), [0, 1], 'without the wall both floors fall');
});

// ------------------------------------------------------------ in the match

test('build mode places a wall in front, spends materials, blocks bullets and breaks', () => {
  const s = groundMatch(2);
  const [a, b] = s.players;
  a.mats = [25, 0, 0];
  const rev = s.buildRev;
  tickMatch(s, [press({ build: true, piece: 0, mat: 0, fire: true })]);
  assert.equal(s.builds.length, 1);
  const wall = s.builds[0];
  assert.deepEqual([wall.kind, wall.mat, wall.owner, wall.dir], [0, 0, a.id, 0]);
  assert.equal(a.mats[0], 25 - BUILD_COST);
  assert.ok(s.buildRev > rev);
  assert.equal(events(s, 'build').length, 1);
  assert.ok(a.building);
  // Holding fire keeps trying the same (now occupied) slot: no duplicate.
  run(s, 0.5, [press({ build: true, piece: 0, mat: 0, fire: true })]);
  assert.equal(s.builds.length, 1);
  // Not enough of the chosen material: nothing happens.
  tickMatch(s, [press({ build: true, piece: 1, mat: 2, fire: true })]);
  assert.equal(s.builds.length, 1);
  // Structures grow to full health.
  assert.ok(wall.hp < MATERIALS[0].hp);
  run(s, MATERIALS[0].time + 0.1, []);
  assert.equal(wall.hp, MATERIALS[0].hp);
  // A shooter behind the wall hits the wall, not its builder.
  put(s, b, a.x, a.z + 12, Math.PI);
  b.weapons[0] = { w: 'rifle', rarity: 0, mag: 30 }; b.current = 0;
  const aim = { yaw: Math.atan2(a.x - b.x, a.z - b.z), pitch: Math.atan2(a.y + 1.15 - (b.y + EYE), 12) };
  const seq = s.eventSeq;
  run(s, 0.4, [press({}), press({ ...aim, aim: true, fire: true })]);
  assert.ok(events(s, 'shot', seq).length >= 2);
  assert.equal(a.hp, 100, 'covered');
  assert.ok(wall.hp < MATERIALS[0].hp, 'the wall soaked the shots');
  run(s, 1.5, [press({}), press({ ...aim, aim: true, fire: true })]);
  assert.equal(s.builds.length, 0, 'the wall broke');
  assert.equal(events(s, 'break', seq).length, 1);
});

test('destroying the wall that holds a floor brings the floor down too', () => {
  const s = groundMatch(1);
  const a = s.players[0];
  a.mats = [0, 50, 0];
  tickMatch(s, [press({ build: true, piece: 0, mat: 1, fire: true })]);
  run(s, 0.2, [press({ build: true, piece: 1, mat: 1, pitch: 0.8 })]);
  tickMatch(s, [press({ build: true, piece: 1, mat: 1, fire: true, pitch: 0.8 })]);
  assert.deepEqual(s.builds.map(b => b.kind), [0, 1]);
  assert.ok(s.builds[1].aabb.maxY > a.y + 2.5, 'the floor is overhead');
  // Swing at the wall with the pickaxe until it breaks.
  const seq = s.eventSeq;
  run(s, 6, [press({ fire: true, slot: PICKAXE_KEY })]);
  assert.equal(s.builds.length, 0);
  assert.equal(events(s, 'break', seq).length, 2);
});

test('edit turns your own wall into a doorway you can walk through; nobody else can edit it', () => {
  const s = groundMatch(2);
  const [a, b] = s.players;
  a.mats = [30, 0, 0];
  tickMatch(s, [press({ build: true, fire: true })]);
  const wall = s.builds[0];
  const edge = tileX(wall.gz) + TILE;
  // Solid: walking forward stops at the wall.
  run(s, 1.5, [press({ mz: 1 })]);
  assert.ok(a.z < edge - 0.3, `blocked at ${a.z.toFixed(2)} (edge ${edge})`);
  // Someone else's edit does nothing.
  put(s, b, a.x + 0.8, a.z - 0.5);
  run(s, 0.1, [press({}), press({ build: true, edit: true })]);
  assert.equal(wall.variant, 0);
  run(s, 0.3, [press({ build: true })]);
  tickMatch(s, [press({ build: true, edit: true })]);
  assert.equal(wall.variant, 1, 'doorway');
  assert.equal(events(s, 'edit').length, 1);
  run(s, 1.5, [press({ mz: 1 })]);
  assert.ok(a.z > edge + 0.5, `walked through to ${a.z.toFixed(2)}`);
});

test('a survivor runs up a ramp they built', () => {
  const s = groundMatch(1);
  const a = s.players[0];
  a.mats = [0, 0, 20];
  const start = a.y;
  tickMatch(s, [press({ build: true, piece: 2, mat: 2, fire: true })]);
  assert.equal(s.builds.length, 1);
  const ramp = s.builds[0];
  assert.equal(ramp.dir, 0);
  let top = -Infinity;
  run(s, 1.6, st => { top = Math.max(top, st.players[0].y); return [press({ mz: 1 })]; });
  assert.ok(top > ramp.aabb.minY + LEVEL - 0.4, `reached ${top.toFixed(2)} (ramp top ${ramp.aabb.maxY})`);
  assert.ok(top > start + 2);
});

test('pickaxe: harvests wood from a trunk and brick from a rock, and hits survivors', () => {
  const s = groundMatch(2);
  const [a, b] = s.players;
  const w = worldFor(MESA);
  const tree = MESA.cylinders.find(c => Math.abs(c.x) < HALF - 30 && Math.abs(c.z) < HALF - 30 && Math.abs(terrainHeight(MESA, c.x, c.z - c.r - 0.9) - terrainHeight(MESA, c.x, c.z)) < 0.3)!;
  put(s, a, tree.x, tree.z - tree.r - 0.9);
  assert.equal(a.current, SLOTS, 'pickaxe in hand from the start');
  const seq = s.eventSeq;
  run(s, 1, [press({ fire: true })]);
  const swings = events(s, 'swing', seq).length;
  assert.ok(swings >= 2 && swings <= 3, `${swings} swings at ${PICKAXE.rate}/s`);
  assert.equal(a.mats[0], swings * PICKAXE.harvest);
  assert.deepEqual(events(s, 'harvest', seq).map(e => e.value), Array(swings).fill(0));
  const rock = MESA.boxes.findIndex((bx, i) => MESA.boxMats[i] === 1 && bx.maxY - terrainHeight(MESA, (bx.minX + bx.maxX) / 2, bx.minZ - 0.9) > 2.4 && Math.abs(terrainHeight(MESA, (bx.minX + bx.maxX) / 2, bx.minZ - 0.9) - terrainHeight(MESA, (bx.minX + bx.maxX) / 2, (bx.minZ + bx.maxZ) / 2)) < 0.4 && !MESA.blocks.includes(bx as never));
  assert.ok(rock >= 0);
  const r = MESA.boxes[rock];
  put(s, a, (r.minX + r.maxX) / 2, r.minZ - 0.9);
  assert.equal(raycast(w, a.x, a.y + EYE, a.z, 0, 0, 1, 3).index, rock);
  run(s, 1, [press({ fire: true })]);
  assert.ok(a.mats[1] > 0, 'brick');
  put(s, b, a.x, a.z - 1.6);
  b.shield = 0;
  run(s, 0.5, [press({ fire: true, yaw: Math.PI })]);
  assert.equal(b.hp, 100 - PICKAXE.damage);
});

test('consumables: hold fire to use, capped, interrupted by damage, stack on pickup', () => {
  const s = groundMatch(2);
  const [a, b] = s.players;
  a.weapons = [{ w: 'bandage', rarity: 0, mag: 13 }, null, null, null, null];
  a.current = 0; a.hp = 50;
  run(s, CONSUMABLES.bandage.time - 0.2, [press({ fire: true })]);
  assert.equal(a.hp, 50, 'not done yet');
  run(s, 0.3, [press({ fire: true })]);
  assert.equal(a.hp, 65);
  assert.equal(heldItem(a)?.mag, 12);
  assert.equal(events(s, 'use').length, 1);
  // Bandages stop at 75.
  run(s, CONSUMABLES.bandage.time * 2 + 0.2, [press({ fire: true })]);
  assert.equal(a.hp, 75);
  run(s, CONSUMABLES.bandage.time + 0.2, [press({ fire: true })]);
  assert.equal(a.hp, 75);
  assert.equal(a.useT, 0, 'nothing to heal');
  // Shield potions: a hit interrupts the drink.
  a.weapons[1] = { w: 'shieldpot', rarity: 2, mag: 1 };
  put(s, b, a.x, a.z + 6, Math.PI);
  b.weapons[0] = { w: 'pistol', rarity: 0, mag: 16 }; b.current = 0;
  run(s, 2, [press({ slot: 2, fire: true })]);
  assert.ok(a.useT > 1);
  tickMatch(s, [press({ slot: 2, fire: true }), press({ yaw: Math.atan2(a.x - b.x, a.z - b.z), pitch: Math.atan2(a.y + 1.15 - (b.y + EYE), 6), fire: true })]);
  assert.ok(a.hp < 75 && a.useT === 0, 'interrupted');
  run(s, CONSUMABLES.shieldpot.time + 0.1, [press({ slot: 2, fire: true })]);
  assert.equal(a.shield, 50);
  assert.equal(a.weapons[1], null, 'used up');
  // Pickup tops the bandage stack to 15, the rest takes a free slot.
  const had = a.weapons[0]!.mag;
  assert.equal(had, 11, 'two bandages used (50 → 65 → 75)');
  s.loot.push({ id: 7777, kind: 'item', weapon: 'bandage', rarity: 0, ammo: null, mat: null, amount: 5, x: a.x, y: a.y, z: a.z });
  tickMatch(s, []);
  assert.equal(a.weapons[0]!.mag, 15);
  assert.deepEqual(a.weapons[1], { w: 'bandage', rarity: 0, mag: 5 - (15 - had) });
  assert.equal(s.loot.length, 0);
});

test('water: survivors float and swim slowly, and cannot shoot or build while swimming', () => {
  const s = createMatch({ map: 'isle', difficulty: 'normal', seed: 5, seats: shooterSeats(humans(1), 0) });
  s.loot = [];
  const lake = mapFor('isle').pois.find(p => p.kind === 'lake')!;
  const a = s.players[0];
  put(s, a, lake.x, lake.z);
  a.weapons[0] = { w: 'rifle', rarity: 0, mag: 30 }; a.current = 0; a.mats = [50, 0, 0];
  tickMatch(s, []);
  const surface = waterLevel(mapFor('isle'), a.x, a.z) - SWIM_DEPTH;
  assert.ok(a.swimming);
  assert.ok(Math.abs(a.y - surface) < 1e-6);
  const x0 = a.x, z0 = a.z, seq = s.eventSeq;
  run(s, 1, [press({ mz: 1, sprint: true, fire: true })]);
  const moved = Math.hypot(a.x - x0, a.z - z0);
  assert.ok(moved > 1 && moved < 3.2, `swam ${moved.toFixed(2)} m`);
  assert.equal(events(s, 'shot', seq).length, 0);
  run(s, 0.2, [press({ build: true, fire: true })]);
  assert.equal(s.builds.length, 0);
  assert.equal(a.sprinting, false);
});

test('walk-in buildings: a survivor climbs the town hall stairs to the first floor', () => {
  const s = createMatch({ map: 'mesa', difficulty: 'normal', seed: 5, seats: shooterSeats(humans(1), 0) });
  s.loot = [];
  const a = s.players[0];
  const town = MESA.pois.find(p => p.kind === 'town')!;
  // Town hall: 14 x 11 centred 4 m north of the square; the first flight hugs the west wall, climbing south to north.
  const x = town.x - 7 + 0.3 + 0.65, z = town.z - 4 - 5.5 + 0.3 + 0.5;
  put(s, a, x, z);
  const base = a.y;
  run(s, 2.5, [press({ mz: 1 })]);
  assert.ok(Math.abs(a.y - (base + 3.6)) < 0.05, `upstairs at ${a.y.toFixed(2)} (ground floor ${base.toFixed(2)})`);
  assert.ok(a.grounded);
  // Indoor loot sits on real floors.
  const w = worldFor(MESA);
  for (const spot of MESA.lootSpots.filter(l => l.y !== undefined)) assert.ok(Math.abs(groundAt(w, spot.x, spot.z, spot.y! + 0.1) - spot.y!) < 0.05, `loot floor at ${spot.x.toFixed(1)},${spot.z.toFixed(1)}`);
});

test('Build type: every placed piece carries its own colliders', () => {
  const s = groundMatch(1);
  s.players[0].mats = [100, 0, 0];
  for (const piece of [0, 1, 2]) { tickMatch(s, [press({ build: true, piece, fire: true })]); run(s, 0.2, [press({ build: true, piece })]); }
  assert.equal(s.builds.length, 3);
  for (const b of s.builds as Build[]) assert.deepEqual({ boxes: b.boxes, aabb: b.aabb }, buildGeometry(b));
  assert.equal(WEAPONS.rifle.mag, 30);
});

// ------------------------------------------------------------ bots

test('bots with no materials walk to a tree and harvest it', () => {
  const s = createMatch({ map: 'mesa', difficulty: 'normal', seed: 11, seats: shooterSeats([], 2) });
  s.loot = []; s.chests = s.chests.map(() => true);
  const [a, b] = s.players;
  put(s, a, SPOT.x, SPOT.z);
  put(s, b, SPOT.x > 0 ? SPOT.x - 150 : SPOT.x + 150, SPOT.z);
  // Calm storm centred on the bot so it has time to spare.
  Object.assign(s.storm, { stage: 0, cx: a.x, cz: a.z, r: 200, fromX: a.x, fromZ: a.z, fromR: 200, toX: a.x, toZ: a.z, toR: 200, timer: 9999, shrinking: false, damage: 1 });
  run(s, 25, []);
  assert.ok(a.mats[0] > 0, `harvested ${a.mats[0]} wood`);
  assert.ok(events(s, 'harvest').some(e => e.player === a.id));
});

test('a hard bot that gets shot walls up toward the shooter', () => {
  let built = 0;
  for (const seed of [1, 2, 3]) {
    const s = createMatch({ map: 'mesa', difficulty: 'hard', seed, seats: shooterSeats(humans(1), 1) });
    s.loot = [];
    const [me, bot] = s.players;
    put(s, me, SPOT.x, SPOT.z);
    put(s, bot, SPOT.x, SPOT.z + 22, Math.PI);
    bot.weapons[0] = { w: 'rifle', rarity: 0, mag: 30 }; bot.current = 0; bot.ammo.medium = 90; bot.mats = [60, 0, 0]; bot.shield = 100;
    me.weapons[0] = { w: 'smg', rarity: 4, mag: 30 }; me.current = 0; me.ammo.light = 200; me.hp = 1000; me.shield = 1000;
    run(s, 4, st => {
      const p = st.players[0], q = st.players[1];
      const d = Math.hypot(q.x - p.x, q.z - p.z);
      return [press({ yaw: Math.atan2(q.x - p.x, q.z - p.z), pitch: Math.atan2(q.y + 1.15 - (p.y + EYE), d), fire: true, aim: true })];
    });
    built += events(s, 'build').filter(e => e.player === bot.id).length;
  }
  assert.ok(built > 0, 'the bot built cover');
});

test('coverFrom picks a spot behind a tall prop, on the far side from the threat', () => {
  const s = groundMatch(1);
  const a = s.players[0];
  a.x = 14; a.z = 0; a.y = 0;
  const tall = { minX: 9, minY: -1, minZ: -1, maxX: 11, maxY: 3, maxZ: 1 }, short = { ...tall, maxY: 0.8 };
  const world = (boxes: typeof tall[]) => ({ ...MESA, boxes }) as ShooterMap;
  const spot = coverFrom(world([tall]), a, { x: 40, z: 0 })!;
  assert.ok(spot, 'found cover');
  assert.ok(Math.abs(spot[0] - (10 - Math.SQRT2 - 1)) < 1e-9 && Math.abs(spot[1]) < 1e-9, `spot ${spot}`);
  // Threat from the other side: the spot flips.
  assert.ok(coverFrom(world([tall]), a, { x: -40, z: 0 })![0] > 11);
  assert.equal(coverFrom(world([short]), a, { x: 40, z: 0 }), null, 'too low to hide behind');
  assert.equal(coverFrom(world([{ ...tall, minX: 39, maxX: 41 }]), a, { x: 80, z: 0 }), null, 'too far away');
});

test('loot the survivor cannot carry does not churn the loot revision (no needless loot snapshots)', () => {
  const s = groundMatch(1);
  const a = s.players[0];
  a.weapons = [{ w: 'bandage', rarity: 0, mag: 15 }, { w: 'pistol', rarity: 0, mag: 16 }, { w: 'smg', rarity: 0, mag: 30 }, { w: 'rifle', rarity: 0, mag: 30 }, { w: 'medkit', rarity: 1, mag: 3 }];
  a.mats = [500, 0, 0]; a.ammo.light = 240;
  s.loot.push(
    { id: 1, kind: 'item', weapon: 'bandage', rarity: 0, ammo: null, mat: null, amount: 5, x: a.x, y: a.y, z: a.z },
    { id: 2, kind: 'mats', weapon: null, rarity: 0, ammo: null, mat: 0, amount: 30, x: a.x, y: a.y, z: a.z },
    { id: 3, kind: 'ammo', weapon: null, rarity: 0, ammo: 'light', mat: null, amount: 30, x: a.x, y: a.y, z: a.z },
  );
  const rev = s.lootRev, seq = s.eventSeq;
  run(s, 1, []);
  assert.equal(s.lootRev, rev);
  assert.equal(s.loot.length, 3);
  assert.equal(events(s, 'pickup', seq).length, 0);
});

test('once the last survivor is crowned, the storm cannot take the win away', () => {
  const s = groundMatch(2);
  const [a, b] = s.players;
  Object.assign(s.storm, { stage: 6, cx: a.x + 200, cz: a.z, r: 1, fromX: a.x + 200, fromZ: a.z, fromR: 1, toX: a.x + 200, toZ: a.z, toR: 0, timer: 9999, shrinking: false, damage: 14 });
  a.hp = 20; b.hp = 1;
  run(s, 1.05, []);
  assert.equal(b.mode, 'dead');
  assert.equal(s.winner, a.id);
  const hp = a.hp;
  run(s, 3.1, []);
  assert.equal(s.phase, 'done');
  assert.equal(a.hp, hp);
  assert.notEqual(a.mode, 'dead');
});
