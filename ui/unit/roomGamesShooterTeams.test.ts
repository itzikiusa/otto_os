import { test } from 'node:test';
import assert from 'node:assert/strict';
import { HALF, climbStep, mapFor, STOREY, terrainHeight, waterLevel, type ShooterMap } from '../src/modules/rooms/games/shooter/map.ts';
import { groundAt, raycast, worldFor } from '../src/modules/rooms/games/shooter/collide.ts';
import { CONSUMABLES, WEAPONS } from '../src/modules/rooms/games/shooter/weapons.ts';
import { makeBuild, TILE, tileOf, tileX } from '../src/modules/rooms/games/shooter/builds.ts';
import {
  BLEED, createMatch, DOWN_HP, DROP_HEIGHT, EYE, HARVEST_LEFT, idleShooterInput, KILL_BLEED, LAUNCH_SPEED, REGROW, REVIVE_HP, REVIVE_TIME, STEP, STORM, teamsAlive, tickMatch,
  type Fighter, type ShooterEventType, type ShooterInput, type ShooterState, type TeamSize,
} from '../src/modules/rooms/games/shooter/sim.ts';
import { decodeSnapshot, encodeSnapshot } from '../src/modules/rooms/games/shooter/net.ts';
import { shooterSeats, type HumanSeat } from '../src/modules/rooms/games/setup.ts';

type Inputs = (ShooterInput | undefined)[];
const humans = (n: number): HumanSeat[] => Array.from({ length: n }, (_, i) => ({ slot: i, name: `P${i}`, pick: 'male-b' }));
const run = (s: ShooterState, seconds: number, inputs: Inputs | ((s: ShooterState) => Inputs) = []): void => {
  for (let i = 0, n = Math.round(seconds / STEP); i < n; i++) tickMatch(s, typeof inputs === 'function' ? inputs(s) : inputs);
};
const events = (s: ShooterState, type: ShooterEventType, from = 0) => s.events.filter(e => e.id > from && e.type === type);
const press = (k: Partial<ShooterInput>): ShooterInput => ({ ...idleShooterInput(), ...k });
const wire = <T>(v: T): unknown => JSON.parse(JSON.stringify(v));

/** First flat, prop-free spot on a map, centred on a build tile. */
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
/** Calm storm centred on (x, z) so nobody is hurried or hurt by it. */
const calm = (s: ShooterState, x: number, z: number) => Object.assign(s.storm, { stage: 0, cx: x, cz: z, r: 300, fromX: x, fromZ: z, fromR: 300, toX: x, toZ: z, toR: 300, timer: 9999, shrinking: false, damage: 1 });
/** Red Mesa, `n` humans in teams of `size`, everyone on the ground near SPOT (fighter i at +3 m east per index), no loot. */
function teamMatch(n: number, size: TeamSize, bots = 0): ShooterState {
  const s = createMatch({ map: 'mesa', difficulty: 'normal', seed: 5, seats: shooterSeats(humans(n), bots), teamSize: size });
  s.loot = [];
  s.players.forEach((p, i) => put(s, p, SPOT.x + i * 3, SPOT.z));
  calm(s, SPOT.x, SPOT.z);
  tickMatch(s, []);
  return s;
}
/** Parks a fighter far away (a second team keeps a match from ending). */
const park = (s: ShooterState, p: Fighter) => put(s, p, SPOT.x + 90, SPOT.z + 90);
const aimAt = (from: Fighter, to: Fighter, h = 1.15): { yaw: number; pitch: number } => {
  const dx = to.x - from.x, dz = to.z - from.z, d = Math.hypot(dx, dz);
  return { yaw: Math.atan2(dx, dz), pitch: Math.atan2(to.y + h - (from.y + EYE), d) };
};
/** Fighter `a` fires one sniper round at `b` (b left with `hp` health and no shield first). */
function snipe(s: ShooterState, a: Fighter, b: Fighter, hp: number): void {
  a.weapons[0] = { w: 'sniper', rarity: 0, mag: 4 }; a.current = 0; a.cooldown = 0; a.reload = 0; a.prevFire = false; a.bloom = 0;
  b.hp = hp; b.shield = 0;
  const inputs: Inputs = [];
  inputs[a.id] = press({ ...aimAt(a, b), fire: true, aim: true });
  tickMatch(s, inputs);
}

// ------------------------------------------------------------ teams

test('teams: seats fill teams in order (humans together), a seat can pin its team, solo keeps everyone apart', () => {
  const duo = createMatch({ map: 'isle', difficulty: 'easy', seed: 1, seats: shooterSeats(humans(3), 13), teamSize: 2 });
  assert.equal(duo.squad, 2);
  assert.deepEqual(duo.players.map(p => p.team), [0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7]);
  const squad = createMatch({ map: 'isle', difficulty: 'easy', seed: 1, seats: shooterSeats(humans(2), 14), teamSize: 4 });
  assert.deepEqual(squad.players.map(p => p.team), [0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3]);
  const seats = shooterSeats(humans(2), 6);
  seats[1].team = 3;
  assert.equal(createMatch({ map: 'isle', difficulty: 'easy', seed: 1, seats, teamSize: 2 }).players[1].team, 3);
  const solo = createMatch({ map: 'isle', difficulty: 'easy', seed: 1, seats: shooterSeats(humans(2), 6) });
  assert.equal(solo.squad, 1);
  assert.deepEqual(solo.players.map(p => p.team), [0, 1, 2, 3, 4, 5, 6, 7]);
  assert.equal(teamsAlive(solo), 8);
  // Bot teammates drop together.
  const lead = squad.players[4], mate = squad.players[5];
  assert.ok(Math.hypot(lead.bot.goalX - mate.bot.goalX, lead.bot.goalZ - mate.bot.goalZ) < 15);
  assert.equal(createMatch({ map: 'isle', difficulty: 'easy', seed: 1, seats: shooterSeats([], 8), teamSize: 3 as TeamSize }).squad, 1, 'unknown sizes fall back to solo');
});

test('knocked, not eliminated, while a teammate stands: crawl, no weapons, bleed out credited to the knocker', () => {
  const s = teamMatch(4, 2);
  const [a, , c] = s.players;
  put(s, c, a.x, a.z + 12, Math.PI);
  snipe(s, c, a, 5);
  assert.equal(a.mode, 'down');
  assert.equal(a.hp, DOWN_HP);
  assert.equal(a.knocker, c.id);
  assert.equal(events(s, 'knock').at(-1)?.target, a.id);
  assert.equal(c.kills, 0, 'a knock is not an elimination');
  assert.equal(teamsAlive(s), 2);
  // Crawling is slow, and nothing else works.
  a.weapons[0] = { w: 'smg', rarity: 0, mag: 30 }; a.current = 0;
  const x0 = a.x, seq = s.eventSeq;
  run(s, 2, [press({ mz: 1, sprint: true, fire: true, jump: true, build: true })]);
  const crawled = Math.hypot(a.x - x0, 0) + Math.abs(a.z - SPOT.z);
  assert.ok(crawled > 2 && crawled < 4, `crawled ${crawled.toFixed(2)} m in 2 s`);
  assert.equal(events(s, 'shot', seq).length, 0);
  assert.equal(s.builds.length, 0);
  assert.ok(Math.abs(a.hp - (DOWN_HP - BLEED * 2)) < 0.2, `bleeding: ${a.hp.toFixed(1)}`);
  run(s, DOWN_HP / BLEED);
  assert.equal(a.mode, 'dead');
  assert.equal(a.killedBy, c.id);
  assert.equal(c.kills, 1);
  assert.equal(events(s, 'kill').at(-1)?.value, KILL_BLEED);
});

test('revive: hold interact next to a knocked teammate; letting go resets progress', () => {
  const s = teamMatch(4, 2);
  const [a, b, c] = s.players;
  put(s, c, a.x, a.z + 12, Math.PI);
  snipe(s, c, a, 5);
  assert.equal(a.mode, 'down');
  put(s, b, a.x + 1.2, a.z);
  const hold: Inputs = [undefined, press({ interact: true, mz: 1 })];
  run(s, 1.5, hold);
  assert.ok(Math.abs(a.revive - 1.5) < 0.05, `progress ${a.revive.toFixed(2)}`);
  assert.ok(Math.abs(b.x - (a.x + 1.2)) < 0.05, 'the reviver stays put');
  tickMatch(s, []);
  assert.equal(a.revive, 0, 'released: progress lost');
  run(s, REVIVE_TIME + 0.1, hold);
  assert.equal(a.mode, 'ground');
  assert.equal(a.hp, REVIVE_HP);
  assert.equal(events(s, 'revive').at(-1)?.player, b.id);
  // An enemy cannot revive.
  snipe(s, c, a, 5);
  assert.equal(a.mode, 'down');
  const d = s.players[3];
  put(s, d, a.x - 1.2, a.z);
  run(s, REVIVE_TIME + 0.5, [undefined, undefined, undefined, press({ interact: true })]);
  assert.equal(a.mode, 'down');
});

test('no friendly fire: shots pass through teammates and hit the enemy behind', () => {
  const s = teamMatch(4, 2);
  const [a, b, c] = s.players;
  put(s, b, a.x, a.z + 6); put(s, c, a.x, a.z + 14);
  a.weapons[0] = { w: 'rifle', rarity: 0, mag: 30 }; a.current = 0;
  b.shield = 0; c.shield = 0;
  run(s, 0.5, st => [press({ ...aimAt(st.players[0], st.players[2]), fire: true, aim: true })]);
  assert.equal(b.hp, 100, 'teammate untouched');
  assert.ok(c.hp < 100, 'enemy behind was hit');
  // Rocket splash spares teammates too (but not the shooter).
  b.hp = 100; a.weapons[1] = { w: 'rocket', rarity: 0, mag: 1 }; a.current = 1; a.cooldown = 0; a.prevFire = false;
  put(s, b, a.x + 3, a.z);
  run(s, 1, [press({ yaw: Math.PI / 2, pitch: -0.6, fire: true })]);
  assert.ok(events(s, 'explode').length > 0);
  assert.equal(b.hp, 100);
});

test('team wipe: the last one standing going down takes the knocked teammates out; both share the place; the last team wins together', () => {
  const s = teamMatch(6, 2);
  const [a, b, c, d] = s.players;
  for (const p of s.players.slice(4)) { put(s, p, SPOT.x + 60, SPOT.z + 60); }
  put(s, c, a.x, a.z + 12, Math.PI);
  snipe(s, c, a, 5);
  assert.equal(a.mode, 'down');
  snipe(s, c, b, 5);
  assert.equal(b.mode, 'dead', 'nobody left standing: eliminated, not knocked');
  assert.equal(a.mode, 'dead', 'the knocked teammate goes too');
  assert.equal(a.killedBy, c.id);
  assert.equal(a.place, 3); assert.equal(b.place, 3);
  assert.equal(teamsAlive(s), 2);
  // Team 2 (fighters 4, 5) knocks d, then is wiped: team 1 wins; its knocked member still places first.
  const [e, f] = s.players.slice(4);
  put(s, e, d.x, d.z - 10);
  snipe(s, e, d, 5);
  assert.equal(d.mode, 'down');
  put(s, e, SPOT.x + 60, SPOT.z + 60);
  put(s, c, e.x, e.z - 10);
  snipe(s, c, e, 5); snipe(s, c, f, 5);
  assert.equal(f.mode, 'dead'); assert.equal(e.mode, 'dead');
  assert.equal(e.place, 2);
  run(s, 3.2);
  assert.equal(s.phase, 'done');
  assert.equal(s.winTeam, c.team);
  assert.equal(s.winner, c.id);
  assert.equal(c.place, 1); assert.equal(d.place, 1);
  assert.equal(events(s, 'win').at(-1)?.value, c.team);
});

test('the storm cannot wipe out the last two survivors in the same tick: the last one standing wins', () => {
  const s = teamMatch(2, 1);
  const [a, b] = s.players;
  park(s, b);
  // Both deep in a final storm, one hit from the end, their storm clocks in step.
  Object.assign(s.storm, { stage: 6, cx: SPOT.x + 300, cz: SPOT.z + 300, r: 0, fromX: SPOT.x + 300, fromZ: SPOT.z + 300, fromR: 0, toX: SPOT.x + 300, toZ: SPOT.z + 300, toR: 0, timer: 9999, shrinking: false, damage: 14 });
  a.hp = b.hp = 5; a.shield = b.shield = 0; a.stormT = b.stormT = 0.99;
  run(s, 0.1);
  assert.equal([a, b].filter(p => p.mode === 'dead').length, 1);
  run(s, 3.2);
  assert.equal(s.phase, 'done');
  assert.ok(s.winner >= 0 && s.players[s.winner].mode !== 'dead' && s.players[s.winner].place === 1);
});

// ------------------------------------------------------------ harvesting

test('harvesting: a tree gives a limited amount, falls (no collider) and regrows; a rock runs dry but stays', () => {
  const s = teamMatch(2, 1);
  const a = s.players[0];
  park(s, s.players[1]);
  const w = worldFor(MESA);
  const index = MESA.cylinders.findIndex(c => Math.abs(c.x) < HALF - 30 && Math.abs(c.z) < HALF - 30 && Math.abs(terrainHeight(MESA, c.x, c.z - c.r - 0.9) - terrainHeight(MESA, c.x, c.z)) < 0.3);
  const tree = MESA.cylinders[index];
  put(s, a, tree.x, tree.z - tree.r - 0.9);
  calm(s, a.x, a.z);
  run(s, 6, [press({ fire: true })]);
  assert.equal(a.mats[0], HARVEST_LEFT[0], 'all the wood the tree had');
  assert.deepEqual(s.felled, [index]);
  assert.equal(events(s, 'fell').at(-1)?.value, index);
  assert.equal(w.felled[index], 1);
  // Gone: rays and survivors pass where the trunk stood.
  assert.notEqual(raycast(w, a.x, a.y + EYE, a.z, 0, 0, 1, 3).kind, 'prop');
  const wood = a.mats[0];
  run(s, 1, [press({ fire: true })]);
  assert.equal(a.mats[0], wood, 'nothing more from a felled tree');
  run(s, REGROW + 0.5);
  assert.deepEqual(s.felled, []);
  assert.equal(w.felled[index], 0);
  assert.equal(s.nodes.length, 0);
  // A rock: harvested dry, still solid.
  const rock = MESA.boxes.findIndex((bx, i) => MESA.boxMats[i] === 1 && bx.maxY - terrainHeight(MESA, (bx.minX + bx.maxX) / 2, bx.minZ - 0.9) > 2.4 && Math.abs(terrainHeight(MESA, (bx.minX + bx.maxX) / 2, bx.minZ - 0.9) - terrainHeight(MESA, (bx.minX + bx.maxX) / 2, (bx.minZ + bx.maxZ) / 2)) < 0.4 && !MESA.blocks.includes(bx as never));
  const r = MESA.boxes[rock];
  put(s, a, (r.minX + r.maxX) / 2, r.minZ - 0.9);
  calm(s, a.x, a.z);
  run(s, 6, [press({ fire: true })]);
  assert.equal(a.mats[1], HARVEST_LEFT[1]);
  assert.equal(raycast(w, a.x, a.y + EYE, a.z, 0, 0, 1, 3).index, rock, 'the rock is still there');
  assert.ok(events(s, 'harvest').at(-1)!.flag === 0, 'dry');
});

// ------------------------------------------------------------ launch pads and supply drops

test('launch pads: a map pad throws a survivor ~20 m up, the glider opens on the way down; a deployed pad works too', () => {
  const s = createMatch({ map: 'mesa', difficulty: 'easy', seed: 5, seats: shooterSeats(humans(2), 0) });
  assert.equal(s.pads.length, MESA.pads.length);
  assert.ok(s.pads.length >= 2, 'set-piece pads on the map');
  const a = s.players[0], pad = s.pads[0];
  park(s, s.players[1]);
  put(s, a, pad.x + 0.3, pad.z);
  calm(s, a.x, a.z);
  tickMatch(s, []);
  assert.equal(a.mode, 'sky');
  assert.equal(events(s, 'launch').length, 1);
  let top = a.y;
  run(s, 1.6, st => { top = Math.max(top, st.players[0].y); return []; });
  assert.ok(top - pad.y > 16 && top - pad.y < 24, `apex ${(top - pad.y).toFixed(1)} m (launch ${LAUNCH_SPEED} m/s)`);
  run(s, 1);
  assert.equal(a.mode, 'glide');
  run(s, 12, [press({ mz: 1 })]);
  assert.equal(a.mode, 'ground');
  // Deploy one from the hotbar.
  put(s, a, SPOT.x, SPOT.z);
  a.weapons[0] = { w: 'launchpad', rarity: 3, mag: 1 }; a.current = 0;
  const pads = s.pads.length;
  run(s, CONSUMABLES.launchpad.time + 0.1, [press({ fire: true })]);
  assert.equal(s.pads.length, pads + 1);
  assert.equal(a.weapons[0], null);
  const mine = s.pads.at(-1)!;
  assert.ok(Math.hypot(mine.x - a.x, mine.z - (a.z + 2.2)) < 0.01);
  run(s, 1, [press({ mz: 1 })]);
  assert.equal(a.mode, 'sky', 'walked onto the deployed pad');
});

test('supply drops fall when the storm starts closing, land, and open into an epic-or-better weapon and shields', () => {
  const s = createMatch({ map: 'mesa', difficulty: 'easy', seed: 5, seats: shooterSeats(humans(2), 0), stormScale: 0.05 });
  assert.ok(Math.abs(s.storm.timer - STORM[0].wait * 0.05) < 1e-9, 'the storm schedule is scaled');
  const a = s.players[0];
  put(s, a, SPOT.x, SPOT.z);
  park(s, s.players[1]);
  a.hp = 1e6; s.players[1].hp = 1e6;
  tickMatch(s, []);
  run(s, STORM[0].wait * 0.05 + 0.1);
  assert.equal(s.storm.stage, 0);
  assert.equal(s.drops.length, 1);
  const d = s.drops[0];
  assert.ok(d.y > DROP_HEIGHT - 2 && !d.landed);
  calm(s, a.x, a.z); // no further phases (or drops) while this one comes down
  assert.equal(events(s, 'supply').at(-1)?.flag, 0);
  run(s, (DROP_HEIGHT - d.ground) / 7 + 0.5);
  assert.ok(d.landed && d.y === d.ground);
  put(s, a, d.x - 1.5, d.z);
  a.y = d.ground;
  s.loot = [];
  run(s, 1.1, [press({ interact: true })]);
  assert.equal(s.drops.length, 0);
  assert.equal(events(s, 'supply').at(-1)?.flag, 2);
  const weapon = s.loot.find(l => l.kind === 'weapon') ?? a.weapons.map(w => w && WEAPONS[w.w as keyof typeof WEAPONS] ? { rarity: w.rarity } : null).find(Boolean);
  assert.ok(weapon && weapon.rarity >= 3, 'epic or legendary');
  assert.ok(s.loot.some(l => l.weapon === 'shieldpot') || a.weapons.some(w => w?.w === 'shieldpot'));
});

// ------------------------------------------------------------ bots

test('bots revive a knocked teammate when no enemy is in sight', () => {
  const s = createMatch({ map: 'mesa', difficulty: 'normal', seed: 3, seats: shooterSeats([], 4), teamSize: 2 });
  s.loot = [];
  const [a, b, c, d] = s.players;
  put(s, a, SPOT.x, SPOT.z); put(s, b, SPOT.x + 14, SPOT.z);
  put(s, c, SPOT.x + 150, SPOT.z + 150); put(s, d, SPOT.x + 152, SPOT.z + 150);
  calm(s, SPOT.x, SPOT.z);
  tickMatch(s, []);
  b.mode = 'down'; b.hp = DOWN_HP; b.knocker = c.id;
  run(s, 12);
  assert.ok(events(s, 'revive').some(e => e.player === a.id && e.target === b.id), 'revived');
  assert.equal(b.mode, 'ground');
});

test('bots climb the stairs to loot on an upper floor', () => {
  const climb = MESA.climbs.find(c => c.floors >= 2)!;
  const s = createMatch({ map: 'mesa', difficulty: 'normal', seed: 4, seats: shooterSeats([], 2) });
  s.loot = []; s.chests = s.chests.map(() => true);
  const a = s.players[0];
  park(s, s.players[1]);
  // Two storeys up: the loot spot the map put there.
  const spot = MESA.lootSpots.find(l => l.y !== undefined && Math.abs(l.y - (climb.base + 2 * STOREY)) < 0.1 && l.x > climb.box.minX && l.x < climb.box.maxX && l.z > climb.box.minZ && l.z < climb.box.maxZ)!;
  assert.ok(spot, 'loot spot upstairs');
  s.loot.push({ id: 9999, kind: 'weapon', weapon: 'rifle', rarity: 2, ammo: null, mat: null, amount: 30, x: spot.x, y: spot.y!, z: spot.z });
  // Start outside, off the front door.
  const [fx, fz] = (() => { const out: [number, number] = [0, 0]; const q = climb.q & 3, lx = climb.w * 0.15, lz = climb.d / 2 + 6; if (q === 1) { out[0] = climb.cx + lz; out[1] = climb.cz - lx; } else if (q === 2) { out[0] = climb.cx - lx; out[1] = climb.cz - lz; } else if (q === 3) { out[0] = climb.cx - lz; out[1] = climb.cz + lx; } else { out[0] = climb.cx + lx; out[1] = climb.cz + lz; } return out; })();
  put(s, a, fx, fz);
  calm(s, a.x, a.z);
  let top = a.y;
  run(s, 40, st => { top = Math.max(top, st.players[0].y); return []; });
  assert.ok(top > climb.base + 2 * STOREY - 0.3, `climbed to ${(top - climb.base).toFixed(1)} m above the ground floor`);
  assert.ok(a.weapons.some(w => w?.w === 'rifle'), 'picked up the rifle upstairs');
});

test('climbStep: routes in through a door, up each flight, back down through the stairwell and out', () => {
  const c = MESA.climbs.find(k => k.floors >= 2)!;
  const inside = (t: [number, number] | null) => !!t && t[0] > c.box.minX && t[0] < c.box.maxX && t[1] > c.box.minZ && t[1] < c.box.maxZ;
  // Far outside, goal on the roof: head for a door or a corner (outside the walls).
  const far = climbStep(c, c.cx + 40, c.base - 0.5, c.cz + 40, c.cx, c.roof, c.cz);
  assert.ok(far && !inside(far));
  // Outside with the goal outside too: no help.
  assert.equal(climbStep(c, c.cx + 40, c.base - 0.5, c.cz + 40, c.cx + 60, NaN, c.cz), null);
  // On the goal's floor: walk straight.
  assert.equal(climbStep(c, c.cx, c.base + STOREY, c.cz, c.cx + 1, c.base + STOREY, c.cz), null);
  // Inside on the ground floor wanting the roof: somewhere inside (the flight), never a wall.
  assert.ok(inside(climbStep(c, c.cx, c.base, c.cz, c.cx, c.roof, c.cz)));
  // Upstairs with a goal outside: down through the stairwell (inside), then out of a door from the ground floor.
  assert.ok(inside(climbStep(c, c.cx, c.base + STOREY, c.cz, c.cx + 50, c.base + STOREY, c.cz)), 'a goal outside at the same height still goes down first');
  const out = climbStep(c, c.cx, c.base, c.cz, c.cx + 50, NaN, c.cz + 50);
  assert.ok(out && !inside(out), 'leaves by a door');
});

test('a hard bot ramp-rushes an enemy holding high ground', () => {
  let built = 0, rose = 0;
  for (const seed of [1, 2, 3]) {
    const s = createMatch({ map: 'mesa', difficulty: 'hard', seed, seats: shooterSeats(humans(1), 1) });
    s.loot = [];
    const [me, bot] = s.players;
    // The human stands on a floor two levels up (a platform held in place for the test).
    const gx = tileOf(SPOT.x), gz = tileOf(SPOT.z + 16);
    const level = Math.round(terrainHeight(MESA, SPOT.x, SPOT.z + 16) / 0.8) + 8;
    s.builds.push(makeBuild(77, { kind: 1, gx, gz, level, dir: 0 }, 2, 0));
    put(s, me, tileX(gx) + TILE / 2, tileX(gz) + TILE / 2);
    me.y = level * 0.8; me.hp = 1e6;
    put(s, bot, SPOT.x, SPOT.z - 2);
    calm(s, bot.x, bot.z);
    bot.weapons[0] = { w: 'rifle', rarity: 0, mag: 30 }; bot.current = 0; bot.ammo.medium = 90; bot.mats = [200, 0, 0];
    const y0 = bot.y, seq = s.eventSeq;
    let top = y0;
    run(s, 5, st => { top = Math.max(top, st.players[1].y); return []; });
    built += events(s, 'build', seq).filter(e => e.player === bot.id && e.value === 2).length;
    rose = Math.max(rose, top - y0);
  }
  assert.ok(built >= 2, `${built} ramps`);
  assert.ok(rose > 2, `climbed ${rose.toFixed(1)} m on them`);
});

test('bots skip trees that are already felled', () => {
  const s = createMatch({ map: 'mesa', difficulty: 'normal', seed: 11, seats: shooterSeats([], 2) });
  s.loot = []; s.chests = s.chests.map(() => true);
  const a = s.players[0];
  park(s, s.players[1]);
  put(s, a, SPOT.x, SPOT.z);
  calm(s, a.x, a.z);
  // Fell every tree within 30 m: the bot must not walk to them for wood.
  const near = MESA.cylinders.map((c, i) => ({ c, i })).filter(({ c }) => Math.hypot(c.x - a.x, c.z - a.z) < 30);
  for (const { i } of near) s.nodes.push({ key: -1 - i, left: 0, regrow: 999 });
  s.felled = near.map(n => n.i);
  run(s, 3);
  assert.ok(a.bot.harvest < 0 || !s.felled.includes(a.bot.harvest), `harvest target ${a.bot.harvest}`);
});

// ------------------------------------------------------------ net

test('snapshots carry teams, knocked state and revive progress, pads, supply drops and felled trees', () => {
  const s = teamMatch(4, 2);
  const [a, b, c] = s.players;
  put(s, c, a.x, a.z + 12, Math.PI);
  snipe(s, c, a, 5);
  put(s, b, a.x + 1.2, a.z);
  run(s, 1, [undefined, press({ interact: true })]);
  s.drops.push({ id: 4242, x: 1.25, y: 50.5, z: -3.5, ground: 7.75, landed: false });
  s.felled = [3, 7];
  s.winTeam = -1;
  const back = decodeSnapshot(wire(encodeSnapshot(s, s.events)), slot => `H${slot}`)!;
  assert.ok(back);
  assert.equal(back.squad, 2);
  assert.equal(back.winTeam, -1);
  assert.deepEqual(back.players.map(p => p.team), s.players.map(p => p.team));
  assert.equal(back.players[0].mode, 'down');
  assert.ok(Math.abs(back.players[0].revive - a.revive) < 0.01 && a.revive > 0.9);
  assert.equal(back.pads.length, s.pads.length);
  back.pads.forEach((p, i) => assert.ok(Math.abs(p.x - s.pads[i].x) <= 0.05 && Math.abs(p.z - s.pads[i].z) <= 0.05));
  assert.deepEqual(back.drops, [{ id: 4242, x: 1.3, y: 50.5, z: -3.5, ground: 7.75, landed: false }]);
  assert.deepEqual(back.felled, [3, 7]);
  assert.ok(back.events.some(e => e.type === 'knock'));
  assert.equal(back.nodes.length, 0, 'harvest bookkeeping stays on the host');
});

test('a duos match on Palm Isle is deterministic per seed', () => {
  const play = () => {
    const s = createMatch({ map: 'isle', difficulty: 'hard', seed: 31, seats: shooterSeats([], 8), teamSize: 2 });
    run(s, 90);
    return JSON.stringify([s.players.map(p => [p.mode, Math.round(p.x * 100), Math.round(p.hp)]), s.felled, s.drops.length, s.eventSeq]);
  };
  assert.equal(play(), play());
});

test('duos: a person out of the match keeps it running while a bot teammate is still in it', () => {
  const s = createMatch({ map: 'mesa', difficulty: 'easy', seed: 29, seats: shooterSeats(humans(1), 3), teamSize: 2 });
  s.loot = [];
  const [me, mate, a, b] = s.players;
  assert.deepEqual(s.players.map(p => p.team), [0, 0, 1, 1]);
  for (const [p, dx] of [[mate, 0], [a, 6], [b, 8]] as const) put(s, p, SPOT.x + dx, SPOT.z + 2);
  put(s, me, SPOT.x + (SPOT.x < 0 ? 90 : -90), SPOT.z);
  s.phase = 'play';
  // The person is caught in the storm far away; the bots stand in a calm circle around SPOT.
  Object.assign(s.storm, { stage: 0, cx: SPOT.x, cz: SPOT.z, r: 40, fromX: SPOT.x, fromZ: SPOT.z, fromR: 40, toX: SPOT.x, toZ: SPOT.z, toR: 40, timer: 9999, shrinking: false, damage: 50 });
  me.hp = 1; me.shield = 0;
  run(s, 6, () => [undefined, idleShooterInput(), idleShooterInput(), idleShooterInput()]);
  assert.equal(me.mode, 'dead', 'the person was knocked, then finished by the storm');
  assert.equal(mate.mode, 'ground');
  assert.equal(s.phase, 'play', 'the match goes on while the person’s team is in it');
  assert.equal(s.endTimer, 0);
  // Once the teammate is out as well, the match wraps up as before.
  mate.hp = 1; mate.shield = 0; put(s, mate, me.x + 1, me.z);
  run(s, 6, () => [undefined, idleShooterInput(), idleShooterInput(), idleShooterInput()]);
  assert.equal(teamsAlive(s), 1);
  assert.equal(s.phase, 'done');
  assert.equal(a.place, 1);
  assert.equal(me.place, 2);
  assert.equal(mate.place, 2);
});
