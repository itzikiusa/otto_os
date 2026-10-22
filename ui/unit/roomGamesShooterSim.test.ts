import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mapFor, terrainHeight, waterLevel, type ShooterMap } from '../src/modules/rooms/games/shooter/map.ts';
import { groundAt, worldFor } from '../src/modules/rooms/games/shooter/collide.ts';
import { AMMO_MAX, damageAt, WEAPONS, type WeaponId } from '../src/modules/rooms/games/shooter/weapons.ts';
import {
  aliveCount, BUS_HEIGHT, createMatch, EYE, idleShooterInput, MAX_FIGHTERS, PICKAXE_KEY, sanitizeInput, SWIM_DEPTH, shooterResults, SLOTS, STEP, stepMatch, STORM, tickMatch,
  type Difficulty, type Fighter, type Loot, type ShooterEventType, type ShooterInput, type ShooterState,
} from '../src/modules/rooms/games/shooter/sim.ts';
import { weaponFor } from '../src/modules/rooms/games/shooter/bots.ts';
import type { ShooterMapId } from '../src/modules/rooms/games/shooter/catalog.ts';
import { shooterSeats, type HumanSeat } from '../src/modules/rooms/games/setup.ts';

type Inputs = (ShooterInput | undefined)[];
const humans = (n: number): HumanSeat[] => Array.from({ length: n }, (_, i) => ({ slot: i, name: `P${i}`, pick: 'male-b' }));
const run = (s: ShooterState, seconds: number, inputs: Inputs | ((s: ShooterState) => Inputs) = []): void => {
  for (let i = 0, n = Math.round(seconds / STEP); i < n; i++) tickMatch(s, typeof inputs === 'function' ? inputs(s) : inputs);
};
const events = (s: ShooterState, type: ShooterEventType, from = 0) => s.events.filter(e => e.id > from && e.type === type);
const press = (k: Partial<ShooterInput>): ShooterInput => ({ ...idleShooterInput(), ...k });

/** First flat spot on a map with no colliders within `clear` metres (deterministic scan). */
function clearSpot(m: ShooterMap, clear = 16): { x: number; z: number } {
  for (let x = -120; x <= 120; x += 4) for (let z = -120; z <= 120; z += 4) {
    const h0 = terrainHeight(m, x, z);
    if (m.theme === 'isle' && h0 < 1.5) continue;
    if (m.boxes.some(b => x > b.minX - clear && x < b.maxX + clear && z > b.minZ - clear && z < b.maxZ + clear)) continue;
    if (m.cylinders.some(c => Math.hypot(c.x - x, c.z - z) < c.r + clear)) continue;
    let flat = 0;
    for (let dx = -12; dx <= 12; dx += 2) for (let dz = -12; dz <= 12; dz += 2) flat = Math.max(flat, Math.abs(terrainHeight(m, x + dx, z + dz) - h0));
    if (flat < 0.6) return { x, z };
  }
  throw new Error(`no clear spot on ${m.id}`);
}
const MESA = mapFor('mesa');
const SPOT = clearSpot(MESA);

/** Puts a fighter on the ground at (x, z), standing still. */
function put(s: ShooterState, p: Fighter, x: number, z: number): void {
  const m = mapFor(s.map);
  p.x = x; p.z = z; p.y = groundAt(worldFor(m), x, z, terrainHeight(m, x, z));
  p.vx = p.vy = p.vz = 0; p.mode = 'ground'; p.grounded = true;
}
/** A match on Red Mesa with `n` humans already on the ground at SPOT, 30 m apart, no loot. */
function groundMatch(n: number, bots = 0): ShooterState {
  const s = createMatch({ map: 'mesa', difficulty: 'normal', seed: 5, seats: shooterSeats(humans(n), bots) });
  s.loot = [];
  s.players.forEach((p, i) => put(s, p, SPOT.x + (i === 0 ? 0 : 30 + i * 4), SPOT.z - (i === 0 ? 0 : 30)));
  tickMatch(s, []);
  assert.equal(s.phase, 'play');
  return s;
}
const loot = (s: ShooterState, l: Partial<Loot> & Pick<Loot, 'kind'>): Loot => {
  const item: Loot = { id: s.nextId++, weapon: null, rarity: 0, ammo: null, mat: null, amount: 0, x: 0, y: 0, z: 0, ...l };
  s.loot.push(item);
  return item;
};
/** Yaw/pitch from a fighter's eye to a point `h` metres above another fighter's feet. */
const aimAt = (from: Fighter, to: Fighter, h = 1.15): { yaw: number; pitch: number } => {
  const dx = to.x - from.x, dz = to.z - from.z, d = Math.hypot(dx, dz);
  return { yaw: Math.atan2(dx, dz), pitch: Math.atan2(to.y + h - (from.y + EYE), d) };
};

test('createMatch: everyone starts on the bus with loot on the ground', () => {
  const s = createMatch({ map: 'isle', difficulty: 'hard', seed: 9, seats: shooterSeats(humans(2), 14) });
  assert.equal(s.players.length, 16);
  assert.equal(s.phase, 'bus');
  assert.ok(s.players.every(p => p.mode === 'bus' && p.y === BUS_HEIGHT && p.hp === 100 && p.shield === 0 && p.weapons.every(w => w === null)));
  assert.deepEqual(s.players.map(p => p.id), [...Array(16).keys()]);
  assert.ok(s.loot.length > 20);
  assert.ok(s.loot.some(l => l.kind === 'weapon') && s.loot.some(l => l.kind === 'ammo'));
  assert.equal(s.chests.length, mapFor('isle').chests.length);
  assert.ok(s.chests.every(c => !c));
  assert.equal(s.storm.stage, -1);
  assert.equal(s.winner, -1);
  for (const p of s.players.filter(p => !p.human)) assert.ok(p.bot.dropAt > -2 && p.bot.dropAt < s.bus.duration + 2);
  // Seats beyond MAX_FIGHTERS are cut; the setup is deterministic per seed.
  assert.equal(createMatch({ map: 'isle', difficulty: 'easy', seed: 1, seats: shooterSeats([], 20) }).players.length, MAX_FIGHTERS);
  const again = createMatch({ map: 'isle', difficulty: 'hard', seed: 9, seats: shooterSeats(humans(2), 14) });
  assert.deepEqual(again.loot, s.loot);
  assert.deepEqual(again.bus, s.bus);
});

test('humans jump on input; sky -> glide -> ground; the bus phase ends once everyone has left', () => {
  const s = createMatch({ map: 'isle', difficulty: 'normal', seed: 3, seats: shooterSeats(humans(2), 0) });
  tickMatch(s, [press({ jump: true }), press({ mx: 1, mz: 1 })]);
  const [a, b] = s.players;
  assert.equal(a.mode, 'sky');
  assert.equal(b.mode, 'bus');
  assert.equal(events(s, 'jump').length, 1);
  const mode = (): string => a.mode; // read through a function: assert.equal narrowed a.mode to 'sky'
  const modes: string[] = [mode()];
  let prevY = a.y;
  for (let i = 0; i < 60 * 30 && mode() !== 'ground'; i++) {
    tickMatch(s, [press({ mz: 1, yaw: 0.5 })]);
    if (modes.at(-1) !== mode()) modes.push(mode());
    assert.ok(a.y <= prevY + 1e-9, 'always descending');
    prevY = a.y;
  }
  assert.deepEqual(modes, ['sky', 'glide', 'ground']);
  assert.ok(a.grounded);
  // Lands on the ground, or floats at the surface when the drop ends in water.
  const floor = groundAt(worldFor(mapFor('isle')), a.x, a.z, a.y), surface = waterLevel(mapFor('isle'), a.x, a.z) - SWIM_DEPTH;
  assert.ok(Math.abs(a.y - Math.max(floor, surface)) < 1e-6);
  assert.equal(a.swimming, surface > floor);
  assert.ok(events(s, 'glide').length >= 1 && events(s, 'land').length >= 1);
  // The idle human rides the bus along its line until the auto-jump near the end.
  assert.equal(b.mode, 'bus');
  const t = s.bus.t / s.bus.duration;
  assert.ok(Math.abs(b.x - (s.bus.ax + (s.bus.bx - s.bus.ax) * t)) < 1e-6);
  assert.equal(s.phase, 'bus');
  for (let i = 0; i < 60 * 20 && b.mode === 'bus'; i++) tickMatch(s, []);
  assert.equal(b.mode, 'sky');
  assert.ok(s.bus.t > s.bus.duration - 0.7);
  tickMatch(s, []);
  assert.equal(s.phase, 'play');
});

test('loot pickup: ammo (capped), heal, shield', () => {
  const s = groundMatch(2);
  const p = s.players[0];
  const rev = s.lootRev;
  loot(s, { kind: 'ammo', ammo: 'light', amount: 36, x: p.x + 0.5, y: p.y, z: p.z });
  tickMatch(s, []);
  assert.equal(p.ammo.light, 36);
  assert.equal(s.loot.length, 0);
  assert.ok(s.lootRev > rev);
  assert.deepEqual(events(s, 'pickup').map(e => e.value), [1]);
  // Near the cap: takes what fits and leaves the rest.
  p.ammo.shells = AMMO_MAX.shells - 3;
  const rest = loot(s, { kind: 'ammo', ammo: 'shells', amount: 8, x: p.x, y: p.y, z: p.z });
  tickMatch(s, []);
  assert.equal(p.ammo.shells, AMMO_MAX.shells);
  assert.equal(rest.amount, 5);
  assert.equal(s.loot.length, 1);
  s.loot = [];
  // Heal only when hurt; shield up to 100.
  const heal = loot(s, { kind: 'heal', amount: 50, x: p.x, y: p.y, z: p.z });
  tickMatch(s, []);
  assert.deepEqual(s.loot, [heal], 'full hp: heal stays');
  p.hp = 40;
  tickMatch(s, []);
  assert.equal(p.hp, 90);
  assert.equal(s.loot.length, 0);
  p.shield = 75;
  loot(s, { kind: 'shield', amount: 50, x: p.x, y: p.y, z: p.z });
  tickMatch(s, []);
  assert.equal(p.shield, 100);
  // Out of reach: nothing happens.
  loot(s, { kind: 'ammo', ammo: 'heavy', amount: 6, x: p.x + 3, y: p.y, z: p.z });
  tickMatch(s, []);
  assert.equal(p.ammo.heavy, 0);
});

test('weapon pickup fills a free slot, never a full inventory or a worse duplicate', () => {
  const s = groundMatch(2);
  const p = s.players[0];
  loot(s, { kind: 'weapon', weapon: 'smg', rarity: 1, amount: 30, x: p.x, y: p.y, z: p.z });
  tickMatch(s, []);
  assert.deepEqual(p.weapons, [{ w: 'smg', rarity: 1, mag: 30, att: 0 }, null, null, null, null]);
  assert.equal(p.current, 0, 'drawn from the pickaxe');
  // Same weapon, same or lower rarity: skipped.
  const dup = loot(s, { kind: 'weapon', weapon: 'smg', rarity: 0, amount: 30, x: p.x, y: p.y, z: p.z });
  tickMatch(s, []);
  assert.deepEqual(s.loot, [dup]);
  s.loot = [];
  loot(s, { kind: 'weapon', weapon: 'rifle', rarity: 2, amount: 30, x: p.x, y: p.y, z: p.z });
  loot(s, { kind: 'weapon', weapon: 'sniper', rarity: 3, amount: 4, x: p.x, y: p.y, z: p.z });
  tickMatch(s, []);
  assert.deepEqual(p.weapons.map(w => w?.w), ['smg', 'sniper', 'rifle', undefined, undefined]);
  assert.equal(p.current, 0, 'already holding a gun: no auto-switch');
  loot(s, { kind: 'weapon', weapon: 'pistol', rarity: 0, amount: 16, x: p.x, y: p.y, z: p.z });
  loot(s, { kind: 'weapon', weapon: 'rocket', rarity: 0, amount: 1, x: p.x, y: p.y, z: p.z });
  tickMatch(s, []);
  const extra = loot(s, { kind: 'weapon', weapon: 'shotgun', rarity: 4, amount: 5, x: p.x, y: p.y, z: p.z });
  tickMatch(s, []);
  assert.deepEqual(s.loot, [extra], 'full inventory: auto-pickup skips');
  // Interact swaps the current weapon for the floor one and drops the old one.
  tickMatch(s, [press({ interact: true })]);
  assert.equal(p.weapons[p.current]?.w, 'shotgun');
  assert.equal(s.loot.length, 1);
  assert.equal(s.loot[0].weapon, 'smg');
  // Slot selection (1-based) switches weapons; the key after the hotbar draws the pickaxe.
  tickMatch(s, [press({ slot: 3 })]);
  assert.equal(p.current, 2);
  tickMatch(s, [press({ slot: PICKAXE_KEY })]);
  assert.equal(p.current, SLOTS);
});

test('firing consumes the magazine; semi-auto needs a fresh press; reload refills from reserve', () => {
  const s = groundMatch(2);
  const p = s.players[0];
  p.yaw = 0;
  p.weapons[0] = { w: 'rifle', rarity: 0, mag: 30, att: 0 };
  p.current = 0;
  p.ammo.medium = 60;
  // Fire straight up (nothing to hit) for one second.
  const seq = s.eventSeq;
  run(s, 1, [press({ fire: true, pitch: 1.4 })]);
  const shots = events(s, 'shot', seq).length;
  assert.equal(p.weapons[0]!.mag, 30 - shots);
  assert.ok(shots >= 6 && shots <= 8, `${shots} rifle shots in 1 s at ${WEAPONS.rifle.rate}/s`);
  // Reload: takes `reload` seconds and moves rounds from the reserve.
  run(s, 0.2, [press({ pitch: 1.4 })]);
  tickMatch(s, [press({ reload: true, fire: true, pitch: 1.4 })]);
  assert.ok(p.reload > 0);
  assert.equal(events(s, 'reload').length, 1);
  // Holding the trigger does not cancel it (only a fresh press with rounds left does); a tactical reload is quicker.
  run(s, WEAPONS.rifle.reload * 0.85 - 0.1, [press({ fire: true, pitch: 1.4 })]);
  assert.equal(p.weapons[0]!.mag, 30 - shots, 'no firing while reloading');
  run(s, 0.2, []);
  assert.equal(p.weapons[0]!.mag, 30);
  assert.equal(p.ammo.medium, 60 - shots);
  // Semi-auto pistol: holding fires once.
  p.weapons[1] = { w: 'pistol', rarity: 0, mag: 16, att: 0 };
  run(s, 0.5, [press({ slot: 2 })]);
  run(s, 1, [press({ slot: 2, fire: true, pitch: 1.4 })]);
  assert.equal(p.weapons[1]!.mag, 15);
  run(s, 0.2, [press({ slot: 2, pitch: 1.4 })]);
  run(s, 0.1, [press({ slot: 2, fire: true, pitch: 1.4 })]);
  assert.equal(p.weapons[1]!.mag, 14);
  // Sprinting needs forward input; pulling the trigger cancels the sprint and fires.
  run(s, 0.5, [press({ slot: 2, pitch: 1.4 })]);
  tickMatch(s, [press({ slot: 2, pitch: 1.4, sprint: true, mz: 1 })]);
  assert.equal(p.sprinting, true);
  tickMatch(s, [press({ slot: 2, pitch: 1.4, sprint: true, mz: 1, fire: true })]);
  assert.equal(p.sprinting, false);
  // The weapon comes up out of the sprint first (a moment), and the click is kept until then.
  assert.equal(p.weapons[1]!.mag, 14);
  run(s, 0.15, [press({ slot: 2, pitch: 1.4, fire: true })]);
  assert.equal(p.weapons[1]!.mag, 13);
});

test('empty magazine: fire starts a reload from reserve; with no reserve nothing happens', () => {
  const s = groundMatch(2);
  const p = s.players[0];
  p.weapons[0] = { w: 'shotgun', rarity: 0, mag: 0, att: 0 };
  p.current = 0;
  p.ammo.shells = 3;
  tickMatch(s, [press({ fire: true, pitch: 1.4 })]);
  assert.ok(p.reload > 0);
  run(s, WEAPONS.shotgun.reload + 0.1, []);
  assert.equal(p.weapons[0]!.mag, 3);
  assert.equal(p.ammo.shells, 0);
  p.weapons[0]!.mag = 0;
  const seq = s.eventSeq;
  run(s, 0.5, [press({ fire: true, pitch: 1.4 })]);
  assert.equal(p.reload, 0);
  assert.equal(events(s, 'shot', seq).length, 0);
});

test('hitscan damage goes to the shield first', () => {
  const s = groundMatch(3);
  const [a, b] = s.players;
  put(s, b, a.x, a.z + 10);
  b.shield = 50;
  a.weapons[0] = { w: 'rifle', rarity: 0, mag: 30, att: 0 };
  a.current = 0;
  const aim = aimAt(a, b);
  a.ads = 1; // sights already up (they take the rifle's ADS time to raise)
  tickMatch(s, [press({ ...aim, aim: true, fire: true })]);
  const expected = damageAt(WEAPONS.rifle, 0, 10, false);
  assert.equal(b.shield, 50 - expected);
  assert.equal(b.hp, 100);
  assert.equal(a.damage, expected);
  const hit = events(s, 'hit').at(-1)!;
  assert.equal(hit.target, b.id);
  assert.equal(hit.player, a.id);
  assert.equal(hit.flag & 1, 1, 'shield hit flag');
  // Keep firing: shield breaks, then hp drops.
  run(s, 0.5, [press({ ...aim, aim: true, fire: true })]);
  assert.equal(b.shield, 0);
  assert.ok(b.hp < 100);
});

test('rocket splash: direct hit hurts most, falls off with distance, shields absorb', () => {
  const s = groundMatch(4);
  const [a, b, c, d] = s.players;
  put(s, b, a.x, a.z + 15);
  put(s, c, b.x + 3, b.z);
  put(s, d, b.x + 10, b.z);
  c.shield = 100;
  a.weapons[0] = { w: 'rocket', rarity: 0, mag: 1, att: 0 };
  a.current = 0;
  tickMatch(s, [press({ ...aimAt(a, b, 1), aim: true, fire: true })]);
  assert.equal(s.rockets.length, 1);
  assert.equal(a.weapons[0]!.mag, 0);
  run(s, 1, [press({ ...aimAt(a, b, 1) })]);
  assert.equal(s.rockets.length, 0);
  assert.equal(events(s, 'explode').length, 1);
  assert.ok(b.hp < 30, `direct hit left ${b.hp} hp`);
  assert.equal(c.hp, 100, 'shield took the splash');
  assert.ok(c.shield < 100 && 100 - c.shield < 100 - b.hp, `splash ${100 - c.shield} < direct ${100 - b.hp}`);
  assert.equal(d.hp, 100);
  assert.equal(a.hp, 100);
});

test('elimination drops the inventory, credits the killer, and the last fighter wins', () => {
  const s = groundMatch(3);
  const [a, b, c] = s.players;
  put(s, b, a.x, a.z + 10);
  b.hp = 10;
  b.weapons = [{ w: 'smg', rarity: 1, mag: 20, att: 0 }, { w: 'medkit', rarity: 1, mag: 2, att: 0 }, null, null, null];
  b.ammo.light = 50;
  b.mats = [40, 0, 12];
  a.weapons[0] = { w: 'rifle', rarity: 0, mag: 30, att: 0 };
  a.current = 0; a.ads = 1;
  tickMatch(s, [press({ ...aimAt(a, b), aim: true, fire: true })]);
  assert.equal(b.mode, 'dead');
  assert.equal(b.place, 3);
  assert.equal(b.killedBy, a.id);
  assert.equal(a.kills, 1);
  assert.equal(aliveCount(s), 2);
  assert.deepEqual(b.weapons, [null, null, null, null, null]);
  assert.equal(b.ammo.light, 0);
  assert.deepEqual(b.mats, [0, 0, 0]);
  const kill = events(s, 'kill').at(-1)!;
  assert.equal(kill.target, b.id);
  const dropped = s.loot.filter(l => Math.hypot(l.x - b.x, l.z - b.z) < 2);
  assert.ok(dropped.some(l => l.kind === 'weapon' && l.weapon === 'smg' && l.rarity === 1 && l.amount === 20));
  assert.ok(dropped.some(l => l.kind === 'ammo' && l.ammo === 'light' && l.amount === 50));
  assert.ok(dropped.some(l => l.kind === 'item' && l.weapon === 'medkit' && l.amount === 2));
  assert.ok(dropped.some(l => l.kind === 'mats' && l.mat === 0 && l.amount === 40) && dropped.some(l => l.kind === 'mats' && l.mat === 2 && l.amount === 12));
  // The dead don't take part any more.
  const frozen = [b.x, b.z, b.aliveTime];
  run(s, 0.5, []);
  assert.deepEqual([b.x, b.z, b.aliveTime], frozen);
  // Last opponent goes down: single winner, places, then done after 3 s.
  s.loot = [];
  put(s, c, b.x, b.z);
  c.hp = 5;
  a.ads = 1; a.kick = 0; a.kickYaw = 0;
  tickMatch(s, [press({ ...aimAt(a, c), aim: true, fire: true })]);
  assert.equal(c.mode, 'dead');
  assert.equal(c.place, 2);
  assert.equal(s.winner, a.id);
  assert.equal(a.place, 1);
  assert.equal(events(s, 'win').length, 1);
  assert.equal(s.phase, 'play');
  run(s, 3.1, []);
  assert.equal(s.phase, 'done');
  assert.deepEqual(shooterResults(s).map(r => [r.id, r.place, r.kills]), [[a.id, 1, 2], [c.id, 2, 0], [b.id, 3, 0]]);
  // Done matches don't advance.
  const clock = s.clock;
  tickMatch(s, []);
  assert.equal(s.clock, clock);
});

test('elimination of a shielded victim also drops a heal', () => {
  const s = groundMatch(3);
  const [a, b] = s.players;
  put(s, b, a.x, a.z + 10);
  b.hp = 5; b.shield = 1;
  b.weapons = [{ w: 'smg', rarity: 1, mag: 20, att: 0 }, null, null, null, null];
  b.ammo.light = 50;
  a.weapons[0] = { w: 'sniper', rarity: 4, mag: 4, att: 0 };
  a.current = 0; a.ads = 1;
  tickMatch(s, [press({ ...aimAt(a, b), aim: true, fire: true })]);
  assert.equal(b.mode, 'dead');
  assert.ok(s.loot.some(l => l.kind === 'heal' && Math.hypot(l.x - b.x, l.z - b.z) < 2));
});

test('storm: waits, shrinks toward the target circle and hurts only players outside (ignoring shields)', () => {
  const s = createMatch({ map: 'isle', difficulty: 'normal', seed: 17, seats: shooterSeats(humans(2), 0) });
  s.loot = [];
  const [inside, outside] = s.players;
  const st = s.storm;
  put(s, inside, st.toX, st.toZ);
  put(s, outside, st.toX > 0 ? -150 : 150, st.toZ > 0 ? -150 : 150);
  outside.shield = 50;
  const startR = st.r;
  run(s, STORM[0].wait - 1, []);
  assert.equal(st.stage, -1);
  assert.equal(st.r, startR);
  assert.equal(outside.hp, 100);
  run(s, 2, []);
  assert.equal(st.stage, 0);
  assert.ok(st.shrinking);
  assert.equal(st.damage, STORM[0].damage);
  assert.ok(st.r < startR);
  run(s, STORM[0].shrink, []);
  assert.equal(st.shrinking, false);
  assert.equal(st.r, STORM[0].radius);
  assert.equal(st.toR, STORM[1].radius);
  assert.ok(Math.abs(st.timer - STORM[1].wait) < 1);
  assert.ok(events(s, 'storm').length >= 2);
  assert.equal(inside.hp, 100);
  assert.ok(outside.hp < 95, `outside hp ${outside.hp}`);
  assert.equal(outside.shield, 50, 'storm ignores shields');
  // The final circle closes to zero.
  assert.equal(STORM.at(-1)!.radius, 0);
});

test('when every human is out, standing bots share the remaining places', () => {
  const s = createMatch({ map: 'mesa', difficulty: 'easy', seed: 23, seats: shooterSeats(humans(1), 3) });
  s.loot = [];
  const [me, ...bots] = s.players;
  bots.forEach((b, i) => put(s, b, SPOT.x + i * 2, SPOT.z + 2));
  put(s, me, SPOT.x + (SPOT.x < 0 ? 90 : -90), SPOT.z);
  me.hp = 3;
  Object.assign(s.storm, { stage: 0, cx: SPOT.x, cz: SPOT.z, r: 60, fromX: SPOT.x, fromZ: SPOT.z, fromR: 60, toX: SPOT.x, toZ: SPOT.z, toR: 60, timer: 9999, shrinking: false, damage: 5 });
  bots[1].shield = 40;
  let n = 0;
  while (s.phase !== 'done' && n++ < 60 * 10) tickMatch(s, []);
  assert.equal(s.phase, 'done');
  assert.equal(me.mode, 'dead');
  assert.equal(me.place, 4);
  assert.equal(me.killedBy, -2, 'storm kill');
  assert.ok(bots.every(b => b.mode !== 'dead'));
  assert.deepEqual(s.players.map(p => p.place).sort(), [1, 2, 3, 4]);
  assert.equal(bots[1].place, 1, 'healthiest bot ranks first');
  assert.equal(s.winner, bots[1].id);
});

test('shooterResults orders by place, then kills; unplaced fighters last', () => {
  const s = createMatch({ map: 'isle', difficulty: 'easy', seed: 2, seats: shooterSeats(humans(1), 4) });
  const places = [3, 1, 0, 2, 0], kills = [0, 4, 1, 2, 5];
  s.players.forEach((p, i) => { p.place = places[i]; p.kills = kills[i]; p.damage = 12.6; });
  const r = shooterResults(s);
  assert.deepEqual(r.map(x => x.id), [1, 3, 0, 4, 2]);
  assert.equal(r[0].damage, 13);
  assert.equal(r[0].human, false);
  assert.equal(r.find(x => x.id === 0)!.human, true);
});

test('sanitizeInput clamps and coerces bad input', () => {
  assert.deepEqual(sanitizeInput(undefined), idleShooterInput());
  const ok = press({ mx: 0.5, mz: -0.5, yaw: 1, pitch: -0.3, fire: true, aim: true, jump: true, sprint: true, reload: true, interact: true, slot: 3 });
  assert.deepEqual(sanitizeInput(ok), ok);
  const c = sanitizeInput({ mx: 5, mz: -9, yaw: 4, pitch: 3, fire: 1, aim: 'y', jump: null, sprint: {}, reload: 'true', interact: [], slot: 2.5 } as unknown as ShooterInput);
  assert.equal(c.mx, 1);
  assert.equal(c.mz, -1);
  assert.ok(Math.abs(c.yaw - (4 - Math.PI * 2)) < 1e-9, 'yaw wrapped');
  assert.equal(c.pitch, 1.45);
  assert.deepEqual([c.fire, c.aim, c.jump, c.sprint, c.reload, c.interact], [false, false, false, false, false, false]);
  assert.equal(c.slot, 0);
  const n = sanitizeInput({ mx: NaN, mz: Infinity, yaw: 1e12, pitch: -Infinity, slot: PICKAXE_KEY + 1, piece: 3, mat: -1, build: 'yes', edit: 1 } as unknown as ShooterInput);
  assert.deepEqual([n.mx, n.mz, n.pitch, n.slot, n.piece, n.mat, n.build, n.edit], [0, 0, 0, 0, 0, 0, false, false]);
  assert.equal(sanitizeInput(press({ slot: PICKAXE_KEY, piece: 2, mat: 2, build: true })).slot, PICKAXE_KEY);
  assert.ok(n.yaw > -Math.PI && n.yaw <= Math.PI);
  assert.equal(sanitizeInput(press({ slot: -1 })).slot, 0);
});

test('stepMatch uses fixed steps and ignores bad dt', () => {
  const s = createMatch({ map: 'isle', difficulty: 'easy', seed: 2, seats: shooterSeats([], 4) });
  stepMatch(s, [], NaN); stepMatch(s, [], -1); stepMatch(s, [], 0);
  assert.equal(s.clock, 0);
  stepMatch(s, [], 0.05);
  assert.ok(Math.abs(s.clock - 3 * STEP) < 1e-9);
  stepMatch(s, [], 30);
  assert.ok(s.clock <= 9 * STEP + 1e-9);
});

test('weaponFor picks shotgun close, rifle mid, sniper far; skips empty weapons', () => {
  const p = createMatch({ map: 'isle', difficulty: 'easy', seed: 2, seats: shooterSeats([], 1) }).players[0];
  p.weapons = [{ w: 'shotgun', rarity: 0, mag: 5, att: 0 }, { w: 'sniper', rarity: 0, mag: 4, att: 0 }, { w: 'rifle', rarity: 0, mag: 30, att: 0 }];
  assert.equal(weaponFor(p, 5), 0);
  assert.equal(weaponFor(p, 40), 2);
  assert.equal(weaponFor(p, 120), 1);
  // An empty sniper with no reserve is ignored.
  p.weapons[1]!.mag = 0;
  assert.equal(weaponFor(p, 120), 2);
  p.ammo.heavy = 4;
  assert.equal(weaponFor(p, 120), 1);
  p.weapons = [null, null, null];
  assert.equal(weaponFor(p, 10), -1);
  const pick = (ws: WeaponId[], d: number) => { p.weapons = ws.map(w => ({ w, rarity: 0 as const, mag: WEAPONS[w].mag, att: 0 })); return ws[weaponFor(p, d)]; };
  assert.equal(pick(['pistol', 'smg'], 10), 'smg');
  assert.equal(pick(['pistol', 'rocket'], 30), 'rocket');
  assert.equal(pick(['pistol', 'rocket'], 5), 'pistol');
});

for (const map of ['isle', 'mesa'] as ShooterMapId[]) {
  test(`a 16-bot match on ${map} ends with a single winner on easy and hard`, () => {
    for (const difficulty of ['easy', 'hard'] as Difficulty[]) {
      const s = createMatch({ map, difficulty, seed: 31, seats: shooterSeats([], 16) });
      const seen = new Set<ShooterEventType>();
      let n = 0, last = 0;
      while (s.phase !== 'done' && n++ < 60 * 600) {
        tickMatch(s, []);
        for (const e of s.events) if (e.id > last) seen.add(e.type);
        last = s.eventSeq;
      }
      const label = `${map}/${difficulty}`;
      assert.equal(s.phase, 'done', `${label} ended (clock ${s.clock.toFixed(0)} s)`);
      assert.ok(s.clock < 600, label);
      assert.ok(s.winner >= 0, `${label} has a winner`);
      const winner = s.players[s.winner];
      assert.notEqual(winner.mode, 'dead');
      assert.equal(winner.place, 1);
      assert.equal(aliveCount(s), 1);
      assert.deepEqual(s.players.map(p => p.place).sort((a, b) => a - b), [...Array(16).keys()].map(i => i + 1), `${label} places`);
      const kills = s.players.reduce((a, p) => a + p.kills, 0);
      assert.ok(kills <= 15 && kills > 0, `${label} kills ${kills}`);
      // Bots loot, fight, heal, harvest with the pickaxe and build when shot.
      for (const type of ['jump', 'land', 'pickup', 'shot', 'hit', 'kill', 'storm', 'win', 'use', 'swing', 'harvest', 'build'] as ShooterEventType[]) assert.ok(seen.has(type), `${label} saw ${type}`);
      // Several minutes of play: the storm runs through most of its phases.
      assert.ok(s.clock > 180, `${label} lasted ${s.clock.toFixed(0)} s`);
      const results = shooterResults(s);
      assert.deepEqual(results.map(r => r.place), [...Array(16).keys()].map(i => i + 1));
      assert.equal(results[0].id, s.winner);
    }
  });
}

test('shooterSeats: humans first, bots fill with distinct fighters, then repeats marked II', () => {
  const seats = shooterSeats([{ slot: 4, name: 'Ann', pick: 'female-d' }, { slot: 1, name: 'Bo', pick: 'nobody' }], 14);
  assert.equal(seats.length, 16);
  assert.deepEqual(seats.slice(0, 2), [{ fighter: 'female-d', human: true, slot: 4, name: 'Ann' }, { fighter: 'male-a', human: true, slot: 1, name: 'Bo' }]);
  const bots = seats.slice(2);
  assert.ok(bots.every(b => !b.human && b.slot === -1));
  // 12 fighters, 2 taken: the first ten bots are distinct and untaken.
  const first = bots.slice(0, 10).map(b => b.fighter);
  assert.equal(new Set(first).size, 10);
  assert.ok(!first.includes('female-d') && !first.includes('male-a'));
  assert.ok(bots.slice(0, 12).every(b => !b.name.endsWith(' II')));
  assert.ok(bots.slice(12).every(b => b.name.endsWith(' II')));
  assert.deepEqual(shooterSeats([], 0), []);
  assert.equal(shooterSeats(humans(3), 0).length, 3);
});
