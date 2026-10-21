import { test } from 'node:test';
import assert from 'node:assert/strict';
import { decodeInput, decodeSnapshot, encodeInput, encodeSnapshot, RACER_FIELDS, type KartSnapshot } from '../src/modules/rooms/games/kart/net.ts';
import { createRace, MAX_HAZARDS, MAX_SHELLS, tick, type KartState } from '../src/modules/rooms/games/kart/sim.ts';
import { RACER_IDS, type RacerId } from '../src/modules/rooms/games/kart/roster.ts';
import { trackFor } from '../src/modules/rooms/games/kart/tracks.ts';
import { kartSeats } from '../src/modules/rooms/games/setup.ts';

const names = (slot: number, racer: RacerId) => `${racer}#${slot}`;
const wire = <T>(v: T): unknown => JSON.parse(JSON.stringify(v));

/** A mid-race state with items, hazards and shells in play (2 humans + 6 CPUs). */
let cached: KartState | null = null;
function midRace(): KartState {
  if (cached) return cached;
  const s = createRace('canyon', kartSeats([{ slot: 2, name: 'Ann', pick: 'kai' }, { slot: 4, name: 'Bo', pick: 'dex' }], true), 'hard', 77);
  for (let i = 0; i < 60 * 50; i++) tick(s, [undefined, undefined, undefined, undefined, undefined, undefined, { throttle: 1, steer: 0.1, drift: false, item: false, respawn: false }]);
  s.racers[0].item = 'triple'; s.racers[0].itemCount = 2;
  s.racers[1].finish = 123.4567; s.racers[1].lapTimes = [40.1234, 41.5, 41.8333];
  s.hazards.push({ id: 900, x: 1.234, y: 2, z: 3.456, owner: 3 });
  s.shells.push({ id: 901, x: 4, y: 5, z: 6, yaw: 0.12345, target: 2, owner: 5, life: 3, hint: 10, seg: 0 });
  cached = s;
  return s;
}

const fresh = () => JSON.parse(JSON.stringify(encodeSnapshot(midRace(), midRace().events))) as KartSnapshot;

test('encodeSnapshot/decodeSnapshot round-trip', () => {
  const s = midRace();
  const snap = encodeSnapshot(s, s.events);
  const back = decodeSnapshot(wire(snap), names);
  assert.ok(back, 'decodes');
  assert.equal(back.track, s.track);
  assert.equal(back.difficulty, s.difficulty);
  assert.equal(back.phase, s.phase);
  assert.ok(Math.abs(back.raceTime - s.raceTime) < 1e-3);
  assert.equal(back.racers.length, s.racers.length);
  s.racers.forEach((r, i) => {
    const b = back.racers[i];
    assert.equal(b.racer, r.racer);
    assert.equal(b.human, r.human);
    assert.equal(b.slot, r.slot);
    assert.equal(b.name, `${r.racer}#${r.slot}`);
    for (const k of ['x', 'y', 'z', 'speed', 'travel', 'lateral', 'boost', 'spin', 'star'] as const) assert.ok(Math.abs(b[k] - r[k]) <= 0.005 + 1e-9, `racer ${i} ${k}`);
    assert.ok(Math.abs(b.yaw - r.yaw) <= 0.0005 + 1e-9);
    assert.equal(b.item, r.item);
    assert.equal(b.itemCount, r.itemCount);
    assert.equal(b.lap, r.lap);
    assert.equal(b.place, r.place);
    assert.equal(b.grounded, r.grounded);
    assert.equal(b.surface, r.surface);
    assert.equal(b.hint, r.hint);
    assert.equal(b.drift, r.drift);
    assert.equal(b.driftLevel, r.driftLevel);
    assert.equal(b.finish === null, r.finish === null);
    if (r.finish !== null) assert.ok(Math.abs(b.finish! - r.finish) < 1e-3);
    assert.equal(b.lapTimes.length, r.lapTimes.length);
  });
  assert.deepEqual(back.boxes.map(b => b.timer), s.boxes.map(b => Math.round(b.timer * 100) / 100));
  assert.deepEqual(back.hazards.map(h => h.id), s.hazards.map(h => h.id));
  assert.deepEqual(back.shells.map(h => [h.id, h.target, h.owner]), s.shells.map(h => [h.id, h.target, h.owner]));
  const sent = s.events.slice(-24);
  assert.deepEqual(back.events.map(e => [e.id, e.type, e.racer]), sent.map(e => [e.id, e.type, e.racer]));
  assert.equal(back.eventSeq, sent.at(-1)?.id ?? 0);
  // Re-encoding the decoded state reproduces the same payload.
  assert.deepEqual(wire(encodeSnapshot(back, back.events)), wire(snap));
});

test('decodeSnapshot rejects malformed payloads', () => {
  const bad: [string, (s: KartSnapshot) => unknown][] = [
    ['old version', s => ({ ...s, v: 1 })],
    ['future version', s => ({ ...s, v: 3 })],
    ['track', s => ({ ...s, tr: 'moon' })],
    ['phase', s => ({ ...s, p: 9 })],
    ['difficulty', s => ({ ...s, d: -1 })],
    ['NaN clock', s => ({ ...s, c: NaN })],
    ['string time', s => ({ ...s, t: '1' })],
    ['no racers', s => ({ ...s, r: [], q: [], lt: [] })],
    ['too many racers', s => ({ ...s, r: [...s.r, s.r[0]], q: [...s.q, s.q[0]], lt: [...s.lt, s.lt[0]] })],
    ['q length mismatch', s => ({ ...s, q: s.q.slice(1) })],
    ['lt length mismatch', s => ({ ...s, lt: s.lt.slice(1) })],
    ['short racer row', s => ({ ...s, r: [s.r[0].slice(1), ...s.r.slice(1)] })],
    ['NaN in racer row', s => ({ ...s, r: [[NaN, ...s.r[0].slice(1)], ...s.r.slice(1)] })],
    ['Infinity in racer row', s => ({ ...s, r: [[Infinity, ...s.r[0].slice(1)], ...s.r.slice(1)] })],
    ['null in racer row (JSON NaN)', s => wire({ ...s, r: [[NaN, ...s.r[0].slice(1)], ...s.r.slice(1)] })],
    ['bad racer id', s => ({ ...s, q: [[99, 0, -1], ...s.q.slice(1)] })],
    ['item out of range', s => { const r = s.r.map(x => [...x]); r[0][17] = 6; return { ...s, r }; }],
    ['place zero', s => { const r = s.r.map(x => [...x]); r[0][22] = 0; return { ...s, r }; }],
    ['surface out of range', s => { const r = s.r.map(x => [...x]); r[0][27] = 3; return { ...s, r }; }],
    ['hint out of range', s => { const r = s.r.map(x => [...x]); r[0][28] = trackFor('canyon').count; return { ...s, r }; }],
    ['path out of range', s => { const r = s.r.map(x => [...x]); r[0][30] = 1; return { ...s, r }; }],
    ['negative path', s => { const r = s.r.map(x => [...x]); r[0][30] = -1; return { ...s, r }; }],
    ['fractional path', s => { const r = s.r.map(x => [...x]); r[0][30] = 0.5; return { ...s, r }; }],
    ['long racer row', s => ({ ...s, r: [[...s.r[0], 0], ...s.r.slice(1)] })],
    ['too many lap times', s => ({ ...s, lt: [[1, 2, 3, 4], ...s.lt.slice(1)] })],
    ['box count', s => ({ ...s, b: s.b.slice(1) })],
    ['NaN box timer', s => ({ ...s, b: [NaN, ...s.b.slice(1)] })],
    ['too many hazards', s => ({ ...s, h: Array.from({ length: MAX_HAZARDS + 1 }, (_, i) => [i, 0, 0, 0, 0]) })],
    ['short hazard', s => ({ ...s, h: [[1, 2, 3]] })],
    ['too many shells', s => ({ ...s, s: Array.from({ length: MAX_SHELLS + 1 }, (_, i) => [i, 0, 0, 0, 0, 0, 0]) })],
    ['short shell', s => ({ ...s, s: [[1, 2, 3, 4, 5, 6]] })],
    ['too many events', s => ({ ...s, e: Array.from({ length: 49 }, (_, i) => [i, 0, 0, 0, 0, 0, 0]) })],
    ['event racer out of range', s => ({ ...s, e: [[1, 0, 8, 0, 0, 0, 0]] })],
    ['event type out of range', s => ({ ...s, e: [[1, 99, 0, 0, 0, 0, 0]] })],
    ['racers not an array', s => ({ ...s, r: 'x' })],
  ];
  assert.ok(decodeSnapshot(fresh(), names), 'baseline decodes');
  for (const [label, mutate] of bad) assert.equal(decodeSnapshot(mutate(fresh()), names), null, label);
  for (const junk of [null, undefined, 0, 'snapshot', [], {}, { v: 1 }]) assert.equal(decodeSnapshot(junk, names), null, String(junk));
});

test('snapshot of 8 racers + 24 events + max hazards/shells stays under 16 KiB', () => {
  const s = midRace();
  const big: KartState = {
    ...s,
    hazards: Array.from({ length: MAX_HAZARDS }, (_, i) => ({ id: 1000 + i, x: -123.456, y: 12.345, z: 234.567, owner: 7 })),
    shells: Array.from({ length: MAX_SHELLS }, (_, i) => ({ id: 2000 + i, x: -123.456, y: 12.345, z: 234.567, yaw: -3.14159, target: 7, owner: 6, life: 5, hint: 400, seg: 0 })),
  };
  const evs = Array.from({ length: 48 }, (_, i) => ({ id: 100000 + i, type: 'explode' as const, racer: 7, x: -123.456, y: 12.345, z: 234.567, value: 123.456 }));
  const json = JSON.stringify(encodeSnapshot(big, evs));
  assert.equal(s.racers.length, 8);
  assert.ok(json.length < 16 * 1024, `snapshot is ${json.length} bytes`);
  assert.ok(decodeSnapshot(JSON.parse(json), names), 'the worst case still decodes');
});

test('encodeInput/decodeInput round-trip', () => {
  for (const racer of RACER_IDS) {
    const input = { throttle: -0.5, steer: 0.33, drift: true, item: false, respawn: true };
    const w = encodeInput(input, racer);
    assert.ok(JSON.stringify(w).length < 1024);
    assert.deepEqual(decodeInput(wire(w)), { input, racer });
  }
  for (let flags = 0; flags < 8; flags++) {
    const input = { throttle: 1, steer: -1, drift: (flags & 1) > 0, item: (flags & 2) > 0, respawn: (flags & 4) > 0 };
    assert.deepEqual(decodeInput(wire(encodeInput(input, 'dex')))?.input, input);
  }
  // Values are rounded to centi-units on the wire.
  assert.equal(decodeInput(encodeInput({ throttle: 0.123456, steer: 0, drift: false, item: false, respawn: false }, 'oobi'))?.input.throttle, 0.12);
});

test('decodeInput rejects malformed input', () => {
  for (const junk of [
    null, undefined, 'k', 42, [], {}, { k: 'abcd' }, { k: [1, 0, 0] }, { k: [1, 0, 0, 0, 0] },
    { k: [1.5, 0, 0, 0] }, { k: [0, -1.01, 0, 0] }, { k: [0, 0, 8, 0] }, { k: [0, 0, -1, 0] },
    { k: [0, 0, 0, 8] }, { k: [0, 0, 0, -1] }, { k: [NaN, 0, 0, 0] }, { k: [0, Infinity, 0, 0] }, { k: [0, 0, 0, null] }, { k: ['1', 0, 0, 0] },
  ]) assert.equal(decodeInput(junk), null, JSON.stringify(junk));
});

test('branching tracks: path, glider and underwater state survive the wire; hints are checked per path', () => {
  const s = createRace('lagoon', kartSeats([{ slot: 1, name: 'Ann', pick: 'rosa' }], true), 'hard', 5);
  for (let i = 0; i < 60 * 8; i++) tick(s, []);
  const t = trackFor('lagoon'), branch = t.paths[1];
  const [a, b, c] = s.racers;
  a.seg = 1; a.hint = branch.count - 1; a.segS = 100;
  b.glide = true; b.grounded = false;
  c.under = true;
  s.events.push({ id: ++s.eventSeq, type: 'splash', racer: 2, x: 1, y: 2, z: 3, value: 2 }, { id: ++s.eventSeq, type: 'glide', racer: 1, x: 1, y: 2, z: 3 }, { id: ++s.eventSeq, type: 'crush', racer: 0, x: 1, y: 2, z: 3 });
  const snap = wire(encodeSnapshot(s, s.events)) as KartSnapshot;
  assert.equal(snap.v, 2);
  assert.ok(snap.r.every(row => row.length === RACER_FIELDS));
  assert.equal(snap.b.length, s.boxes.length, 'boxes on every route');
  const back = decodeSnapshot(snap, names);
  assert.ok(back);
  assert.deepEqual(back.racers.map(r => r.seg), s.racers.map(r => r.seg));
  assert.deepEqual(back.racers.map(r => [r.glide, r.under, r.grounded]), s.racers.map(r => [r.glide, r.under, r.grounded]));
  assert.equal(back.racers[0].hint, branch.count - 1);
  assert.deepEqual(back.events.slice(-3).map(e => [e.type, e.value]), [['splash', 2], ['glide', 0], ['crush', 0]]);
  assert.deepEqual(wire(encodeSnapshot(back, back.events)), snap, 'stable re-encode');
  // A branch hint beyond that branch's samples (but valid on the main loop) is rejected.
  const bad = structuredClone(snap);
  bad.r[0][28] = branch.count;
  assert.ok(branch.count < t.count);
  assert.equal(decodeSnapshot(bad, names), null);
  // So is a path index this track does not have.
  const bad2 = structuredClone(snap);
  bad2.r[0][30] = t.paths.length;
  assert.equal(decodeSnapshot(bad2, names), null);
  // A box list sized for the main loop only no longer fits.
  assert.equal(decodeSnapshot({ ...snap, b: snap.b.slice(0, t.def.itemRows.length * 5) }, names), null);
});
