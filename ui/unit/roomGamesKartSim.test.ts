import { test } from 'node:test';
import assert from 'node:assert/strict';
import { clamp, Rng, wrapAngle } from '../src/modules/rooms/games/common/math.ts';
import {
  advance, circ, CRUSHER_HALF, drawLift, F_RAILS, FORK_ZONE, F_UNDER, inLaunchZone, nearest, newNear, obstacleAt, pointAt, progressOf, ROLLER_RADIUS, RUN_UP, sampleAt, TRACK_IDS, trackFor, type TrackId,
} from '../src/modules/rooms/games/kart/tracks.ts';
import {
  BURNOUT_SECONDS, COIN_LOSS, COIN_RESPAWN, COUNTDOWN_SECONDS, createRace, DRIFT_LEVELS, GHOST_SECONDS, idleInput, INTRO_SECONDS, itemBoxes, LAPS, MAX_COINS, MAX_RACERS, pickRoute, rank, RESPAWN_BOOST,
  sanitize, spinOut, standings, STEP, stepRace, STUCK_SECONDS, tick, TURBO_POWER, TURBO_SECONDS, type Difficulty, type KartEventType, type KartInput, type KartSeat, type KartState, type Racer,
} from '../src/modules/rooms/games/kart/sim.ts';
import { cpuInput } from '../src/modules/rooms/games/kart/cpu.ts';
import { kartSeats } from '../src/modules/rooms/games/setup.ts';

type Inputs = (KartInput | undefined)[];
const human = (slot: number, racer: KartSeat['racer'] = 'oobi'): KartSeat => ({ racer, human: true, slot, name: `P${slot}` });
const run = (state: KartState, seconds: number, inputs: Inputs | ((s: KartState) => Inputs) = []): void => {
  for (let i = 0, n = Math.round(seconds / STEP); i < n; i++) tick(state, typeof inputs === 'function' ? inputs(state) : inputs);
};
/** Ticks through intro + countdown with no input. */
const toRace = (state: KartState): void => {
  let n = 0;
  while (state.phase !== 'race' && n++ < 1000) tick(state, []);
  assert.equal(state.phase, 'race');
};
const humansRace = (count: number, track: TrackId = 'meadow'): KartState => {
  const s = createRace(track, Array.from({ length: count }, (_, i) => human(i)), 'normal', 99);
  toRace(s);
  return s;
};
/** Teleports a racer onto the track at distance `s` (lap distance) and lateral offset. */
function put(state: KartState, r: Racer, s: number, lateral: number, speed = 0, yawOffset = 0): void { putOn(state, r, 0, s, lateral, speed, yawOffset); }
/** Teleports a racer onto path `seg` (0 = main loop, k = branch k) at local distance `s`. */
function putOn(state: KartState, r: Racer, seg: number, s: number, lateral: number, speed = 0, yawOffset = 0): void {
  const track = trackFor(state.track), path = track.paths[seg];
  const pt = pointAt(path, s, lateral);
  const near = nearest(path, pt[0], pt[2], sampleAt(path, s), newNear());
  r.x = pt[0]; r.z = pt[2]; r.y = track.ground(pt[0], pt[2], near); r.yaw = pt[3] + yawOffset;
  r.speed = speed; r.vx = Math.sin(r.yaw) * speed; r.vz = Math.cos(r.yaw) * speed; r.vy = 0; r.grounded = true; r.glide = false; r.under = false;
  r.seg = seg; r.segS = near.s; r.safeSeg = seg; r.safeS = near.s;
  r.hint = -1; r.lastS = progressOf(track, seg, near.s); r.travel = r.lastS; r.spin = 0; r.boost = 0; r.drift = 0; r.driftCharge = 0; r.driftLevel = 0;
}
/** Autopilot for a human racer: steer for the road ahead along `route` (a CPU-style branch bitmask). */
function pilot(state: KartState, r: Racer, route: number, extra: Partial<KartInput> = {}): KartInput {
  const track = trackFor(state.track);
  const spot = advance(track, r.seg, r.segS, 7 + Math.max(0, r.speed) * 0.3, route, { seg: 0, s: 0 });
  const p = pointAt(track.paths[spot.seg], spot.s, 0);
  const angle = wrapAngle(Math.atan2(p[0] - r.x, p[2] - r.z) - r.yaw);
  return { ...idleInput(), throttle: 1, steer: clamp(-angle * 2.4, -1, 1), ...extra };
}
const events = (state: KartState, type: KartEventType, from = 0) => state.events.filter(e => e.id > from && e.type === type);
const press = (k: Partial<KartInput>): KartInput => ({ ...idleInput(), ...k });

test('createRace: humans start mid-grid with CPU rivals ahead and behind', () => {
  const seats = kartSeats([{ slot: 0, name: 'Ann', pick: 'kai' }, { slot: 1, name: 'Bo', pick: 'rosa' }], true);
  const s = createRace('meadow', seats, 'normal', 1);
  assert.equal(s.racers.length, 8);
  assert.equal(s.phase, 'intro');
  assert.equal(s.laps, LAPS);
  assert.deepEqual(s.racers.map(r => r.human), [false, false, false, true, true, false, false, false]);
  assert.deepEqual(s.racers.map(r => r.id), [0, 1, 2, 3, 4, 5, 6, 7]);
  // Everyone starts behind the line; the humans are 4th and 5th on the grid.
  assert.ok(s.racers.every(r => r.travel < 0 && r.lap === 0 && r.finish === null));
  assert.deepEqual(s.racers.filter(r => r.human).map(r => r.place).sort(), [4, 5]);
  assert.deepEqual(s.racers.map(r => r.place).sort((a, b) => a - b), [1, 2, 3, 4, 5, 6, 7, 8]);
  // A solo human starts 4th of 8.
  assert.equal(createRace('meadow', kartSeats([{ slot: 0, name: 'Ann', pick: 'kai' }], true), 'easy', 1).racers.find(r => r.human)!.place, 4);
  // Grid slots don't overlap and sit on the road.
  const track = trackFor('meadow');
  for (const a of s.racers) {
    const near = nearest(track, a.x, a.z);
    assert.ok(Math.abs(near.lateral) < track.halfWidth);
    for (const b of s.racers) if (a !== b) assert.ok(Math.hypot(a.x - b.x, a.z - b.z) > 2.3, `${a.id}/${b.id} overlap`);
  }
  // Item boxes: five per row on the main road, plus the rows on its branches; every coin in place.
  assert.equal(s.boxes.length, itemBoxes(track).length);
  assert.ok(s.boxes.length > track.def.itemRows.length * 5);
  assert.ok(s.coins.length > 0 && s.coins.every(c => c.timer === 0) && s.racers.every(r => r.coins === 0));
  // More than MAX_RACERS seats are cut.
  assert.equal(createRace('meadow', [...seats, ...seats], 'easy', 1).racers.length, MAX_RACERS);
});

test('createRace is deterministic for a seed', () => {
  const a = createRace('canyon', kartSeats([], true), 'hard', 5), b = createRace('canyon', kartSeats([], true), 'hard', 5);
  run(a, 12); run(b, 12);
  assert.deepEqual(a.racers.map(r => [r.x, r.z, r.travel]), b.racers.map(r => [r.x, r.z, r.travel]));
});

test('intro -> countdown (3, 2, 1) -> GO -> race', () => {
  const s = createRace('meadow', kartSeats([], true), 'normal', 3);
  let countdownAt = -1, raceAt = -1;
  for (let i = 0; i < 400 && raceAt < 0; i++) {
    tick(s, []);
    if (s.phase === 'countdown' && countdownAt < 0) countdownAt = s.clock;
    if (s.phase === 'race') raceAt = s.clock;
  }
  assert.ok(Math.abs(countdownAt - INTRO_SECONDS) < STEP * 1.5, `countdown at ${countdownAt}`);
  assert.ok(Math.abs(raceAt - INTRO_SECONDS - COUNTDOWN_SECONDS) < STEP * 2.5, `race at ${raceAt}`);
  assert.deepEqual(events(s, 'count').map(e => e.value), [3, 2, 1]);
  assert.equal(events(s, 'go').length, 1);
  // Nobody moves before GO.
  const fresh = createRace('meadow', kartSeats([], true), 'normal', 3);
  assert.deepEqual(s.racers.map(r => [r.x, r.z]), fresh.racers.map(r => [r.x, r.z]));
  assert.equal(s.raceTime, 0);
  tick(s, []);
  assert.ok(s.raceTime > 0);
});

test('stepRace uses fixed steps and ignores bad dt', () => {
  const s = createRace('meadow', kartSeats([], true), 'normal', 3);
  stepRace(s, [], NaN); stepRace(s, [], -1); stepRace(s, [], 0);
  assert.equal(s.clock, 0);
  stepRace(s, [], 0.05);
  assert.ok(Math.abs(s.clock - 3 * STEP) < 1e-9);
  // A huge frame is capped (no spiral of death).
  stepRace(s, [], 10);
  assert.ok(s.clock <= 9 * STEP + 1e-9);
});

test('rocket start: gas as "2" fades is a perfect start, later a good one; floored from "3" is a burnout; at GO nothing', () => {
  const s = createRace('meadow', [human(0), human(1), human(2), human(3), human(4)], 'normal', 4);
  while (s.phase === 'intro') tick(s, []);
  while (s.phase === 'countdown') {
    const cd = s.countdown;
    tick(s, [press({ throttle: 1 }), press({ throttle: cd <= 1.2 ? 1 : 0 }), press({ throttle: cd <= 0.5 ? 1 : 0 }), press({ throttle: cd <= 0.1 ? 1 : 0 }), press({ throttle: cd <= 1.7 ? 1 : 0 })]);
  }
  const [early, perfect, good, late, between] = s.racers;
  const at = s.racers.map(r => r.travel);
  assert.equal(perfect.boost, 1.4); assert.equal(perfect.boostPower, 1.5);
  assert.equal(good.boost, 0.8); assert.equal(good.boostPower, 1.38);
  // The perfect start is felt at once: a shove before the first driving tick.
  assert.ok(perfect.speed > good.speed && good.speed > 0, `kick ${perfect.speed.toFixed(1)} vs ${good.speed.toFixed(1)}`);
  for (const r of [early, late, between]) assert.equal(r.boost, 0, `racer ${r.id}`);
  assert.equal(early.stall, BURNOUT_SECONDS);
  assert.equal(between.stall, 0); assert.equal(late.stall, 0);
  assert.deepEqual(events(s, 'boost').map(e => [e.racer, e.value ?? 0]), [[perfect.id, 2], [good.id, 0]]);
  assert.deepEqual(events(s, 'burnout').map(e => e.racer), [early.id]);
  // The burnout spins its wheels in place while everyone else gets going...
  run(s, BURNOUT_SECONDS - STEP, Array(5).fill(press({ throttle: 1 })));
  assert.ok(early.speed < 1 && late.speed > 5, `burnout ${early.speed.toFixed(1)} vs ${late.speed.toFixed(1)}`);
  // ...and a second in: perfect ahead of good ahead of a plain start.
  run(s, 0.2, Array(5).fill(press({ throttle: 1 })));
  const gained = s.racers.map((r, i) => r.travel - at[i]);
  assert.ok(gained[1] > gained[2] && gained[2] > gained[3], gained.map(g => g.toFixed(1)).join(' '));
  assert.ok(early.stall <= 0 && early.speed > 0, 'the burnout wears off');
});

test('rocket start: hard CPUs nail it, normal ones get the good start, easy ones none', () => {
  for (const [difficulty, power] of [['hard', 1.5], ['normal', 1.38], ['easy', 0]] as [Difficulty, number][]) {
    const s = createRace('meadow', kartSeats([], true), difficulty, 7);
    toRace(s);
    for (const r of s.racers) assert.equal(r.boost > 0 ? r.boostPower : 0, power, `${difficulty} CPU ${r.id}`);
    assert.equal(events(s, 'burnout').length, 0);
  }
});

test('travel tracks progress forward and backward; wrong-way is detected', () => {
  const s = humansRace(2);
  const [a, b] = s.racers;
  put(s, a, 40, 3, 20);
  put(s, b, 40, -4, 15, Math.PI);
  run(s, 1, [press({ throttle: 1 }), press({ throttle: 1 })]);
  assert.ok(a.travel > 55 && a.travel < 70, `forward travel ${a.travel}`);
  assert.ok(b.travel < 30, `reverse travel ${b.travel}`);
  assert.ok(b.wrong > 0.5 && a.wrong === 0);
  assert.ok(a.place < b.place);
});

test('laps: crossing the line counts laps, the final lap is announced, then finish', () => {
  const s = createRace('downtown', [human(0)], 'normal', 8);
  toRace(s);
  const track = trackFor('downtown');
  const r = s.racers[0];
  const cross = (lap: number) => {
    put(s, r, track.length - 4, 0, 25);
    r.travel = (lap + 1) * track.length - 4; r.lap = lap;
    for (let i = 0; i < 120 && r.lap === lap; i++) tick(s, [press({ throttle: 1 })]);
    assert.equal(r.lap, lap + 1);
  };
  run(s, 1, [press({ throttle: 1 })]);
  cross(0);
  assert.equal(events(s, 'lap').at(-1)?.value, 1);
  assert.equal(r.lapTimes.length, 1);
  run(s, 2, [press({ throttle: 1 })]);
  cross(1);
  assert.equal(events(s, 'final').at(-1)?.value, 2);
  assert.equal(r.finish, null);
  run(s, 2, [press({ throttle: 1 })]);
  cross(2);
  assert.ok(r.finish !== null && Math.abs(r.finish - s.raceTime) < STEP * 1.5);
  assert.equal(s.firstFinish, r.finish);
  assert.equal(events(s, 'finish').length, 1);
  assert.equal(r.lapTimes.length, 3);
  assert.ok(Math.abs(r.lapTimes.reduce((a, b) => a + b, 0) - r.finish!) < 1e-6);
  // Solo human finished -> the race ends.
  assert.equal(s.phase, 'done');
  const st = standings(s);
  assert.equal(st.length, 1);
  assert.equal(st[0].place, 1);
  assert.equal(st[0].time, r.finish);
  assert.equal(st[0].best, Math.min(...r.lapTimes));
  // Done races don't advance.
  const frozen = JSON.stringify(s.racers);
  const raceTime = s.raceTime;
  tick(s, [press({ throttle: 1 })]);
  assert.equal(s.raceTime, raceTime);
  assert.equal(JSON.stringify(s.racers), frozen);
});

test('rank orders finishers by time, then everyone else by travel', () => {
  const s = createRace('meadow', kartSeats([], true), 'normal', 2);
  const travels = [50, 300, 120, 10, 900, 400, 60, 250];
  s.racers.forEach((r, i) => { r.travel = travels[i]; });
  rank(s);
  assert.deepEqual(s.racers.map(r => r.place), [7, 3, 5, 8, 1, 2, 6, 4]);
  s.racers[3].finish = 80; s.racers[0].finish = 75;
  rank(s);
  assert.equal(s.racers[0].place, 1);
  assert.equal(s.racers[3].place, 2);
  assert.equal(s.racers[4].place, 3);
  assert.deepEqual(s.racers.map(r => r.place).sort((a, b) => a - b), [1, 2, 3, 4, 5, 6, 7, 8]);
});

test('item box: driving through rolls an item after the roulette, box respawns', () => {
  const s = humansRace(2);
  const [r, other] = s.racers;
  put(s, other, 500, 0);
  const box = s.boxes[2];
  put(s, r, 30, 0);
  r.x = box.x; r.z = box.z;
  tick(s, [idleInput(), idleInput()]);
  assert.ok(box.timer > 1.9, `box timer ${box.timer}`);
  assert.equal(events(s, 'box').length, 1);
  assert.equal(r.roll, 1.1);
  assert.equal(r.item, null);
  run(s, 1.2, [idleInput(), idleInput()]);
  assert.ok(r.item !== null, 'rolled an item');
  assert.equal(r.itemCount, r.item === 'triple' ? 3 : 1);
  assert.equal(events(s, 'item').length, 1);
  // The box re-arms after two seconds; holding an item, a second hit doesn't re-roll.
  const before = r.item;
  let n = 0;
  while (box.timer > 0 && n++ < 200) tick(s, [idleInput(), idleInput()]);
  assert.equal(box.timer, 0);
  tick(s, [idleInput(), idleInput()]);
  assert.equal(box.timer, 2);
  assert.equal(events(s, 'box').length, 2);
  assert.ok(r.roll <= 0, 'no second roulette');
  assert.equal(r.item, before);
});

test('banana: dropped behind, spins out whoever drives into it', () => {
  const s = humansRace(2);
  const [a, b] = s.racers;
  put(s, a, 100, 0); put(s, b, 300, 0);
  tick(s, []);
  a.item = 'banana'; a.itemCount = 1;
  tick(s, [press({ item: true })]);
  assert.equal(a.item, null);
  assert.equal(s.hazards.length, 1);
  const hz = s.hazards[0];
  assert.equal(hz.owner, a.id);
  // Behind the racer: lower lap distance.
  const near = nearest(trackFor('meadow'), hz.x, hz.z);
  assert.ok(near.s < 100 && near.s > 96, `banana at s=${near.s}`);
  assert.equal(events(s, 'drop').length, 1);
  b.x = hz.x; b.z = hz.z; b.y = hz.y; b.hint = -1;
  tick(s, []);
  assert.equal(b.spin, 1);
  assert.equal(s.hazards.length, 0);
  assert.ok(events(s, 'spin').some(e => e.racer === b.id));
  assert.equal(a.spin, 0);
});

test('red shell: homes in on the racer ahead and hits only them (never its owner, even point blank)', () => {
  for (const gap of [25, 3]) {
    const s = humansRace(3);
    const [shooter, target, bystander] = s.racers;
    put(s, shooter, 100, 0); put(s, target, 100 + gap, 1.5); put(s, bystander, 400, 0);
    tick(s, []);
    assert.equal(target.place, shooter.place - 1);
    shooter.item = 'red'; shooter.itemCount = 1;
    tick(s, [press({ item: true })]);
    // (Point blank it may already have struck in the throwing tick.)
    if (gap > 5) { assert.equal(s.shells.length, 1); assert.equal(s.shells[0].kind, 'red'); assert.equal(s.shells[0].target, target.id); }
    assert.deepEqual(events(s, 'throw').map(e => e.value), [1]);
    for (let i = 0; i < 120 && s.shells.length; i++) tick(s, []);
    assert.equal(s.shells.length, 0, 'shell exploded');
    assert.equal(events(s, 'explode').length, 1);
    assert.ok(target.spin > 0, 'target spun');
    assert.equal(shooter.spin, 0, `gap ${gap}: the shooter is spared`);
    assert.equal(bystander.spin, 0);
  }
});

test('star: blocks spin-outs and spins others on contact', () => {
  const s = humansRace(2);
  const [a, b] = s.racers;
  put(s, a, 100, 0); put(s, b, 300, 0);
  tick(s, []);
  a.item = 'star'; a.itemCount = 1;
  tick(s, [press({ item: true })]);
  assert.equal(a.star, 7);
  assert.equal(events(s, 'star').length, 1);
  assert.equal(spinOut(s, a), false);
  assert.equal(a.spin, 0);
  // A banana under the star racer is swept away harmlessly.
  s.hazards.push({ id: 999, x: a.x, y: a.y, z: a.z, owner: b.id, age: 9 });
  tick(s, []);
  assert.equal(a.spin, 0);
  assert.equal(s.hazards.length, 0);
  // Contact spins the other racer.
  put(s, b, 100, 1.6);
  tick(s, []);
  assert.ok(b.spin > 0, 'bumped racer spins');
  assert.equal(a.spin, 0);
  // The star wears off after seven seconds.
  run(s, 7.1);
  assert.equal(a.star, 0);
});

test('lightning: strikes every opponent (longest for the leaders, items lost), never the user, who keeps full speed', () => {
  for (const userPlace of [1, 2, 4]) {
    const s = humansRace(4);
    const user = s.racers[0], others = s.racers.slice(1);
    const spots = [400, 300, 200, 100];
    put(s, user, spots[userPlace - 1], 0, 24);
    others.forEach((o, k) => put(s, o, spots.filter((_, i) => i !== userPlace - 1)[k], 0, 24));
    tick(s, []);
    assert.equal(user.place, userPlace);
    others[0].item = 'banana'; others[0].itemCount = 1;
    others[1].star = 5; // a star shrugs it off
    user.item = 'bolt'; user.itemCount = 1;
    const speed = user.speed, seq = s.eventSeq;
    tick(s, [press({ throttle: 1, item: true })]);
    assert.equal(user.spin, 0); assert.equal(user.slow, 0);
    assert.ok(user.speed >= speed - 0.5, `user speed ${user.speed.toFixed(1)} vs ${speed.toFixed(1)}`);
    assert.equal(user.item, null);
    const hit = others.filter(o => o.star <= 0);
    for (const o of hit) assert.ok(o.spin > 0 && o.slow > 1.9, `racer ${o.id} struck`);
    assert.equal(others[1].spin, 0, 'the star racer is untouched');
    assert.equal(others[0].item, null, 'the struck racer lost its item');
    // The further ahead, the longer the shrink.
    const byPlace = [...hit].sort((a, b) => a.place - b.place);
    for (let k = 1; k < byPlace.length; k++) assert.ok(byPlace[k - 1].slow >= byPlace[k].slow);
    assert.deepEqual(events(s, 'bolt', seq).map(e => [e.racer, e.value]), [[user.id, hit.length]]);
    assert.deepEqual(events(s, 'zap', seq).map(e => e.racer).sort(), hit.map(o => o.id).sort());
    assert.ok(!events(s, 'zap', seq).some(e => e.racer === user.id) && !events(s, 'spin', seq).some(e => e.racer === user.id));
  }
});

test('triple mushroom gives three boosts, one per press', () => {
  const s = humansRace(1);
  const r = s.racers[0];
  put(s, r, 60, 0, 20);
  r.item = 'triple'; r.itemCount = 3;
  for (let k = 3; k > 0; k--) {
    tick(s, [press({ throttle: 1, item: true })]);
    assert.equal(r.itemCount, k - 1);
    assert.ok(r.boost > 1.4);
    tick(s, [press({ throttle: 1 })]);
  }
  assert.equal(r.item, null);
});

/** Drives a solo racer at speed and drifts right until `level`, then releases. */
function driftTo(level: number): { state: KartState; r: Racer; charges: number[]; levels: number[] } {
  const s = humansRace(1);
  const r = s.racers[0];
  put(s, r, 30, 7, 25); // drifting right circles toward negative lateral: start left so it stays in bounds
  const charges: number[] = [], levels: number[] = [];
  for (let i = 0; i < 600 && r.driftLevel < level; i++) {
    tick(s, [press({ throttle: 1, steer: 0.7, drift: true })]);
    charges.push(r.driftCharge); levels.push(r.driftLevel);
    assert.equal(r.respawn, 0, 'stayed in bounds while drifting');
  }
  assert.equal(r.driftLevel, level, `reached level ${level}`);
  return { state: s, r, charges, levels };
}

test('drift: charge crosses DRIFT_LEVELS thresholds in order', () => {
  const { r, charges, levels } = driftTo(3);
  assert.ok(r.drift === 1, 'drifting right');
  for (let k = 1; k <= 3; k++) {
    const i = levels.indexOf(k);
    assert.ok(i > 0, `level ${k} reached`);
    assert.ok(charges[i] >= DRIFT_LEVELS[k - 1] && charges[i - 1] < DRIFT_LEVELS[k - 1], `level ${k} at charge ${charges[i]}`);
  }
  // Levels never decrease while drifting.
  for (let i = 1; i < levels.length; i++) assert.ok(levels[i] >= levels[i - 1]);
});

for (const level of [1, 2, 3]) {
  test(`drift: releasing at level ${level} fires a ${TURBO_SECONDS[level - 1]} s mini-turbo`, () => {
    const { state: s, r } = driftTo(level);
    const seq = s.eventSeq;
    tick(s, [press({ throttle: 1 })]);
    assert.equal(r.drift, 0);
    assert.equal(r.driftLevel, 0);
    assert.equal(r.driftCharge, 0);
    assert.equal(r.boost, TURBO_SECONDS[level - 1]);
    assert.equal(r.boostPower, TURBO_POWER[level - 1]);
    assert.deepEqual(events(s, 'turbo', seq).map(e => e.value), [level]);
  });
}

test('drift: releasing before the first level gives no boost', () => {
  const s = humansRace(1);
  const r = s.racers[0];
  put(s, r, 30, 7, 25); // drifting right circles toward negative lateral: start left so it stays in bounds
  for (let i = 0; i < 20; i++) tick(s, [press({ throttle: 1, steer: 0.7, drift: true })]);
  assert.equal(r.drift, 1);
  assert.equal(r.driftLevel, 0);
  tick(s, [press({ throttle: 1 })]);
  assert.equal(r.drift, 0);
  assert.equal(r.boost, 0);
  assert.equal(events(s, 'turbo').length, 0);
});

test('walls: racers on wall tracks cannot leave the road', () => {
  for (const track of ['downtown', 'canyon'] as TrackId[]) {
    for (const side of [1, -1]) {
      const s = humansRace(1, track);
      const t = trackFor(track);
      const r = s.racers[0];
      put(s, r, 200, 0, 28, side * Math.PI / 2);
      let max = 0;
      for (let i = 0; i < 240; i++) {
        tick(s, [press({ throttle: 1 })]);
        max = Math.max(max, Math.abs(nearest(t, r.x, r.z).lateral));
        assert.equal(r.respawn, 0, `${track}: no rescue on wall tracks`);
      }
      assert.ok(max <= t.halfWidth + 1.3 + 0.6, `${track} side ${side}: reached lateral ${max.toFixed(2)}`);
      assert.ok(events(s, 'bump').length > 0, `${track}: wall bump`);
    }
  }
});

test('out of bounds on grass tracks: the racer is rescued back onto the road', () => {
  for (const track of ['meadow', 'frost'] as TrackId[]) {
    const s = humansRace(1, track);
    const t = trackFor(track);
    const r = s.racers[0];
    put(s, r, 200, 0, 28, Math.PI / 2);
    let rescued = false;
    for (let i = 0; i < 600 && !rescued; i++) {
      tick(s, [press({ throttle: 1 })]);
      if (r.respawn > 0) rescued = true;
    }
    assert.ok(rescued, `${track}: respawn triggered`);
    assert.equal(r.speed, 0);
    assert.ok(Math.abs(r.lateral) - t.halfWidth > t.def.outOfBounds - 3, `left the road by ${Math.abs(r.lateral) - t.halfWidth}`);
    assert.equal(events(s, 'respawn').length, 1);
    // Throttle is ignored while waiting (about a second); held as the kart lands, it fires a respawn boost.
    let wait = 0;
    while (r.respawn > 0 && wait++ < 200) tick(s, [press({ throttle: 1 })]);
    assert.ok(wait * STEP < 1.1, `${track}: rescued in ${(wait * STEP).toFixed(2)} s`);
    const near = nearest(t, r.x, r.z);
    assert.ok(Math.abs(near.lateral) < 1, `${track}: back on the centre line (${near.lateral})`);
    assert.ok(events(s, 'land').length >= 1);
    assert.equal(r.ghost, GHOST_SECONDS);
    assert.equal(r.boost, RESPAWN_BOOST);
    // Untouchable for a moment: no spin-outs.
    assert.equal(spinOut(s, r), false);
  }
});

test('manual respawn when stuck and slow', () => {
  const s = humansRace(1);
  const r = s.racers[0];
  put(s, r, 120, 6);
  tick(s, [press({ respawn: true })]);
  assert.ok(r.respawn > 0);
  run(s, 0.7);
  assert.ok(Math.abs(nearest(trackFor('meadow'), r.x, r.z).lateral) < 1);
});

test('sanitize clamps and coerces bad input', () => {
  assert.deepEqual(sanitize(undefined), idleInput());
  assert.deepEqual(sanitize({ throttle: 7, steer: -9, drift: true, item: false, respawn: false }), { throttle: 1, steer: -1, drift: true, item: false, respawn: false });
  const junk = { throttle: NaN, steer: Infinity, drift: 'yes', item: 1, respawn: {} } as unknown as KartInput;
  assert.deepEqual(sanitize(junk), idleInput());
  assert.deepEqual(sanitize({ throttle: '1', steer: null } as unknown as KartInput), idleInput());
  // NaN input in a live race never poisons the state.
  const s = humansRace(1);
  run(s, 1, [junk]);
  const r = s.racers[0];
  assert.ok([r.x, r.y, r.z, r.speed, r.yaw, r.travel].every(Number.isFinite));
});

test('cpuInput returns bounded controls for every racer', () => {
  const s = createRace('canyon', kartSeats([], true), 'hard', 12);
  toRace(s);
  run(s, 20);
  const rng = new Rng(1);
  for (const r of s.racers) {
    const i = cpuInput(s, trackFor(s.track), r, rng, false);
    assert.ok(Math.abs(i.steer) <= 1 && i.throttle >= 0 && i.throttle <= 1);
    const cruise = cpuInput(s, trackFor(s.track), r, rng, true);
    assert.equal(cruise.throttle, 0.6);
    assert.equal(cruise.item, false);
  }
});

/** Runs an all-CPU race to the end, counting every event type (the state keeps only the last 48). */
const fullRace = (track: TrackId, difficulty: Difficulty, seed = 7): { s: KartState; seen: Map<KartEventType, number> } => {
  const s = createRace(track, kartSeats([], true), difficulty, seed);
  const seen = new Map<KartEventType, number>();
  let n = 0, last = 0;
  while (s.phase !== 'done' && n++ < 60 * 400) {
    tick(s, []);
    for (const e of s.events) if (e.id > last) seen.set(e.type, (seen.get(e.type) ?? 0) + 1);
    last = s.eventSeq;
  }
  return { s, seen };
};

test('an 8-CPU race finishes on every track with a time for every racer; hard beats easy', () => {
  for (const track of TRACK_IDS) {
    const mean: Record<string, number> = {};
    for (const difficulty of ['easy', 'hard'] as Difficulty[]) {
      const { s, seen } = fullRace(track, difficulty);
      assert.equal(s.phase, 'done', `${track}/${difficulty} finished`);
      const t = trackFor(track);
      // About 20 m/s average including items, jumps and rescues (the new Grand Prix tracks run 4–6 minutes).
      assert.ok(s.raceTime < Math.max(240, LAPS * t.length / 20 + 30), `${track}/${difficulty}: ${s.raceTime.toFixed(1)} s`);
      for (const r of s.racers) {
        assert.ok(r.finish !== null && r.finish > 60, `${track}/${difficulty} racer ${r.id}: ${r.finish}`);
        assert.equal(r.lap, LAPS);
        assert.equal(r.lapTimes.length, LAPS);
        assert.ok(r.travel >= LAPS * t.length);
      }
      const st = standings(s);
      assert.deepEqual(st.map(x => x.place), [1, 2, 3, 4, 5, 6, 7, 8]);
      for (let i = 1; i < st.length; i++) assert.ok(st[i].time >= st[i - 1].time);
      assert.ok(st.every(x => x.best > 0 && x.best <= x.time / 2));
      for (const type of ['box', 'item', 'lap', 'final', 'finish'] as KartEventType[]) assert.ok((seen.get(type) ?? 0) > 0, `${track}/${difficulty}: saw ${type}`);
      assert.equal(seen.get('finish'), 8);
      mean[difficulty] = s.racers.reduce((a, r) => a + r.finish!, 0) / s.racers.length;
    }
    assert.ok(mean.hard < mean.easy, `${track}: hard ${mean.hard.toFixed(1)} vs easy ${mean.easy.toFixed(1)}`);
  }
});

test('with an idle human the race still ends: unfinished racers get a projected time', () => {
  const s = createRace('downtown', kartSeats([{ slot: 0, name: 'Zzz', pick: 'oobi' }], true), 'normal', 21);
  let n = 0;
  while (s.phase !== 'done' && n++ < 60 * 400) tick(s, []);
  assert.equal(s.phase, 'done');
  const me = s.racers.find(r => r.human)!;
  assert.ok(me.finish !== null && me.finish > s.raceTime, 'projected time beyond the clock');
  assert.equal(me.place, 8);
  // Ended ~45 s after the first finisher.
  assert.ok(s.firstFinish !== null && s.raceTime - s.firstFinish >= 45 && s.raceTime - s.firstFinish < 50);
  assert.ok(s.racers.every(r => r.finish !== null));
});

// ── Branching routes, vertical features, water, hazards ──────────────────────

/** Runs `racer` (a human, slot input 0) with the autopilot until `until` or `seconds` pass; records what happened. */
function drive(state: KartState, r: Racer, route: number, seconds: number, until: (r: Racer) => boolean = () => false) {
  const segs = new Set<number>();
  let maxWrong = 0, maxAir = 0, glided = false, under = false;
  const from = state.eventSeq;
  const startTravel = r.travel, startTime = state.raceTime;
  for (let i = 0, n = Math.round(seconds / STEP); i < n && !until(r); i++) {
    const inputs: (KartInput | undefined)[] = state.racers.map(o => (o === r ? pilot(state, r, route) : idleInput()));
    tick(state, inputs);
    segs.add(r.seg); maxWrong = Math.max(maxWrong, r.wrong); maxAir = Math.max(maxAir, r.air);
    glided ||= r.glide; under ||= r.under;
  }
  return { segs, maxWrong, maxAir, glided, under, travelled: r.travel - startTravel, time: state.raceTime - startTime, events: (type: KartEventType) => events(state, type, from) };
}

test('branches: the shortcut switches path, keeps lap progress, merges back and saves time', () => {
  const times: number[] = [];
  for (const route of [0, 1]) {
    const s = humansRace(1, 'lagoon');
    const t = trackFor('lagoon'), b = t.paths[1];
    const r = s.racers[0];
    put(s, r, b.from - 60, 0, 25);
    const target = b.to + 40;
    const run = drive(s, r, route, 40, x => x.seg === 0 && x.lastS > target && x.lastS < target + 50);
    assert.ok(r.lastS > target, `route ${route}: reached the merge (at ${r.lastS.toFixed(0)})`);
    assert.equal(r.seg, 0);
    assert.equal(run.segs.has(1), route === 1, `route ${route}: took the branch`);
    assert.equal(run.events('respawn').length + run.events('splash').length, 0, `route ${route}: clean run`);
    assert.ok(run.maxWrong < 0.2, 'never the wrong way');
    // Progress is continuous across the fork and the merge: travel matches the distance covered on the lap.
    assert.ok(Math.abs(run.travelled - (r.lastS - (b.from - 60))) < 3, `route ${route}: travelled ${run.travelled.toFixed(1)}`);
    times.push(run.time);
  }
  assert.ok(times[1] < times[0] - 1, `shortcut ${times[1].toFixed(1)} s vs main ${times[0].toFixed(1)} s`);
});

test('branches: driving backwards on a branch is wrong-way; ranking uses mapped progress', () => {
  const s = humansRace(2, 'skyway');
  const t = trackFor('skyway'), b = t.paths[1];
  const [a, c] = s.racers;
  putOn(s, a, 1, b.length / 2, 0, 15, Math.PI);
  putOn(s, c, 0, b.from + (b.to - b.from) * 0.4, 0, 0);
  run(s, 1.2, [press({ throttle: 1 }), idleInput()]);
  assert.equal(a.seg, 1);
  assert.ok(a.wrong > 0.5, `wrong ${a.wrong}`);
  // Halfway along the alley is further round the lap than 40 % along the main road it skips.
  const s2 = humansRace(2, 'skyway');
  putOn(s2, s2.racers[0], 1, b.length / 2, 0);
  putOn(s2, s2.racers[1], 0, b.from + (b.to - b.from) * 0.4, 0);
  tick(s2, []);
  assert.deepEqual(s2.racers.map(r => r.place), [1, 2]);
});

test('falling off the rail-less boardwalk: splash, rescue behind on the road, no progress credit', () => {
  const s = humansRace(1, 'lagoon');
  const t = trackFor('lagoon'), b = t.paths[1];
  const r = s.racers[0];
  putOn(s, r, 1, b.length / 2, 0, 24, Math.PI / 2);
  const before = r.travel;
  let n = 0;
  while (r.respawn <= 0 && n++ < 300) tick(s, [press({ throttle: 1 })]);
  assert.ok(r.respawn > 0, 'fell in');
  assert.equal(events(s, 'splash').at(-1)?.value, 0);
  run(s, 1.5, [idleInput()]);
  assert.equal(r.respawn <= 0, true);
  const path = t.paths[r.seg];
  const near = nearest(path, r.x, r.z, -1, newNear());
  assert.ok(Math.abs(near.lateral) < 0.5 && r.grounded, 'back on the deck centre');
  assert.ok(r.travel <= before + 0.5, `travel ${r.travel.toFixed(1)} vs ${before.toFixed(1)}`);
  // Then it drives on normally.
  const d = drive(s, r, 1, 3);
  assert.equal(d.events('splash').length, 0);
});

test('glider: the cliff launch opens the glider, the racer soars over the cove and lands on the beach', () => {
  const s = humansRace(1, 'lagoon');
  const t = trackFor('lagoon');
  const r = s.racers[0];
  const launch = t.def.launches!.find(l => l.glide)!, gapEnd = t.def.gaps![0][1] * t.length;
  put(s, r, launch.at * t.length - 50, 0, 26);
  const run = drive(s, r, 0, 14, x => x.grounded && x.lastS > gapEnd + 5 && x.lastS < gapEnd + 300);
  assert.equal(run.events('glide').length, 1);
  assert.ok(run.glided && run.maxAir > 3, `glided for ${run.maxAir.toFixed(1)} s`);
  assert.equal(run.events('splash').length + run.events('respawn').length, 0);
  assert.ok(r.grounded && !r.glide && r.lastS > gapEnd, 'landed past the gap');
});

test('launch ramps clear their gaps from any speed: broken freeway and lava river', () => {
  for (const [id, speed] of [['skyway', 30], ['skyway', 9], ['volcano', 30], ['volcano', 9]] as [TrackId, number][]) {
    const s = humansRace(1, id);
    const t = trackFor(id);
    const r = s.racers[0];
    const l = t.def.launches!.find(x => !x.glide)!, gap = t.def.gaps!.find(([a]) => a > l.at && a - l.at < 0.01)!;
    put(s, r, l.at * t.length - 6, 0, speed);
    const run = drive(s, r, 0, 6, x => x.grounded && x.lastS > gap[1] * t.length + 3);
    assert.equal(run.events('splash').length + run.events('respawn').length, 0, `${id} @ ${speed}`);
    assert.ok(r.lastS > gap[1] * t.length, `${id} @ ${speed}: landed beyond the gap`);
    assert.ok(run.maxAir > 0.6, `${id}: airtime ${run.maxAir.toFixed(2)}`);
  }
});

test('missing the lava jump: burnt, rescued before the run-up and set back in progress', () => {
  const s = humansRace(1, 'volcano');
  const t = trackFor('volcano');
  const r = s.racers[0];
  const l = t.def.launches![0], gap = t.def.gaps![0];
  // Rolling slowly off the end of the ramp (no take-off) into the lava river.
  put(s, r, (gap[0] + gap[1]) / 2 * t.length, 0, 3);
  r.safeS = l.at * t.length - 4; // last on-road spot: the ramp
  const before = r.travel;
  let n = 0;
  while (r.respawn <= 0 && n++ < 300) tick(s, [idleInput()]);
  assert.equal(events(s, 'splash').at(-1)?.value, 2);
  run(s, 1.5, [idleInput()]);
  assert.ok(r.respawn <= 0 && r.grounded);
  assert.ok(!inLaunchZone(t, r.segS), `rescued at ${r.segS.toFixed(0)}, outside the run-up`);
  assert.ok(r.segS < l.at * t.length - RUN_UP + 1);
  assert.ok(r.travel < before - RUN_UP, 'no progress credit for the fall');
  // From there a normal run clears the jump.
  const run2 = drive(s, r, 0, 8, x => x.lastS > gap[1] * t.length + 10);
  assert.equal(run2.events('splash').length, 0);
});

test('underwater: the seabed road is drivable, slower and floaty; diving in splashes but never rescues', () => {
  const s = humansRace(1, 'lagoon');
  const t = trackFor('lagoon');
  const r = s.racers[0];
  let dive = 0;
  while (!(t.flags[dive] & F_UNDER)) dive++;
  put(s, r, t.dist[dive] - 30, 0, 25);
  const d = drive(s, r, 0, 4);
  assert.ok(d.under, 'went under');
  assert.equal(events(s, 'splash').filter(e => e.value === 1).length, 1, 'one splash going in');
  assert.equal(d.events('respawn').length + events(s, 'splash').filter(e => e.value !== 1).length, 0);
  // Top speed under water is lower than on land.
  const deepest = t.py.indexOf(Math.min(...t.py));
  const wet = humansRace(1, 'lagoon'), dry = humansRace(1, 'lagoon');
  put(wet, wet.racers[0], t.dist[deepest] - 60, 0, 0);
  put(dry, dry.racers[0], 20, 0, 0);
  for (let i = 0; i < 240; i++) { tick(wet, [pilot(wet, wet.racers[0], 0)]); tick(dry, [pilot(dry, dry.racers[0], 0)]); }
  assert.ok(wet.racers[0].under);
  assert.ok(wet.racers[0].speed < dry.racers[0].speed - 1, `under ${wet.racers[0].speed.toFixed(1)} vs dry ${dry.racers[0].speed.toFixed(1)}`);
});

test('rails keep racers on bridge decks and in tunnels', () => {
  for (const [id, from] of [['lagoon', 0.65], ['volcano', 0.455]] as [TrackId, number][]) {
    const s = humansRace(1, id);
    const t = trackFor(id);
    const at = from * t.length;
    assert.ok(t.flags[sampleAt(t, at)] & F_RAILS, `${id}: railed at ${from}`);
    for (const side of [1, -1]) {
      put(s, s.racers[0], at, 0, 28, side * Math.PI / 2);
      const r = s.racers[0];
      let max = 0;
      for (let i = 0; i < 90; i++) { tick(s, [press({ throttle: 1 })]); max = Math.max(max, Math.abs(r.lateral)); }
      assert.ok(max < t.halfWidth + 2, `${id}: lateral ${max.toFixed(1)}`);
      assert.equal(r.respawn, 0);
    }
  }
});

test('obstacles: rollers and crushers spin racers out; a star shrugs them off', () => {
  const s = humansRace(1, 'volcano');
  const t = trackFor('volcano');
  const r = s.racers[0];
  const roller = t.obstacles.find(o => o.kind === 'roller')!;
  // Pick a moment when the roller is crossing the centre line, and park the racer right there.
  const at = (u: number, o: typeof roller) => (u - o.phase) * o.period + 10 * o.period;
  s.raceTime = at(0, roller) - STEP;
  const pose = obstacleAt(t, roller, s.raceTime + STEP, [0, 0, 0, 0]);
  put(s, r, roller.s - 1, 0, 8);
  r.x = pose[0] - Math.sin(r.yaw) * (ROLLER_RADIUS + 0.6); r.z = pose[2] - Math.cos(r.yaw) * (ROLLER_RADIUS + 0.6);
  tick(s, [press({ throttle: 1 })]);
  assert.ok(r.spin > 0, 'rolled over');
  assert.ok(events(s, 'bump').length > 0);
  // Crusher slammed down on its lane.
  const crusher = t.obstacles.find(o => o.kind === 'crusher')!;
  const s2 = humansRace(2, 'volcano');
  const [a, star] = s2.racers;
  s2.raceTime = at(0.74, crusher) - STEP;
  const lat = crusher.lane * t.halfWidth * 0.5;
  put(s2, a, crusher.s, lat, 2); put(s2, star, crusher.s, lat + 2.6, 2); // under the block, not touching
  star.star = 5;
  tick(s2, [idleInput(), idleInput()]);
  assert.equal(events(s2, 'crush').length, 1);
  assert.equal(events(s2, 'crush')[0].racer, a.id);
  assert.ok(a.spin > 1 && a.speed <= 4);
  assert.equal(star.spin, 0);
  // Raised: harmless.
  const s3 = humansRace(1, 'volcano');
  s3.raceTime = at(0.3, crusher) - STEP;
  put(s3, s3.racers[0], crusher.s, lat, 2);
  tick(s3, [idleInput()]);
  assert.equal(events(s3, 'crush').length, 0);
  assert.ok(Math.abs(s3.racers[0].x - pointAt(t, crusher.s, lat)[0]) < CRUSHER_HALF + 2);
});

test('obstacles never pin a kart: knocked down once, it is untouchable for that obstacle\'s cycle and drives clear', () => {
  // A kart stopped dead in the Reef Tunnel, right in front of a sea mine's track (it once sat there for good).
  const s = humansRace(1, 'lagoon');
  const t = trackFor('lagoon');
  const r = s.racers[0];
  const mine = t.obstacles.find(o => o.kind === 'roller' && t.paths[o.seg].name === 'Reef Tunnel')!;
  const from = s.eventSeq;
  for (const u of [0, 0.25, 0.5, 0.75]) {
    s.raceTime = (u - mine.phase) * mine.period + 20 * mine.period;
    putOn(s, r, mine.seg, mine.s - 2, 0);
    r.ghost = 0;
    const route = 1 << (mine.seg - 1), hits = events(s, 'spin', from).length;
    let n = 0;
    while (n++ < 60 * 6 && !(r.seg === mine.seg && r.segS > mine.s + 6)) tick(s, [pilot(s, r, route)]);
    assert.ok(r.seg === mine.seg && r.segS > mine.s + 6, `@${u}: drove clear of the mine (stuck at ${r.segS.toFixed(1)} m)`);
    assert.ok(events(s, 'spin', from).length - hits <= 1, `@${u}: knocked down at most once`);
  }
  // Every hit came with the grace.
  const s2 = humansRace(1, 'lagoon');
  const r2 = s2.racers[0];
  s2.raceTime = (0 - mine.phase) * mine.period + 10 * mine.period - STEP;
  const pose = obstacleAt(t, mine, s2.raceTime + STEP, [0, 0, 0, 0]);
  putOn(s2, r2, mine.seg, mine.s - 1, 0, 8);
  r2.x = pose[0] - Math.sin(r2.yaw) * (ROLLER_RADIUS + 0.6); r2.z = pose[2] - Math.cos(r2.yaw) * (ROLLER_RADIUS + 0.6);
  tick(s2, [press({ throttle: 1 })]);
  assert.ok(r2.spin > 0 && Math.abs(r2.ghost - mine.period) < 1e-9, `ghost ${r2.ghost} for a ${mine.period} s cycle`);
});

test('glider: a boost carried into the launch bleeds off, so the glide still lands short of the next fork', () => {
  // Magma Peak's summit glider at a mushroom's 46 m/s once sailed past the Obsidian Ridge fork and off its deck.
  const s = humansRace(1, 'volcano');
  const t = trackFor('volcano');
  const r = s.racers[0];
  const launch = t.def.launches!.find(l => l.glide)!, ridge = t.paths.find(p => p.name === 'Obsidian Ridge')!;
  put(s, r, launch.at * t.length - 40, 0, 44);
  r.boost = 1.5; r.boostPower = 1.55;
  let landed = -1, top = 0;
  const run = drive(s, r, 0, 20, x => {
    if (x.glide) top = Math.max(top, x.speed);
    if (landed < 0 && top > 0 && x.grounded) landed = x.lastS;
    return landed >= 0;
  });
  assert.equal(run.events('glide').length, 1);
  assert.equal(run.events('splash').length + run.events('respawn').length, 0);
  assert.ok(landed > 0 && landed < ridge.from - FORK_ZONE, `landed at ${landed.toFixed(0)} m, the ridge forks at ${ridge.from.toFixed(0)} m`);
});

test('CPU watchdog: a CPU held in place calls the rescue after a few seconds; a human never gets one', () => {
  const s = createRace('meadow', [human(0), { racer: 'kai', human: false, slot: -1, name: 'CPU' }], 'hard', 5);
  toRace(s);
  const h = s.racers.find(r => r.human)!, cpu = s.racers.find(r => !r.human)!;
  put(s, h, 300, -4); put(s, cpu, 300, 4);
  const hx = h.x, hz = h.z, cx = cpu.x, cz = cpu.z, from = s.eventSeq;
  const inputs: Inputs = [];
  inputs[h.id] = press({ throttle: 1 });
  let n = 0;
  // Both pinned (as by a wall corner or an obstacle): put back every tick.
  while (n++ < 60 * (STUCK_SECONDS + 1) && cpu.respawn <= 0) {
    tick(s, inputs);
    h.x = hx; h.z = hz; h.speed = 0;
    if (cpu.respawn <= 0) { cpu.x = cx; cpu.z = cz; cpu.speed = 0; }
  }
  const rescue = events(s, 'respawn', from);
  assert.equal(rescue.length, 1);
  assert.deepEqual([rescue[0].racer, rescue[0].value], [cpu.id, 2]);
  assert.ok(Math.abs(n * STEP - STUCK_SECONDS) < 0.1, `called after ${(n * STEP).toFixed(2)} s`);
  run(s, 1.2, [idleInput()]);
  assert.ok(cpu.respawn <= 0 && cpu.ghost > 0 && cpu.cpuStuck < 1, 'back on the road, the watch restarted');
});

test('CPU routes: a CPU that missed a branch\'s mouth and sits on the main road\'s wall past the fork gives up the branch', () => {
  const s = createRace('skyway', [{ racer: 'kai', human: false, slot: -1, name: 'CPU' }], 'hard', 3);
  toRace(s);
  const t = trackFor('skyway'), alley = t.paths.find(p => p.name === 'Back Alley')!, bit = 1 << (alley.index - 1);
  const cpu = s.racers[0];
  // 20 m past the fork, still counted on the main road and up against its wall.
  put(s, cpu, alley.from + 20, 0, 12);
  cpu.cpuRoute = bit; cpu.lateral = t.halfWidth + 1.3;
  cpuInput(s, t, cpu, new Rng(1), false);
  assert.equal(cpu.cpuRoute & bit, 0, 'dropped the branch');
  // Inside the road (still able to turn in), or against the wall short of the fork: keeps it.
  for (const [at, lateral] of [[alley.from + 20, 3], [alley.from - 20, t.halfWidth + 1.3]]) {
    put(s, cpu, at, 0, 12);
    cpu.cpuRoute = bit; cpu.lateral = lateral;
    cpuInput(s, t, cpu, new Rng(1), false);
    assert.equal(cpu.cpuRoute & bit, bit, `kept the branch at ${at - alley.from} m, ${lateral} m`);
  }
});

test('CPU routes: per-difficulty odds, re-rolled each lap, and CPUs really take the shortcut', () => {
  const t = trackFor('lagoon'), b = t.paths[1];
  for (const [k, diff] of (['easy', 'normal', 'hard'] as Difficulty[]).entries()) {
    const rng = new Rng(99 + k);
    let taken = 0;
    for (let i = 0; i < 4000; i++) taken += pickRoute(t, diff, rng) & 1;
    assert.ok(Math.abs(taken / 4000 - b.cpu[k]) < 0.03, `${diff}: ${taken / 4000} vs ${b.cpu[k]}`);
  }
  assert.equal(pickRoute(trackFor('meadow'), 'hard', new Rng(1)), 0, 'no branches, no bits');
  const s = createRace('lagoon', kartSeats([], true), 'hard', 41);
  let onBranch = 0;
  const routes = new Set<number>();
  for (let i = 0; i < 60 * 200 && s.phase !== 'done'; i++) {
    tick(s, []);
    for (const r of s.racers) { if (r.seg === 1) onBranch++; routes.add(r.cpuRoute); }
  }
  assert.ok(onBranch > 100, `CPU ticks on the boardwalk: ${onBranch}`);
  // Two branches: each bit gets rolled on (and off) across the field and the laps.
  assert.ok(routes.has(0) && [...routes].some(r => r & 1) && [...routes].some(r => r & 2), `routes ${[...routes]}`);
});

test('races on branching tracks are deterministic for a seed', () => {
  for (const id of ['lagoon', 'volcano', 'skyway'] as TrackId[]) {
    const a = createRace(id, kartSeats([], true), 'hard', 5), b = createRace(id, kartSeats([], true), 'hard', 5);
    run(a, 40); run(b, 40);
    assert.deepEqual(a.racers.map(r => [r.x, r.z, r.travel, r.seg]), b.racers.map(r => [r.x, r.z, r.travel, r.seg]));
  }
});

test('progress never jumps: travel tracks lap distance across every fork, merge, jump and dive', () => {
  for (const id of TRACK_IDS) {
    const s = createRace(id, kartSeats([], true), 'normal', 13);
    const t = trackFor(id);
    const prev = s.racers.map(r => r.travel), seg = s.racers.map(r => r.seg);
    let switches = 0;
    for (let i = 0; i < 60 * 150; i++) {
      tick(s, []);
      s.racers.forEach((r, k) => {
        if (r.respawn > 0) { prev[k] = r.travel; seg[k] = r.seg; return; }
        const d = r.travel - prev[k];
        // < 1 m per tick at top speed (a little more cutting a corner on the grass); switching between a
        // branch and the main loop may add a few metres (the centre lines part ways through a fork zone).
        if (r.seg !== seg[k]) switches++;
        assert.ok(d < (r.seg !== seg[k] ? 10 : 2), `${id} racer ${k}: +${d.toFixed(2)} m in one tick (seg ${seg[k]} -> ${r.seg})`);
        seg[k] = r.seg;
        // travel mirrors lap progress
        assert.ok(Math.abs(circ(r.travel - r.lastS, t.length)) < 1e-6 || r.respawn > 0, `${id} racer ${k}: travel ${r.travel} vs lastS ${r.lastS}`);
        prev[k] = r.travel;
      });
    }
    assert.ok(switches > 0, `${id}: some CPU took the branch`);
  }
});

// ── Coins, drawbridges, half-pipes, boost-only shortcuts ─────────────────────

/** Autopilot holding lateral offset `lane` (metres) along the racer's current path. */
function lanePilot(state: KartState, r: Racer, lane: number, extra: Partial<KartInput> = {}): KartInput {
  const track = trackFor(state.track), path = track.paths[r.seg];
  const p = pointAt(path, r.segS + 8 + Math.max(0, r.speed) * 0.3, lane);
  const angle = wrapAngle(Math.atan2(p[0] - r.x, p[2] - r.z) - r.yaw);
  return { ...idleInput(), throttle: 1, steer: clamp(-angle * 2.4, -1, 1), ...extra };
}

test('coins: driving through a line fills the purse (capped at ten), they reappear later, and top speed rises with them', () => {
  const s = humansRace(1, 'meadow');
  const r = s.racers[0];
  const c0 = s.coins[0];
  // Drop the racer just behind the first coin line, on its lane.
  const t = trackFor('meadow');
  const n = nearest(t, c0.x, c0.z, -1, newNear());
  put(s, r, n.s - 12, n.lateral, 20);
  run(s, 2, () => [lanePilot(s, r, n.lateral)]);
  assert.ok(r.coins >= 4, `collected ${r.coins}`);
  const taken = s.coins.filter(c => c.timer > 0).length;
  assert.equal(taken, r.coins);
  // They come back after COIN_RESPAWN seconds.
  run(s, COIN_RESPAWN + 0.1, [idleInput()]);
  assert.ok(s.coins.every(c => c.timer === 0));
  // The purse never passes ten.
  r.coins = MAX_COINS - 1;
  put(s, r, n.s - 12, n.lateral, 20);
  run(s, 2, () => [lanePilot(s, r, n.lateral)]);
  assert.equal(r.coins, MAX_COINS);
  // Ten coins are faster than none on the same straight.
  const top = (coins: number) => {
    const x = humansRace(1, 'meadow');
    put(x, x.racers[0], 20, 0, 0);
    x.racers[0].coins = coins;
    for (const c of x.coins) c.timer = 999; // nothing to pick up on the way
    run(x, 4, () => [pilot(x, x.racers[0], 0)]);
    return x.racers[0].speed;
  };
  assert.ok(top(MAX_COINS) > top(0) * 1.04, `${top(MAX_COINS).toFixed(2)} vs ${top(0).toFixed(2)}`);
});

test('coins: a spin-out or a rescue costs some of the purse', () => {
  const s = humansRace(1, 'meadow');
  const r = s.racers[0];
  r.coins = 7;
  assert.ok(spinOut(s, r, 1));
  assert.equal(r.coins, 7 - COIN_LOSS);
  r.coins = 2;
  r.spin = 0;
  spinOut(s, r, 1);
  assert.equal(r.coins, 0, 'never below zero');
  // A star blocks the hit and keeps the coins.
  r.coins = 5; r.spin = 0; r.star = 3;
  assert.equal(spinOut(s, r, 1), false);
  assert.equal(r.coins, 5);
  // Rescued after driving off the grass edge.
  r.star = 0; r.coins = 6;
  put(s, r, 300, trackFor('meadow').halfWidth + 30, 0);
  run(s, 2.5, [idleInput()]);
  assert.ok(events(s, 'respawn').length > 0);
  assert.equal(r.coins, 6 - COIN_LOSS);
});

test('drawbridge: raised, it throws a racer at speed over the opening; lowered, it is a plain deck', () => {
  const t = trackFor('meadow'), d = t.def.drawbridges![0], a = d.from * t.length, b = d.to * t.length;
  for (const raised of [true, false]) {
    const s = humansRace(1, 'meadow');
    const r = s.racers[0];
    // Arrives about a second and a half later: while the leaves are up (or long before they rise).
    s.raceTime = raised ? d.period * 0.55 : 0;
    put(s, r, a - 40, 0, 28);
    const run1 = drive(s, r, 0, 6, x => x.grounded && x.lastS > b + 10);
    assert.ok(r.lastS > b + 5, `raised=${raised}: crossed`);
    assert.equal(run1.events('splash').length + run1.events('respawn').length, 0, `raised=${raised}: dry`);
    if (raised) {
      assert.ok(drawLift(d, s.raceTime - run1.time + 1.5) > 0.3);
      assert.ok(run1.maxAir > 0.4 && run1.events('land').length > 0, `airtime ${run1.maxAir.toFixed(2)}`);
    } else assert.ok(run1.maxAir < 0.2, `flat: ${run1.maxAir.toFixed(2)}`);
  }
  // Stopped in the opening while the leaves are up: into the lake, rescued before the run-up.
  const s = humansRace(1, 'meadow');
  const r = s.racers[0];
  put(s, r, (a + b) / 2, 0, 0);
  r.safeS = a + 2;
  s.raceTime = d.period * 0.7;
  let n = 0;
  while (r.respawn <= 0 && n++ < 300) tick(s, [idleInput()]);
  assert.ok(r.respawn > 0 && events(s, 'splash').some(e => e.value === 0), 'fell in');
  run(s, 1.6, [idleInput()]);
  assert.ok(r.grounded && r.respawn <= 0, 'back on the road');
  assert.ok(!inLaunchZone(t, r.segS - 0.5) && r.segS < a - RUN_UP + 1, `rescued at ${r.segS.toFixed(1)} (bridge at ${a.toFixed(1)})`);
});

test('half-pipe: riding high on the wall off the lip is a trick jump (boost on landing); the floor is not', () => {
  const t = trackFor('downtown'), [, to] = t.def.halfpipes![0], b = to * t.length, hw = t.halfWidth;
  for (const lane of [hw * 0.8, 0]) {
    const s = humansRace(1, 'downtown');
    const r = s.racers[0];
    put(s, r, b - 30, lane, 24);
    let launched = false, boosted = false;
    for (let i = 0; i < 60 * 4; i++) {
      const trick = !r.grounded && r.trickReady && r.air > 0.15;
      tick(s, [lanePilot(s, r, lane, { drift: trick && i % 2 === 0 })]);
      launched ||= !r.grounded && r.vy > 3;
      if (launched && r.grounded && r.boost > 0) boosted = true;
    }
    if (lane) {
      assert.ok(launched, 'off the lip');
      assert.ok(events(s, 'trick').length > 0 && boosted, 'trick boost on landing');
    } else assert.ok(!launched && events(s, 'trick').length === 0, 'the flat floor does not launch');
  }
});

test('boost-only shortcut: CPUs take it only holding a mushroom, fire it in the mud; without one the mud is crawling', () => {
  const t = trackFor('downtown'), b = t.paths[1];
  assert.ok(b.boost && b.shortcut);
  for (const item of [null, 'mushroom'] as const) {
    const s = createRace('downtown', [{ racer: 'kai', human: false, slot: -1, name: 'CPU' }], 'hard', 3);
    toRace(s);
    const r = s.racers[0];
    put(s, r, b.from - 120, 0, 25);
    r.cpuRoute = 1; r.item = item; r.itemCount = item ? 1 : 0; r.cpuItemDelay = 0;
    for (const box of s.boxes) box.timer = 999; // no item boxes on the way: only what it holds counts
    let onBranch = 0;
    const from = s.eventSeq;
    for (let i = 0; i < 60 * 30 && !(r.seg === 0 && r.lastS > b.to + 20); i++) { tick(s, []); if (r.seg === 1 && r.segS > 50) onBranch++; }
    assert.ok(r.lastS > b.to + 20, `item=${item}: got past the merge`);
    if (item) {
      assert.ok(onBranch > 30, 'took the market cut');
      assert.ok(events(s, 'boost', from).some(e => e.value === 1), 'used the mushroom');
      assert.equal(r.item, null);
    } else {
      assert.equal(onBranch, 0, 'stayed on the boulevard');
      assert.equal(r.cpuRoute & 1, 0, 'dropped the cut from its route');
    }
  }
  // A human crawls through the deep mud without a boost (well under the grass speed).
  const s = humansRace(1, 'downtown');
  const r = s.racers[0];
  const mud = b.features.offroad[0];
  putOn(s, r, 1, (mud.from + 0.02) * b.length, 0, 25);
  run(s, 2, [press({ throttle: 1 })]);
  assert.equal(r.surface, 'grass');
  assert.ok(r.speed < 13, `mud speed ${r.speed.toFixed(1)}`);
});
