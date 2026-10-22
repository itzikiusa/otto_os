/**
 * The infected (host-authoritative, part of the 60 Hz sim), for Zombies and
 * Apocalypse Royale. Four kinds: walkers, fast runners, brutes that soak
 * damage and smash barricades, spitters that keep their distance and lob
 * acid. Horde steering stays cheap at 60+ of them: targets are re-picked on
 * a staggered think timer, separation uses a fixed spatial hash rebuilt in
 * place each tick (typed arrays, no allocation), and collision reuses the
 * fighters' capsule-vs-world resolve. Blocked by a structure they claw at
 * it; stuck at a low ledge they climb it.
 *
 * Around the Zombies base they path-find: a flow field (breadth-first
 * distance to the nearest survivor over a 1.25 m walkability grid of the
 * static props, rebuilt twice a second) leads them around buildings and in
 * through the doors; player barricades are left out of it on purpose, so the
 * horde walks into them and tears them down.
 *
 * Zombies mode (`stepZombies`): escalating rounds around a base, points for
 * every hit and kill, buy-walls (`buyWallsFor`, derived from the map so
 * guests know them without the wire), the dead return at the next round, and
 * after the last round a helicopter: survive until it lands, be on the
 * landing zone when it leaves. Apocalypse Royale keeps a roaming horde topped
 * up across the island (`seedInfected`).
 */
import { approach, clamp, type Rng, wrapAngle } from '../common/math.ts';
import { HALF, mapFor, terrainHeight, type ShooterMap } from './map.ts';
import { groundAt, resolve, visible, type World } from './collide.ts';
import { AMMO_KINDS, AMMO_MAX, AMMO_PACK, isWeapon, WEAPONS, type WeaponId } from './weapons.ts';
import {
  creditStreak, crown, damageBuild, dropLoot, emit, FROM_INFECTED, hurt, KILL_INFECTED, MAX_FIGHTERS, STEP,
  type Fighter, type ShooterState,
} from './sim.ts';
import { arenaCentre, bounds, clearSpot, place, WAVE_BOARD, WAVE_HELI, WAVE_ON, WAVE_OVER, WAVE_PRE, zombieKit, type ModeState } from './modes/index.ts';
import { lobAt } from './modes/gear.ts';

export type ZombieKind = 0 | 1 | 2 | 3;
/** Per kind: health, speed, melee damage / reach / interval, body radius and scale (hit spheres, render), Zombies points. */
export const ZOMBIE_KINDS: { name: string; hp: number; speed: number; damage: number; reach: number; rate: number; radius: number; scale: number; points: number }[] = [
  { name: 'Walker', hp: 70, speed: 2.5, damage: 18, reach: 1.5, rate: 1.1, radius: 0.45, scale: 1, points: 60 },
  { name: 'Runner', hp: 50, speed: 6.0, damage: 12, reach: 1.4, rate: 0.8, radius: 0.4, scale: 0.95, points: 70 },
  { name: 'Brute', hp: 420, speed: 3.1, damage: 42, reach: 2.1, rate: 1.6, radius: 0.8, scale: 1.55, points: 200 },
  { name: 'Spitter', hp: 90, speed: 2.7, damage: 14, reach: 1.5, rate: 1.2, radius: 0.45, scale: 1.05, points: 100 },
];
export interface Zombie {
  id: number; kind: ZombieKind; x: number; y: number; z: number; vx: number; vy: number; vz: number; yaw: number; grounded: boolean;
  hp: number; max: number;
  /** Fighter being hunted (-1: wandering), think timer, seconds to the next swing / spit, stunned (flash) seconds. */
  target: number; think: number; attackT: number; spitT: number; stun: number;
  /** Stuck detection and the sidestep that gets out of it; wander goal; provoked (hunts from farther); swing animation (s). */
  stuck: number; lastX: number; lastZ: number; side: number; sideT: number; wx: number; wz: number; aggro: boolean; swing: number; los: boolean;
  /** Seconds since it was last close to its target (Zombies brings a straggler back in when this runs long). */
  idle: number;
}
export const MAX_INFECTED = 64;
/** Apocalypse Royale's roaming horde size (it shares snapshots with the full floor loot). */
export const APOC_CAP = 22;
/** Zombies keeps at most this much floor loot (the oldest goes), so a full horde fits in a snapshot. */
export const ZOMBIE_LOOT_CAP = 60;
/** Zombies: helicopter flight in, boarding window, landing-zone radius. */
export const HELI_ETA = 40;
export const BOARD_TIME = 20;
export const LZ_R = 12;
/** Buy-wall prices (refilling ammo for a gun already owned costs half). */
export const WALL_PRICE: Record<WeaponId, number> = { pistol: 300, smg: 1000, shotgun: 1200, rifle: 1500, sniper: 1750, rocket: 2500 };

// Spatial hash for separation: 2 m cells over the island, heads reset after use.
const CELL = 2;
const HN = Math.ceil((HALF * 2) / CELL);
const head = new Int32Array(HN * HN).fill(-1);
const next = new Int32Array(MAX_INFECTED);
const cellOf = new Int32Array(MAX_INFECTED);
const hcell = (x: number, z: number) => clamp(Math.floor((z + HALF) / CELL), 0, HN - 1) * HN + clamp(Math.floor((x + HALF) / CELL), 0, HN - 1);

/**
 * Flow field around the Zombies base: a walkability grid of the static props
 * (built once per map and base) and the breadth-first distance (in cells) to
 * the nearest survivor on the ground.
 */
const FLOW_CELL = 1.25;
interface Flow { key: string; x0: number; z0: number; n: number; blocked: Uint8Array; dist: Uint16Array; queue: Int32Array; t: number }
const UNREACHED = 65535;
let flow: Flow | null = null;
function flowFor(map: ShooterMap, world: World, cx: number, cz: number, half: number): Flow {
  const key = `${map.id}:${cx}:${cz}:${half}`;
  if (flow?.key === key) return flow;
  const n = Math.ceil((half * 2) / FLOW_CELL), x0 = cx - half, z0 = cz - half;
  const blocked = new Uint8Array(n * n);
  // A cell is blocked when a prop stands across walking height at its centre (walls, crates, cars, trunks), or it is under water.
  for (const b of map.boxes) {
    const i0 = Math.max(0, Math.floor((b.minX - x0) / FLOW_CELL)), i1 = Math.min(n - 1, Math.floor((b.maxX - x0) / FLOW_CELL));
    const j0 = Math.max(0, Math.floor((b.minZ - z0) / FLOW_CELL)), j1 = Math.min(n - 1, Math.floor((b.maxZ - z0) / FLOW_CELL));
    for (let j = j0; j <= j1; j++) for (let i = i0; i <= i1; i++) {
      const x = x0 + (i + 0.5) * FLOW_CELL, z = z0 + (j + 0.5) * FLOW_CELL;
      if (x < b.minX || x > b.maxX || z < b.minZ || z > b.maxZ) continue;
      const g = terrainHeight(map, x, z);
      if (b.minY < g + 1.5 && b.maxY > g + 0.6) blocked[j * n + i] = 1;
    }
  }
  for (const c of map.cylinders) {
    const i = Math.floor((c.x - x0) / FLOW_CELL), j = Math.floor((c.z - z0) / FLOW_CELL);
    if (i >= 0 && j >= 0 && i < n && j < n) blocked[j * n + i] = 1;
  }
  void world;
  flow = { key, x0, z0, n, blocked, dist: new Uint16Array(n * n), queue: new Int32Array(n * n), t: 0 };
  return flow;
}
/** Rebuilds the distance field from every survivor on the ground (multi-source breadth-first search). */
function fillFlow(f: Flow, state: ShooterState): void {
  const { n, dist, queue, blocked } = f;
  dist.fill(UNREACHED);
  let head = 0, tail = 0;
  for (const p of state.players) {
    if (p.mode !== 'ground' && p.mode !== 'down') continue;
    const i = Math.floor((p.x - f.x0) / FLOW_CELL), j = Math.floor((p.z - f.z0) / FLOW_CELL);
    if (i < 0 || j < 0 || i >= n || j >= n || dist[j * n + i] === 0) continue;
    dist[j * n + i] = 0; queue[tail++] = j * n + i;
  }
  while (head < tail) {
    const c = queue[head++], i = c % n, j = (c - i) / n, d = dist[c] + 1;
    if (i > 0 && !blocked[c - 1] && dist[c - 1] === UNREACHED) { dist[c - 1] = d; queue[tail++] = c - 1; }
    if (i < n - 1 && !blocked[c + 1] && dist[c + 1] === UNREACHED) { dist[c + 1] = d; queue[tail++] = c + 1; }
    if (j > 0 && !blocked[c - n] && dist[c - n] === UNREACHED) { dist[c - n] = d; queue[tail++] = c - n; }
    if (j < n - 1 && !blocked[c + n] && dist[c + n] === UNREACHED) { dist[c + n] = d; queue[tail++] = c + n; }
  }
}
/** The flow-field direction at (x, z) (into `out`), or false off the grid / where no survivor is reachable. */
const _dir = [0, 0];
function flowDir(f: Flow, x: number, z: number): boolean {
  const i = Math.floor((x - f.x0) / FLOW_CELL), j = Math.floor((z - f.z0) / FLOW_CELL), n = f.n;
  if (i < 0 || j < 0 || i >= n || j >= n) return false;
  let best = f.dist[j * n + i], bi = -1, bj = -1;
  for (let dj = -1; dj <= 1; dj++) for (let di = -1; di <= 1; di++) {
    const ii = i + di, jj = j + dj;
    if ((di === 0 && dj === 0) || ii < 0 || jj < 0 || ii >= n || jj >= n) continue;
    // No corner cutting past a blocked cell.
    if (di !== 0 && dj !== 0 && (f.blocked[j * n + ii] || f.blocked[jj * n + i])) continue;
    const d = f.dist[jj * n + ii];
    if (d < best) { best = d; bi = ii; bj = jj; }
  }
  if (bi < 0) return false;
  const tx = f.x0 + (bi + 0.5) * FLOW_CELL - x, tz = f.z0 + (bj + 0.5) * FLOW_CELL - z, l = Math.hypot(tx, tz) || 1;
  _dir[0] = tx / l; _dir[1] = tz / l;
  return true;
}

const _zh = { t: -1, head: false };
const Z_SPHERES: [number, number, boolean][] = [[1.62, 0.3, true], [1.15, 0.46, false], [0.55, 0.42, false]];
/** Ray vs an infected (head + two body spheres, scaled by kind; a brute is half again as big). */
export function rayZombie(ox: number, oy: number, oz: number, dx: number, dy: number, dz: number, z: { x: number; y: number; z: number; kind: number }, max: number): { t: number; head: boolean } {
  _zh.t = -1; _zh.head = false;
  const s = ZOMBIE_KINDS[z.kind]?.scale ?? 1;
  for (const [h, r, isHead] of Z_SPHERES) {
    const cx = z.x - ox, cy = z.y + h * s - oy, cz = z.z - oz, rr = r * s;
    const b = cx * dx + cy * dy + cz * dz;
    if (b < 0) continue;
    const c = cx * cx + cy * cy + cz * cz - rr * rr;
    const disc = b * b - c;
    if (disc < 0) continue;
    const t = b - Math.sqrt(disc);
    if (t >= 0 && t < max && (_zh.t < 0 || t < _zh.t)) { _zh.t = t; _zh.head = isHead; }
  }
  return _zh;
}

function spawnZombie(state: ShooterState, world: World, kind: ZombieKind, x: number, z: number, hpMul: number, aggro: boolean): void {
  if (state.infected.length >= MAX_INFECTED) return;
  const max = Math.round(ZOMBIE_KINDS[kind].hp * hpMul);
  const y = groundAt(world, x, z, terrainHeight(world.map, x, z) + 0.5, state.builds);
  state.infected.push({
    id: state.nextId++, kind, x, y, z, vx: 0, vy: 0, vz: 0, yaw: 0, grounded: true, hp: max, max,
    target: -1, think: (state.nextId % 7) * 0.06, attackT: 0.5, spitT: 2, stun: 0, stuck: 0, lastX: x, lastZ: z, side: 1, sideT: 0, wx: x, wz: z, aggro, swing: 0, los: false, idle: 0,
  });
}

/** Damages an infected; the kill pays points (Zombies), killstreak progress and sometimes a little loot. */
export function hurtZombie(state: ShooterState, z: Zombie, amount: number, from: number, head: boolean, code: number): void {
  if (z.hp <= 0 || amount <= 0 || state.phase === 'done') return;
  z.hp -= amount; z.aggro = true;
  const p = from >= 0 ? state.players[from] : null;
  if (p) {
    p.damage += amount;
    if (p.mode === 'ground' || p.mode === 'down') z.target = from;
    if (state.mode === 'zombies') p.points += 10;
  }
  const s = ZOMBIE_KINDS[z.kind].scale;
  emit(state, 'zhit', from, z.x, z.y + (head ? 1.7 : 1.1) * s, z.z, { target: z.id, value: Math.round(amount), flag: head ? 2 : 0 });
  if (z.hp > 0) return;
  z.hp = 0;
  emit(state, 'zkill', from, z.x, z.y, z.z, { target: z.id, value: z.kind, flag: (head ? 1 : 0) | (code << 1) });
  if (p) {
    creditStreak(state, p, state.mode === 'zombies' ? 0.25 : 0.34);
    if (state.mode === 'zombies') { p.kills++; p.points += ZOMBIE_KINDS[z.kind].points + (head ? 40 : 0); }
  }
  // A few drop ammo (for the gun in the killer's hands when they have one) or, rarely, armour.
  const roll = (Math.imul(z.id, 2654435761) >>> 0) % 100;
  const held = p ? p.weapons[p.current] : null;
  const kind = held && isWeapon(held.w) ? WEAPONS[held.w].ammo : AMMO_KINDS[z.id % AMMO_KINDS.length];
  if (roll < (state.mode === 'zombies' ? 14 : 30)) dropLoot(state, 'ammo', kind, AMMO_PACK[kind], z.x, z.y, z.z);
  else if (roll > 93 && state.mode === 'apoc') dropLoot(state, 'shield', null, 25, z.x, z.y, z.z);
}

/** Per tick: sweep the dead, steer / fight every infected, keep Apocalypse Royale's horde topped up. */
export function stepInfected(state: ShooterState, map: ShooterMap, world: World, rng: Rng): void {
  for (let i = state.infected.length - 1; i >= 0; i--) if (state.infected[i].hp <= 0) state.infected.splice(i, 1);
  const list = state.infected, n = list.length;
  for (let i = 0; i < n; i++) { const c = hcell(list[i].x, list[i].z); cellOf[i] = c; next[i] = head[c]; head[c] = i; }
  // Zombies: the flow field toward the survivors, twice a second.
  let field: Flow | null = null;
  if (state.mode === 'zombies' && state.obj.ar > 0) {
    field = flowFor(map, world, state.obj.ax, state.obj.az, state.obj.ar + 10);
    field.t -= STEP;
    if (field.t <= 0) { field.t = 0.5; fillFlow(field, state); }
  }
  for (let i = 0; i < n; i++) stepZombie(state, map, world, list, i, rng, field);
  for (let i = 0; i < n; i++) head[cellOf[i]] = -1;
  if (state.mode === 'apoc' && state.phase === 'play') {
    state.obj.spawnT -= STEP;
    if (state.obj.spawnT <= 0) {
      state.obj.spawnT = 2.5;
      if (state.infected.length < APOC_CAP) roam(state, map, world, rng, 1);
    }
  }
}

/** The fighter worth chasing: nearest one on the ground (knocked ones count for less) within `range`. */
function pickTarget(state: ShooterState, z: Zombie, range: number): number {
  let best = -1, bd = range;
  for (const p of state.players) {
    if (p.mode !== 'ground' && p.mode !== 'down') continue;
    const d = Math.hypot(p.x - z.x, p.z - z.z) * (p.mode === 'down' ? 1.5 : 1) + Math.abs(p.y - z.y) * 2;
    if (d < bd) { bd = d; best = p.id; }
  }
  return best;
}

function stepZombie(state: ShooterState, map: ShooterMap, world: World, list: Zombie[], i: number, rng: Rng, field: Flow | null): void {
  const z = list[i], k = ZOMBIE_KINDS[z.kind];
  z.attackT = Math.max(0, z.attackT - STEP); z.stun = Math.max(0, z.stun - STEP); z.swing = Math.max(0, z.swing - STEP);
  z.spitT -= STEP; z.think -= STEP; z.sideT -= STEP;
  const hunt = state.mode === 'zombies' ? 400 : z.aggro ? 60 : 28;
  if (z.think <= 0) {
    z.think = 0.45 + rng.range(0, 0.2);
    z.target = pickTarget(state, z, hunt);
    const moved = Math.hypot(z.x - z.lastX, z.z - z.lastZ);
    z.stuck = moved < 0.25 && z.stun <= 0 ? z.stuck + 0.5 : Math.max(0, z.stuck - 0.5);
    z.lastX = z.x; z.lastZ = z.z;
    if (z.stuck >= 1 && z.sideT <= 0) { z.side = rng.chance(0.5) ? 1 : -1; z.sideT = 1.2; }
    const t = z.target >= 0 ? state.players[z.target] : null;
    z.los = !!t && z.kind === 3 && Math.hypot(t.x - z.x, t.z - z.z) < 18 && visible(world, z.x, z.y + 1.6, z.z, t.x, t.y + 1.2, t.z, state.builds);
    // Wandering (Apocalypse Royale): a new nearby spot once this one is reached.
    if (!t && Math.hypot(z.wx - z.x, z.wz - z.z) < 3) { z.wx = clamp(z.x + rng.range(-25, 25), -HALF + 10, HALF - 10); z.wz = clamp(z.z + rng.range(-25, 25), -HALF + 10, HALF - 10); }
    z.idle = t && Math.hypot(t.x - z.x, t.z - z.z) < 8 ? 0 : z.idle + 0.5;
    // Zombies: a straggler that has not reached anyone for a while (stuck, lost, wading) comes back in
    // at the spawn point nearest its target that is not on top of anyone.
    if (state.mode === 'zombies' && (z.idle > 25 || z.stuck > 8) && state.obj.zspawns.length) {
      let bx = z.x, bz = z.z, bd = Infinity;
      for (const [sx, sz] of state.obj.zspawns) {
        const d = t ? Math.hypot(t.x - sx, t.z - sz) : rng.next();
        if (d < bd && !state.players.some(p => p.mode !== 'dead' && Math.hypot(p.x - sx, p.z - sz) < 15)) { bd = d; bx = sx; bz = sz; }
      }
      z.x = bx; z.z = bz; z.y = groundAt(world, bx, bz, terrainHeight(map, bx, bz) + 0.5, state.builds); z.stuck = 0; z.idle = 0; z.vx = 0; z.vz = 0;
    }
  }
  const t = z.target >= 0 ? state.players[z.target] : null;
  const live = !!t && (t.mode === 'ground' || t.mode === 'down');
  const gx = live ? t!.x : z.wx, gz = live ? t!.z : z.wz;
  let dx = gx - z.x, dz = gz - z.z;
  const d = Math.hypot(dx, dz) || 1;
  dx /= d; dz /= d;
  // Farther out, follow the flow field around buildings and through doors (straight at them once close).
  if (live && field && d > 3.5 && flowDir(field, z.x, z.z)) { dx = _dir[0]; dz = _dir[1]; }
  let speed = z.stun > 0 ? 0 : k.speed * (live ? 1 : 0.35) * (state.mode === 'zombies' ? 1 + Math.min(0.3, state.obj.round * 0.03) : 1);
  // Spitters keep their distance and lob acid from it.
  if (live && z.kind === 3 && z.los) {
    if (d < 8) { dx = -dx; dz = -dz; } else if (d < 14) speed = 0;
    if (z.spitT <= 0 && d < 18 && z.stun <= 0) { z.spitT = 2.8; z.swing = 0.5; lobAt(state, FROM_INFECTED, -1, 4, z.x, z.y + 1.7, z.z, t!.x + t!.vx * 0.5, t!.y + 0.5, t!.z + t!.vz * 0.5); }
  }
  // Close enough to swing: slow down and hit.
  const close = live && d < k.reach + 0.35 && Math.abs(t!.y - z.y) < 1.9;
  if (close) speed *= 0.25;
  let mx = dx, mz = dz;
  if (z.sideT > 0) { mx += -dz * z.side * 0.9; mz += dx * z.side * 0.9; }
  // Separation from neighbours (3 x 3 cells of the spatial hash).
  let sx = 0, sz = 0;
  const cx = clamp(Math.floor((z.x + HALF) / CELL), 0, HN - 1), cz = clamp(Math.floor((z.z + HALF) / CELL), 0, HN - 1);
  for (let gzi = Math.max(0, cz - 1); gzi <= Math.min(HN - 1, cz + 1); gzi++) for (let gxi = Math.max(0, cx - 1); gxi <= Math.min(HN - 1, cx + 1); gxi++) {
    for (let j = head[gzi * HN + gxi]; j >= 0; j = next[j]) {
      if (j === i) continue;
      const o = list[j], ox = z.x - o.x, oz = z.z - o.z, dd = Math.hypot(ox, oz), min = (k.radius + ZOMBIE_KINDS[o.kind].radius) * 1.1;
      if (dd >= min || dd < 1e-4) continue;
      sx += (ox / dd) * (min - dd); sz += (oz / dd) * (min - dd);
    }
  }
  z.vx = approach(z.vx, mx * speed + sx * 6, 9, STEP);
  z.vz = approach(z.vz, mz * speed + sz * 6, 9, STEP);
  z.x += z.vx * STEP; z.z += z.vz * STEP;
  const blocked = resolve(world, z, state.builds);
  const g = groundAt(world, z.x, z.z, z.y, state.builds);
  if (z.y > g + 0.05) { z.vy -= 22 * STEP; z.y = Math.max(g, z.y + z.vy * STEP); } else { z.y = g; z.vy = 0; }
  z.y = Math.max(z.y, terrainHeight(map, z.x, z.z) - 1.2);
  // Facing: the target while swinging, else the way it walks.
  const face = close || (live && speed === 0) ? Math.atan2(t!.x - z.x, t!.z - z.z) : Math.hypot(z.vx, z.vz) > 0.3 ? Math.atan2(z.vx, z.vz) : z.yaw;
  z.yaw += wrapAngle(face - z.yaw) * Math.min(1, STEP * 8);
  if (close && z.attackT <= 0 && z.stun <= 0) {
    z.attackT = k.rate; z.swing = 0.45;
    // The computer difficulty softens the horde's claws too (Zombies is co-op against it).
    hurt(state, t!, Math.round(k.damage * (state.difficulty === 'easy' ? 0.6 : state.difficulty === 'normal' ? 0.8 : 1)), FROM_INFECTED, false, true, KILL_INFECTED);
    return;
  }
  if (!blocked || z.stuck < 0.5 || z.stun > 0) return;
  // Blocked: claw at a structure in the way (brutes tear them down), or climb a low ledge.
  const ax = z.x + dx * 0.9, az = z.z + dz * 0.9, ay = z.y + 1;
  if (z.attackT <= 0) for (const b of state.builds) {
    const a = b.aabb;
    if (ax < a.minX - 0.3 || ax > a.maxX + 0.3 || az < a.minZ - 0.3 || az > a.maxZ + 0.3 || ay < a.minY || ay > a.maxY + 0.5) continue;
    z.attackT = k.rate; z.swing = 0.45;
    damageBuild(state, world, b, k.damage * (z.kind === 2 ? 6 : 2.5));
    return;
  }
  if (z.stuck >= 1) {
    const top = groundAt(world, ax, az, z.y + 1.8, state.builds);
    if (top - z.y > 0.6 && top - z.y < 2.4) { z.x = ax; z.z = az; z.y = top; z.stuck = 0; }
  }
}

/** Spawns `count` roaming infected around the island (away from everyone; inside the storm once it forms). */
function roam(state: ShooterState, map: ShooterMap, world: World, rng: Rng, count: number): void {
  const s = state.storm;
  for (let c = 0; c < count; c++) for (let tries = 0; tries < 8; tries++) {
    const spot = rng.chance(0.6) ? rng.pick(map.lootSpots) : rng.pick(map.pois);
    if (!spot) return;
    const [x, z] = clearSpot(map, world, spot.x + rng.range(-12, 12), spot.z + rng.range(-12, 12), 6, rng);
    if (s.stage >= 0 && Math.hypot(x - s.toX, z - s.toZ) > s.toR + 30) continue;
    if (state.players.some(p => p.mode !== 'dead' && p.mode !== 'bus' && Math.hypot(p.x - x, p.z - z) < 35)) continue;
    const r = rng.next();
    spawnZombie(state, world, r < 0.03 ? 2 : r < 0.1 ? 3 : r < 0.4 ? 1 : 0, x, z, 1, false);
    break;
  }
}

/** Apocalypse Royale: the horde is already out there when the drop starts. */
export function seedInfected(state: ShooterState, map: ShooterMap, world: World, rng: Rng): void {
  for (let i = 0; i < APOC_CAP - 4; i++) roam(state, map, world, rng, 1);
}

/** The map's Zombies arena anchors (`maps/anchors.ts`) when it has a usable one. */
const arenaOf = (map: ShooterMap) => { const z = map.anchors?.zombies; return z && z.r > 0 && z.spawns.length >= 4 ? z : null; };
/** The Zombies base: the map's Zombies arena, else the first town (the arena centre). */
export const zombieBase = (map: ShooterMap): [number, number] => { const a = arenaOf(map); return a ? [a.cx, a.cz] : arenaCentre(map); };

/** A wall-buy: where it hangs (facing `yaw`), the gun and its price. */
export interface BuyWall { x: number; y: number; z: number; yaw: number; w: WeaponId; price: number }
const walls = new Map<string, BuyWall[]>();
/** Buy-walls around the Zombies base: loot spots nearest the base, spaced out, cheapest first. Deterministic per map. */
export function buyWallsFor(map: ShooterMap): BuyWall[] {
  const hit = walls.get(map.id);
  if (hit) return hit;
  // The map's own wall-buys (chalk outlines on the outer walls round the arena) when it has them.
  const anchored = arenaOf(map)?.buyWalls ?? [];
  if (anchored.length >= 3) {
    const list = anchored.map(w => ({ x: w.x, y: w.y, z: w.z, yaw: w.yaw, w: w.weapon, price: w.cost }));
    walls.set(map.id, list);
    return list;
  }
  const [bx, bz] = zombieBase(map);
  const spots = map.lootSpots.map(s => ({ x: s.x, z: s.z, y: s.y ?? terrainHeight(map, s.x, s.z), d: Math.hypot(s.x - bx, s.z - bz) }))
    .filter(s => s.d > 6 && s.d < 70).sort((a, b) => a.d - b.d);
  const order: WeaponId[] = ['smg', 'shotgun', 'rifle', 'sniper', 'rocket', 'rifle'];
  const out: BuyWall[] = [];
  for (const s of spots) {
    if (out.length >= order.length) break;
    if (out.some(o => Math.hypot(o.x - s.x, o.z - s.z) < 9)) continue;
    const w = order[out.length];
    out.push({ x: s.x, y: s.y, z: s.z, yaw: Math.atan2(bx - s.x, bz - s.z), w, price: WALL_PRICE[w] });
  }
  walls.set(map.id, out);
  return out;
}
/** The buy-wall within reach of a fighter (index, or -1). */
export function nearestWall(state: ShooterState, p: Fighter, range = 2.4): number {
  if (state.mode !== 'zombies') return -1;
  const list = buyWallsFor(mapFor(state.map));
  let best = -1, bd = range;
  list.forEach((w, i) => { const d = Math.hypot(w.x - p.x, w.z - p.z); if (d < bd && Math.abs(w.y - p.y) < 2.5) { bd = d; best = i; } });
  return best;
}
/** Buys from the wall in reach (or refills its ammo at half price). Returns whether a wall was in reach. */
export function buyWall(state: ShooterState, p: Fighter): boolean {
  const i = nearestWall(state, p);
  if (i < 0) return false;
  const wall = buyWallsFor(mapFor(state.map))[i], def = WEAPONS[wall.w];
  const owned = p.weapons.findIndex(h => h?.w === wall.w);
  const price = owned >= 0 ? Math.round(wall.price / 2) : wall.price;
  if (p.points < price || (owned >= 0 && p.ammo[def.ammo] >= AMMO_MAX[def.ammo])) { emit(state, 'buy', p.id, wall.x, wall.y, wall.z, { value: i, flag: 0 }); return true; }
  p.points -= price;
  if (owned >= 0) { p.ammo[def.ammo] = AMMO_MAX[def.ammo]; p.weapons[owned]!.mag = def.mag; }
  else {
    const free = p.weapons.findIndex(h => h === null);
    const slot = free >= 0 ? free : p.current < p.weapons.length ? p.current : 0;
    p.weapons[slot] = { w: wall.w, rarity: 1, mag: def.mag, att: 0 };
    p.ammo[def.ammo] = Math.min(AMMO_MAX[def.ammo], p.ammo[def.ammo] + (wall.w === 'rocket' ? 4 : def.mag * 3));
    p.current = slot; p.reload = 0;
  }
  emit(state, 'buy', p.id, wall.x, wall.y, wall.z, { value: i, flag: 1 });
  return true;
}

/** Zombies: the base, spawn ring, landing zone and everyone's starting kit. */
export function setupZombies(state: ShooterState, map: ShooterMap, world: World, rng: Rng, obj: ModeState, rounds?: number): void {
  const [bx, bz] = zombieBase(map);
  const arena = arenaOf(map);
  obj.ax = bx; obj.az = bz; obj.ar = Math.max(115, (arena?.r ?? 0) + 60);
  obj.lastRound = clamp(Number.isInteger(rounds) ? rounds! : 8, 1, 30);
  bounds(state, bx, bz, obj.ar);
  state.storm.damage = 5;
  // The infected rise from the map's spawn ring (or a ring 42–72 m out); the helicopter lands on its pad (or 55 m out).
  if (arena) for (const sp of arena.spawns) obj.zspawns.push([sp.x, sp.z]);
  for (let k = 0; k < 40 && obj.zspawns.length < 14; k++) {
    const a = rng.range(0, Math.PI * 2), r = rng.range(42, 72);
    const [x, z] = clearSpot(map, world, bx + Math.cos(a) * r, bz + Math.sin(a) * r, 6, rng);
    if (Math.hypot(x - bx, z - bz) > 30) obj.zspawns.push([x, z]);
  }
  if (!obj.zspawns.length) obj.zspawns.push([bx + 50, bz]);
  const a = rng.range(0, Math.PI * 2);
  const ex = arena?.extraction;
  const [lx, lz] = ex && ex.r > 0 ? [ex.x, ex.z] : clearSpot(map, world, bx + Math.cos(a) * 55, bz + Math.sin(a) * 55, 10, rng);
  obj.lzX = lx; obj.lzZ = lz; obj.lzY = ex && ex.r > 0 ? ex.y : groundAt(world, lx, lz, terrainHeight(map, lx, lz) + 0.5);
  state.players.forEach((p, i) => {
    const start = arena?.start[i % Math.max(1, arena.start.length)];
    const [x, z] = start ? clearSpot(map, world, start.x, start.z, 2, rng) : clearSpot(map, world, bx, bz, 9, rng);
    place(world, p, x, z, start ? start.yaw : rng.range(-Math.PI, Math.PI));
    zombieKit(p);
    p.points = 500; p.mats = [150, 0, 0];
  });
  obj.wave = WAVE_PRE; obj.timer = 8; obj.round = 0;
}

/** Infected per round (more with more survivors), alive at once, and seconds between spawns. */
export const waveSize = (round: number, fighters: number): number => Math.round((5 + round * 4) * clamp(0.55 + fighters * 0.12, 0.7, 2.2));
const aliveCap = (round: number): number => Math.min(MAX_INFECTED, 12 + round * 5);
const spawnGap = (round: number): number => Math.max(0.3, 1.6 - round * 0.12);
function kindFor(round: number, r: number): ZombieKind {
  if (round >= 4 && r < 0.04 + round * 0.006) return 2;
  if (round >= 3 && r < 0.17) return 3;
  if (round >= 2 && r < 0.42 + round * 0.02) return 1;
  return 0;
}

/** Zombies rounds: intermission, waves, the helicopter; decides the match (extracted or overrun). */
export function stepZombies(state: ShooterState, map: ShooterMap, world: World, rng: Rng, obj: ModeState): void {
  while (state.loot.length > ZOMBIE_LOOT_CAP) { state.loot.shift(); state.lootRev++; }
  if (!state.players.some(p => p.mode === 'ground')) { overrun(state, obj); return; }
  const spawn = (round: number) => {
    obj.spawnT -= STEP;
    if (obj.spawnT > 0 || state.infected.length >= aliveCap(round)) return;
    obj.spawnT = spawnGap(round);
    // The spawn point farthest from everyone of a few tries (never on top of a survivor).
    let bx = 0, bz = 0, bd = -1;
    for (let t = 0; t < 4; t++) {
      const [x, z] = obj.zspawns[rng.int(obj.zspawns.length)];
      let near = 999;
      for (const p of state.players) if (p.mode !== 'dead') near = Math.min(near, Math.hypot(p.x - x, p.z - z));
      if (near > bd) { bd = near; bx = x; bz = z; }
    }
    spawnZombie(state, world, kindFor(round, rng.next()), bx + rng.range(-2, 2), bz + rng.range(-2, 2), 1 + 0.13 * (round - 1), true);
    if (obj.wave === WAVE_ON) obj.left--;
  };
  if (obj.wave === WAVE_PRE) {
    obj.timer -= STEP;
    if (obj.timer > 0) return;
    obj.round++; obj.wave = WAVE_ON; obj.spawnT = 1;
    obj.left = waveSize(obj.round, Math.min(MAX_FIGHTERS, state.players.length));
    // The dead come back for the new round with a pistol.
    for (const p of state.players) {
      if (p.mode !== 'dead' || p.respawn !== -1) continue;
      const [x, z] = clearSpot(map, world, obj.ax, obj.az, 9, rng);
      place(world, p, x, z, 0); zombieKit(p); p.respawn = 0;
      emit(state, 'spawn', p.id, p.x, p.y, p.z, { value: 0 });
    }
    emit(state, 'round', -1, obj.ax, 0, obj.az, { value: obj.round });
    return;
  }
  if (obj.wave === WAVE_ON) {
    if (obj.left > 0) spawn(obj.round);
    if (obj.left > 0 || state.infected.some(z => z.hp > 0)) return;
    // Round cleared: a breather, wood for barricades and a grenade; after the last round, call the helicopter.
    for (const p of state.players) if (p.mode === 'ground') { p.mats[0] = Math.min(500, p.mats[0] + 40); p.lethalN = Math.max(p.lethalN, 1); p.points += 100; }
    if (obj.round >= obj.lastRound) { obj.wave = WAVE_HELI; obj.timer = HELI_ETA; emit(state, 'extract', -1, obj.lzX, obj.lzY, obj.lzZ, { flag: 0, value: obj.round }); }
    else { obj.wave = WAVE_PRE; obj.timer = 10; }
    return;
  }
  // The helicopter: the infected keep coming, harder, until it leaves.
  spawn(obj.round + 1);
  obj.timer -= STEP;
  if (obj.timer > 0) return;
  if (obj.wave === WAVE_HELI) { obj.wave = WAVE_BOARD; obj.timer = BOARD_TIME; emit(state, 'extract', -1, obj.lzX, obj.lzY, obj.lzZ, { flag: 1 }); return; }
  const aboard = state.players.filter(p => p.mode === 'ground' && Math.hypot(p.x - obj.lzX, p.z - obj.lzZ) <= LZ_R && Math.abs(p.y - obj.lzY) < 6);
  if (!aboard.length) { overrun(state, obj); return; }
  obj.aboard = aboard.length; obj.wave = WAVE_OVER; obj.over = 0;
  crown(state, 0);
  for (const p of state.players) p.place = aboard.includes(p) ? 1 : 2;
  state.winner = aboard[0].id;
  emit(state, 'extract', aboard[0].id, obj.lzX, obj.lzY, obj.lzZ, { flag: 2, value: aboard.length });
  emit(state, 'win', aboard[0].id, obj.lzX, obj.lzY, obj.lzZ, { value: 0 });
}

/** Nobody left standing (or nobody made the helicopter): the infected win. */
function overrun(state: ShooterState, obj: ModeState): void {
  if (obj.over >= 0) return;
  obj.wave = WAVE_OVER; obj.over = 0;
  state.winTeam = -1;
  for (const p of state.players) p.place = 2;
  emit(state, 'extract', -1, obj.ax, 0, obj.az, { flag: 3, value: obj.round });
}
