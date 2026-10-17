import { test } from 'node:test';
import assert from 'node:assert/strict';
import { Rng } from '../src/modules/rooms/games/common/math.ts';
import { nearest, pointAt, TRACK_IDS, trackFor, type TrackId } from '../src/modules/rooms/games/kart/tracks.ts';
import {
  COUNTDOWN_SECONDS, createRace, DRIFT_LEVELS, idleInput, INTRO_SECONDS, LAPS, MAX_RACERS, rank, sanitize, spinOut, standings, STEP, stepRace, tick, TURBO_SECONDS,
  type Difficulty, type KartEventType, type KartInput, type KartSeat, type KartState, type Racer,
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
function put(state: KartState, r: Racer, s: number, lateral: number, speed = 0, yawOffset = 0): void {
  const track = trackFor(state.track);
  const pt = pointAt(track, s, lateral);
  const near = nearest(track, pt[0], pt[2], -1, { index: 0, t: 0, s: 0, lateral: 0, y: 0, tx: 0, tz: 1, distance: 0 });
  r.x = pt[0]; r.z = pt[2]; r.y = track.ground(pt[0], pt[2], near); r.yaw = pt[3] + yawOffset;
  r.speed = speed; r.vx = Math.sin(r.yaw) * speed; r.vz = Math.cos(r.yaw) * speed; r.vy = 0; r.grounded = true;
  r.hint = -1; r.lastS = near.s; r.travel = s; r.spin = 0; r.boost = 0; r.drift = 0; r.driftCharge = 0; r.driftLevel = 0;
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
  // Item boxes: five per row.
  assert.equal(s.boxes.length, track.def.itemRows.length * 5);
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

test('rocket start: throttle held from around "1" boosts at GO; early, late or no throttle does not', () => {
  const s = createRace('meadow', [human(0), human(1), human(2), human(3)], 'normal', 4);
  while (s.phase === 'intro') tick(s, []);
  while (s.phase === 'countdown') {
    const cd = s.countdown;
    tick(s, [press({ throttle: 1 }), press({ throttle: cd <= 1.0 ? 1 : 0 }), press({ throttle: cd <= 0.15 ? 1 : 0 }), idleInput()]);
  }
  const [early, timed, late, idle] = s.racers;
  assert.equal(timed.boost, 1.1);
  assert.equal(timed.boostPower, 1.35);
  for (const r of [early, late, idle]) assert.equal(r.boost, 0, `racer ${r.id}`);
  assert.deepEqual(events(s, 'boost').map(e => e.racer), [timed.id]);
  // The boost carries the timed racer ahead.
  run(s, 1.5, [press({ throttle: 1 }), press({ throttle: 1 }), press({ throttle: 1 }), press({ throttle: 1 })]);
  assert.ok(timed.speed > early.speed && timed.speed > late.speed);
});

test('rocket start: CPUs time the throttle and get the GO boost', () => {
  const s = createRace('meadow', kartSeats([], true), 'hard', 7);
  toRace(s);
  assert.ok(s.racers.some(r => r.boost > 0), 'at least one CPU rocket-starts');
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

test('bomb: a homing shell chases the racer ahead and explodes', () => {
  const s = humansRace(3);
  const [shooter, target, bystander] = s.racers;
  put(s, shooter, 100, 0); put(s, target, 125, 3); put(s, bystander, 400, 0);
  tick(s, []);
  assert.equal(target.place, shooter.place - 1);
  shooter.item = 'bomb'; shooter.itemCount = 1;
  tick(s, [press({ item: true })]);
  assert.equal(s.shells.length, 1);
  assert.equal(s.shells[0].target, target.id);
  assert.equal(events(s, 'throw').length, 1);
  for (let i = 0; i < 120 && s.shells.length; i++) tick(s, []);
  assert.equal(s.shells.length, 0, 'shell exploded');
  assert.ok(events(s, 'explode').length === 1);
  assert.ok(target.spin > 0, 'target spun');
  assert.equal(shooter.spin, 0);
  assert.equal(bystander.spin, 0);
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
  s.hazards.push({ id: 999, x: a.x, y: a.y, z: a.z, owner: b.id });
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

test('bolt: hits only racers ahead of the user', () => {
  const s = humansRace(3);
  const [front, user, back] = s.racers;
  put(s, front, 300, 0); put(s, user, 200, 0); put(s, back, 100, 0);
  tick(s, []);
  assert.deepEqual([front.place, user.place, back.place], [1, 2, 3]);
  user.item = 'bolt'; user.itemCount = 1;
  tick(s, [undefined, press({ item: true })]);
  assert.ok(front.spin > 0 && Math.abs(front.slow - 3) < 1e-9);
  assert.equal(back.spin, 0); assert.equal(back.slow, 0);
  assert.equal(user.spin, 0); assert.equal(user.slow, 0);
  assert.equal(events(s, 'bolt').length, 1);
});

test('triple mushroom gives three boosts, one per press', () => {
  const s = humansRace(1);
  const r = s.racers[0];
  put(s, r, 60, 0, 20);
  r.item = 'triple'; r.itemCount = 3;
  for (let k = 3; k > 0; k--) {
    tick(s, [press({ throttle: 1, item: true })]);
    assert.equal(r.itemCount, k - 1);
    assert.ok(r.boost > 1.1);
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
    assert.ok(Math.abs(r.boostPower - (1.28 + level * 0.04)) < 1e-9);
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
    // Throttle is ignored while waiting.
    run(s, 1.5, [press({ throttle: 1 })]);
    assert.equal(r.respawn <= 0, true);
    const near = nearest(t, r.x, r.z);
    assert.ok(Math.abs(near.lateral) < 1, `${track}: back on the centre line (${near.lateral})`);
    assert.ok(events(s, 'land').length >= 1);
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
      assert.ok(s.raceTime < 240, `${track}/${difficulty}: ${s.raceTime.toFixed(1)} s`);
      const t = trackFor(track);
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
