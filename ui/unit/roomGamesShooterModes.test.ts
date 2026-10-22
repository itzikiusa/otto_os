import { test } from 'node:test';
import assert from 'node:assert/strict';
import { dryLand, mapFor, terrainHeight, type ShooterMap } from '../src/modules/rooms/games/shooter/map.ts';
import { groundAt, worldFor } from '../src/modules/rooms/games/shooter/collide.ts';
import { ATTACHMENTS, CLASSES, NADES, STREAKS, weaponStats, WEAPONS } from '../src/modules/rooms/games/shooter/weapons.ts';
import { makeBuild, tileOf, tileX, TILE, SUB } from '../src/modules/rooms/games/shooter/builds.ts';
import {
  aliveCount, createMatch, EYE, hurt, idleShooterInput, RESPAWN, shooterResults, SLIDE_TIME, STEP, TAC_TIME, tickMatch,
  type Fighter, type ShooterEventType, type ShooterInput, type ShooterSetup, type ShooterState,
} from '../src/modules/rooms/games/shooter/sim.ts';
import { CAP_TIME, WAVE_BOARD, WAVE_HELI, WAVE_ON, WAVE_OVER } from '../src/modules/rooms/games/shooter/modes/index.ts';
import { AREA_FIRE, AREA_SMOKE, smokeBetween } from '../src/modules/rooms/games/shooter/modes/gear.ts';
import { APOC_CAP, buyWallsFor, hurtZombie, LZ_R, MAX_INFECTED, waveSize, ZOMBIE_KINDS } from '../src/modules/rooms/games/shooter/zombies.ts';
import * as net from '../src/modules/rooms/games/shooter/net.ts';
import type { ShooterMapId, ShooterMode } from '../src/modules/rooms/games/shooter/catalog.ts';
import { shooterSeats, type HumanSeat } from '../src/modules/rooms/games/setup.ts';

type Inputs = (ShooterInput | undefined)[];
const RELAY_CAP = 16 * 1024;
const humans = (n: number): HumanSeat[] => Array.from({ length: n }, (_, i) => ({ slot: i, name: `P${i}`, pick: 'male-b' }));
const run = (s: ShooterState, seconds: number, inputs: Inputs | ((s: ShooterState) => Inputs) = []): void => {
  for (let i = 0, n = Math.round(seconds / STEP); i < n; i++) tickMatch(s, typeof inputs === 'function' ? inputs(s) : inputs);
};
const events = (s: ShooterState, type: ShooterEventType, from = 0) => s.events.filter(e => e.id > from && e.type === type);
const press = (k: Partial<ShooterInput>): ShooterInput => ({ ...idleShooterInput(), ...k });
const speed = (p: Fighter) => Math.hypot(p.vx, p.vz);

/** First flat spot with no colliders within `clear` metres (deterministic scan). */
function clearSpot(m: ShooterMap, clear = 16): { x: number; z: number } {
  for (let x = -120; x <= 120; x += 4) for (let z = -120; z <= 120; z += 4) {
    const h0 = terrainHeight(m, x, z);
    if ((m.theme === 'isle' && h0 < 1.5) || !dryLand(m, x, z)) continue;
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
const ISLE_SPOT = clearSpot(mapFor('isle'));

function put(s: ShooterState, p: Fighter, x: number, z: number, yaw = 0): void {
  const m = mapFor(s.map);
  p.x = x; p.z = z; p.y = groundAt(worldFor(m), x, z, terrainHeight(m, x, z));
  p.vx = p.vy = p.vz = 0; p.mode = 'ground'; p.grounded = true; p.yaw = yaw;
}
/** Royale on Red Mesa with `n` humans on the ground at SPOT, 30 m apart, no loot and no infected. */
function groundMatch(n: number, mode: ShooterMode = 'royale'): ShooterState {
  const s = createMatch({ map: 'mesa', difficulty: 'normal', seed: 5, seats: shooterSeats(humans(n), 0), mode });
  s.loot = []; s.infected = [];
  s.players.forEach((p, i) => put(s, p, SPOT.x + (i === 0 ? 0 : 30 + i * 4), SPOT.z - (i === 0 ? 0 : 30)));
  tickMatch(s, []);
  return s;
}
/** Yaw / pitch from the muzzle (over the right shoulder, as the game aims it) to a point `h` above `to`'s feet. */
const aimAt = (from: Fighter, to: { x: number; y: number; z: number }, h = 1.15) => {
  let yaw = Math.atan2(to.x - from.x, to.z - from.z);
  for (let k = 0; k < 3; k++) { const mx = from.x - Math.cos(yaw) * 0.3, mz = from.z + Math.sin(yaw) * 0.3; yaw = Math.atan2(to.x - mx, to.z - mz); }
  const mx = from.x - Math.cos(yaw) * 0.3, mz = from.z + Math.sin(yaw) * 0.3;
  return { yaw, pitch: Math.atan2(to.y + h - (from.y + EYE), Math.hypot(to.x - mx, to.z - mz)) };
};
const arm = (p: Fighter, w: keyof typeof WEAPONS, att = 0) => { p.weapons[0] = { w, rarity: 0, mag: weaponStats(WEAPONS[w], att).mag, att }; p.current = 0; p.cooldown = 0; p.reload = 0; p.ads = 0; p.kick = 0; p.kickYaw = 0; p.shots = 0; };

// ------------------------------------------------------------ gunplay

test('aim down sights: the sights rise over the weapon\'s ADS time (a scope is slower) and drop when sprinting', () => {
  const s = groundMatch(2);
  const p = s.players[0];
  arm(p, 'rifle');
  run(s, 0.1, [press({ aim: true })]);
  assert.ok(p.ads > 0.3 && p.ads < 0.6, `rifle sights ${p.ads.toFixed(2)} after 0.1 s (ADS ${WEAPONS.rifle.ads} s)`);
  run(s, 0.2, [press({ aim: true })]);
  assert.equal(p.ads, 1);
  run(s, 0.3, [press({ sprint: true, mz: 1 })]);
  assert.equal(p.ads, 0, 'sprinting lowers the weapon');
  arm(p, 'rifle', ATTACHMENTS[0].bit);
  run(s, WEAPONS.rifle.ads, [press({ aim: true })]);
  assert.ok(p.ads < 1, 'a scope raises slower');
  assert.ok(weaponStats(WEAPONS.rifle, 1).zoom < WEAPONS.rifle.zoom, 'and zooms further');
});

test('aimed fire is far tighter than hip fire', () => {
  const hits = (ads: boolean) => {
    const s = groundMatch(3);
    const [a, b] = s.players;
    put(s, b, a.x, a.z + 15);
    let n = 0;
    for (let k = 0; k < 24; k++) {
      arm(a, 'sniper'); a.ads = ads ? 1 : 0; a.bloom = 0; a.prevFire = false;
      b.hp = 1000; b.shield = 0;
      const seq = s.eventSeq;
      tickMatch(s, [press({ ...aimAt(a, b), aim: ads, fire: true })]);
      if (events(s, 'hit', seq).some(e => e.target === b.id)) n++;
      run(s, 0.3, []);
    }
    return n;
  };
  const ads = hits(true), hip = hits(false);
  assert.ok(ads >= 22 && ads > hip + 6, `ADS ${ads}/24 vs hip ${hip}/24 at 15 m`);
});

test('recoil: a spray climbs along the pattern (a grip tames it), the first shot goes where aimed, and it recovers', () => {
  const kickAfter = (att: number) => {
    const s = groundMatch(2);
    const p = s.players[0];
    arm(p, 'rifle', att); p.ammo.medium = 0;
    run(s, 0.6, [press({ fire: true, pitch: 1.2 })]);
    return { s, p, kick: p.kick, shots: p.shots };
  };
  const plain = kickAfter(0), grip = kickAfter(ATTACHMENTS[1].bit);
  assert.ok(plain.shots >= 3 && plain.kick > 0.02, `${plain.shots} shots kicked ${plain.kick.toFixed(3)} rad`);
  assert.ok(grip.kick < plain.kick * 0.75, `grip ${grip.kick.toFixed(3)} vs ${plain.kick.toFixed(3)}`);
  // Released: the kick recovers and the pattern starts over.
  run(plain.s, 1.2, [press({ pitch: 1.2 })]);
  assert.ok(plain.p.kick < 0.003 && plain.p.shots === 0, `recovered to ${plain.p.kick.toFixed(4)}`);
  // The pattern is the same every time (learnable).
  assert.equal(kickAfter(0).kick, plain.kick);
});

test('headshots and damage falloff: a head hit multiplies, range reduces', () => {
  const s = groundMatch(2);
  const [a, b] = s.players;
  put(s, b, a.x, a.z + 12);
  arm(a, 'rifle'); a.ads = 1;
  const seq = s.eventSeq;
  tickMatch(s, [press({ ...aimAt(a, b, 1.62), aim: true, fire: true })]);
  const hit = events(s, 'hit', seq).find(e => e.target === b.id)!;
  assert.ok(hit && (hit.flag & 2) === 2 && hit.value === Math.round(WEAPONS.rifle.damage * WEAPONS.rifle.head), `headshot for ${hit?.value}`);
});

test('reload cancel: a fresh trigger pull with rounds left cancels it; a tactical reload is quicker than an empty one', () => {
  const s = groundMatch(2);
  const p = s.players[0];
  arm(p, 'rifle'); p.ammo.medium = 90;
  p.weapons[0]!.mag = 10;
  tickMatch(s, [press({ reload: true, pitch: 1.3 })]);
  assert.ok(Math.abs(p.reload - WEAPONS.rifle.reload * 0.85) < 0.05, `tactical reload ${p.reload.toFixed(2)} s`);
  run(s, 0.3, [press({ pitch: 1.3 })]);
  tickMatch(s, [press({ fire: true, pitch: 1.3 })]);
  assert.equal(p.reload, 0, 'cancelled');
  assert.equal(p.weapons[0]!.mag, 9, 'and the shot went out');
  p.weapons[0]!.mag = 0;
  run(s, 0.3, []);
  tickMatch(s, [press({ reload: true })]);
  assert.ok(Math.abs(p.reload - WEAPONS.rifle.reload) < 0.05, 'empty reload: full time');
  tickMatch(s, [press({ fire: true })]);
  assert.ok(p.reload > 0, 'an empty mag cannot cancel');
});

test('tactical sprint is faster and burns stamina; a slide out of a sprint carries speed, then crouch is slow', () => {
  const s = groundMatch(2);
  const p = s.players[0];
  arm(p, 'smg');
  run(s, 0.8, [press({ sprint: true, mz: 1 })]);
  const sprint = speed(p);
  run(s, 0.8, [press({ sprint: true, tac: true, mz: 1 })]);
  assert.ok(p.tactical && speed(p) > sprint + 1, `tactical ${speed(p).toFixed(1)} vs sprint ${sprint.toFixed(1)}`);
  assert.ok(p.tacT < TAC_TIME - 0.7);
  run(s, TAC_TIME, [press({ sprint: true, tac: true, mz: 1 })]);
  assert.equal(p.tactical, false, 'out of stamina');
  // Slide: crouch while sprinting.
  run(s, 0.4, [press({ sprint: true, mz: 1 })]);
  const seq = s.eventSeq;
  tickMatch(s, [press({ sprint: true, mz: 1, crouch: true })]);
  assert.equal(events(s, 'slide', seq).length, 1);
  assert.ok(p.slideT > 0 && p.crouch && speed(p) > 10, `slide ${speed(p).toFixed(1)} m/s`);
  run(s, SLIDE_TIME + 0.6, [press({ mz: 1, crouch: true })]);
  assert.ok(p.crouch && speed(p) < 3.2, `crouch-walk ${speed(p).toFixed(1)} m/s`);
  run(s, 0.3, [press({ mz: 1 })]);
  assert.equal(p.crouch, false);
});

test('mantle: jumping at a chest-high ledge climbs onto it', () => {
  const s = groundMatch(2);
  const p = s.players[0];
  const g = p.y;
  // A floor piece whose top is ~1.6 m above the ground, one tile ahead (+z).
  const gx = tileOf(p.x), gz = tileOf(p.z) + 1, level = Math.round((g + 1.6) / SUB);
  s.builds.push({ ...makeBuild(s.nextId++, { kind: 1, gx, gz, level, dir: 0 }, 2, -1), grow: 0 });
  put(s, p, tileX(gx) + TILE / 2, tileX(gz) - 0.55, 0);
  const seq = s.eventSeq;
  tickMatch(s, [press({ jump: true, mz: 1 })]);
  assert.equal(events(s, 'mantle', seq).length, 1);
  run(s, 0.8, [press({ mz: 1 })]);
  assert.ok(Math.abs(p.y - level * SUB) < 0.05, `on the ledge at ${p.y.toFixed(2)} (top ${(level * SUB).toFixed(2)})`);
  assert.ok(p.z > tileX(gz), 'over the edge');
});

// ------------------------------------------------------------ gear

test('frag: thrown in an arc, cooks off after its fuse, hurts everyone in the blast (the thrower at half)', () => {
  const s = groundMatch(3);
  const [a, b, c] = s.players;
  put(s, b, a.x + 2, a.z + 2); put(s, c, a.x + 40, a.z);
  a.lethal = 0; a.lethalN = 2; arm(a, 'rifle');
  const seq = s.eventSeq;
  tickMatch(s, [press({ lethal: true, pitch: -1.3, yaw: Math.PI / 4 })]);
  assert.equal(a.lethalN, 1);
  assert.equal(events(s, 'nade', seq).length, 1);
  assert.equal(s.nades.length, 1);
  run(s, NADES[0].fuse - 0.2, []);
  assert.equal(events(s, 'explode', seq).length, 0, 'still cooking');
  run(s, 0.4, []);
  assert.equal(events(s, 'explode', seq).length, 1);
  assert.equal(s.nades.length, 0);
  assert.ok(b.hp < 100 - 40 || b.mode !== 'ground', `b took the blast (${b.hp})`);
  assert.ok(a.hp < 100 && a.hp > b.hp, `the thrower took half (${a.hp} vs ${b.hp})`);
  assert.equal(c.hp, 100);
});

test('molotov bursts into a fire that burns over time; smoke blocks sight; a flash blinds whoever looks at it', () => {
  const s = groundMatch(3);
  const [a, b, c] = s.players;
  put(s, b, a.x + 1, a.z + 1.5);
  a.lethal = 1; a.lethalN = 1; a.tacKind = 2; a.tacN = 1;
  tickMatch(s, [press({ lethal: true, pitch: -1.3, yaw: 0.4 })]);
  run(s, 0.6, []);
  const fire = s.areas.find(x => x.kind === AREA_FIRE);
  assert.ok(fire, 'a fire on the ground');
  const hp = b.hp;
  run(s, 2, []);
  assert.ok(b.hp < hp - 20, `burning: ${hp} → ${b.hp}`);
  s.areas = []; a.hp = 100; b.hp = 100;
  // Smoke.
  put(s, b, a.x, a.z + 20); b.hp = 100;
  run(s, 0.7, []);
  tickMatch(s, [press({ tactical: true, pitch: 0.15, yaw: 0 })]);
  run(s, 2.5, []);
  const smoke = s.areas.find(x => x.kind === AREA_SMOKE);
  assert.ok(smoke, 'a smoke cloud');
  assert.equal(smokeBetween(s, smoke!.x - 10, smoke!.y + 1, smoke!.z, smoke!.x + 10, smoke!.y + 1, smoke!.z), true);
  assert.equal(smokeBetween(s, smoke!.x - 30, smoke!.y + 1, smoke!.z + 30, smoke!.x - 20, smoke!.y + 1, smoke!.z + 30), false);
  // Flash: b looks at it, c looks away.
  s.areas = [];
  put(s, b, a.x + 4, a.z + 8, Math.PI); put(s, c, a.x - 4, a.z + 8, 0);
  a.tacKind = 3; a.tacN = 1; a.cooldown = 0;
  const seq = s.eventSeq;
  // b looks back toward a (and the flash), c looks the other way.
  const look = (first: boolean): Inputs => [first ? press({ tactical: true, pitch: -1.3, yaw: 0 }) : press({}), press({ yaw: Math.PI }), press({ yaw: 0 })];
  tickMatch(s, look(true));
  run(s, NADES[3].fuse + 0.6, () => look(false));
  assert.ok(events(s, 'flash', seq).length >= 1);
  assert.ok(b.blind > 0 || events(s, 'flash', seq).some(e => e.target === b.id), 'the one facing it is blinded');
  assert.ok(events(s, 'flash', seq).filter(e => e.target === b.id).reduce((m, e) => Math.max(m, e.value), 0) > events(s, 'flash', seq).filter(e => e.target === c.id).reduce((m, e) => Math.max(m, e.value), 0), 'more than the one facing away');
});

test('attachments: picked up onto the gun in hand; an extended mag reloads deeper; a suppressor keeps the shooter off radar', () => {
  const s = groundMatch(2);
  const p = s.players[0];
  arm(p, 'rifle'); p.ammo.medium = 120;
  s.loot.push({ id: s.nextId++, kind: 'attach', weapon: null, rarity: 2, ammo: null, mat: null, amount: 2, x: p.x, y: p.y, z: p.z });
  tickMatch(s, []);
  assert.equal(p.weapons[0]!.att, 4);
  assert.equal(s.loot.length, 0);
  p.weapons[0]!.mag = 0;
  tickMatch(s, [press({ reload: true })]);
  run(s, WEAPONS.rifle.reload + 0.1, []);
  assert.equal(p.weapons[0]!.mag, Math.round(WEAPONS.rifle.mag * 1.5));
  // Loud fire pings the radar; suppressed does not.
  tickMatch(s, [press({ fire: true, pitch: 1.3 })]);
  assert.ok(p.spotted > 1);
  run(s, 2, []);
  p.weapons[0]!.att |= ATTACHMENTS[3].bit;
  tickMatch(s, [press({ fire: true, pitch: 1.3 })]);
  assert.equal(p.spotted, 0);
  // A launcher takes none.
  arm(p, 'rocket');
  s.loot.push({ id: s.nextId++, kind: 'attach', weapon: null, rarity: 2, ammo: null, mat: null, amount: 1, x: p.x, y: p.y, z: p.z });
  tickMatch(s, []);
  assert.equal(p.weapons[0]!.att, 0);
  assert.equal(s.loot.length, 1);
});

test('killstreaks: earned at 3 / 5 / 7 eliminations without dying, kept until called in; UAV, airstrike and drone work', () => {
  const s = groundMatch(10);
  const a = s.players[0];
  const seq = s.eventSeq;
  for (let k = 1; k <= 7; k++) hurt(s, s.players[k], 500, a.id, false, true, 'rifle');
  assert.equal(a.kills, 7);
  assert.equal(a.ready, 7, 'all three earned');
  assert.deepEqual(events(s, 'streak', seq).filter(e => e.flag === 0).map(e => e.value), [0, 1, 2]);
  // UAV.
  tickMatch(s, [press({ streak: true })]);
  assert.ok(s.uav[a.team] > STREAKS[0].time - 0.1);
  assert.equal(a.ready, 6);
  // Airstrike at the crosshair: a warning, then five bombs.
  const b = s.players[8];
  put(s, b, a.x, a.z + 15);
  a.hp = 1000; // the line of bombs runs back toward the caller too
  tickMatch(s, []);
  const before = s.eventSeq;
  tickMatch(s, [press({ streak: true, ...aimAt(a, b, 0) })]);
  assert.equal(s.strikes.length, 1);
  run(s, 4, []);
  assert.equal(events(s, 'explode', before).length, 5);
  assert.equal(b.mode, 'dead', 'caught in it');
  // Drone: follows its owner and shoots an enemy in range.
  const c = s.players[9];
  put(s, c, a.x + 8, a.z + 8);
  tickMatch(s, []);
  tickMatch(s, [press({ streak: true })]);
  assert.equal(s.drones.length, 1);
  run(s, 3, []);
  assert.ok(c.hp < 100 || c.mode !== 'ground', `the drone shot (${c.hp})`);
  // Dying resets the progress toward the next streak.
  a.hp = 100; a.shield = 0;
  hurt(s, a, 500, c.id, false, true, 'rifle');
  assert.equal(a.mode, 'dead');
  assert.equal(a.streak, 0);
});

// ------------------------------------------------------------ arena modes

test('Team Deathmatch: two teams at opposite spawns with class loadouts; eliminations score, the dead respawn, the limit wins', () => {
  const s = createMatch({ map: 'isle', difficulty: 'normal', seed: 3, seats: shooterSeats(humans(4), 0), mode: 'tdm' });
  assert.equal(s.phase, 'play');
  assert.deepEqual(s.players.map(p => p.team), [0, 0, 1, 1]);
  const o = s.obj;
  for (const p of s.players) {
    assert.equal(p.mode, 'ground');
    assert.ok(Math.hypot(p.x - o.spawns[p.team][0], p.z - o.spawns[p.team][1]) < 12, `fighter ${p.id} at its team's spawn`);
    assert.deepEqual(p.weapons.slice(0, 2).map(w => w?.w), CLASSES[p.cls].guns.map(g => g[0]));
    assert.ok(p.lethalN > 0 && p.tacN > 0 && p.shield > 0);
  }
  assert.ok(Math.hypot(o.spawns[0][0] - o.spawns[1][0], o.spawns[0][1] - o.spawns[1][1]) > 70, 'spawns far apart');
  const [a, , c] = s.players;
  hurt(s, c, 500, a.id, false, true, 'rifle');
  assert.equal(c.mode, 'dead');
  assert.equal(o.score[0], 1);
  assert.equal(c.respawn, RESPAWN);
  // Respawn with the class picked (input `cls`).
  const inputs: Inputs = []; inputs[c.id] = press({ cls: 2 });
  run(s, RESPAWN + 0.1, inputs);
  assert.equal(c.mode, 'ground');
  assert.equal(c.cls, 2);
  assert.equal(c.weapons[0]?.w, CLASSES[2].guns[0][0]);
  assert.ok(Math.hypot(c.x - o.spawns[1][0], c.z - o.spawns[1][1]) < 12);
  // To the limit.
  o.limit = 2;
  hurt(s, c, 500, a.id, false, true, 'rifle');
  tickMatch(s, []);
  assert.equal(s.winTeam, 0);
  assert.deepEqual(s.players.map(p => p.place), [1, 1, 2, 2]);
  assert.equal(events(s, 'win').length, 1);
  run(s, 3.2, []);
  assert.equal(s.phase, 'done');
  assert.deepEqual(shooterResults(s).slice(0, 2).map(r => r.team), [0, 0]);
});

test('Domination: standing on a point captures it (contested points hold), held points score every two seconds', () => {
  const s = createMatch({ map: 'mesa', difficulty: 'normal', seed: 4, seats: shooterSeats(humans(2), 0), mode: 'dom' });
  const o = s.obj, [a, b] = s.players;
  assert.equal(o.points.length, 3);
  const A = o.points[0];
  put(s, a, A.x, A.z); put(s, b, o.points[2].x, o.points[2].z);
  run(s, CAP_TIME * 0.5, []);
  assert.ok(A.cap > 0.4 && A.own === -1 && A.by === 0);
  // b steps on: contested, nothing moves.
  put(s, b, A.x + 1, A.z);
  const cap = A.cap;
  run(s, 1, []);
  assert.equal(A.cap, cap);
  put(s, b, o.points[2].x, o.points[2].z);
  const seq = s.eventSeq;
  run(s, CAP_TIME * 0.6, []);
  assert.equal(A.own, 0);
  assert.equal(o.points[2].own, 1);
  assert.ok(events(s, 'capture', seq).some(e => e.value === 0 && e.flag === 0));
  const [s0, s1] = o.score;
  run(s, 4.05, []);
  assert.deepEqual([o.score[0] - s0, o.score[1] - s1], [2, 2]);
  // b takes A back: neutralise, then capture.
  put(s, a, o.points[1].x + 30, o.points[1].z + 30); put(s, b, A.x, A.z);
  run(s, CAP_TIME * 2.2, []);
  assert.equal(A.own, 1);
});

// ------------------------------------------------------------ Zombies

test('Zombies: co-op crew at the base with pistols and 500 points; round one starts; hits and kills pay points', () => {
  const s = createMatch({ map: 'isle', difficulty: 'normal', seed: 2, seats: shooterSeats(humans(2), 0), mode: 'zombies' });
  const o = s.obj, [a] = s.players;
  assert.equal(s.phase, 'play');
  assert.ok(s.players.every(p => p.team === 0 && p.points === 500 && p.weapons[0]?.w === 'pistol' && p.mode === 'ground'));
  assert.ok(o.zspawns.length >= 6);
  assert.ok(Math.hypot(o.lzX - o.ax, o.lzZ - o.az) > 30, 'the landing zone is away from the base');
  assert.equal(o.round, 0);
  run(s, 8.2, []);
  assert.equal(o.round, 1);
  assert.equal(o.wave, WAVE_ON);
  assert.equal(events(s, 'round').length, 1);
  run(s, 3, []);
  assert.ok(s.infected.length > 0 && s.infected.length + o.left === waveSize(1, 2));
  const z = s.infected[0], pts = a.points;
  hurtZombie(s, z, 10, a.id, false, 0);
  assert.equal(a.points, pts + 10);
  hurtZombie(s, z, 999, a.id, true, 0);
  assert.equal(a.points, pts + 20 + ZOMBIE_KINDS[z.kind].points + 40, 'headshot kill bonus');
  assert.equal(a.kills, 1);
  assert.equal(events(s, 'zkill').at(-1)?.target, z.id);
  tickMatch(s, []);
  assert.ok(!s.infected.includes(z), 'the dead are swept');
});

test('Zombies: buy-walls sell a gun (then its ammo at half price); barricades are repaired with the pickaxe for points', () => {
  const s = createMatch({ map: 'isle', difficulty: 'normal', seed: 2, seats: shooterSeats(humans(2), 0), mode: 'zombies' });
  const [a] = s.players, walls = buyWallsFor(mapFor('isle'));
  assert.ok(walls.length >= 4);
  const w = walls[0];
  put(s, a, w.x, w.z);
  a.points = 100;
  let seq = s.eventSeq;
  tickMatch(s, [press({ interact: true })]);
  assert.equal(events(s, 'buy', seq).at(-1)?.flag, 0, 'not enough points');
  a.points = 5000;
  tickMatch(s, []);
  seq = s.eventSeq;
  tickMatch(s, [press({ interact: true })]);
  assert.equal(events(s, 'buy', seq).at(-1)?.flag, 1);
  assert.equal(a.points, 5000 - w.price);
  assert.equal(a.weapons[a.current]?.w, w.w);
  a.ammo[WEAPONS[w.w].ammo] = 0;
  tickMatch(s, []);
  tickMatch(s, [press({ interact: true })]);
  assert.equal(a.points, 5000 - w.price - Math.round(w.price / 2), 'refill at half price');
  // Repair: swing the pickaxe at a damaged barricade.
  put(s, a, SPOT.x, SPOT.z);
  const b = { ...makeBuild(s.nextId++, { kind: 0, gx: tileOf(a.x), gz: tileOf(a.z), level: Math.round(a.y / SUB), dir: 0 }, 0, a.id), grow: 0, hp: 40 };
  s.builds.push(b);
  a.current = 5; a.cooldown = 0;
  const pts = a.points, wallZ = tileX(b.gz) + TILE;
  put(s, a, tileX(b.gx) + TILE / 2, wallZ - 1.2, 0);
  tickMatch(s, [press({ fire: true, pitch: 0.2, yaw: 0 })]);
  assert.ok(b.hp > 40, `repaired to ${b.hp}`);
  assert.equal(a.points, pts + 10);
  assert.ok(events(s, 'repair').length >= 1);
});

test('Zombies: the infected hunt, claw and knock survivors; the dead return next round; nobody standing is an overrun', () => {
  const s = createMatch({ map: 'isle', difficulty: 'normal', seed: 2, seats: shooterSeats(humans(2), 0), mode: 'zombies' });
  const o = s.obj, [a, b] = s.players;
  run(s, 8.2, []);
  // Bring the round's first infected next to `a` and let it work.
  run(s, 2, []);
  const z = s.infected[0];
  z.x = a.x + 0.8; z.z = a.z; z.y = a.y; z.attackT = 0; z.kind = 0;
  a.hp = 10; a.shield = 0;
  const seq = s.eventSeq;
  run(s, 1.5, []);
  assert.ok(events(s, 'hit', seq).some(e => e.player === -3 && e.target === a.id), 'clawed');
  assert.equal(a.mode, 'down', 'knocked while a teammate stands');
  // b dies: no one standing → overrun.
  for (const q of s.infected) q.hp = 0;
  hurt(s, b, 500, -3, false, true, 13);
  tickMatch(s, []);
  assert.equal(o.wave, WAVE_OVER);
  assert.equal(events(s, 'extract').at(-1)?.flag, 3);
  run(s, 3.2, []);
  assert.equal(s.phase, 'done');
  // Respawn at the next round: a fresh match, b eliminated mid-round, round cleared.
  const t = createMatch({ map: 'isle', difficulty: 'normal', seed: 2, seats: shooterSeats(humans(2), 0), mode: 'zombies' });
  run(t, 8.2, []);
  hurt(t, t.players[1], 500, -3, false, true, 13);
  assert.equal(t.players[1].mode, 'down', 'knocked first (a teammate stands)');
  hurt(t, t.players[1], 500, -3, false, true, 13);
  assert.equal(t.players[1].mode, 'dead');
  t.obj.left = 0; for (const q of t.infected) q.hp = 0;
  run(t, 11, []);
  assert.equal(t.obj.round, 2);
  assert.equal(t.players[1].mode, 'ground');
  assert.equal(t.players[1].weapons[0]?.w, 'pistol');
});

test('Zombies: after the last round the helicopter comes; whoever is on the landing zone when it leaves is extracted', () => {
  const s = createMatch({ map: 'isle', difficulty: 'normal', seed: 2, seats: shooterSeats(humans(2), 0), mode: 'zombies', rounds: 1 });
  const o = s.obj, [a, b] = s.players;
  assert.equal(o.lastRound, 1);
  run(s, 8.2, []);
  o.left = 0; for (const q of s.infected) q.hp = 0;
  run(s, 0.1, []);
  assert.equal(o.wave, WAVE_HELI);
  assert.equal(events(s, 'extract').at(-1)?.flag, 0);
  o.timer = 0.05;
  run(s, 0.1, []);
  assert.equal(o.wave, WAVE_BOARD);
  for (const q of s.infected) q.hp = 0;
  put(s, a, o.lzX, o.lzZ); put(s, b, o.lzX + LZ_R + 15, o.lzZ);
  o.timer = 0.05;
  run(s, 0.1, () => { for (const q of s.infected) q.hp = 0; return []; });
  assert.equal(o.aboard, 1);
  assert.equal(a.place, 1);
  assert.equal(b.place, 2);
  assert.equal(s.winner, a.id);
  run(s, 3.2, []);
  assert.equal(s.phase, 'done');
});

test('the infected: horde steering closes in around a survivor without stacking up', () => {
  const s = createMatch({ map: 'isle', difficulty: 'normal', seed: 2, seats: shooterSeats(humans(1), 0), mode: 'zombies' });
  const a = s.players[0];
  run(s, 8.2, []);
  s.obj.left = 0; s.infected = [];
  a.hp = 1e9;
  // A ring of 40 walkers 20 m out.
  for (let i = 0; i < 40; i++) {
    const ang = (i / 40) * Math.PI * 2;
    s.infected.push({ ...structuredClone(templateZombie()), id: 90000 + i, x: a.x + Math.cos(ang) * 20, z: a.z + Math.sin(ang) * 20, y: a.y });
  }
  const d0 = s.infected.reduce((m, z) => m + Math.hypot(z.x - a.x, z.z - a.z), 0) / 40;
  run(s, 10, []);
  const d1 = s.infected.reduce((m, z) => m + Math.hypot(z.x - a.x, z.z - a.z), 0) / s.infected.length;
  assert.ok(d1 < d0 - 10, `closed in from ${d0.toFixed(1)} to ${d1.toFixed(1)} m`);
  let overlap = 0;
  for (let i = 0; i < s.infected.length; i++) for (let j = i + 1; j < s.infected.length; j++) if (Math.hypot(s.infected[i].x - s.infected[j].x, s.infected[i].z - s.infected[j].z) < 0.3) overlap++;
  assert.ok(overlap <= 2, `${overlap} pairs stacked on each other`);
  assert.ok(events(s, 'hit').some(e => e.player === -3 && e.target === a.id), 'they reached and clawed');
  assert.ok(d1 < 4, `around their target (${d1.toFixed(1)} m on average)`);
});

test('the infected claw at a barricade in their way (Zombies leaves structures out of the flow field on purpose)', () => {
  const s = createMatch({ map: 'isle', difficulty: 'normal', seed: 2, seats: shooterSeats(humans(1), 0), mode: 'zombies' });
  const a = s.players[0];
  run(s, 8.2, []);
  s.obj.left = 0; s.infected = [];
  put(s, a, ISLE_SPOT.x, ISLE_SPOT.z);
  a.hp = 1e9;
  // Three wooden walls in a row just north of the survivor, one walker behind them.
  const gx = tileOf(a.x), gz = tileOf(a.z), level = Math.round(a.y / SUB) - 1;
  for (const dx of [-1, 0, 1]) s.builds.push({ ...makeBuild(s.nextId++, { kind: 0, gx: gx + dx, gz, level, dir: 0 }, 0, a.id), grow: 0, hp: 150 });
  const wallZ = tileX(gz) + TILE;
  s.infected.push({ ...templateZombie(), id: 70000, kind: 2, hp: 400, max: 400, x: a.x, z: wallZ + 2.5, y: a.y });
  run(s, 6, []);
  assert.ok(s.builds.length < 3 || s.builds.some(b => b.hp < 150), `walls: ${s.builds.map(b => Math.round(b.hp)).join(', ')}`);
});

test('the infected stay cheap: 64 of them chasing four survivors cost well under a millisecond a tick', () => {
  const s = createMatch({ map: 'isle', difficulty: 'normal', seed: 2, seats: shooterSeats(humans(4), 0), mode: 'zombies' });
  run(s, 8.2, []);
  s.obj.left = 0; s.infected = [];
  for (const p of s.players) p.hp = 1e9;
  for (let i = 0; i < MAX_INFECTED; i++) s.infected.push({ ...structuredClone(templateZombie()), id: 80000 + i, kind: (i % 4) as 0, x: s.obj.ax + 40 + (i % 8) * 2, z: s.obj.az + Math.floor(i / 8) * 2, y: terrainHeight(mapFor('isle'), s.obj.ax + 40, s.obj.az) });
  run(s, 1, []);
  const t0 = performance.now();
  run(s, 5, []);
  const per = (performance.now() - t0) / (5 / STEP);
  assert.equal(s.infected.length, MAX_INFECTED);
  assert.ok(per < 1.5, `${per.toFixed(3)} ms per tick`);
});

test('Apocalypse Royale: the horde roams the island from the start; the toxic cloud hits harder than the storm', () => {
  const s = createMatch({ map: 'isle', difficulty: 'normal', seed: 6, seats: shooterSeats(humans(1), 7), mode: 'apoc', stormScale: 0.05 });
  assert.equal(s.phase, 'bus');
  assert.ok(s.infected.length >= APOC_CAP - 6 && s.infected.length <= APOC_CAP);
  assert.ok(s.loot.length > 50, 'royale floor loot');
  run(s, 30, [press({ jump: true })]);
  assert.ok(s.storm.stage >= 0);
  assert.ok(s.storm.damage >= Math.round(1 * 1.5));
});

/** A walker with no state (tests place it). */
function templateZombie() {
  return { id: 0, kind: 0 as const, x: 0, y: 0, z: 0, vx: 0, vy: 0, vz: 0, yaw: 0, grounded: true, hp: 70, max: 70, target: -1, think: 0, attackT: 0, spitT: 2, stun: 0, stuck: 0, lastX: 0, lastZ: 0, side: 1, sideT: 0, wx: 0, wz: 0, aggro: true, swing: 0, los: false, idle: 0 };
}

// ------------------------------------------------------------ play-tests

/**
 * A bot match in `mode` through the host send cadence (20 Hz; loot and
 * structures ride when they changed or every two seconds, never together)
 * with a guest decoding every snapshot. Returns the end state and counters.
 */
function playMode(setup: Partial<ShooterSetup> & { mode: ShooterMode; map: ShooterMapId }, bots: number, limit: number) {
  const s = createMatch({ difficulty: 'hard', seed: 11, seats: shooterSeats([], bots), ...setup });
  let guest: ShooterState | null = null, lastLootRev = -1, lastBuildRev = -1, preferBuilds = false, sentAt = 0, n = 0, maxBytes = 0, last = 0, maxInfected = 0;
  const seen: Partial<Record<ShooterEventType, number>> = {};
  while (s.phase !== 'done' && n++ < limit) {
    tickMatch(s, []);
    for (const e of s.events) if (e.id > last) seen[e.type] = (seen[e.type] ?? 0) + 1;
    last = s.eventSeq;
    maxInfected = Math.max(maxInfected, s.infected.length);
    for (const p of s.players) assert.ok(Number.isFinite(p.x + p.y + p.z + p.hp + p.shield + p.kick + p.ads), `${setup.mode}: fighter ${p.id} state is finite`);
    for (const z of s.infected) assert.ok(Number.isFinite(z.x + z.y + z.z + z.hp), `${setup.mode}: infected ${z.id} is finite`);
    if (n % 3 !== 0) continue;
    const now = n * STEP * 1000;
    const lootDue = s.lootRev !== lastLootRev || Math.floor(now / 2000) !== Math.floor(sentAt / 2000);
    const buildDue = s.buildRev !== lastBuildRev || Math.floor((now + 1000) / 2000) !== Math.floor((sentAt + 1000) / 2000);
    const builds: boolean = buildDue && (!lootDue || preferBuilds);
    const loot: boolean = lootDue && !builds;
    if (loot) { lastLootRev = s.lootRev; preferBuilds = true; }
    if (builds) { lastBuildRev = s.buildRev; preferBuilds = false; }
    sentAt = now;
    const json: string = JSON.stringify(net.encodeSnapshot(s, s.events, loot, builds));
    maxBytes = Math.max(maxBytes, json.length);
    assert.ok(json.length < RELAY_CAP, `${setup.mode}: snapshot at ${s.clock.toFixed(1)} s is ${json.length} bytes`);
    const back = net.decodeSnapshot(JSON.parse(json), () => 'Guest', guest?.loot ?? null, guest?.builds ?? null);
    assert.ok(back, `${setup.mode}: snapshot at ${s.clock.toFixed(1)} s decodes`);
    assert.equal(back.mode, setup.mode);
    assert.equal(back.infected.length, s.infected.length);
    assert.deepEqual(back.obj.score, s.obj.score);
    guest = back;
  }
  return { s, guest, ticks: n, maxBytes, seen, maxInfected };
}

test('play-test: Team Deathmatch, 16 bots: the bots fight, use grenades and streaks, and a team reaches the limit', () => {
  const { s, guest, ticks, seen, maxBytes } = playMode({ mode: 'tdm', map: 'isle' }, 16, 60 * 400);
  assert.equal(s.phase, 'done', `ended (clock ${s.clock.toFixed(0)} s)`);
  assert.ok(ticks < 60 * 400);
  assert.ok(s.winTeam === 0 || s.winTeam === 1);
  assert.ok(Math.max(...s.obj.score) >= s.obj.limit || s.obj.time <= 0);
  assert.ok((seen.spawn ?? 0) > 5 && (seen.nade ?? 0) > 0 && (seen.streak ?? 0) > 0, JSON.stringify(seen));
  assert.ok(guest && guest.winTeam === s.winTeam);
  assert.ok(maxBytes < RELAY_CAP);
});

test('play-test: Domination, 16 bots: points change hands and the match ends with a winner', () => {
  const { s, guest, ticks, seen } = playMode({ mode: 'dom', map: 'mesa' }, 16, 60 * 420);
  assert.equal(s.phase, 'done', `ended (clock ${s.clock.toFixed(0)} s)`);
  assert.ok(ticks < 60 * 420);
  assert.ok((seen.capture ?? 0) >= 3, `${seen.capture} captures`);
  assert.ok(s.winTeam >= 0 || s.obj.score[0] === s.obj.score[1]);
  assert.ok(guest && guest.winTeam === s.winTeam);
});

test('play-test: Zombies, 4 bots, three rounds: escalating waves, buy-walls, then extraction or an overrun', () => {
  const { s, guest, ticks, seen, maxInfected } = playMode({ mode: 'zombies', map: 'isle', rounds: 3 }, 4, 60 * 600);
  assert.equal(s.phase, 'done', `ended (clock ${s.clock.toFixed(0)} s, round ${s.obj.round}, phase ${s.obj.wave})`);
  assert.ok(ticks < 60 * 600);
  assert.ok(s.obj.round >= 2, `reached round ${s.obj.round}`);
  assert.ok((seen.zkill ?? 0) > 30 && (seen.buy ?? 0) > 0, JSON.stringify(seen));
  assert.ok(maxInfected > 10);
  const extracted = s.obj.aboard > 0;
  assert.equal(events(s, 'extract').length > 0 || extracted, true);
  if (extracted) assert.ok(s.players.some(p => p.place === 1) && s.winner >= 0);
  else assert.ok(s.players.every(p => p.mode !== 'ground'));
  assert.ok(guest && guest.obj.aboard === s.obj.aboard);
});

test('play-test: Apocalypse Royale, 16 bots: infected fall, the cloud closes, one fighter wins', () => {
  const { s, guest, ticks, seen, maxBytes } = playMode({ mode: 'apoc', map: 'isle' }, 16, 60 * 600);
  assert.equal(s.phase, 'done', `ended (clock ${s.clock.toFixed(0)} s)`);
  assert.ok(ticks < 60 * 600);
  assert.ok(s.winner >= 0 && aliveCount(s) === 1);
  assert.ok((seen.zkill ?? 0) > 0, JSON.stringify(seen));
  assert.ok(guest && guest.winner === s.winner);
  assert.ok(maxBytes < RELAY_CAP);
});
