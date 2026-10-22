/**
 * Route audit for Kart Grand Prix: no branch (or main road) ever drives into
 * a wall. The road corridor of every route — forks and merges included — is
 * swept against everything solid the renderer places (layout.ts: barrier
 * walls, rails, deck sides, embankment / ramp / half-pipe skirts, abutments,
 * tunnel tubes, gantry posts, drawbridge towers, bridge pillars, fork signs,
 * every themed prop's footprint and the landscape mesh itself), and against
 * the simulation's colliders (wall pushes, out-of-bounds rescues, ledges)
 * with ghost lines across five lanes and CPU drivers at several speeds.
 */
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import {
  F_DRAW, F_GAP, F_PIPE, F_RAILS, FORK_ZONE, locate, nearest, newNear, pointAt, progressOf, rampOn, sampleAt, TRACK_IDS, trackFor, TUNNEL_HEIGHT,
  type Path, type Track,
} from '../src/modules/rooms/games/kart/tracks.ts';
import {
  abutments, barriers, DECOR, edgeClear, forkSign, gantries, MOUTH_PAD, pillars, POST_HEIGHT, PROP_BOXES, propBlocksRoad, propLayout, roadIndex, skirts, terrainGrid, TOWER_HALF, towers, TUBE_PAD,
  tubeMask, worldHeight, type RoadHit,
} from '../src/modules/rooms/games/kart/layout.ts';
import { createRace, tick, type KartState } from '../src/modules/rooms/games/kart/sim.ts';
import { cpuInput } from '../src/modules/rooms/games/kart/cpu.ts';
import { Rng } from '../src/modules/rooms/games/common/math.ts';

const pt = [0, 0, 0, 0];
const hit: RoadHit = { seg: -1, index: 0, lateral: 0, y: 0, edge: 0 };
/** A solid only blocks a kart when it rises this far above that kart's road surface (a curb does not). */
const CURB = 0.15;
/** Solids may not stand on any tarmac, nor within this of its edge. */
const PAD = 0.3;

/** Collects the routes a solid spanning [y0, y1] at (x, z) stands on; `own` may be excused near `ownS` (its own ramp sides / abutments). */
function audit(track: Track) {
  const idx = roadIndex(track), found: string[] = [];
  return {
    found,
    check(what: string, x: number, z: number, y0: number, y1: number, pad = PAD, own?: { path: Path; s: number }): void {
      if (!idx.hit(x, z, y0, y1 - CURB, pad, -1, hit)) return;
      const q = track.paths[hit.seg];
      if (own && q === own.path && Math.abs(q.dist[hit.index] - own.s) < 25) return;
      found.push(`${what} stands on ${q.name} at s=${Math.round(q.dist[hit.index])} (road y ${hit.y.toFixed(1)}, solid ${y0.toFixed(1)}..${y1.toFixed(1)})`);
    },
  };
}
const unique = (list: string[]) => [...new Set(list)].slice(0, 8).join('\n  ');

test('prop footprints match the shipped kart kit', () => {
  // The GLB's JSON chunk: every top-level node is a model; positions are quantized (normalized) accessors.
  const buf = fs.readFileSync(new URL('../public/room-games/kart-kit.glb', import.meta.url));
  const json = JSON.parse(buf.subarray(20, 20 + buf.readUInt32LE(12)).toString());
  type Node = { name?: string; children?: number[]; mesh?: number; translation?: number[]; rotation?: number[]; scale?: number[] };
  const nodes = json.nodes as Node[];
  const scale: Record<number, number> = { 5120: 127, 5121: 255, 5122: 32767, 5123: 65535 };
  const bounds = (root: Node) => {
    const min = [Infinity, Infinity, Infinity], max = [-Infinity, -Infinity, -Infinity];
    const walk = (n: Node, t: number[], s: number[]) => {
      // Kit models only translate and uniformly scale their parts (no rotation) — assert it.
      assert.ok(!n.rotation || Math.abs(n.rotation[3]) > 0.9999, `${root.name}: unexpected rotation`);
      const ns = n.scale ?? [1, 1, 1], nt = n.translation ?? [0, 0, 0];
      const S = s.map((v, k) => v * ns[k]), T = t.map((v, k) => v + nt[k] * s[k]);
      if (n.mesh !== undefined) for (const p of json.meshes[n.mesh].primitives) {
        const a = json.accessors[p.attributes.POSITION], d = a.normalized ? scale[a.componentType] : 1;
        for (let k = 0; k < 3; k++) { min[k] = Math.min(min[k], T[k] + (a.min[k] / d) * S[k]); max[k] = Math.max(max[k], T[k] + (a.max[k] / d) * S[k]); }
      }
      for (const c of n.children ?? []) walk(nodes[c], T, S);
    };
    for (const c of root.children ?? []) walk(nodes[c], [0, 0, 0], [1, 1, 1]);
    if (root.mesh !== undefined) walk({ ...root, translation: undefined, scale: undefined, rotation: undefined, children: [] }, [0, 0, 0], [1, 1, 1]);
    return { min, max };
  };
  const roots = new Map<string, Node>();
  for (const i of json.scenes[json.scene ?? 0].nodes as number[]) roots.set(nodes[i].name ?? '', nodes[i]);
  for (const [key, [hx, hz, h]] of Object.entries(PROP_BOXES)) {
    const root = roots.get(key);
    assert.ok(root, `${key} is in the kit`);
    const { min, max } = bounds(root);
    assert.ok(Math.abs((max[0] - min[0]) / 2 - hx) < 0.04, `${key}: half width ${((max[0] - min[0]) / 2).toFixed(2)} vs ${hx}`);
    assert.ok(Math.abs((max[2] - min[2]) / 2 - hz) < 0.04, `${key}: half depth ${((max[2] - min[2]) / 2).toFixed(2)} vs ${hz}`);
    assert.ok(Math.abs(max[1] - min[1] - h) < 0.08, `${key}: height ${(max[1] - min[1]).toFixed(2)} vs ${h}`);
  }
});

test('road index: finds the tarmac under a point at kart height, nothing beside it, and a building dropped on the road', () => {
  const t = trackFor('downtown'), idx = roadIndex(t);
  pointAt(t, 100, 0, pt);
  assert.ok(idx.hit(pt[0], pt[2], pt[1], pt[1] + 2, 0, -1, hit) && hit.seg === 0 && Math.abs(hit.lateral) < 0.01);
  // Well above (an overpass) or below (an underpass) it does not count.
  assert.equal(idx.hit(pt[0], pt[2], pt[1] + 6, pt[1] + 9, 0), false);
  assert.equal(idx.hit(pt[0], pt[2], pt[1] - 9, pt[1] - 5, 0), false);
  pointAt(t, 100, t.halfWidth + 25, pt);
  assert.equal(idx.hit(pt[0], pt[2], pt[1], pt[1] + 2, PAD), false);
  pointAt(t, 100, t.halfWidth + 4, pt);
  assert.ok(propBlocksRoad(t, { key: 'city-building-j', x: pt[0], y: pt[1], z: pt[2], yaw: 0, scale: 12, sy: 12 }, PAD), 'a 25 m block 4 m off the edge covers the road');
  assert.equal(propBlocksRoad(t, { key: 'race-flagRed', x: pt[0], y: pt[1], z: pt[2], yaw: 0, scale: 4, sy: 4 }, PAD), false);
});

for (const id of TRACK_IDS) {
  test(`route audit (${id}): walls, rails, skirts, abutments and tunnel tubes never stand on any route's road`, () => {
    const t = trackFor(id), a = audit(t);
    for (const p of t.paths) {
      for (const b of barriers(t, p)) for (let i = 0; i < p.count; i++) {
        if (!b.mask[i]) continue;
        for (const lat of [b.lateral, b.lateral + b.side * b.thick]) {
          pointAt(p, p.dist[i], lat, pt);
          const y = p.py[i] - lat * p.bank[i];
          a.check(`${p.name} ${b.kind} (${b.side > 0 ? 'left' : 'right'})`, pt[0], pt[2], y + b.bottom, y + b.top);
        }
      }
      for (const sk of skirts(t, p)) for (let i = 0; i < p.count; i++) {
        if (!sk.mask[i]) continue;
        pointAt(p, p.dist[i], sk.lateral, pt);
        const y = p.py[i] - sk.lateral * p.bank[i];
        a.check(`${p.name} ${sk.kind} skirt`, pt[0], pt[2], y + sk.bottom[i], y + sk.top[i], PAD, { path: p, s: p.dist[i] });
      }
      const quads = abutments(t, p);
      for (let k = 0; k < quads.length; k += 6) {
        const n = nearest(p, (quads[k] + quads[k + 2]) / 2, (quads[k + 1] + quads[k + 3]) / 2, -1, newNear());
        for (let f = 0; f <= 1; f += 0.05) a.check(`${p.name} abutment`, quads[k] + (quads[k + 2] - quads[k]) * f, quads[k + 1] + (quads[k + 3] - quads[k + 1]) * f, quads[k + 4], quads[k + 5], PAD, { path: p, s: n.s });
      }
      for (const glass of [false, true]) {
        const m = tubeMask(p, glass), R = p.halfWidth + TUBE_PAD;
        for (let i = 0; i < p.count; i++) {
          if (!m[i]) continue;
          for (let k = 0; k <= 12; k++) {
            const th = (k / 12) * Math.PI, lat = Math.cos(th) * R;
            pointAt(p, p.dist[i], lat, pt);
            const y = p.py[i] - lat * p.bank[i] + Math.sin(th) * TUNNEL_HEIGHT - 0.4;
            a.check(`${p.name} ${glass ? 'glass ' : ''}tunnel tube`, pt[0], pt[2], y - 0.3, y + 0.3);
          }
        }
      }
    }
    assert.equal(a.found.length, 0, `${id}:\n  ${unique(a.found)}`);
  });

  test(`route audit (${id}): gantry posts, towers, pillars, fork signs and every prop stand clear of every road`, () => {
    const t = trackFor(id), a = audit(t);
    for (const g of gantries(t)) for (const [x, y, z] of g.posts) a.check(`gantry "${g.text}" post`, x, z, y - 1, y + POST_HEIGHT, PAD + 0.25);
    for (const p of t.paths) {
      for (const [x, y, z] of towers(p)) a.check(`${p.name} drawbridge tower`, x, z, y - 1.5, y + 7.5, PAD + TOWER_HALF);
      const spots = pillars(t, p);
      for (let k = 0; k < spots.length; k += 4) a.check(`${p.name} pillar`, spots[k], spots[k + 2], spots[k + 1], spots[k + 1] + spots[k + 3], PAD + 0.9);
      const sign = forkSign(t, p);
      if (p.index > 0) assert.ok(sign, `${id}/${p.name}: has a fork sign`);
      if (sign) a.check(`${p.name} fork sign`, sign.x, sign.z, sign.y, sign.y + 5, PAD + 0.2);
    }
    const props = propLayout(t);
    assert.ok(props.length > 300, `${id}: ${props.length} props`);
    for (const p of props) {
      if (p.key === 'toy-gate-finish') continue; // the finish arch spans the road; its feet stand outside the edges
      if (propBlocksRoad(t, p, DECOR.test(p.key) ? 0 : PAD)) a.found.push(`${p.key} at (${p.x.toFixed(0)}, ${p.z.toFixed(0)}) x${p.scale.toFixed(1)}`);
    }
    // The finish arch's feet.
    const gate = props.find(p => p.key === 'toy-gate-finish')!;
    for (const side of [-1, 1]) {
      const half = 0.77 * gate.scale;
      a.check('finish arch foot', gate.x + Math.cos(gate.yaw) * half * side, gate.z - Math.sin(gate.yaw) * half * side, gate.y, gate.y + 5, 0);
    }
    assert.equal(a.found.length, 0, `${id}:\n  ${unique(a.found)}`);
  });

  test(`route audit (${id}): the landscape mesh never rises through a road, and branches meet the main road level`, () => {
    const t = trackFor(id), g = terrainGrid(t), cache = new Map<number, number>();
    const vertex = (ix: number, iz: number) => {
      const k = ix * 100003 + iz;
      let v = cache.get(k);
      if (v === undefined) { v = worldHeight(t, g.x0 + ix * g.step, g.z0 + iz * g.step); cache.set(k, v); }
      return v;
    };
    // The renderer's triangles: (a, c, b) and (b, c, d) per grid cell.
    const mesh = (x: number, z: number) => {
      const fx = (x - g.x0) / g.step, fz = (z - g.z0) / g.step, ix = Math.floor(fx), iz = Math.floor(fz), u = fx - ix, w = fz - iz;
      const a = vertex(ix, iz), b = vertex(ix + 1, iz), c = vertex(ix, iz + 1), d = vertex(ix + 1, iz + 1);
      return u + w <= 1 ? a + (b - a) * u + (c - a) * w : d + (c - d) * (1 - u) + (b - d) * (1 - w);
    };
    const found: string[] = [];
    for (const p of t.paths) for (let i = 0; i < p.count; i++) {
      if (p.flags[i] & (F_GAP | F_PIPE | F_DRAW)) continue;
      for (const f of [-0.8, -0.4, 0, 0.4, 0.8]) {
        const lat = f * p.halfWidth;
        pointAt(p, p.dist[i], lat, pt);
        const road = p.py[i] - lat * p.bank[i] + rampOn(p, p.dist[i]);
        const land = mesh(pt[0], pt[2]);
        if (land > road + 0.5) found.push(`${p.name} s=${Math.round(p.dist[i])} lane ${f}: land ${(land - road).toFixed(1)} m above the road`);
      }
    }
    assert.equal(found.length, 0, `${id}:\n  ${unique(found)}`);
    // Where a branch still shares tarmac with the main road, the two surfaces are one.
    const near = newNear();
    for (const b of t.paths.slice(1)) for (let i = 0; i < b.count; i++) {
      const s = b.dist[i];
      nearest(t, b.px[i], b.pz[i], sampleAt(t, s < b.length / 2 ? b.from + s : b.to - (b.length - s)), near, 40);
      if (near.distance > t.halfWidth + b.halfWidth - 2) continue;
      assert.ok(Math.abs(b.py[i] - near.y) < 0.3, `${id}/${b.name} s=${s.toFixed(0)}: branch ${b.py[i].toFixed(2)} vs main ${near.y.toFixed(2)}`);
    }
  });
}

test('edge furniture opens wide at every fork and merge mouth (and only there)', () => {
  for (const id of TRACK_IDS) {
    const t = trackFor(id);
    if (t.paths.length < 2) continue;
    const idx = roadIndex(t);
    for (const p of t.paths) {
      const { left, right } = edgeClear(t, p);
      let open = 0;
      for (let i = 0; i < p.count; i++) {
        for (const [side, mask] of [[1, left], [-1, right]] as [number, Uint8Array][]) {
          if (mask[i]) continue;
          open++;
          // Every opening is within reach of another route's tarmac.
          let near = false;
          for (let k = -2; k <= 2 && !near; k++) {
            const j = p.closed ? (i + k + p.count) % p.count : Math.max(0, Math.min(p.count - 1, i + k));
            for (const lat of [p.halfWidth + 0.3, p.halfWidth + 1.4, p.halfWidth + 2, p.halfWidth + TUBE_PAD]) {
              pointAt(p, p.dist[j], side * lat, pt);
              if (idx.gap(pt[0], pt[2], p.index) < MOUTH_PAD + 0.01) { near = true; break; }
            }
          }
          assert.ok(near, `${id}/${p.name} s=${Math.round(p.dist[i])}: opening away from any other route`);
        }
      }
      if (p.index > 0) assert.ok(open > 0, `${id}/${p.name}: the branch opens onto the main road`);
    }
  }
});

// ── Simulation colliders ──────────────────────────────────────────────────

/**
 * Ghost line: a point travelling along `points`, handed from path to path by
 * `locate` exactly as a racer is. At each step the simulation's colliders for
 * the path it is on must leave it alone: no wall or rail push, no rescue line,
 * and the ground it drives on matches the road it is really on (no ledge).
 */
function ghost(t: Track, points: { x: number; y: number; z: number; p: Path; s: number }[]): string[] {
  let seg = 0, hint = -1;
  const issues: string[] = [], near = newNear(), n2 = newNear();
  for (const g of points) {
    locate(t, g.x, g.y, g.z, seg, hint, near);
    seg = near.seg; hint = near.index;
    const q = t.paths[seg];
    nearest(q, g.x, g.z, hint, n2, 4);
    const flags = q.flags[n2.index], off = Math.abs(n2.lateral) - q.halfWidth;
    if ((q.edge === 'wall' || flags & F_RAILS) && Math.abs(n2.lateral) > q.halfWidth + 1.3) issues.push(`wall of ${q.name} at s=${g.s.toFixed(0)}`);
    else if (q.edge !== 'wall' && !(flags & F_RAILS) && off > q.outOfBounds && q.drop[n2.index] < 0.5) issues.push(`rescue line of ${q.name} at s=${g.s.toFixed(0)}`);
    const own = g.p.flags[sampleAt(g.p, g.s)];
    if (own & (F_GAP | F_DRAW | F_PIPE)) continue;
    // Skip the metre just past a ramp lip (the road really does drop there).
    if ([...g.p.features.ramps, ...g.p.features.launches.map(l => l.at)].some(at => { const d = g.s - (g.p.index ? g.p.from : 0) - at * g.p.length; return d >= -0.5 && d < 2; })) continue;
    const ground = t.ground(g.x, g.z, n2, 0), want = g.y + rampOn(g.p, g.s - (g.p.index ? g.p.from : 0));
    if (Math.abs(ground - want) > 0.6) issues.push(`ledge of ${(ground - want).toFixed(1)} m on ${q.name} at s=${g.s.toFixed(0)}`);
  }
  return issues;
}
const surface = (p: Path, s: number, lat: number) => {
  pointAt(p, s, lat, pt);
  const n = nearest(p, pt[0], pt[2], sampleAt(p, s), newNear(), 4);
  return { x: pt[0], y: pt[1] - n.lateral * p.bank[n.index], z: pt[2] };
};

for (const id of TRACK_IDS) {
  test(`route audit (${id}): ghost lines across five lanes of every route meet no wall, rescue line or ledge`, () => {
    const t = trackFor(id);
    const found: string[] = [];
    for (const b of t.paths.slice(1)) for (const f of [-0.85, -0.5, 0, 0.5, 0.85]) {
      const points: { x: number; y: number; z: number; p: Path; s: number }[] = [];
      const mainLat = Math.max(-t.halfWidth + 1.2, Math.min(t.halfWidth - 1.2, f * b.halfWidth));
      const lat = Math.sign(f) * Math.min(Math.abs(f * b.halfWidth), b.halfWidth - 1.2);
      for (let s = b.from - 60; s < b.from; s += 1) points.push({ ...surface(t, s, mainLat), p: t, s });
      for (let s = 0; s <= b.length; s += 1) points.push({ ...surface(b, s, lat), p: b, s: b.from + s });
      for (let s = b.to; s < b.to + 60; s += 1) points.push({ ...surface(t, s, mainLat), p: t, s });
      for (const issue of ghost(t, points)) found.push(`${b.name} lane ${f}: ${issue}`);
    }
    for (const f of [-0.85, 0, 0.85]) {
      const points: { x: number; y: number; z: number; p: Path; s: number }[] = [];
      for (let s = 0; s < t.length; s += 1) points.push({ ...surface(t, s, f * (t.halfWidth - 1.2)), p: t, s });
      for (const issue of ghost(t, points)) found.push(`main lane ${f}: ${issue}`);
    }
    assert.equal(found.length, 0, `${id}:\n  ${unique(found)}`);
  });
}

/** One CPU driver forced down `route` from 90 m before the fork at `speed`; reports wall bumps, stops and rescues. */
function cpuThrough(id: (typeof TRACK_IDS)[number], branch: number, speed: number, difficulty: 'easy' | 'normal' | 'hard') {
  const t = trackFor(id), b = t.paths[branch];
  const s: KartState = createRace(id, [{ racer: 'oobi', human: false, slot: -1, name: 'CPU' }], difficulty, 3);
  while (s.phase !== 'race') tick(s, []);
  const r = s.racers[0], start = b.from - 90;
  const p = pointAt(t, start, 0), near = nearest(t, p[0], p[2], sampleAt(t, start), newNear());
  r.x = p[0]; r.z = p[2]; r.y = t.ground(p[0], p[2], near); r.yaw = p[3];
  r.speed = speed; r.vx = Math.sin(r.yaw) * speed; r.vz = Math.cos(r.yaw) * speed;
  r.seg = 0; r.segS = near.s; r.safeSeg = 0; r.safeS = near.s; r.hint = -1; r.lastS = progressOf(t, 0, near.s); r.travel = r.lastS;
  r.cpuRoute = 1 << (branch - 1); r.cpuMistake = -99;
  if (b.boost) { r.item = 'mushroom'; r.itemCount = 1; }
  const rng = new Rng(5), from = s.eventSeq;
  let deep = false, slowFor = 0, worstSlow = 0, n = 0;
  while (n++ < 60 * 40) {
    // The CPU keeps its forced route every lap (no re-roll mid-test).
    r.cpuRoute = 1 << (branch - 1);
    const input = cpuInput(s, t, r, rng, false);
    tick(s, [input]);
    if (r.seg === branch && r.segS > FORK_ZONE && r.segS < b.length - FORK_ZONE) deep = true;
    // A stop: crawling (not spun out, not stalled in deep mud by design) for a while.
    if (r.speed < 6 && r.spin <= 0 && r.respawn <= 0 && n > 60) slowFor += 1 / 60; else slowFor = 0;
    worstSlow = Math.max(worstSlow, slowFor);
    if (r.seg === 0 && deep && progressOf(t, 0, r.segS) > b.to + 40 && progressOf(t, 0, r.segS) < b.to + 200) break;
  }
  const evs = s.events.filter(e => e.id > from);
  // Wall bumps: a bump with no spin-out beside it (a roller's bump comes with a spin).
  const wallBumps = evs.filter(e => e.type === 'bump' && !evs.some(o => o.type === 'spin' && o.racer === e.racer && Math.abs(o.id - e.id) <= 1)).length;
  const rescues = evs.filter(e => e.type === 'respawn' || (e.type === 'splash' && e.value !== 1)).length;
  return { deep, wallBumps, rescues, worstSlow, reached: n < 60 * 40 };
}

for (const id of TRACK_IDS) {
  test(`route audit (${id}): CPU drivers take every branch at 12, 24 and 34 m/s without touching a wall, stopping or falling`, () => {
    const t = trackFor(id);
    for (const b of t.paths.slice(1)) for (const [speed, difficulty] of [[12, 'easy'], [24, 'normal'], [34, 'hard']] as [number, 'easy' | 'normal' | 'hard'][]) {
      const run = cpuThrough(id, b.index, speed, difficulty);
      const label = `${id}/${b.name} @${speed} (${difficulty})`;
      assert.ok(run.reached && run.deep, `${label}: drove the branch to the merge`);
      assert.equal(run.wallBumps, 0, `${label}: wall bumps`);
      assert.equal(run.rescues, 0, `${label}: rescues`);
      assert.ok(run.worstSlow < 0.75, `${label}: stuck below 6 m/s for ${run.worstSlow.toFixed(2)} s`);
    }
  });
}
