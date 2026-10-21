import { test } from 'node:test';
import assert from 'node:assert/strict';
import { decodeBuilds, decodeInput, decodeSnapshot, encodeInput, encodeSnapshot, FIGHTER_FIELDS, type ShooterSnapshot } from '../src/modules/rooms/games/shooter/net.ts';
import { createMatch, idleShooterInput, MAX_LOOT, MAX_ROCKETS, PICKAXE_KEY, SLOTS, tickMatch, type ShooterEvent, type ShooterInput, type ShooterState } from '../src/modules/rooms/games/shooter/sim.ts';
import { makeBuild, MAX_BUILDS, packBuild, TILES, type Build } from '../src/modules/rooms/games/shooter/builds.ts';
import { ITEM_IDS, ITEM_SLOTS } from '../src/modules/rooms/games/shooter/weapons.ts';
import { FIGHTER_IDS } from '../src/modules/rooms/games/shooter/catalog.ts';
import { shooterSeats } from '../src/modules/rooms/games/setup.ts';

const names = (slot: number) => `Human ${slot}`;
const wire = <T>(v: T): unknown => JSON.parse(JSON.stringify(v));

/** 2 humans + 14 bots, 70 s in: bots have landed, looted and started fighting. */
let cached: ShooterState | null = null;
function midMatch(): ShooterState {
  if (cached) return cached;
  const s = createMatch({ map: 'isle', difficulty: 'hard', seed: 41, seats: shooterSeats([{ slot: 0, name: 'Ann', pick: 'male-c' }, { slot: 3, name: 'Bo', pick: 'female-a' }], 14) });
  for (let i = 0; i < 60 * 70; i++) tickMatch(s, [{ ...idleShooterInput(), jump: true }]);
  s.rockets.push({ id: 5000, owner: 2, rarity: 3, x: 1.234, y: 20.5, z: -3.21, vx: 40, vy: 1, vz: -10, life: 3 });
  // A few structures in every shape: walls (one edited into a door), a floor, a ramp, all three materials.
  s.builds.push(
    makeBuild(9001, { kind: 0, gx: 40, gz: 50, level: 5, dir: 0 }, 0, 0, 1), makeBuild(9002, { kind: 0, gx: 40, gz: 50, level: 5, dir: 1 }, 2, 1),
    makeBuild(9003, { kind: 1, gx: 41, gz: 50, level: 9, dir: 0 }, 1, 0), makeBuild(9004, { kind: 2, gx: 42, gz: 50, level: 5, dir: 3 }, 0, 2),
  );
  s.builds[2].hp = 123.4;
  s.builds[2].grow = 0; // finished; the others are still going up
  // Every kind of hotbar item and materials on the first fighter.
  s.players[0].weapons = [{ w: 'rifle', rarity: 2, mag: 17 }, { w: 'bandage', rarity: 0, mag: 9 }, null, { w: 'shieldpot', rarity: 2, mag: 2 }, { w: 'shotgun', rarity: 4, mag: 3 }];
  s.players[0].mats = [120, 30, 7];
  cached = s;
  return s;
}
const fresh = (withLoot = true, withBuilds = false) => wire(encodeSnapshot(midMatch(), midMatch().events, withLoot, withBuilds)) as ShooterSnapshot;

test('encodeSnapshot/decodeSnapshot round-trip', () => {
  const s = midMatch();
  assert.ok(s.players.some(p => p.mode === 'ground') && s.loot.length > 0);
  const snap = encodeSnapshot(s, s.events, true, true);
  const back = decodeSnapshot(wire(snap), names);
  assert.ok(back);
  assert.ok(s.loot.some(l => l.kind === 'item') && s.loot.some(l => l.kind === 'weapon'), 'loot mix');
  assert.equal(back.map, s.map);
  assert.equal(back.difficulty, s.difficulty);
  assert.equal(back.phase, s.phase);
  assert.equal(back.winner, s.winner);
  assert.equal(back.lootRev, s.lootRev);
  assert.deepEqual(back.chests, s.chests);
  assert.equal(back.players.length, 16);
  s.players.forEach((p, i) => {
    const b = back.players[i];
    assert.equal(b.fighter, p.fighter);
    assert.equal(b.human, p.human);
    assert.equal(b.slot, p.slot);
    assert.equal(b.name, p.human ? `Human ${p.slot}` : p.name);
    assert.equal(b.mode, p.mode);
    for (const k of ['x', 'y', 'z', 'vx', 'vy', 'vz'] as const) assert.ok(Math.abs(b[k] - p[k]) <= 0.005 + 1e-9, `${i}.${k}`);
    assert.equal(b.hp, Math.round(p.hp));
    assert.equal(b.shield, Math.round(p.shield));
    assert.equal(b.kills, p.kills);
    assert.equal(b.place, p.place);
    assert.equal(b.current, p.current);
    assert.deepEqual(b.weapons, p.weapons);
    assert.deepEqual(b.ammo, p.ammo);
    assert.deepEqual(b.mats, p.mats);
    assert.deepEqual([b.building, b.piece, b.mat, b.swimming], [p.building, p.piece, p.mat, p.swimming]);
    assert.ok(Math.abs(b.useT - p.useT) <= 0.005 + 1e-9 && Math.abs(b.swingT - p.swingT) <= 0.005 + 1e-9);
    assert.equal(b.killedBy, p.killedBy);
    assert.equal(b.grounded, p.grounded);
  });
  assert.equal(back.loot.length, s.loot.length);
  back.loot.forEach((l, i) => {
    const o = s.loot[i];
    assert.deepEqual([l.id, l.kind, l.weapon, l.rarity, l.ammo, l.mat, l.amount], [o.id, o.kind, o.weapon, o.rarity, o.ammo, o.mat, o.amount]);
    assert.ok(Math.abs(l.x - o.x) <= 0.05 + 1e-9 && Math.abs(l.z - o.z) <= 0.05 + 1e-9);
  });
  assert.deepEqual(back.rockets.map(r => r.id), s.rockets.map(r => r.id));
  // Structures: same slots, edits, materials, rounded hp and identical colliders.
  assert.equal(back.buildRev, s.buildRev);
  assert.equal(back.builds.length, s.builds.length);
  back.builds.forEach((b, i) => {
    const o = s.builds[i];
    assert.deepEqual([b.kind, b.gx, b.gz, b.level, b.dir, b.variant, b.mat, b.hp], [o.kind, o.gx, o.gz, o.level, o.dir, o.variant, o.mat, Math.round(o.hp)]);
    assert.equal(b.grow > 0, o.grow > 0, 'under-construction flag');
    assert.deepEqual(b.boxes, o.boxes);
    assert.deepEqual(b.aabb, o.aabb);
  });
  assert.equal(back.storm.stage, s.storm.stage);
  assert.ok(Math.abs(back.storm.r - s.storm.r) <= 0.05 + 1e-9);
  assert.equal(back.storm.shrinking, s.storm.shrinking);
  const sent = s.events.slice(-24);
  assert.deepEqual(back.events.map(e => [e.id, e.type, e.player, e.target]), sent.map(e => [e.id, e.type, e.player, e.target]));
  // Re-encoding the decoded state reproduces the same payload.
  assert.deepEqual(wire(encodeSnapshot(back, back.events, true, true)), wire({ ...snap, b: s.builds.flatMap(b => [packBuild(b), Math.round(b.hp) * (b.grow > 0 ? -1 : 1)]) }));
});

test('snapshots without structures reuse the previous list; loot and structures travel separately', () => {
  const s = midMatch();
  const plain = fresh(true, false);
  assert.equal('b' in plain, false);
  const withBuilds = decodeSnapshot(fresh(false, true), names)!;
  assert.equal(withBuilds.builds.length, s.builds.length);
  assert.equal(decodeSnapshot(plain, names, null, withBuilds.builds)!.builds, withBuilds.builds, 'same array instance');
  assert.equal(decodeSnapshot(plain, names)!.builds.length, 0);
  assert.equal(decodeSnapshot({ ...fresh(false, true), b: [] }, names, null, withBuilds.builds)!.builds.length, 0);
  // Defaults: loot yes, structures no.
  const def = wire(encodeSnapshot(s, s.events)) as ShooterSnapshot;
  assert.ok('l' in def && !('b' in def));
});

test('packBuild/unpackBuild round-trip every shape at the grid corners', () => {
  for (const [gx, gz, level] of [[0, 0, -16], [TILES - 1, TILES - 1, 111], [37, 81, 0]]) {
    for (const [kind, dir, variant] of [[0, 0, 0], [0, 1, 2], [0, 0, 1], [1, 0, 0], [2, 0, 0], [2, 3, 0]] as const) for (const mat of [0, 1, 2] as const) {
      const b = makeBuild(1, { kind, gx, gz, level, dir }, mat, 0, variant);
      const back = decodeBuilds([packBuild(b), 50])!;
      assert.equal(back.length, 1);
      assert.deepEqual([back[0].kind, back[0].gx, back[0].gz, back[0].level, back[0].dir, back[0].variant, back[0].mat], [kind, gx, gz, level, dir, variant, mat]);
      assert.ok(packBuild(b) < 2 ** 31);
    }
  }
  for (const junk of [[1], [-1, 5], [0.5, 5], [2 ** 31, 5], [0, 0], [0, NaN], 'x', null, Array(MAX_BUILDS * 2 + 2).fill(1)]) assert.equal(decodeBuilds(junk), null, JSON.stringify(junk)?.slice(0, 40));
  // Impossible combinations: a floor facing a direction, an edited ramp, a wall on a -z edge.
  assert.equal(decodeBuilds([packBuild({ kind: 1, gx: 1, gz: 1, level: 0, dir: 2, mat: 0, variant: 0 }), 5]), null);
  assert.equal(decodeBuilds([packBuild({ kind: 2, gx: 1, gz: 1, level: 0, dir: 2, mat: 0, variant: 1 }), 5]), null);
  assert.equal(decodeBuilds([packBuild({ kind: 0, gx: 1, gz: 1, level: 0, dir: 3, mat: 0, variant: 0 }), 5]), null);
});

test('snapshots without loot reuse the previous loot list', () => {
  const s = midMatch();
  const snap = fresh(false);
  assert.equal('l' in snap, false);
  const withLoot = decodeSnapshot(fresh(true), names)!;
  const reused = decodeSnapshot(snap, names, withLoot.loot);
  assert.ok(reused);
  assert.equal(reused.loot, withLoot.loot, 'same array instance');
  assert.equal(decodeSnapshot(snap, names)!.loot.length, 0);
  // An explicit empty loot list wins over prevLoot.
  assert.equal(decodeSnapshot({ ...fresh(true), l: [] }, names, withLoot.loot)!.loot.length, 0);
  assert.ok(JSON.stringify(snap).length < JSON.stringify(fresh(true)).length);
  assert.equal(snap.f[0].length, FIGHTER_FIELDS);
  assert.ok(s.loot.length > 0);
});

test('decodeSnapshot rejects malformed payloads', () => {
  const row = (s: ShooterSnapshot, i: number, v: number) => { const f = s.f.map(r => [...r]); f[0][i] = v; return { ...s, f }; };
  const bad: [string, (s: ShooterSnapshot) => unknown][] = [
    ['version', s => ({ ...s, v: 2 })],
    ['missing buildRev', s => ({ ...s, br: undefined })],
    ['bad structure code', s => ({ ...s, b: [-5, 100] })],
    ['structure hp zero', s => ({ ...s, b: [packBuild({ kind: 1, gx: 3, gz: 3, level: 0, dir: 0, mat: 0, variant: 0 }), 0] })],
    ['odd structure list', s => ({ ...s, b: [1, 2, 3] })],
    ['structure hp over the material max', s => ({ ...s, b: [packBuild({ kind: 1, gx: 3, gz: 3, level: 0, dir: 0, mat: 0, variant: 0 }), 151] })],
    ['fractional structure hp', s => ({ ...s, b: [packBuild({ kind: 1, gx: 3, gz: 3, level: 0, dir: 0, mat: 0, variant: 0 }), -12.5] })],
    ['mats over the cap', s => row(s, FIGHTER_FIELDS - 8, 501)],
    ['fractional mats', s => row(s, FIGHTER_FIELDS - 7, 1.5)],
    ['bad build piece', s => row(s, FIGHTER_FIELDS - 5, 3)],
    ['bad build material', s => row(s, FIGHTER_FIELDS - 4, -1)],
    ['negative revive progress', s => row(s, FIGHTER_FIELDS - 1, -1)],
    ['revive progress past the revive time', s => row(s, FIGHTER_FIELDS - 1, 99)],
    ['bad team size', s => ({ ...s, sq: 3 })],
    ['missing winning team', s => ({ ...s, wt: undefined })],
    ['team out of range', s => ({ ...s, q: [[0, 0, -1, 16], ...s.q.slice(1)] })],
    ['short q row (no team)', s => ({ ...s, q: [s.q[0].slice(0, 3), ...s.q.slice(1)] })],
    ['pads not triples', s => ({ ...s, pd: [1, 2] })],
    ['too many pads', s => ({ ...s, pd: Array(17 * 3).fill(1) })],
    ['bad supply drop', s => ({ ...s, dr: [[1, 2, 3, 4, 5, 2]] })],
    ['too many supply drops', s => ({ ...s, dr: Array.from({ length: 5 }, (_, i) => [i, 0, 90, 0, 1, 0]) })],
    ['felled trunk out of range', s => ({ ...s, fl: [999999] })],
    ['fractional felled trunk', s => ({ ...s, fl: [1.5] })],
    ['felled not an array', s => ({ ...s, fl: 'x' })],
    ['map', s => ({ ...s, m: 'moon' })],
    ['phase', s => ({ ...s, p: 3 })],
    ['difficulty', s => ({ ...s, d: 5 })],
    ['NaN clock', s => ({ ...s, c: NaN })],
    ['missing lootRev', s => ({ ...s, lr: undefined })],
    ['short bus', s => ({ ...s, bus: s.bus.slice(1) })],
    ['short storm', s => ({ ...s, st: s.st.slice(1) })],
    ['chests not binary', s => ({ ...s, ch: s.ch.replace(/.$/, '2') })],
    ['chest count', s => ({ ...s, ch: s.ch + '0' })],
    ['no fighters', s => ({ ...s, f: [], q: [], n: [] })],
    ['too many fighters', s => ({ ...s, f: [...s.f, s.f[0]], q: [...s.q, s.q[0]], n: [...s.n, s.n[0]] })],
    ['q mismatch', s => ({ ...s, q: s.q.slice(1) })],
    ['n mismatch', s => ({ ...s, n: s.n.slice(1) })],
    ['name too long', s => ({ ...s, n: ['x'.repeat(41), ...s.n.slice(1)] })],
    ['name not a string', s => ({ ...s, n: [7, ...s.n.slice(1)] })],
    ['short fighter row', s => ({ ...s, f: [s.f[0].slice(1), ...s.f.slice(1)] })],
    ['NaN in fighter row', s => row(s, 0, NaN)],
    ['null in fighter row (JSON NaN)', s => wire(row(s, 0, NaN))],
    ['bad mode', s => row(s, 8, 9)],
    ['knocked with impossible health', s => row(row(s, 8, 5), 9, 150)],
    ['bad current slot', s => row(s, 13, SLOTS + 1)],
    ['fractional current slot', s => row(s, 13, 1.5)],
    ['bad fighter id', s => ({ ...s, q: [[99, 0, -1, 0], ...s.q.slice(1)] })],
    ['too many rockets', s => ({ ...s, r: Array.from({ length: MAX_ROCKETS + 1 }, (_, i) => [i, 0, 0, 0, 0, 0, 0]) })],
    ['short rocket', s => ({ ...s, r: [[1, 2, 3]] })],
    ['too many events', s => ({ ...s, e: Array.from({ length: 65 }, (_, i) => [i, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]) })],
    ['bad event type', s => ({ ...s, e: [[1, 99, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]] })],
    ['too much loot', s => ({ ...s, l: Array.from({ length: MAX_LOOT + 1 }, (_, i) => [i, 1, 10, 0, 0, 0]) })],
    ['loot not an array', s => ({ ...s, l: 'x' })],
    ['loot code too big', s => ({ ...s, l: [[1, 6 * ITEM_SLOTS * 5 * 6, 10, 0, 0, 0]] })],
    ['loot code fractional', s => ({ ...s, l: [[1, 2.5, 10, 0, 0, 0]] })],
    // code = ((kind * ITEM_SLOTS + item + 1) * 5 + rarity) * 6 + sub + 1; ITEM_IDS: six weapons, then the consumables.
    ['weapon loot carrying a consumable', s => ({ ...s, l: [[1, ((0 * ITEM_SLOTS + 7 + 1) * 5 + 0) * 6, 10, 0, 0, 0]] })],
    ['item loot carrying a weapon', s => ({ ...s, l: [[1, ((4 * ITEM_SLOTS + 1 + 1) * 5 + 0) * 6, 10, 0, 0, 0]] })],
    ['ammo loot carrying an item', s => ({ ...s, l: [[1, ((1 * ITEM_SLOTS + 1 + 1) * 5 + 0) * 6 + 1, 10, 0, 0, 0]] })],
    ['short loot', s => ({ ...s, l: [[1, 0, 0, 0]] })],
  ];
  assert.ok(decodeSnapshot(fresh(), names), 'baseline decodes');
  for (const [label, mutate] of bad) assert.equal(decodeSnapshot(mutate(fresh()), names), null, label);
  for (const junk of [null, undefined, 1, 'x', [], {}, { v: 1, m: 'isle' }]) assert.equal(decodeSnapshot(junk, names), null, String(junk));
});

test('a full 16-player snapshot with loot stays under 16 KiB, even at the caps', () => {
  const s = midMatch();
  const json = JSON.stringify(encodeSnapshot(s, s.events));
  assert.ok(json.length < 16 * 1024, `snapshot ${json.length} bytes`);
  // Worst case: MAX_LOOT items, MAX_ROCKETS rockets, 24 events with long coordinates.
  const big: ShooterState = {
    ...s,
    loot: Array.from({ length: MAX_LOOT }, (_, i) => ({ id: 100000 + i, kind: 'weapon' as const, weapon: 'sniper' as const, rarity: 4 as const, ammo: null, mat: null, amount: 30, x: -123.45, y: -12.34, z: 156.78 })),
    rockets: Array.from({ length: MAX_ROCKETS }, (_, i) => ({ id: 200000 + i, owner: 15, rarity: 4 as const, x: -123.456, y: 12.345, z: 156.789, vx: -47.123, vy: -1.234, vz: 12.345, life: 4 })),
    builds: Array.from({ length: MAX_BUILDS }, (_, i): Build => ({ ...makeBuild(i, { kind: 0, gx: TILES - 1 - (i % 50), gz: TILES - 1, level: 100 + (i % 10), dir: 1 }, 2, 15, 2), hp: 449.6 })),
    pads: Array.from({ length: 16 }, (_, i) => ({ id: i, x: -123.45, y: 12.34, z: 156.78 })),
    drops: Array.from({ length: 4 }, (_, i) => ({ id: 300000 + i, x: -123.45, y: 89.99, z: 156.78, ground: 12.34, landed: i % 2 === 0 })),
    felled: Array.from({ length: 128 }, (_, i) => 200 + i),
    players: s.players.map(p => ({ ...p, revive: 3.21, x: -123.456, y: 12.345, z: 156.789, vx: -12.345, vy: -12.345, vz: -12.345, yaw: -3.141, pitch: -1.234, damage: 12345, aliveTime: 456.7, reload: 2.345, useT: 6.789, swingT: 0.345, mats: [500, 500, 500] as [number, number, number],
      weapons: Array.from({ length: SLOTS }, () => ({ w: ITEM_IDS.at(-1)!, rarity: 4 as const, mag: 15 })), ammo: { light: 240, medium: 240, shells: 40, heavy: 24, rockets: 6 } })),
  };
  const evs: ShooterEvent[] = Array.from({ length: 64 }, (_, i) => ({ id: 900000 + i, type: 'hit', player: 15, target: 14, value: 123, flag: 3, x: -123.456, y: 12.345, z: 156.789, x2: -123.456, y2: 12.345, z2: 156.789 }));
  // The two bulky lists never ride together; each worst case fits on its own.
  const worstLoot = JSON.stringify(encodeSnapshot(big, evs, true, false));
  assert.ok(worstLoot.length < 16 * 1024, `worst-case loot snapshot ${worstLoot.length} bytes`);
  const worstBuilds = JSON.stringify(encodeSnapshot(big, evs, false, true));
  assert.ok(worstBuilds.length < 16 * 1024, `worst-case structures snapshot ${worstBuilds.length} bytes`);
  assert.ok(decodeSnapshot(JSON.parse(worstLoot), names), 'worst loot case decodes');
  assert.equal(decodeSnapshot(JSON.parse(worstBuilds), names)?.builds.length, MAX_BUILDS, 'worst structures case decodes');
});

test('encodeInput/decodeInput round-trip', () => {
  const input: ShooterInput = { mx: -0.71, mz: 1, yaw: -2.345, pitch: 0.4, fire: true, aim: false, jump: true, sprint: false, reload: true, interact: false, slot: PICKAXE_KEY, build: true, piece: 2, mat: 1, edit: true };
  for (const fighter of FIGHTER_IDS) {
    const w = encodeInput(input, fighter);
    assert.ok(JSON.stringify(w).length < 1024);
    assert.deepEqual(decodeInput(wire(w)), { input, fighter });
  }
  for (let flags = 0; flags < 256; flags++) {
    const i: ShooterInput = { ...idleShooterInput(), fire: (flags & 1) > 0, aim: (flags & 2) > 0, jump: (flags & 4) > 0, sprint: (flags & 8) > 0, reload: (flags & 16) > 0, interact: (flags & 32) > 0, edit: (flags & 64) > 0, build: (flags & 128) > 0, piece: flags % 3, mat: (flags >> 2) % 3 };
    assert.deepEqual(decodeInput(wire(encodeInput(i, 'male-a')))?.input, i);
  }
});

test('decodeInput rejects malformed input', () => {
  const ok = [0, 0, 0, 0, 0, 0, 0, 0, 0];
  const with_ = (i: number, v: unknown) => ({ k: ok.map((x, j) => (j === i ? v : x)) });
  assert.ok(decodeInput({ k: ok }));
  for (const junk of [
    null, undefined, 'k', 3, [], {}, { k: ok.slice(1) }, { k: [...ok, 0] }, { k: 'abcdefghi' },
    with_(0, 1.01), with_(1, -2), with_(2, 7.5), with_(3, 1.7), with_(4, 256), with_(4, -1), with_(4, 1.5),
    with_(5, 1.5), with_(5, -1), with_(5, PICKAXE_KEY + 1), with_(6, 12), with_(6, -1),
    with_(7, 3), with_(7, -1), with_(7, 0.5), with_(8, 3), with_(8, -1),
    with_(0, NaN), with_(2, Infinity), with_(3, null), with_(1, '0'),
  ]) assert.equal(decodeInput(junk), null, JSON.stringify(junk));
});
