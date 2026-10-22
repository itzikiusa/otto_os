/**
 * Grenades, lingering areas and killstreaks (host-authoritative, part of the
 * 60 Hz sim). Grenades fly in an arc and bounce (frag, smoke, flash) or burst
 * on impact (molotov, the spitter's acid): frags explode, molotovs and acid
 * leave a burning / corroding patch, smoke a cloud that blocks bots' sight,
 * flashes blind whoever looks at them. Killstreaks: a UAV puts every enemy on
 * the team's radar, an airstrike drops a line of bombs through the point
 * under the crosshair after a warning, an attack drone follows its owner and
 * shoots enemies (and the infected) close by. Allocation-free per tick apart
 * from spawning the occasional object.
 */
import { clamp, type Rng } from '../../common/math.ts';
import { terrainHeight, type ShooterMap } from '../map.ts';
import { groundAt, raycast, visible, type World } from '../collide.ts';
import { NADES, STREAKS, type NadeKind } from '../weapons.ts';
import {
  blast, emit, EYE, FROM_INFECTED, fwd, hurt, KILL_ACID, KILL_AIRSTRIKE, KILL_DRONE, KILL_FIRE, KILL_FRAG, STEP, standing,
  type Fighter, type ShooterState,
} from '../sim.ts';
import { hurtZombie } from '../zombies.ts';

/** A grenade (or acid glob) in flight; it rests once it stops bouncing. */
export interface Nade { id: number; owner: number; team: number; kind: NadeKind; x: number; y: number; z: number; vx: number; vy: number; vz: number; fuse: number; rest: boolean }
/** Area kinds: smoke cloud, fire, acid, airstrike warning. */
export const AREA_SMOKE = 0, AREA_FIRE = 1, AREA_ACID = 2, AREA_STRIKE = 3;
export interface Area { id: number; kind: number; owner: number; team: number; x: number; y: number; z: number; r: number; t: number; tick: number }
export interface Drone { id: number; owner: number; team: number; x: number; y: number; z: number; yaw: number; t: number; cool: number }
/** An airstrike on the way (host only): first bomb in `t` seconds, `n` bombs left, along (dx, dz). */
export interface Strike { owner: number; team: number; x: number; z: number; dx: number; dz: number; t: number; n: number; area: number }
export const MAX_FLYING = 16;
export const MAX_AREAS = 16;
export const MAX_DRONES = 4;
const GRAVITY = 20;
const THROW = 17;

/** Throws a grenade along the fighter's aim with a little loft. */
export function throwNade(state: ShooterState, p: Fighter, kind: NadeKind): void {
  if (state.nades.length >= MAX_FLYING) return;
  const [fx, fz] = fwd(p.yaw), cp = Math.cos(p.pitch), sp = Math.sin(p.pitch);
  const x = p.x + fx * 0.6, y = p.y + EYE + 0.1, z = p.z + fz * 0.6;
  state.nades.push({ id: state.nextId++, owner: p.id, team: p.team, kind, x, y, z, vx: fx * cp * THROW + p.vx * 0.5, vy: sp * THROW + 4, vz: fz * cp * THROW + p.vz * 0.5, fuse: NADES[kind].fuse || 6, rest: false });
  emit(state, 'nade', p.id, x, y, z, { value: kind });
}

/** Lobs a glob at a point (the spitter): a ballistic arc that lands there. */
export function lobAt(state: ShooterState, owner: number, team: number, kind: NadeKind, x: number, y: number, z: number, tx: number, ty: number, tz: number): void {
  if (state.nades.length >= MAX_FLYING) return;
  const dx = tx - x, dz = tz - z, d = Math.hypot(dx, dz), t = clamp(d / 15, 0.35, 1.6);
  state.nades.push({ id: state.nextId++, owner, team, kind, x, y, z, vx: dx / t, vy: (ty - y + 0.5 * GRAVITY * t * t) / t, vz: dz / t, fuse: 6, rest: false });
  emit(state, 'nade', owner, x, y, z, { value: kind });
}

/** Per tick: grenades, areas, airstrikes, drones and UAV timers. */
export function stepGear(state: ShooterState, map: ShooterMap, world: World, rng: Rng): void {
  void rng;
  for (let i = state.nades.length - 1; i >= 0; i--) {
    const n = state.nades[i], def = NADES[n.kind];
    n.fuse -= STEP;
    let burst = n.fuse <= 0;
    if (!n.rest && !burst) {
      n.vy -= GRAVITY * STEP;
      const sp = Math.hypot(n.vx, n.vy, n.vz), step = sp * STEP;
      if (sp > 1e-3) {
        const hit = raycast(world, n.x, n.y, n.z, n.vx / sp, n.vy / sp, n.vz / sp, step, true, state.builds);
        if (hit.kind !== 'none' && hit.t < step) {
          // Impact grenades burst; the others bounce off walls and settle on the ground.
          if (def.fuse === 0) { const k = Math.max(0, hit.t - 0.1) / sp; n.x += n.vx * k; n.y += n.vy * k; n.z += n.vz * k; burst = true; }
          else if (n.vy < 0 && (hit.kind === 'terrain' || hit.t < step * 0.5)) { n.vy = -n.vy * 0.32; n.vx *= 0.55; n.vz *= 0.55; if (Math.abs(n.vy) < 1.6) n.rest = true; }
          else { n.vx = -n.vx * 0.4; n.vz = -n.vz * 0.4; }
        }
      }
      if (!burst) {
        n.x += n.vx * STEP; n.y += n.vy * STEP; n.z += n.vz * STEP;
        const g = groundAt(world, n.x, n.z, n.y + 0.3, state.builds);
        if (n.y <= g + 0.08) {
          n.y = g + 0.08;
          if (def.fuse === 0) burst = true;
          else { n.vy = Math.abs(n.vy) * 0.3; n.vx *= 0.6; n.vz *= 0.6; if (n.vy < 1.6) { n.rest = true; n.vy = 0; } }
        }
        // An impact grenade that meets a fighter (other than its thrower) bursts on them.
        if (def.fuse === 0 && !burst) for (const q of state.players) {
          if (q.id === n.owner || !standing(q) || Math.abs(q.y + 1 - n.y) > 1.1 || Math.hypot(q.x - n.x, q.z - n.z) > 0.7) continue;
          if (n.kind === 4) hurt(state, q, 15, FROM_INFECTED, false, true, KILL_ACID);
          burst = true; break;
        }
      }
    }
    if (n.y < terrainHeight(map, n.x, n.z) - 2) { state.nades.splice(i, 1); continue; }
    if (!burst) continue;
    state.nades.splice(i, 1);
    detonate(state, world, n);
  }
  for (let i = state.areas.length - 1; i >= 0; i--) {
    const a = state.areas[i];
    a.t -= STEP;
    if (a.t <= 0) { state.areas.splice(i, 1); continue; }
    if (a.kind !== AREA_FIRE && a.kind !== AREA_ACID) continue;
    a.tick += STEP;
    if (a.tick < 0.5) continue;
    a.tick -= 0.5;
    // Burning / corroding patches: damage every half second to anyone standing in them (the infected too).
    const per = a.kind === AREA_FIRE ? NADES[1].damage : NADES[4].damage;
    for (const q of state.players) {
      if (q.mode !== 'ground' && q.mode !== 'down') continue;
      if (Math.abs(q.y - a.y) > 2.5 || Math.hypot(q.x - a.x, q.z - a.z) > a.r) continue;
      hurt(state, q, per, a.owner, false, true, a.kind === AREA_FIRE ? KILL_FIRE : KILL_ACID);
    }
    if (a.kind === AREA_FIRE) for (const z of state.infected) if (z.hp > 0 && Math.abs(z.y - a.y) < 2.5 && Math.hypot(z.x - a.x, z.z - a.z) <= a.r) hurtZombie(state, z, per * 2, a.owner, false, KILL_FIRE);
  }
  for (let i = state.strikes.length - 1; i >= 0; i--) {
    const s = state.strikes[i];
    s.t -= STEP;
    if (s.t > 0) continue;
    // Five bombs, 7 m apart, centred on the marked point.
    const k = 5 - s.n - 2, bx = s.x + s.dx * k * 7, bz = s.z + s.dz * k * 7;
    const by = groundAt(world, bx, bz, 300, state.builds);
    blast(state, world, s.owner, 0, 7.5, 150, 420, KILL_AIRSTRIKE, bx, by + 0.5, bz);
    s.n--; s.t = 0.2;
    if (s.n <= 0) { state.strikes.splice(i, 1); const at = state.areas.findIndex(a => a.id === s.area); if (at >= 0) state.areas.splice(at, 1); }
  }
  for (let i = state.drones.length - 1; i >= 0; i--) stepDrone(state, world, state.drones[i], i);
  for (let t = 0; t < state.uav.length; t++) if (state.uav[t] > 0) state.uav[t] = Math.max(0, state.uav[t] - STEP);
}

function addArea(state: ShooterState, kind: number, owner: number, team: number, x: number, y: number, z: number, r: number, t: number): number {
  if (state.areas.length >= MAX_AREAS) state.areas.shift();
  const id = state.nextId++;
  state.areas.push({ id, kind, owner, team, x, y, z, r, t, tick: 0.4 });
  return id;
}

function detonate(state: ShooterState, world: World, n: Nade): void {
  const def = NADES[n.kind];
  switch (n.kind) {
    case 0: blast(state, world, n.owner, 0, def.radius, def.damage, 260, KILL_FRAG, n.x, n.y + 0.2, n.z); break;
    case 1: addArea(state, AREA_FIRE, n.owner, n.team, n.x, n.y, n.z, def.radius, def.time); emit(state, 'fire', n.owner, n.x, n.y, n.z, { value: def.radius }); break;
    case 2: addArea(state, AREA_SMOKE, n.owner, n.team, n.x, n.y, n.z, def.radius, def.time); emit(state, 'smoke', n.owner, n.x, n.y, n.z, { value: def.radius }); break;
    case 3: flash(state, world, n); break;
    default: addArea(state, AREA_ACID, FROM_INFECTED, -1, n.x, n.y, n.z, def.radius, def.time); emit(state, 'fire', FROM_INFECTED, n.x, n.y, n.z, { value: def.radius, flag: 1 }); break;
  }
}

/** Blinds fighters who can see the flash, more the more directly they look (its thrower too, at half); stuns the infected. */
function flash(state: ShooterState, world: World, n: Nade): void {
  const R = NADES[3].radius;
  emit(state, 'flash', n.owner, n.x, n.y, n.z, { value: 0 });
  for (const q of state.players) {
    if (!standing(q) || q.mode !== 'ground') continue;
    const dx = n.x - q.x, dy = n.y - (q.y + EYE), dz = n.z - q.z, d = Math.hypot(dx, dy, dz);
    if (d > R || (q.team === n.team && q.id !== n.owner) || !visible(world, q.x, q.y + EYE, q.z, n.x, n.y + 0.2, n.z, state.builds)) continue;
    const [fx, fz] = fwd(q.yaw), cp = Math.cos(q.pitch);
    const facing = d < 1e-3 ? 1 : (fx * cp * dx + Math.sin(q.pitch) * dy + fz * cp * dz) / d;
    const t = (0.5 + 2.6 * Math.max(0, facing)) * (1 - d / R) * (q.id === n.owner ? 0.5 : 1);
    if (t < 0.3) continue;
    q.blind = Math.max(q.blind, t);
    if (!q.human) { q.bot.target = -1; q.bot.zt = -1; q.bot.seen = 0; }
    emit(state, 'flash', n.owner, q.x, q.y + 1.6, q.z, { target: q.id, value: Math.round(t * 10) });
  }
  for (const z of state.infected) if (z.hp > 0 && Math.hypot(z.x - n.x, z.z - n.z) < R * 0.6) { z.stun = Math.max(z.stun, 2.5); }
}

/** Is (x, z) inside a smoke cloud? (bots cannot see through or into smoke) */
export function inSmoke(state: ShooterState, x: number, y: number, z: number): boolean {
  for (const a of state.areas) if (a.kind === AREA_SMOKE && Math.abs(y - a.y) < 6 && Math.hypot(x - a.x, z - a.z) < a.r) return true;
  return false;
}
/** Does the segment between two points pass through smoke? (sampled: ends and middle) */
export function smokeBetween(state: ShooterState, ax: number, ay: number, az: number, bx: number, by: number, bz: number): boolean {
  if (!state.areas.length) return false;
  for (let k = 0; k <= 4; k++) { const t = k / 4; if (inSmoke(state, ax + (bx - ax) * t, ay + (by - ay) * t, az + (bz - az) * t)) return true; }
  return false;
}

/** Calls in the earliest earned killstreak. */
export function useStreak(state: ShooterState, map: ShooterMap, world: World, p: Fighter, rng: Rng): void {
  void map; void rng;
  let k = 0;
  while (k < STREAKS.length && !(p.ready & (1 << k))) k++;
  if (k >= STREAKS.length) return;
  const id = STREAKS[k].id;
  if (id === 'uav') {
    while (state.uav.length <= p.team) state.uav.push(0);
    state.uav[p.team] = Math.max(state.uav[p.team], STREAKS[k].time);
  } else if (id === 'airstrike') {
    // The point under the crosshair (or 70 m ahead when aiming at the sky).
    const [fx, fz] = fwd(p.yaw), cp = Math.cos(p.pitch);
    const dx = fx * cp, dy = Math.sin(p.pitch), dz = fz * cp;
    const hit = raycast(world, p.x, p.y + EYE, p.z, dx, dy, dz, 240, true, state.builds);
    const t = hit.kind === 'none' ? 70 : hit.t;
    const x = p.x + dx * t, z = p.z + dz * t;
    const area = addArea(state, AREA_STRIKE, p.id, p.team, x, groundAt(world, x, z, 300, state.builds), z, 18, 4);
    state.strikes.push({ owner: p.id, team: p.team, x, z, dx: fx, dz: fz, t: 2.6, n: 5, area });
  } else {
    if (state.drones.length >= MAX_DRONES) return;
    state.drones.push({ id: state.nextId++, owner: p.id, team: p.team, x: p.x, y: p.y + 6, z: p.z, yaw: p.yaw, t: STREAKS[k].time, cool: 1 });
  }
  p.ready &= ~(1 << k);
  emit(state, 'streak', p.id, p.x, p.y, p.z, { value: k, flag: 1 });
}

/** An attack drone: orbits above its owner, shoots the nearest enemy or infected it can see within 32 m. */
function stepDrone(state: ShooterState, world: World, d: Drone, index: number): void {
  d.t -= STEP;
  const owner = state.players[d.owner];
  if (d.t <= 0 || !owner) { state.drones.splice(index, 1); return; }
  const a = state.clock * 0.9 + d.id;
  const tx = owner.mode === 'dead' ? d.x : owner.x + Math.cos(a) * 3.5, tz = owner.mode === 'dead' ? d.z : owner.z + Math.sin(a) * 3.5;
  const ty = Math.max(owner.y, terrainHeight(world.map, tx, tz)) + 6.5;
  const k = 1 - Math.exp(-2.5 * STEP);
  d.x += (tx - d.x) * k; d.y += (ty - d.y) * k; d.z += (tz - d.z) * k;
  d.cool -= STEP;
  if (d.cool > 0) return;
  d.cool = 0.45;
  let best = 32, fx = 0, fy = 0, fz = 0, foe: Fighter | null = null, zi = -1;
  for (const q of state.players) {
    if (q.team === d.team || q.mode === 'dead' || q.mode === 'bus' || q.mode === 'sky') continue;
    const r = Math.hypot(q.x - d.x, q.y + 1 - d.y, q.z - d.z);
    if (r < best && visible(world, d.x, d.y, d.z, q.x, q.y + 1.1, q.z, state.builds)) { best = r; foe = q; zi = -1; fx = q.x; fy = q.y + 1.1; fz = q.z; }
  }
  for (let i = 0; i < state.infected.length; i++) {
    const z = state.infected[i];
    if (z.hp <= 0) continue;
    const r = Math.hypot(z.x - d.x, z.y + 1 - d.y, z.z - d.z);
    if (r < best && visible(world, d.x, d.y, d.z, z.x, z.y + 1.1, z.z, state.builds)) { best = r; foe = null; zi = i; fx = z.x; fy = z.y + 1.1; fz = z.z; }
  }
  if (!foe && zi < 0) return;
  d.yaw = Math.atan2(fx - d.x, fz - d.z);
  emit(state, 'shot', d.owner, d.x, d.y - 0.3, d.z, { value: 1, flag: 2, x2: fx, y2: fy, z2: fz });
  if (foe) hurt(state, foe, 16, d.owner, false, true, KILL_DRONE);
  else hurtZombie(state, state.infected[zi], 22, d.owner, false, KILL_DRONE);
}
