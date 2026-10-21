import { test } from 'node:test';
import assert from 'node:assert/strict';
import { Rng } from '../src/modules/rooms/games/common/math.ts';
import {
  advance, CRUSHER_RISE, F_BRIDGE, F_GAP, F_RAILS, F_TUNNEL, F_UNDER, inLaunchZone, inOffroad, locate, nearest, newNear, obstacleAt, pointAt, progressOf,
  RAMP_HEIGHT, RAMP_LENGTH, rampHeight, rampOn, ROLLER_RADIUS, RUN_UP, sampleAt, TRACK_DEFS, TRACK_IDS, trackFor, type Spot, type TrackId,
} from '../src/modules/rooms/games/kart/tracks.ts';
import { itemBoxes } from '../src/modules/rooms/games/kart/sim.ts';
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
  assert.deepEqual([...TRACK_IDS].sort(), ['canyon', 'downtown', 'frost', 'lagoon', 'meadow', 'skyway', 'volcano']);
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

test('no track runs into itself: non-adjacent stretches are a road width apart or an overpass above', () => {
  for (const id of TRACK_IDS) {
    const t = trackFor(id);
    const n = t.count, spacing = t.length / n;
    let min = Infinity, crossings = 0;
    for (let i = 0; i < n; i++) for (let j = i + 1; j < n; j++) {
      const sep = Math.min(j - i, n - (j - i)) * spacing;
      if (sep < 3 * t.def.width) continue; // neighbours along the same stretch
      const d = Math.hypot(t.px[i] - t.px[j], t.pz[i] - t.pz[j]);
      if (d <= t.def.width) {
        // Stacked roads: at least 6 m apart vertically, and the upper one is a railed bridge.
        const dy = t.py[i] - t.py[j];
        assert.ok(Math.abs(dy) >= 6, `${id}: samples ${i}/${j} overlap with only ${dy.toFixed(1)} m between them`);
        const upper = dy > 0 ? i : j;
        if (d < t.halfWidth) assert.ok((t.flags[upper] & (F_BRIDGE | F_RAILS)) === (F_BRIDGE | F_RAILS), `${id}: overpass at ${upper} is a railed bridge`);
        crossings++;
        continue;
      }
      min = Math.min(min, d);
    }
    assert.ok(min > t.def.width, `${id}: closest non-adjacent samples ${min.toFixed(1)} m vs width ${t.def.width}`);
    if (['meadow', 'downtown', 'canyon', 'frost'].includes(id)) assert.equal(crossings, 0, `${id} is flat`);
    else assert.ok(crossings > 0, `${id} passes over itself`);
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
      // On the centre line the round trip is exact (the height picks the right level at overpasses).
      const c = pointAt(t, s, 0), cn = nearest(t, c[0], c[2], -1, newNear(), 24, c[1]);
      assert.ok(Math.abs(ds(cn.s, s, t.length)) < 1e-6 && Math.abs(cn.lateral) < 1e-6, `${id}: centre s ${s} -> ${cn.s}`);
      // Offset points: the offset uses the sample's tangent, so on tight corners the
      // projected distance can drift by up to one sample (2 m); lateral stays tight.
      const pt = pointAt(t, s, lat);
      const near = nearest(t, pt[0], pt[2], -1, newNear(), 24, pt[1]);
      assert.ok(Math.abs(ds(near.s, s, t.length)) < 2, `${id}: s ${s} -> ${near.s}`);
      assert.ok(Math.abs(near.lateral - lat) < 0.2, `${id}: lateral ${lat} -> ${near.lateral}`);
      assert.ok(near.distance - Math.abs(near.lateral) < 0.2);
      // The windowed search around the hint agrees with the full search.
      const hinted = nearest(t, pt[0], pt[2], sampleAt(t, s) + 3, newNear());
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
  const frac = (f: number) => f > 0 && f < 1;
  const range = ([a, b]: [number, number]) => frac(a) && frac(b) && a < b;
  for (const id of TRACK_IDS) {
    const d = TRACK_DEFS[id];
    assert.equal(d.id, id);
    assert.ok(d.itemRows.length > 0 && d.itemRows.every(frac));
    if (d.edge === 'grass') assert.ok(d.outOfBounds > 0);
    for (const f of [d, ...(d.branches ?? [])]) {
      assert.ok((f.ramps ?? []).every(frac));
      assert.ok((f.boostPads ?? []).every(p => frac(p.at) && Math.abs(p.lane) <= 1));
      assert.ok((f.ice ?? []).every(range) && (f.gaps ?? []).every(range) && (f.banked ?? []).every(range));
      assert.ok((f.bridges ?? []).every(b => range([b.from, b.to])) && (f.tunnels ?? []).every(b => range([b.from, b.to])) && (f.offroad ?? []).every(b => range([b.from, b.to])));
      assert.ok((f.obstacles ?? []).every(o => frac(o.at) && o.period > 0));
      // Every gap is preceded by a launch ramp just before it (and every launch clears into a gap).
      for (const [a] of f.gaps ?? []) assert.ok((f.launches ?? []).some(l => l.at < a && a - l.at < 0.01), `${id}: launch before the gap at ${a}`);
      for (const l of f.launches ?? []) assert.ok(frac(l.at) && l.power > 0 && (f.gaps ?? []).some(([a]) => a > l.at && a - l.at < 0.01), `${id}: gap after the launch at ${l.at}`);
    }
    for (const b of d.branches ?? []) {
      assert.ok(frac(b.from) && frac(b.to) && b.from < b.to, `${id}/${b.name} forks before it merges, never across the line`);
      assert.ok(b.cpu.every(p => p >= 0 && p <= 1) && b.cpu[0] <= b.cpu[1] && b.cpu[1] <= b.cpu[2], `${id}/${b.name}: harder CPUs take it more`);
    }
  }
});

test('new Grand Prix tracks: each lap is far longer than the originals and has every course element', () => {
  const longestOriginal = Math.max(...(['meadow', 'downtown', 'canyon', 'frost'] as TrackId[]).map(id => trackFor(id).length));
  for (const id of ['lagoon', 'volcano', 'skyway'] as TrackId[]) {
    const t = trackFor(id);
    assert.ok(t.length > longestOriginal * 1.6, `${id}: ${t.length.toFixed(0)} m vs ${longestOriginal.toFixed(0)} m`);
    assert.ok(t.paths.length >= 2, `${id} has a branch`);
    assert.ok(t.paths.some(p => p.shortcut), `${id} has a shortcut`);
    const all = (f: number) => t.paths.some(p => p.flags.some(v => (v & f) !== 0));
    assert.ok(all(F_GAP) && all(F_BRIDGE), `${id}: jumps and bridges`);
    assert.ok(t.liquid !== null, `${id}: water or lava`);
    assert.ok(t.def.launches?.length, `${id}: launch ramps`);
    // Banking follows curvature: steep on the spirals, gentle on the lagoon's sweepers.
    assert.ok(t.paths.some(p => p.bank.some(b => Math.abs(b) > (id === 'lagoon' ? 0.07 : 0.3))), `${id}: banked turns`);
  }
  // Underwater roads, a glider and tunnels across the set; obstacles of both kinds.
  assert.ok(trackFor('lagoon').flags.some(v => (v & F_UNDER) !== 0) && trackFor('skyway').flags.some(v => (v & F_UNDER) !== 0));
  assert.ok(trackFor('lagoon').def.launches!.some(l => l.glide) && trackFor('volcano').def.launches!.some(l => l.glide));
  assert.ok(trackFor('volcano').flags.some(v => (v & F_TUNNEL) !== 0) && trackFor('skyway').flags.some(v => (v & F_TUNNEL) !== 0));
  const kinds = new Set(TRACK_IDS.flatMap(id => trackFor(id).obstacles.map(o => o.kind)));
  assert.deepEqual([...kinds].sort(), ['crusher', 'roller']);
  // Lava never counts as drivable: no underwater flags there.
  assert.ok(trackFor('volcano').lava && !trackFor('volcano').paths.some(p => p.flags.some(v => (v & F_UNDER) !== 0)));
});

test('branches: open centre lines that leave and rejoin the main loop smoothly', () => {
  for (const id of TRACK_IDS) {
    const t = trackFor(id);
    for (const p of t.paths.slice(1)) {
      assert.equal(p.closed, false);
      assert.ok(p.count > 50);
      const spacing = p.length / (p.count - 1);
      assert.ok(Math.abs(spacing - 2) < 0.05, `${id}/${p.name}: spacing ${spacing}`);
      for (let i = 1; i < p.count; i++) {
        const step = Math.hypot(p.px[i] - p.px[i - 1], p.pz[i] - p.pz[i - 1]);
        assert.ok(step > 1.85 && step < 2.15, `${id}/${p.name}: step ${i} = ${step}`);
      }
      // Starts on the main loop at the fork, ends on it at the merge, heading the same way.
      const a = pointAt(t, p.from, 0), b = pointAt(t, p.to, 0);
      assert.ok(Math.hypot(p.px[0] - a[0], p.pz[0] - a[2]) < 1e-6 && Math.abs(p.py[0] - a[1]) < 1e-6, `${id}/${p.name}: fork`);
      assert.ok(Math.hypot(p.px[p.count - 1] - b[0], p.pz[p.count - 1] - b[2]) < 1e-6, `${id}/${p.name}: merge`);
      const dot = (i: number, s: number) => { const q = pointAt(t, s, 0); return Math.sin(q[3]) * p.tx[i] + Math.cos(q[3]) * p.tz[i]; };
      assert.ok(dot(0, p.from) > 0.7 && dot(p.count - 1, p.to) > 0.7, `${id}/${p.name}: tangent at the fork/merge`);
      // Drivable: no corner tighter than the road is wide.
      for (let i = 0; i < p.count; i++) assert.ok(Math.abs(p.curve[i]) < 1 / (p.halfWidth * 1.5), `${id}/${p.name}: radius at ${i}`);
      // The shortcut is shorter than the main-loop stretch it skips.
      if (p.shortcut) assert.ok(p.length < p.to - p.from, `${id}/${p.name}: ${p.length.toFixed(0)} vs ${(p.to - p.from).toFixed(0)}`);
    }
  }
});

test('progressOf maps branch distance linearly onto the main loop between fork and merge', () => {
  const t = trackFor('lagoon'), b = t.paths[1];
  assert.equal(progressOf(t, 0, 123.4), 123.4);
  assert.equal(progressOf(t, 1, 0), b.from);
  assert.ok(Math.abs(progressOf(t, 1, b.length) - b.to) < 1e-9);
  assert.ok(Math.abs(progressOf(t, 1, b.length / 2) - (b.from + b.to) / 2) < 1e-9);
  // Clamped beyond the ends; unknown paths fall back to the raw distance.
  assert.equal(progressOf(t, 1, -5), b.from);
  assert.equal(progressOf(t, 9, 42), 42);
});

test('locate: switches onto a branch once off the main road near the fork, and back at the merge', () => {
  for (const id of TRACK_IDS) {
    const t = trackFor(id);
    for (const b of t.paths.slice(1)) {
      // Deep on the branch: located there from the main loop's fork zone.
      const on = pointAt(b, 30, 0);
      const fromMain = locate(t, on[0], on[1], on[2], 0, sampleAt(t, b.from + 20), newNear());
      assert.equal(fromMain.seg, b.index, `${id}/${b.name}: picked up 30 m in`);
      assert.ok(Math.abs(fromMain.s - 30) < 1 && Math.abs(fromMain.lateral) < 0.2);
      // Still on the main road next to the fork: stays on the main loop.
      const main = pointAt(t, b.from + 4, 0);
      assert.equal(locate(t, main[0], main[1], main[2], 0, sampleAt(t, b.from + 4), newNear()).seg, 0);
      // Past the merge on the main road: back on the main loop.
      const after = pointAt(t, b.to + 20, 0);
      const back = locate(t, after[0], after[1], after[2], b.index, b.count - 2, newNear());
      assert.equal(back.seg, 0, `${id}/${b.name}: merged`);
      assert.ok(Math.abs(back.s - (b.to + 20)) < 1);
      // Mid-branch (outside the zones) a racer stays on its path.
      const mid = pointAt(b, b.length / 2, 0);
      const stay = locate(t, mid[0], mid[1], mid[2], b.index, sampleAt(b, b.length / 2), newNear());
      assert.equal(stay.seg, b.index);
    }
  }
});

test('locate tells stacked roads apart by height (overpasses and spirals)', () => {
  for (const id of ['lagoon', 'volcano', 'skyway'] as TrackId[]) {
    const t = trackFor(id);
    let checked = 0;
    for (let i = 0; i < t.count; i += 3) {
      if (!(t.flags[i] & F_BRIDGE) || !(t.flags[i] & F_RAILS) || t.flags[i] & F_GAP) continue;
      const p = pointAt(t, t.dist[i], 0);
      // A full (hint-less) search from above lands on this deck, not the road below.
      const n = locate(t, p[0], p[1] + 0.5, p[2], 0, -1, newNear());
      assert.ok(Math.abs(n.y - p[1]) < 1, `${id}@${i}: y ${n.y.toFixed(1)} vs ${p[1].toFixed(1)}`);
      checked++;
    }
    assert.ok(checked > 10, `${id}: ${checked} overpass samples`);
  }
});

test('advance follows the route: a set bit takes the branch, a branch always rejoins the main loop', () => {
  const t = trackFor('lagoon'), b = t.paths[1];
  const out: Spot = { seg: 0, s: 0 };
  advance(t, 0, b.from - 10, 30, 0, out);
  assert.deepEqual([out.seg, Math.round(out.s)], [0, Math.round(b.from + 20)]);
  advance(t, 0, b.from - 10, 30, 1, out);
  assert.equal(out.seg, 1);
  assert.ok(Math.abs(out.s - 20) < 1e-9);
  // Already a few metres past the fork while still counted on the main loop.
  advance(t, 0, b.from + 12, 10, 1, out);
  assert.equal(out.seg, 1);
  assert.ok(Math.abs(out.s - 22) < 1e-9);
  advance(t, 1, b.length - 5, 25, 1, out);
  assert.equal(out.seg, 0);
  assert.ok(Math.abs(out.s - (b.to + 20)) < 1e-9);
  // Wraps the start line on the main loop.
  advance(t, 0, t.length - 5, 20, 0, out);
  assert.ok(out.seg === 0 && Math.abs(out.s - 15) < 1e-9);
});

test('ground: water, bridges, gaps, banking and tunnels', () => {
  const lagoon = trackFor('lagoon');
  // The seabed road is under the surface; the deck above it is a railed bridge.
  let deep = -1;
  for (let i = 0; i < lagoon.count; i++) if (lagoon.py[i] < lagoon.liquid! - 6) { deep = i; break; }
  assert.ok(deep >= 0 && (lagoon.flags[deep] & F_UNDER) !== 0);
  // Beside an unrailed deck (the boardwalk) the ground drops below the water.
  const walk = lagoon.paths[1];
  const mid = walk.length / 2, side = pointAt(walk, mid, walk.halfWidth + 3);
  const n = nearest(walk, side[0], side[2], sampleAt(walk, mid), newNear());
  assert.ok(lagoon.ground(side[0], side[2], n) < lagoon.liquid! - 1, 'water beside the boardwalk');
  const on = pointAt(walk, mid, 0);
  assert.ok(Math.abs(lagoon.ground(on[0], on[2], nearest(walk, on[0], on[2], sampleAt(walk, mid), newNear())) - on[1]) < 0.05, 'the deck itself');
  // Over a gap there is nothing to drive on; the terrain under it is low too.
  const gapS = lagoon.def.gaps![0][0] * lagoon.length + 20, g = pointAt(lagoon, gapS, 0);
  const gn = nearest(lagoon, g[0], g[2], sampleAt(lagoon, gapS), newNear());
  assert.ok(lagoon.ground(g[0], g[2], gn) < g[1] - 30);
  assert.ok(lagoon.terrain(g[0], g[2], gn) < g[1] - 4);
  // Banked: the outside of the turn is higher (lateral > 0 is the left, the inside of a left-hander).
  for (const id of ['lagoon', 'volcano', 'skyway'] as TrackId[]) {
    const t = trackFor(id);
    let bi = 0;
    for (let i = 0; i < t.count; i++) if (Math.abs(t.bank[i]) > Math.abs(t.bank[bi])) bi = i;
    assert.ok(Math.abs(t.bank[bi]) > 0.07 && Math.sign(t.bank[bi]) === Math.sign(t.curve[bi]), `${id}: banked into the turn`);
    const inside = Math.sign(t.curve[bi]) * 6;
    const pi = pointAt(t, t.dist[bi], inside), po = pointAt(t, t.dist[bi], -inside);
    const yi = t.ground(pi[0], pi[2], nearest(t, pi[0], pi[2], bi, newNear()));
    const yo = t.ground(po[0], po[2], nearest(t, po[0], po[2], bi, newNear()));
    assert.ok(yo - yi > 0.8, `${id}: outside higher: ${yo.toFixed(2)} vs ${yi.toFixed(2)}`);
  }
  // Rock tunnels: the land rises steeply right beside the road.
  const volcano = trackFor('volcano');
  const ti = volcano.flags.findIndex(v => (v & F_TUNNEL) !== 0) + 10;
  const w = pointAt(volcano, volcano.dist[ti], volcano.halfWidth + 6);
  assert.ok(volcano.ground(w[0], w[2], nearest(volcano, w[0], w[2], ti, newNear())) > volcano.py[ti] + 6);
  // Launch ramps rise to the lip like kickers, only taller.
  const l = volcano.def.launches![0], ls = l.at * volcano.length;
  assert.ok(rampOn(volcano, ls) > RAMP_HEIGHT && rampOn(volcano, ls - 1) > rampOn(volcano, ls - 5) && rampOn(volcano, ls + 1) === 0);
});

test('inLaunchZone covers the run-up, the ramp and the gap; inOffroad respects lanes', () => {
  const t = trackFor('volcano'), l = t.def.launches![0], ls = l.at * t.length;
  assert.ok(inLaunchZone(t, ls - RUN_UP + 1) && inLaunchZone(t, ls) && inLaunchZone(t, ls + 8));
  assert.ok(!inLaunchZone(t, ls - RUN_UP - 5));
  const gapEnd = t.def.gaps![0][1] * t.length;
  assert.ok(inLaunchZone(t, gapEnd - 1) && !inLaunchZone(t, gapEnd + 6));
  const lagoon = trackFor('lagoon'), o = lagoon.def.offroad![0], s = (o.from + o.to) / 2 * lagoon.length;
  assert.ok(inOffroad(lagoon, s, lagoon.halfWidth * 0.8) && !inOffroad(lagoon, s, -lagoon.halfWidth * 0.8) && !inOffroad(lagoon, (o.to + 0.02) * lagoon.length, lagoon.halfWidth * 0.8));
  const alley = trackFor('skyway').paths[1];
  assert.ok(inOffroad(alley, alley.length / 2, 0) && inOffroad(alley, alley.length / 2, -alley.halfWidth));
});

test('obstacles move as a pure function of time: rollers stay on the road, crushers slam down then rise', () => {
  for (const id of TRACK_IDS) {
    const t = trackFor(id);
    for (const o of t.obstacles) {
      const p = t.paths[o.seg];
      let minLift = Infinity, maxLift = -Infinity, maxLat = 0;
      for (let k = 0; k < 400; k++) {
        const time = k * o.period / 200;
        const a = obstacleAt(t, o, time, [0, 0, 0, 0]), b = obstacleAt(t, o, time, [0, 0, 0, 0]);
        assert.deepEqual(a, b, 'deterministic');
        const later = obstacleAt(t, o, time + o.period, [0, 0, 0, 0]);
        assert.ok(later.every((v, i) => Math.abs(v - a[i]) < 1e-6), 'periodic');
        if (o.kind === 'roller') maxLat = Math.max(maxLat, Math.abs(a[3]));
        else { minLift = Math.min(minLift, a[3]); maxLift = Math.max(maxLift, a[3]); }
      }
      if (o.kind === 'roller') assert.ok(maxLat + ROLLER_RADIUS <= p.halfWidth && maxLat > p.halfWidth * 0.5, `${id}: roller sweeps the road (${maxLat})`);
      else assert.ok(minLift === 0 && maxLift === CRUSHER_RISE, `${id}: crusher lift ${minLift}..${maxLift}`);
    }
  }
});

test('item boxes: rows on every route, five across main roads, three on narrow branches', () => {
  for (const id of TRACK_IDS) {
    const t = trackFor(id);
    const expected = t.paths.reduce((n, p) => n + p.features.itemRows.length * (p.halfWidth < 6 ? 3 : 5), 0);
    assert.equal(itemBoxes(t).length, expected);
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
