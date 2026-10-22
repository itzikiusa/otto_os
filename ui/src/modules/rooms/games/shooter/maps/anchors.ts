/**
 * Mode anchors for every Storm Royale map, derived once from the generated
 * world so battle royale, team deathmatch, domination and zombies all spawn
 * from the same data: free-for-all respawns spread over the map, two team
 * bases with spawn rings facing each other, three domination flags between
 * them, a zombies arena (survivor start, infected spawn ring, the ground-floor
 * doors and windows to board up, wall-buys on building walls and an
 * extraction pad) and a coarse street nav graph for bots (`navNext`). Pure
 * data and deterministic: no three.js, no randomness beyond the map itself.
 */
import { hashString, Rng } from '../../common/math.ts';
import type { WeaponId } from '../weapons.ts';
import { HALF, inClimb, terrainHeight, waterLevel, type Opening, type Poi, type ShooterMap } from '../map.ts';

/** A place to (re)spawn, facing `yaw` (forward = sin/cos yaw, like fighters). */
export interface Spawn { x: number; y: number; z: number; yaw: number }
/** A domination flag: `name` is the letter, `label` the place it stands. */
export interface Flag { name: string; label: string; x: number; y: number; z: number; r: number }
/** A boardable ground-floor door or window (see `Opening`). */
export type Barricade = Opening;
/** A chalk outline on an outer wall where a weapon can be bought (zombies). */
export interface BuyWall { x: number; y: number; z: number; yaw: number; weapon: WeaponId; cost: number }
export interface ZombieArena {
  cx: number; cz: number; r: number;
  /** Survivors start here, the infected rise from `spawns`, the chopper waits on `extraction`. */
  start: Spawn[]; spawns: Spawn[]; barricades: Barricade[]; buyWalls: BuyWall[];
  extraction: { x: number; y: number; z: number; r: number };
}
/** Walkable outdoor waypoints (about 16 m apart) and who connects to whom (both ways). */
export interface NavGraph { nodes: { x: number; y: number; z: number }[]; links: number[][] }
export interface ModeAnchors {
  /** Free-for-all respawns, spread far apart. */
  spawns: Spawn[];
  /** Team deathmatch / domination bases: eight spawns each, facing the other base. */
  teamSpawns: [Spawn[], Spawn[]];
  /** Domination flags A, B, C from the first base toward the second. */
  flags: Flag[];
  zombies: ZombieArena;
  nav: NavGraph;
}
/** Hand-placed anchors a map can ask for (Dead City); anything left out is derived. */
export interface AnchorHints {
  bases?: [[number, number], [number, number]];
  flags?: { label: string; x: number; z: number }[];
  arena?: { x: number; z: number; r: number };
  extraction?: [number, number];
}

/** Wall-buys in order of the walls found nearest the arena centre, with their prices. */
export const BUY_WEAPONS: { weapon: WeaponId; cost: number }[] = [
  { weapon: 'smg', cost: 1000 }, { weapon: 'shotgun', cost: 1200 }, { weapon: 'pistol', cost: 500 },
  { weapon: 'rifle', cost: 1400 }, { weapon: 'sniper', cost: 1800 }, { weapon: 'rocket', cost: 2500 },
];
const CELL = 16;
const NAV_STEP = 16;

/** Box/trunk lookup in 16 m cells, for clearance checks at map-build time. */
class Clearance {
  private cells = new Map<number, number[]>();
  private map: ShooterMap;
  constructor(map: ShooterMap) {
    this.map = map;
    map.boxes.forEach((b, i) => this.insert(b.minX, b.minZ, b.maxX, b.maxZ, i));
    map.cylinders.forEach((c, i) => this.insert(c.x - c.r, c.z - c.r, c.x + c.r, c.z + c.r, -1 - i));
  }
  private key(cx: number, cz: number): number { return (cx + 64) * 256 + cz + 64; }
  private insert(x0: number, z0: number, x1: number, z1: number, id: number): void {
    for (let cx = Math.floor(x0 / CELL); cx <= Math.floor(x1 / CELL); cx++) for (let cz = Math.floor(z0 / CELL); cz <= Math.floor(z1 / CELL); cz++) {
      const k = this.key(cx, cz);
      const list = this.cells.get(k);
      if (list) list.push(id); else this.cells.set(k, [id]);
    }
  }
  /** Is a body of radius `r` standing at (x, z) on the ground blocked by something it cannot step onto? */
  blocked(x: number, z: number, r: number): boolean {
    const feet = terrainHeight(this.map, x, z);
    for (let cx = Math.floor((x - r) / CELL); cx <= Math.floor((x + r) / CELL); cx++) for (let cz = Math.floor((z - r) / CELL); cz <= Math.floor((z + r) / CELL); cz++) {
      const list = this.cells.get(this.key(cx, cz));
      if (!list) continue;
      for (const id of list) {
        if (id >= 0) {
          const b = this.map.boxes[id];
          if (b.maxY > feet + 0.6 && b.minY < feet + 1.8 && x > b.minX - r && x < b.maxX + r && z > b.minZ - r && z < b.maxZ + r) return true;
        } else {
          const c = this.map.cylinders[-1 - id];
          if ((x - c.x) * (x - c.x) + (z - c.z) * (z - c.z) < (c.r + r) * (c.r + r)) return true;
        }
      }
    }
    return false;
  }
  /** Open, dry, fairly level ground inside the play area. */
  open(x: number, z: number, r: number): boolean {
    if (Math.abs(x) > HALF - 12 || Math.abs(z) > HALF - 12) return false;
    const h = terrainHeight(this.map, x, z);
    if (h < waterLevel(this.map, x, z) + 0.8) return false;
    for (const [dx, dz] of [[r + 1, 0], [-r - 1, 0], [0, r + 1], [0, -r - 1]]) if (Math.abs(terrainHeight(this.map, x + dx, z + dz) - h) > 1.4) return false;
    return !this.blocked(x, z, r);
  }
  /** The nearest open spot to (x, z), searching rings out to `reach` metres (the point itself when none is). */
  near(x: number, z: number, r: number, reach = 48): [number, number] {
    if (this.open(x, z, r)) return [x, z];
    for (let ring = 2; ring <= reach; ring += 2) {
      const n = Math.max(8, Math.round(ring * 1.6));
      for (let k = 0; k < n; k++) {
        const a = (k / n) * Math.PI * 2 + ring * 0.37, px = x + Math.cos(a) * ring, pz = z + Math.sin(a) * ring;
        if (this.open(px, pz, r)) return [px, pz];
      }
    }
    return [x, z];
  }
}

const facing = (x: number, z: number, tx: number, tz: number): number => Math.atan2(tx - x, tz - z);
const spawnAt = (map: ShooterMap, x: number, z: number, yaw: number): Spawn => ({ x, y: terrainHeight(map, x, z), z, yaw });

/** Up to `n` open spots in a widening spiral around (x, z), at least `gap` metres apart, facing (tx, tz). */
function ring(map: ShooterMap, clear: Clearance, x: number, z: number, n: number, gap: number, tx: number, tz: number, from = 3): Spawn[] {
  const out: Spawn[] = [];
  for (let k = 0; out.length < n && k < 120; k++) {
    const a = k * 2.399963, r = from + k * 0.9, px = x + Math.cos(a) * r, pz = z + Math.sin(a) * r;
    if (!clear.open(px, pz, 1) || out.some(s => Math.hypot(s.x - px, s.z - pz) < gap)) continue;
    out.push(spawnAt(map, px, pz, facing(px, pz, tx, tz)));
  }
  return out;
}

export function buildAnchors(map: ShooterMap, hints: AnchorHints = {}): ModeAnchors {
  const clear = new Clearance(map);
  const rng = new Rng(hashString(map.id) ^ 0x5eed);
  const sites = map.pois.filter(p => p.kind !== 'lake');

  // Free-for-all: open ground every 12 m, then farthest-point picks from the most central one.
  const cands: [number, number][] = [];
  for (let x = -HALF + 16; x <= HALF - 16; x += 12) for (let z = -HALF + 16; z <= HALF - 16; z += 12) {
    const px = x + rng.range(-3, 3), pz = z + rng.range(-3, 3);
    if (clear.open(px, pz, 1.4)) cands.push([px, pz]);
  }
  const spawns: Spawn[] = [];
  if (cands.length) {
    const dist = new Float64Array(cands.length).fill(Infinity);
    let pick = 0;
    cands.forEach((c, i) => { if (Math.hypot(c[0], c[1]) < Math.hypot(cands[pick][0], cands[pick][1])) pick = i; });
    while (spawns.length < Math.min(24, cands.length)) {
      const [x, z] = cands[pick];
      spawns.push(spawnAt(map, x, z, facing(x, z, 0, 0)));
      let best = -1;
      for (let i = 0; i < cands.length; i++) {
        dist[i] = Math.min(dist[i], Math.hypot(cands[i][0] - x, cands[i][1] - z));
        if (best < 0 || dist[i] > dist[best]) best = i;
      }
      if (dist[best] < 8) break;
      pick = best;
    }
  }

  // Team bases: as hinted, else the two landmarks farthest apart.
  let a: Poi | null = null, b: Poi | null = null;
  for (const p of sites) for (const q of sites) if (!a || Math.hypot(p.x - q.x, p.z - q.z) > Math.hypot(a.x - b!.x, a.z - b!.z)) { a = p; b = q; }
  const [hax, haz] = hints.bases?.[0] ?? [a?.x ?? -100, a?.z ?? 0], [hbx, hbz] = hints.bases?.[1] ?? [b?.x ?? 100, b?.z ?? 0];
  const baseA = clear.near(hax, haz, 1.5), baseB = clear.near(hbx, hbz, 1.5);
  const teamSpawns: [Spawn[], Spawn[]] = [
    ring(map, clear, baseA[0], baseA[1], 8, 2.5, baseB[0], baseB[1]),
    ring(map, clear, baseB[0], baseB[1], 8, 2.5, baseA[0], baseA[1]),
  ];

  // Domination: the landmarks nearest a quarter, half and three quarters of the way between the bases.
  const flags: Flag[] = [];
  const ax = baseB[0] - baseA[0], az = baseB[1] - baseA[1], len2 = Math.max(1, ax * ax + az * az);
  const wanted = hints.flags ?? [0.25, 0.5, 0.75].map(t => {
    let best: Poi | null = null, score = Infinity;
    for (const p of sites) {
      const along = ((p.x - baseA[0]) * ax + (p.z - baseA[1]) * az) / len2;
      const across = Math.abs((p.x - baseA[0]) * az - (p.z - baseA[1]) * ax) / Math.sqrt(len2);
      const s = Math.abs(along - t) * 400 + across * 0.5;
      if (s < score && !flags.some(f => f.label === p.name)) { score = s; best = p; }
    }
    const x = best ? best.x : baseA[0] + ax * t, z = best ? best.z : baseA[1] + az * t;
    const label = best?.name ?? `Point ${t}`;
    // Reserve the landmark so the next letter picks another one.
    flags.push({ name: '', label, x, y: 0, z, r: 0 });
    return { label, x, z };
  });
  flags.length = 0;
  wanted.forEach((w, i) => {
    const [x, z] = clear.near(w.x, w.z, 3, 60);
    flags.push({ name: 'ABC'[i] ?? String(i + 1), label: w.label, x, y: terrainHeight(map, x, z), z, r: 7 });
  });

  // Zombies: hinted, else the town.
  const town = sites.find(p => p.kind === 'town') ?? sites[0];
  const arena = hints.arena ?? { x: town?.x ?? 0, z: town?.z ?? 0, r: (town?.r ?? 40) * 1.1 };
  const inside = (x: number, z: number, pad = 0) => Math.hypot(x - arena.x, z - arena.z) < arena.r + pad;
  const barricades = map.openings.filter(o => inside(o.x, o.z) && Math.abs(o.y - terrainHeight(map, o.x, o.z)) < 3)
    .sort((p, q) => Math.hypot(p.x - arena.x, p.z - arena.z) - Math.hypot(q.x - arena.x, q.z - arena.z)).slice(0, 18);
  const buyWalls: BuyWall[] = [];
  const climbs = map.climbs.filter(c => inside(c.cx, c.cz)).sort((p, q) => Math.hypot(p.cx - arena.x, p.cz - arena.z) - Math.hypot(q.cx - arena.x, q.cz - arena.z));
  for (const c of climbs) {
    // The bare halves of the side walls (their window is at +/-d*0.2): outside face, chest height.
    for (const [lx, lz, yaw] of [[-c.w / 2 - 0.05, -c.d * 0.25, -Math.PI / 2], [c.w / 2 + 0.05, c.d * 0.25, Math.PI / 2]] as const) {
      if (buyWalls.length >= BUY_WEAPONS.length) break;
      const [x, z] = local(c.cx, c.cz, c.q, lx, lz);
      const buy = BUY_WEAPONS[buyWalls.length];
      buyWalls.push({ x, y: c.base + 1.3, z, yaw: Math.atan2(Math.sin(yaw + c.q * Math.PI / 2), Math.cos(yaw + c.q * Math.PI / 2)), weapon: buy.weapon, cost: buy.cost });
    }
  }
  const [hex, hez] = hints.extraction ?? [arena.x + arena.r * 0.6, arena.z];
  const [ex, ez] = clear.near(hex, hez, 4, 40);
  const spawnsZ: Spawn[] = [];
  for (let k = 0; k < 16; k++) {
    const t = (k / 16) * Math.PI * 2, r = arena.r * (1.05 + (k % 3) * 0.1);
    const [x, z] = clear.near(arena.x + Math.cos(t) * r, arena.z + Math.sin(t) * r, 1, 16);
    spawnsZ.push(spawnAt(map, x, z, facing(x, z, arena.x, arena.z)));
  }
  const [sx, sz] = clear.near(arena.x, arena.z, 1.5, 30);
  const zombies: ZombieArena = {
    cx: arena.x, cz: arena.z, r: arena.r,
    start: ring(map, clear, sx, sz, 4, 2.2, ex, ez, 1), spawns: spawnsZ, barricades, buyWalls,
    extraction: { x: ex, y: terrainHeight(map, ex, ez), z: ez, r: 4 },
  };
  return { spawns, teamSpawns, flags, zombies, nav: navGraph(map, clear) };
}

/** World position of a building-frame point (front = +z, turned by quarter `q`, see map.ts Frame). */
function local(cx: number, cz: number, q: number, lx: number, lz: number): [number, number] {
  switch (q & 3) {
    case 1: return [cx + lz, cz - lx];
    case 2: return [cx - lx, cz - lz];
    case 3: return [cx - lz, cz + lx];
    default: return [cx + lx, cz + lz];
  }
}

/** Open ground on a 16 m lattice (nudged off obstacles), linked to its eight neighbours where the walk between is clear. */
function navGraph(map: ShooterMap, clear: Clearance): NavGraph {
  const n = Math.floor((HALF * 2 - 32) / NAV_STEP) + 1;
  const id = new Int32Array(n * n).fill(-1);
  const nodes: NavGraph['nodes'] = [];
  for (let j = 0; j < n; j++) for (let i = 0; i < n; i++) {
    const x0 = -HALF + 16 + i * NAV_STEP, z0 = -HALF + 16 + j * NAV_STEP;
    for (const [dx, dz] of [[0, 0], [4, 0], [-4, 0], [0, 4], [0, -4], [5, 5], [-5, -5], [5, -5], [-5, 5]]) {
      // Outdoors only: inside a walk-in the climb helpers take over.
      if (!clear.open(x0 + dx, z0 + dz, 1.1) || map.climbs.some(c => inClimb(c, x0 + dx, z0 + dz, 1))) continue;
      id[j * n + i] = nodes.length;
      nodes.push({ x: x0 + dx, y: terrainHeight(map, x0 + dx, z0 + dz), z: z0 + dz });
      break;
    }
  }
  const links: number[][] = nodes.map(() => []);
  const walkable = (ax: number, az: number, bx: number, bz: number) => {
    const len = Math.hypot(bx - ax, bz - az), steps = Math.ceil(len / 0.5);
    let prev = terrainHeight(map, ax, az);
    for (let s = 1; s < steps; s++) {
      const x = ax + (bx - ax) * (s / steps), z = az + (bz - az) * (s / steps), h = terrainHeight(map, x, z);
      if (Math.abs(h - prev) > 0.5 || h < waterLevel(map, x, z) + 0.6 || clear.blocked(x, z, 0.5)) return false;
      prev = h;
    }
    return true;
  };
  for (let j = 0; j < n; j++) for (let i = 0; i < n; i++) {
    const a = id[j * n + i];
    if (a < 0) continue;
    // Forward half of the eight neighbours; links are added both ways.
    for (const [di, dj] of [[1, 0], [0, 1], [1, 1], [-1, 1]]) {
      const ii = i + di, jj = j + dj;
      if (ii < 0 || ii >= n || jj >= n) continue;
      const b = id[jj * n + ii];
      if (b < 0 || !walkable(nodes[a].x, nodes[a].z, nodes[b].x, nodes[b].z)) continue;
      links[a].push(b); links[b].push(a);
    }
  }
  return { nodes, links };
}

/** Index of the nav node nearest (x, z), or -1 on an empty graph. */
export function nearestNavNode(map: ShooterMap, x: number, z: number): number {
  const nodes = map.anchors.nav.nodes;
  let best = -1, bd = Infinity;
  for (let i = 0; i < nodes.length; i++) {
    const d = (nodes[i].x - x) * (nodes[i].x - x) + (nodes[i].z - z) * (nodes[i].z - z);
    if (d < bd) { bd = d; best = i; }
  }
  return best;
}

// A* scratch, grown to the largest graph seen (no per-call allocation after that).
let _g = new Float64Array(0), _f = new Float64Array(0), _from = new Int32Array(0), _seen = new Uint32Array(0), _heap = new Int32Array(0);
let _stamp = 0;
const _next: [number, number] = [0, 0];

/**
 * The next waypoint on the street graph from (x, z) toward (goalX, goalZ):
 * the nearest node first, then node to node (A*, straight-line heuristic).
 * Null when the walk is direct (both ends share the nearest node) or no route
 * exists. Allocation-free: the returned pair is reused.
 */
export function navNext(map: ShooterMap, x: number, z: number, goalX: number, goalZ: number): [number, number] | null {
  const { nodes, links } = map.anchors.nav;
  const start = nearestNavNode(map, x, z), goal = nearestNavNode(map, goalX, goalZ);
  if (start < 0 || goal < 0 || start === goal) return null;
  const n = nodes.length;
  if (_g.length < n) { _g = new Float64Array(n); _f = new Float64Array(n); _from = new Int32Array(n); _seen = new Uint32Array(n); _heap = new Int32Array(n * 8); }
  const stamp = ++_stamp;
  const h = (i: number) => Math.hypot(nodes[i].x - nodes[goal].x, nodes[i].z - nodes[goal].z);
  let size = 0;
  const push = (i: number) => {
    let k = size++;
    _heap[k] = i;
    while (k > 0) { const p = (k - 1) >> 1; if (_f[_heap[p]] <= _f[_heap[k]]) break; const t = _heap[p]; _heap[p] = _heap[k]; _heap[k] = t; k = p; }
  };
  const pop = (): number => {
    const top = _heap[0];
    _heap[0] = _heap[--size];
    let k = 0;
    for (;;) {
      const l = k * 2 + 1, r = l + 1;
      let m = k;
      if (l < size && _f[_heap[l]] < _f[_heap[m]]) m = l;
      if (r < size && _f[_heap[r]] < _f[_heap[m]]) m = r;
      if (m === k) break;
      const t = _heap[m]; _heap[m] = _heap[k]; _heap[k] = t; k = m;
    }
    return top;
  };
  _seen[start] = stamp; _g[start] = 0; _f[start] = h(start); _from[start] = -1;
  push(start);
  let found = false;
  while (size > 0) {
    const cur = pop();
    if (cur === goal) { found = true; break; }
    for (const nb of links[cur]) {
      const cost = _g[cur] + Math.hypot(nodes[nb].x - nodes[cur].x, nodes[nb].z - nodes[cur].z);
      if (_seen[nb] === stamp && cost >= _g[nb]) continue;
      _seen[nb] = stamp; _g[nb] = cost; _f[nb] = cost + h(nb); _from[nb] = cur;
      if (size < _heap.length) push(nb);
    }
  }
  if (!found) return null;
  // Walk back to the node after the start; go to the start node itself while still far from it.
  let step = goal;
  while (_from[step] !== start && _from[step] !== -1) step = _from[step];
  const s = nodes[start];
  const target = Math.hypot(s.x - x, s.z - z) > 6 ? s : nodes[step];
  _next[0] = target.x; _next[1] = target.z;
  return _next;
}
