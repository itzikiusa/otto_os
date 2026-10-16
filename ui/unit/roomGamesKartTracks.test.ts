import { test } from 'node:test';
import assert from 'node:assert/strict';
import { Rng } from '../src/modules/rooms/games/common/math.ts';
import { nearest, pointAt, RAMP_HEIGHT, RAMP_LENGTH, rampHeight, sampleAt, TRACK_DEFS, TRACK_IDS, trackFor } from '../src/modules/rooms/games/kart/tracks.ts';
import { ITEM_KINDS, itemWeights, rollItem, type ItemKind } from '../src/modules/rooms/games/kart/items.ts';
import { RACER_IDS, RACERS } from '../src/modules/rooms/games/kart/roster.ts';
import { kartSeats } from '../src/modules/rooms/games/setup.ts';

/** Circular difference of two lap distances, in (-L/2, L/2]. */
const ds = (a: number, b: number, L: number): number => {
  let d = (a - b) % L;
  if (d > L / 2) d -= L;
  if (d <= -L / 2) d += L;
  return d;
};

test('trackFor falls back to meadow for unknown ids and caches each track', () => {
  assert.equal(trackFor('nope').def.id, 'meadow');
  assert.equal(trackFor('meadow'), trackFor('meadow'));
  assert.deepEqual([...TRACK_IDS].sort(), ['canyon', 'downtown', 'frost', 'meadow']);
});

test('every track is a closed loop sampled every ~2 m with matching distances', () => {
  for (const id of TRACK_IDS) {
    const t = trackFor(id);
    assert.ok(t.count > 300, `${id}: ${t.count} samples`);
    assert.ok(Math.abs(t.length / t.count - 2) < 0.05, `${id}: spacing ${t.length / t.count}`);
    assert.equal(t.halfWidth, t.def.width / 2);
    for (let i = 0; i < t.count; i++) {
      const j = (i + 1) % t.count;
      // Includes the wrap from the last sample back to the start line: the loop is closed.
      const step = Math.hypot(t.px[j] - t.px[i], t.pz[j] - t.pz[i]);
      assert.ok(step > 1.9 && step < 2.1, `${id}: step ${i}->${j} = ${step}`);
      assert.ok(Math.abs(Math.hypot(t.tx[i], t.tz[i]) - 1) < 1e-9, `${id}: unit tangent at ${i}`);
      assert.ok(Number.isFinite(t.py[i]) && Number.isFinite(t.curve[i]));
      if (i > 0) assert.ok(t.dist[i] > t.dist[i - 1]);
    }
    assert.equal(t.dist[0], 0);
    // The start line sits at the first control point.
    const [x0, z0] = t.def.points[0];
    assert.ok(Math.hypot(t.px[0] - x0, t.pz[0] - z0) < 1e-6, `${id}: start line`);
  }
});

test('no track crosses itself: non-adjacent stretches stay more than a road width apart', () => {
  for (const id of TRACK_IDS) {
    const t = trackFor(id);
    const n = t.count, spacing = t.length / n;
    let min = Infinity;
    for (let i = 0; i < n; i++) for (let j = i + 1; j < n; j++) {
      const sep = Math.min(j - i, n - (j - i)) * spacing;
      if (sep < 3 * t.def.width) continue; // neighbours along the same stretch
      min = Math.min(min, Math.hypot(t.px[i] - t.px[j], t.pz[i] - t.pz[j]));
    }
    assert.ok(min > t.def.width, `${id}: closest non-adjacent samples ${min.toFixed(1)} m vs width ${t.def.width}`);
    // Even the tightest corner leaves the inside edge a positive radius.
    let maxCurve = 0;
    for (let i = 0; i < n; i++) maxCurve = Math.max(maxCurve, Math.abs(t.curve[i]));
    assert.ok(1 / maxCurve > t.halfWidth, `${id}: tightest radius ${(1 / maxCurve).toFixed(1)}`);
  }
});

test('pointAt and nearest round-trip distance and lateral offset', () => {
  const rng = new Rng(42);
  for (const id of TRACK_IDS) {
    const t = trackFor(id);
    for (let k = 0; k < 200; k++) {
      const s = rng.range(0, t.length), lat = rng.range(-t.halfWidth, t.halfWidth);
      // On the centre line the round trip is exact.
      const c = pointAt(t, s, 0), cn = nearest(t, c[0], c[2]);
      assert.ok(Math.abs(ds(cn.s, s, t.length)) < 1e-6 && Math.abs(cn.lateral) < 1e-6, `${id}: centre s ${s} -> ${cn.s}`);
      // Offset points: the offset uses the sample's tangent, so on tight corners the
      // projected distance can drift by up to one sample (2 m); lateral stays tight.
      const pt = pointAt(t, s, lat);
      const near = nearest(t, pt[0], pt[2]);
      assert.ok(Math.abs(ds(near.s, s, t.length)) < 2, `${id}: s ${s} -> ${near.s}`);
      assert.ok(Math.abs(near.lateral - lat) < 0.2, `${id}: lateral ${lat} -> ${near.lateral}`);
      assert.ok(near.distance - Math.abs(near.lateral) < 0.2);
      // The windowed search around the hint agrees with the full search.
      const hinted = nearest(t, pt[0], pt[2], sampleAt(t, s) + 3, { index: 0, t: 0, s: 0, lateral: 0, y: 0, tx: 0, tz: 1, distance: 0 });
      assert.ok(Math.abs(hinted.s - near.s) < 1e-9 && Math.abs(hinted.lateral - near.lateral) < 1e-9);
    }
    // s wraps: one lap further lands on the same spot.
    const a = pointAt(t, 10, 2), b = pointAt(t, 10 + t.length, 2), c = pointAt(t, 10 - t.length, 2);
    assert.ok(Math.hypot(a[0] - b[0], a[2] - b[2]) < 1e-6 && Math.hypot(a[0] - c[0], a[2] - c[2]) < 1e-6);
  }
});

test('lateral sign: positive is left of travel and the inside of left turns', () => {
  for (const id of TRACK_IDS) {
    const t = trackFor(id);
    for (let i = 0; i < t.count; i += 7) {
      const s = t.dist[i];
      const c = pointAt(t, s, 0), l = pointAt(t, s, 3);
      const fx = Math.sin(c[3]), fz = Math.cos(c[3]);
      // Y up, facing (fx, fz): left is (fz, -fx) — e.g. facing +Z, left is +X.
      const ox = l[0] - c[0], oz = l[2] - c[2];
      assert.ok(Math.abs(ox * fz - oz * fx - 3) < 1e-6, `${id}@${i}: offset is not to the left`);
    }
    // On the sharpest left-hander, the left (positive) lane is the shorter path.
    let best = 0;
    for (let i = 0; i < t.count; i++) if (t.curve[i] > t.curve[best]) best = i;
    assert.ok(t.curve[best] > 0, `${id} has a left turn`);
    const len = (lat: number) => {
      let sum = 0;
      for (let k = -6; k < 6; k++) {
        const a = pointAt(t, t.dist[best] + k * 2, lat), b = pointAt(t, t.dist[best] + (k + 1) * 2, lat);
        sum += Math.hypot(b[0] - a[0], b[2] - a[2]);
      }
      return sum;
    };
    assert.ok(len(5) < len(0) && len(0) < len(-5), `${id}: inside of a left turn is positive lateral`);
  }
});

test('rampHeight rises over RAMP_LENGTH metres to the lip and is zero elsewhere', () => {
  const t = trackFor('canyon');
  for (const at of t.def.ramps) {
    const lip = at * t.length;
    assert.equal(rampHeight(t.def, t.length, lip), RAMP_HEIGHT);
    assert.ok(Math.abs(rampHeight(t.def, t.length, lip - RAMP_LENGTH / 2) - RAMP_HEIGHT / 2) < 1e-9);
    assert.equal(rampHeight(t.def, t.length, lip - RAMP_LENGTH - 0.01), 0);
    assert.equal(rampHeight(t.def, t.length, lip + 0.5), 0);
    // Rises monotonically toward the lip.
    let prev = -1;
    for (let d = RAMP_LENGTH - 0.1; d >= 0; d -= 0.5) {
      const h = rampHeight(t.def, t.length, lip - d);
      assert.ok(h > prev);
      prev = h;
    }
  }
  assert.equal(rampHeight(t.def, t.length, 1), 0);
  // The ground on the road includes the ramp; off-road it does not.
  const lip = t.def.ramps[0] * t.length - 1;
  const p = pointAt(t, lip, 0);
  const near = nearest(t, p[0], p[2]);
  assert.ok(Math.abs(t.ground(p[0], p[2], near) - (near.y + rampHeight(t.def, t.length, near.s))) < 1e-9);
  assert.ok(rampHeight(t.def, t.length, near.s) > 1);
});

test('sampleAt maps any distance (including negative / multi-lap) to a valid sample', () => {
  const t = trackFor('frost');
  for (const s of [0, 1.9, 2.1, -1, t.length, t.length * 2.5, -t.length * 3.2]) {
    const i = sampleAt(t, s);
    assert.ok(Number.isInteger(i) && i >= 0 && i < t.count, `${s} -> ${i}`);
  }
  assert.equal(sampleAt(t, 2.1), 1);
  assert.equal(sampleAt(t, -0.5), t.count - 1);
});

test('track definitions are internally consistent', () => {
  for (const id of TRACK_IDS) {
    const d = TRACK_DEFS[id];
    assert.equal(d.id, id);
    assert.ok(d.itemRows.length > 0 && d.itemRows.every(f => f > 0 && f < 1));
    assert.ok(d.ramps.every(f => f > 0 && f < 1));
    assert.ok(d.boostPads.every(p => p.at > 0 && p.at < 1 && Math.abs(p.lane) <= 1));
    if (d.edge === 'grass') assert.ok(d.outOfBounds > 0);
  }
});

test('itemWeights: the leader never gets a star, bolt or triple', () => {
  const w = itemWeights(1, 8);
  assert.equal(w.star, 0);
  assert.equal(w.bolt, 0);
  assert.equal(w.triple, 0);
  assert.ok(w.banana > w.mushroom);
  // A single racer counts as the front.
  assert.deepEqual(itemWeights(1, 1), w);
});

test('itemWeights: the back of the pack gets more catch-up items', () => {
  const front = itemWeights(1, 8), back = itemWeights(8, 8);
  for (const k of ['star', 'bolt', 'triple'] as ItemKind[]) assert.ok(back[k] > front[k], k);
  assert.ok(back.banana < front.banana);
  assert.deepEqual(back, { mushroom: 14, triple: 28, banana: 4, bomb: 16, star: 24, bolt: 14 });
});

test('itemWeights interpolate front -> middle -> back', () => {
  const front = itemWeights(1, 5), quarter = itemWeights(2, 5), mid = itemWeights(3, 5), threeQ = itemWeights(4, 5), back = itemWeights(5, 5);
  for (const k of ITEM_KINDS) {
    assert.ok(Math.abs(quarter[k] - (front[k] + mid[k]) / 2) < 1e-9, `${k} at 1/4`);
    assert.ok(Math.abs(threeQ[k] - (mid[k] + back[k]) / 2) < 1e-9, `${k} at 3/4`);
  }
  assert.deepEqual(mid, { mushroom: 26, triple: 14, banana: 18, bomb: 30, star: 9, bolt: 3 });
  for (let place = 1; place <= 8; place++) {
    const w = itemWeights(place, 8);
    assert.ok(ITEM_KINDS.every(k => w[k] >= 0));
    assert.ok(ITEM_KINDS.reduce((a, k) => a + w[k], 0) > 0);
  }
});

class FixedRng extends Rng {
  value: number;
  constructor(value: number) { super(1); this.value = value; }
  override next(): number { return this.value; }
}

test('rollItem respects the weights', () => {
  assert.equal(rollItem(new FixedRng(0), 1, 8), 'mushroom');
  // The top of the range never lands on a zero-weight item for the leader.
  assert.equal(rollItem(new FixedRng(0.999999), 1, 8), 'bomb');
  assert.equal(rollItem(new FixedRng(0.999999), 8, 8), 'bolt');
  const rng = new Rng(7);
  const count = (place: number) => {
    const c: Record<string, number> = {};
    for (let i = 0; i < 4000; i++) { const k = rollItem(rng, place, 8); c[k] = (c[k] ?? 0) + 1; }
    return c;
  };
  const front = count(1), back = count(8);
  assert.equal(front.star ?? 0, 0);
  assert.equal(front.bolt ?? 0, 0);
  assert.equal(front.triple ?? 0, 0);
  const catchUp = (c: Record<string, number>) => (c.star ?? 0) + (c.bolt ?? 0) + (c.triple ?? 0);
  assert.ok(catchUp(back) > 1500, `back catch-up rolls ${catchUp(back)}`);
  // ~4% bananas at the back vs ~50% at the front.
  assert.ok((back.banana ?? 0) < 400 && (front.banana ?? 0) > 1700);
});

test('kartSeats: CPU fill to 8 with distinct racers first', () => {
  const seats = kartSeats([], true);
  assert.equal(seats.length, 8);
  assert.equal(new Set(seats.map(s => s.racer)).size, 8);
  assert.ok(seats.every(s => !s.human && s.slot === -1 && s.name.length > 0));
  assert.deepEqual(seats.map(s => s.name), RACERS.map(r => r.name));

  const two = kartSeats([{ slot: 3, name: 'Ann', pick: 'kai' }, { slot: 5, name: 'Bo', pick: 'kai' }], true);
  assert.equal(two.length, 8);
  assert.deepEqual(two.slice(0, 2).map(s => [s.racer, s.human, s.slot, s.name]), [['kai', true, 3, 'Ann'], ['kai', true, 5, 'Bo']]);
  const cpus = two.slice(2);
  assert.ok(cpus.every(s => !s.human && s.slot === -1));
  assert.equal(new Set(cpus.map(s => s.racer)).size, 6);
  assert.ok(!cpus.some(s => s.racer === 'kai'), 'untaken racers are used first');
});

test('kartSeats: invalid picks fall back, no fill without cpuFill, caps at total', () => {
  const solo = kartSeats([{ slot: 0, name: 'X', pick: 'bowser' }], false);
  assert.deepEqual(solo, [{ racer: 'oobi', human: true, slot: 0, name: 'X' }]);
  const many = kartSeats(Array.from({ length: 10 }, (_, i) => ({ slot: i, name: `P${i}`, pick: RACER_IDS[i % 8] })), true);
  assert.equal(many.length, 8);
  assert.ok(many.every(s => s.human));
  const four = kartSeats([{ slot: 0, name: 'A', pick: 'dex' }], true, 4);
  assert.equal(four.length, 4);
  assert.equal(new Set(four.map(s => s.racer)).size, 4);
  // More CPUs than racers: the pool wraps.
  const wrap = kartSeats([], true, 8);
  assert.ok(wrap.every(s => RACER_IDS.includes(s.racer)));
});
