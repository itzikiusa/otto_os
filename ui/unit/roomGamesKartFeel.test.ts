/**
 * Kart Grand Prix items and driving feel: long strong mushrooms (triple and
 * golden too) that blast through off-road, green / red / blue shells and the
 * fuse bomb, lightning that never touches the user, nothing a racer fires or
 * drops ever hurting it, mini-turbo tiers that grow, slipstreams, trick
 * boosts, and quick fair rescues.
 */
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { clamp, wrapAngle } from '../src/modules/rooms/games/common/math.ts';
import { advance, nearest, newNear, pointAt, progressOf, sampleAt, trackFor, type TrackId } from '../src/modules/rooms/games/kart/tracks.ts';
import {
  BANANA_GRACE, createRace, DRAFT_SECONDS, GHOST_SECONDS, GOLDEN_SECONDS, GREEN_GRACE, idleInput, MUSHROOM_POWER, MUSHROOM_SECONDS, RESCUE_SECONDS, RESPAWN_BOOST, SHELL, spinOut, STEP, tick,
  TRICK_POWER, TRICK_SECONDS, TURBO_POWER, TURBO_SECONDS, type KartEventType, type KartInput, type KartState, type Racer,
} from '../src/modules/rooms/games/kart/sim.ts';
import { ITEM_KINDS, ITEMS, type ItemKind } from '../src/modules/rooms/games/kart/items.ts';
import { racerDef } from '../src/modules/rooms/games/kart/roster.ts';

const press = (k: Partial<KartInput> = {}): KartInput => ({ ...idleInput(), ...k });
const events = (s: KartState, type: KartEventType, from = 0) => s.events.filter(e => e.id > from && e.type === type);
/** Top speed of the all-rounder (oobi). */
const TOP = 29 + (racerDef('oobi').speed - 3) * 0.9;

function race(count: number, track: TrackId = 'meadow'): KartState {
  const s = createRace(track, Array.from({ length: count }, (_, i) => ({ racer: 'oobi' as const, human: true, slot: i, name: `P${i}` })), 'normal', 99);
  while (s.phase !== 'race') tick(s, []);
  return s;
}
function put(s: KartState, r: Racer, at: number, lateral: number, speed = 0, yawOffset = 0, seg = 0): void {
  const t = trackFor(s.track), path = t.paths[seg];
  const p = pointAt(path, at, lateral), near = nearest(path, p[0], p[2], sampleAt(path, at), newNear());
  r.x = p[0]; r.z = p[2]; r.y = t.ground(p[0], p[2], near); r.yaw = p[3] + yawOffset;
  r.speed = speed; r.vx = Math.sin(r.yaw) * speed; r.vz = Math.cos(r.yaw) * speed; r.vy = 0; r.grounded = true; r.glide = false;
  r.seg = seg; r.segS = near.s; r.safeSeg = seg; r.safeS = near.s; r.hint = -1; r.lastS = progressOf(t, seg, near.s); r.travel = r.lastS;
  r.spin = 0; r.boost = 0; r.drift = 0; r.driftCharge = 0; r.driftLevel = 0; r.ghost = 0; r.slow = 0;
}
/** Steers for the centre line (plus `lane`) ahead, gas down. */
function pilot(s: KartState, r: Racer, extra: Partial<KartInput> = {}, lane = 0): KartInput {
  const t = trackFor(s.track);
  const spot = advance(t, r.seg, r.segS, 7 + Math.max(0, r.speed) * 0.3, 0, { seg: 0, s: 0 });
  const p = pointAt(t.paths[spot.seg], spot.s, lane);
  const angle = wrapAngle(Math.atan2(p[0] - r.x, p[2] - r.z) - r.yaw);
  return press({ throttle: 1, steer: clamp(-angle * 2.4, -1, 1), ...extra });
}
const run = (s: KartState, seconds: number, inputs: (s: KartState) => (KartInput | undefined)[]) => {
  for (let i = 0, n = Math.round(seconds / STEP); i < n; i++) tick(s, inputs(s));
};

test('turbo mushroom: a 1.5 s strong burst felt at once, carrying a racer well clear of an unboosted twin', () => {
  const s = race(2);
  const [a, b] = s.racers;
  put(s, a, 60, 4, 20); put(s, b, 60, -4, 20);
  a.item = 'mushroom'; a.itemCount = 1;
  const seq = s.eventSeq, before = a.speed;
  tick(s, [pilot(s, a, { item: true }, 4), pilot(s, b, {}, -4)]);
  assert.equal(a.boostPower, MUSHROOM_POWER);
  assert.ok(Math.abs(a.boost - MUSHROOM_SECONDS) < 1e-9);
  assert.ok(a.speed > before + 8, `instant shove: ${before} -> ${a.speed.toFixed(1)}`);
  assert.deepEqual(events(s, 'boost', seq).map(e => e.value), [1]);
  const t0 = [a.travel, b.travel];
  let peak = 0;
  run(s, 2.5, x => { peak = Math.max(peak, a.speed); return [pilot(x, a, {}, 4), pilot(x, b, {}, -4)]; });
  assert.ok(peak > TOP * 1.45, `peak ${peak.toFixed(1)} vs top ${TOP}`);
  assert.ok(a.travel - t0[0] > b.travel - t0[1] + 18, `gain ${(a.travel - t0[0] - (b.travel - t0[1])).toFixed(1)} m`);
  assert.equal(a.item, null);
});

test('boosts blast through off-road: a mushroom (or a star) ignores the grass a plain kart bogs down in', () => {
  for (const kind of ['mushroom', 'star'] as ItemKind[]) {
    const s = race(2);
    const t = trackFor('meadow');
    const [a, b] = s.racers;
    // Side by side on the grass beside a straight, well short of the rescue line.
    put(s, a, 60, t.halfWidth + 6, 24); put(s, b, 60, -t.halfWidth - 6, 24);
    a.item = kind; a.itemCount = 1;
    tick(s, [pilot(s, a, { item: true }, t.halfWidth + 6), pilot(s, b, {}, -t.halfWidth - 6)]);
    run(s, 1, x => [pilot(x, a, {}, t.halfWidth + 6), pilot(x, b, {}, -t.halfWidth - 6)]);
    assert.equal(a.surface, 'grass'); assert.equal(b.surface, 'grass');
    assert.ok(a.speed > TOP, `${kind}: ${a.speed.toFixed(1)} on grass`);
    assert.ok(b.speed < TOP * 0.6, `plain: ${b.speed.toFixed(1)} on grass`);
  }
});

test('golden mushroom: every press inside its 7.5 s window is another full mushroom, then it is used up', () => {
  const s = race(1);
  const r = s.racers[0];
  put(s, r, 40, 0, 20);
  r.item = 'golden'; r.itemCount = 1;
  const seq = s.eventSeq;
  for (let k = 0; k < 5; k++) {
    tick(s, [pilot(s, r, { item: true })]);
    assert.ok(r.boost > MUSHROOM_SECONDS - 0.02, `press ${k}`);
    assert.equal(r.item, 'golden');
    run(s, 0.8, x => [pilot(x, r)]);
  }
  assert.deepEqual(events(s, 'boost', seq).filter(e => e.value === 3).length, 5);
  assert.ok(r.golden > 0 && r.golden < GOLDEN_SECONDS);
  run(s, GOLDEN_SECONDS, x => [pilot(x, r)]);
  assert.equal(r.item, null);
  assert.equal(r.golden, 0);
});

test('green shell: flies straight, bounces off walls, and never hits its thrower during the grace', () => {
  // Fired straight at the barrier on a walled street, it comes straight back.
  const s = race(1, 'downtown');
  const r = s.racers[0];
  put(s, r, 120, 0, 0, Math.PI / 2);
  r.item = 'green'; r.itemCount = 1;
  tick(s, [press({ item: true })]);
  assert.equal(s.shells.length, 1);
  assert.equal(s.shells[0].kind, 'green');
  const yaw0 = s.shells[0].yaw;
  let bounced = false;
  for (let i = 0; i < Math.round(GREEN_GRACE / STEP) - 2 && s.shells.length; i++) {
    tick(s, [press()]);
    if (s.shells[0] && Math.abs(wrapAngle(s.shells[0].yaw - yaw0)) > 2) bounced = true;
  }
  assert.ok(bounced, 'bounced back off the barrier');
  assert.equal(r.spin, 0, 'the thrower is spared while the shell is fresh');
  // In line with a rival 20 m ahead: a direct hit.
  const s2 = race(2, 'downtown');
  const [a, b] = s2.racers;
  put(s2, a, 100, 0, 0); put(s2, b, 120, 0, 0);
  a.item = 'green'; a.itemCount = 1;
  tick(s2, [press({ item: true }), press()]);
  run(s2, 1, () => [press(), press()]);
  assert.ok(b.spin > 0 || events(s2, 'spin').some(e => e.racer === b.id), 'the rival was hit');
  assert.equal(a.spin, 0);
  assert.equal(events(s2, 'explode').at(-1)?.value, 0);
});

test('blue shell: flies over the pack to whoever leads and blasts them; never its owner, even from first place', () => {
  const s = race(4);
  const [owner, mid, other, leader] = s.racers;
  put(s, leader, 420, 0, 0); put(s, mid, 300, 0, 0); put(s, other, 200, 2, 0); put(s, owner, 120, 0, 0);
  tick(s, []);
  assert.equal(leader.place, 1);
  owner.item = 'blue'; owner.itemCount = 1;
  tick(s, [press({ item: true })]);
  assert.equal(s.shells[0].kind, 'blue');
  assert.equal(s.shells[0].target, leader.id);
  let maxY = -Infinity;
  for (let i = 0; i < 60 * 10 && s.shells.length; i++) {
    tick(s, []);
    const sh = s.shells[0];
    if (sh) maxY = Math.max(maxY, sh.y - leader.y);
  }
  assert.equal(s.shells.length, 0);
  assert.ok(maxY > 4, `flew high (${maxY.toFixed(1)} m)`);
  assert.ok(leader.spin > 0 || events(s, 'spin').some(e => e.racer === leader.id), 'leader blasted');
  for (const r of [mid, other, owner]) assert.ok(!events(s, 'spin').some(e => e.racer === r.id), `racer ${r.id} untouched`);
  assert.equal(events(s, 'explode').at(-1)?.value, 2);
  // Fired from first place it goes for second, and the blast spares its owner right beside them.
  const s2 = race(2);
  const [first, second] = s2.racers;
  put(s2, first, 300, 2, 0); put(s2, second, 296, -2, 0);
  tick(s2, []);
  assert.equal(first.place, 1);
  first.item = 'blue'; first.itemCount = 1;
  tick(s2, [press({ item: true }), press()]);
  assert.equal(s2.shells[0]?.target ?? second.id, second.id);
  for (let i = 0; i < 60 * 10 && s2.shells.length; i++) tick(s2, []);
  assert.ok(events(s2, 'spin').some(e => e.racer === second.id), 'second place blasted');
  assert.ok(!events(s2, 'spin').some(e => e.racer === first.id), 'owner spared');
});

test('fuse bomb: lobbed ahead, it goes off when its fuse runs out; the blast catches everyone near but its thrower', () => {
  const s = race(2);
  const [a, b] = s.racers;
  put(s, a, 100, 0, 0); put(s, b, 300, 0, 0);
  a.item = 'bomb'; a.itemCount = 1;
  tick(s, [press({ item: true }), press()]);
  const bomb = s.shells[0];
  assert.equal(bomb.kind, 'bomb');
  assert.ok(bomb.vy > 0, 'lobbed');
  // It lands a few kart lengths ahead and rolls to a stop; then the rival stands beside it (not touching) and so does the thrower.
  run(s, 1.4, () => [press(), press()]);
  assert.ok(s.shells.length === 1 && s.shells[0].speed < 3, 'rolled to a stop');
  const at = s.shells[0];
  const d = Math.hypot(at.x - a.x, at.z - a.z);
  assert.ok(d > 8 && d < 40, `landed ${d.toFixed(1)} m ahead`);
  const n = nearest(trackFor('meadow'), at.x, at.z);
  put(s, b, n.s, n.lateral + 3.5, 0); put(s, a, n.s, n.lateral - 3.5, 0);
  run(s, SHELL.bomb.life, () => [press(), press()]);
  assert.equal(s.shells.length, 0);
  assert.equal(events(s, 'explode').at(-1)?.value, 3);
  assert.ok(events(s, 'spin').some(e => e.racer === b.id), 'the rival beside it spun');
  assert.ok(!events(s, 'spin').some(e => e.racer === a.id), 'the thrower is spared');
});

test('banana: its dropper can back straight over it for a moment; after that it trips anyone', () => {
  const s = race(2);
  const [a, b] = s.racers;
  put(s, a, 100, 0, 0); put(s, b, 300, 0, 0);
  a.item = 'banana'; a.itemCount = 1;
  tick(s, [press({ item: true }), press()]);
  assert.equal(s.hazards.length, 1);
  run(s, BANANA_GRACE - 0.3, () => [press({ throttle: -1 }), press()]);
  assert.equal(a.spin, 0, 'reversed over its own banana unharmed');
  assert.equal(s.hazards.length, 1);
  const h = s.hazards[0];
  put(s, b, 0, 0, 0);
  b.x = h.x; b.z = h.z; b.y = h.y;
  tick(s, [press(), press()]);
  assert.ok(b.spin > 0, 'a rival slips');
  assert.equal(s.hazards.length, 0);
});

test('nothing a racer fires or drops ever hurts it, whatever the item and however close the pack', () => {
  for (const kind of ITEM_KINDS.filter(k => k !== 'green')) {
    for (const userPlace of [1, 3]) {
      const s = race(5);
      const user = s.racers[0], rest = s.racers.slice(1);
      // The pack packed tight around the user: two just ahead, two just behind (or all behind from first).
      const offsets = userPlace === 1 ? [-4, -8, -12, -16] : [6, 3.5, -4, -8];
      put(s, user, 200, 0, 0);
      rest.forEach((o, k) => put(s, o, 200 + offsets[k], k % 2 ? 3 : -3, 0));
      tick(s, []);
      user.item = kind; user.itemCount = kind === 'triple' ? 3 : 1;
      const seq = s.eventSeq;
      tick(s, [press({ item: true })]);
      for (let i = 0; i < 60 * 4; i++) tick(s, [press({ item: i % 20 === 0 })]);
      const own = events(s, 'spin', seq).filter(e => e.racer === user.id);
      assert.equal(own.length, 0, `${kind} from place ${userPlace}: the user spun`);
      assert.equal(user.slow, 0, `${kind}: the user was slowed`);
    }
  }
});

test('mini-turbo tiers: blue, orange, purple sparks each fire a longer, stronger boost that carries further', () => {
  assert.ok(TURBO_SECONDS[0] < TURBO_SECONDS[1] && TURBO_SECONDS[1] < TURBO_SECONDS[2]);
  assert.ok(TURBO_POWER[0] < TURBO_POWER[1] && TURBO_POWER[1] < TURBO_POWER[2]);
  const gains: number[] = [];
  for (const level of [0, 1, 2, 3]) {
    const s = race(1);
    const r = s.racers[0];
    put(s, r, 40, 0, 24);
    // Mid-drift at `level`, then let go of the drift button on the straight.
    r.drift = 1; r.driftLevel = level; r.driftCharge = 3;
    const seq = s.eventSeq, t0 = r.travel;
    tick(s, [pilot(s, r)]);
    if (level) assert.deepEqual(events(s, 'turbo', seq).map(e => e.value), [level]);
    run(s, 3, x => [pilot(x, r)]);
    gains.push(r.travel - t0);
  }
  for (let k = 1; k < gains.length; k++) assert.ok(gains[k] > gains[k - 1] + 2, `tier ${k}: ${gains.map(g => g.toFixed(1)).join(' < ')}`);
});

test('slipstream: tucked in a rival\'s wake for a second fires a draft boost; alongside does not', () => {
  for (const lane of [0, 5]) {
    const s = race(2);
    const [front, back] = s.racers;
    put(s, front, 40, 0, 26); put(s, back, 30, lane, 26);
    const seq = s.eventSeq;
    run(s, DRAFT_SECONDS + 0.5, x => [pilot(x, front), pilot(x, back, {}, lane)]);
    const drafts = events(s, 'draft', seq);
    if (lane === 0) {
      assert.deepEqual(drafts.map(e => e.racer), [back.id], 'the follower drafts');
      assert.ok(back.speed > front.speed, 'and slingshots past');
    } else assert.equal(drafts.length, 0, 'side by side: no draft');
  }
});

test('trick off a kicker ramp: a trick in the air lands with a boost; no trick, no boost', () => {
  const t = trackFor('meadow'), at = t.def.ramps[0] * t.length;
  for (const trick of [true, false]) {
    const s = race(1);
    const r = s.racers[0];
    put(s, r, at - 30, 0, 28);
    let tricked = false, landed = false;
    for (let i = 0; i < 60 * 4 && !landed; i++) {
      const wasAir = !r.grounded;
      const doTrick = trick && !tricked && !r.grounded && r.trickReady && r.air > 0.15;
      if (doTrick) tricked = true;
      tick(s, [pilot(s, r, { drift: doTrick })]);
      if (wasAir && r.grounded && r.air > 0.3) landed = true;
    }
    assert.ok(landed, 'jumped and landed');
    if (trick) { assert.ok(Math.abs(r.boost - TRICK_SECONDS) < 0.05 && r.boostPower === TRICK_POWER, `landing boost ${r.boost.toFixed(2)}`); }
    else assert.equal(r.boost, 0);
  }
});

test('rescue: about a second, back on the road with the item kept, untouchable for a moment; gas on landing boosts', () => {
  for (const gas of [false, true]) {
    const s = race(2);
    const t = trackFor('meadow');
    const [r, other] = s.racers;
    put(s, other, 600, 0, 0);
    put(s, r, 200, 0, 28, Math.PI / 2);
    r.item = 'red'; r.itemCount = 1;
    let n = 0;
    while (r.respawn <= 0 && n++ < 600) tick(s, [press({ throttle: 1 }), press()]);
    assert.ok(r.respawn > 0 && r.respawn <= RESCUE_SECONDS, 'rescue started');
    let wait = 0;
    while (r.respawn > 0 && wait++ < 200) tick(s, [press({ throttle: gas ? 1 : 0 }), press()]);
    assert.ok(wait * STEP <= RESCUE_SECONDS + 0.05, `waited ${(wait * STEP).toFixed(2)} s`);
    assert.ok(Math.abs(nearest(t, r.x, r.z).lateral) < 1, 'on the centre line');
    assert.equal(r.item, 'red', 'kept its item');
    assert.equal(r.ghost, GHOST_SECONDS);
    assert.equal(r.boost, gas ? RESPAWN_BOOST : 0);
    // A banana right where it lands, a shell blast: shrugged off while the moment lasts.
    s.hazards.push({ id: 7777, x: r.x, y: r.y, z: r.z, owner: other.id, age: 9 });
    tick(s, [press(), press()]);
    assert.equal(r.spin, 0);
    assert.equal(spinOut(s, r), false);
    run(s, GHOST_SECONDS, x => [pilot(x, r, { throttle: 0 }), press()]);
    assert.equal(r.ghost, 0);
    assert.equal(spinOut(s, r), true, 'vulnerable again afterwards');
  }
});

test('every item has a HUD name, hint and an existing portrait icon', () => {
  const portraits = new Set(['banana', 'bomb', 'box', 'mushroom', 'star']);
  for (const k of ITEM_KINDS) {
    assert.ok(ITEMS[k].name && ITEMS[k].hint, k);
    assert.ok(portraits.has(ITEMS[k].icon), `${k}: icon ${ITEMS[k].icon}`);
  }
  // Wire order is append-only: the round-1/2 items keep their indices.
  assert.deepEqual(ITEM_KINDS.slice(0, 6), ['mushroom', 'triple', 'banana', 'bomb', 'star', 'bolt']);
});
