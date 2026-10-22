/**
 * Storm Royale game modes (host-authoritative; part of the 60 Hz sim):
 *
 * - `royale`: the battle royale (see sim.ts).
 * - `apoc`: Apocalypse Royale: the battle royale under a toxic cloud with
 *   infected roaming the island (zombies.ts keeps the horde topped up).
 * - `tdm`: Team Deathmatch: two teams in an arena around a town, class
 *   loadouts, respawns at the team's side; first to the score limit (or
 *   ahead when time runs out) wins.
 * - `dom`: Domination: the same arena with three capture points (A near one
 *   spawn, B in the middle, C near the other); held points score every two
 *   seconds.
 * - `zombies`: co-op survival around a base against escalating rounds of
 *   infected (zombies.ts): points for hits and kills, buy-walls, barricades
 *   built and repaired with the pickaxe, the dead return at the next round,
 *   and after the last round a helicopter extraction.
 *
 * The arena modes and Zombies use the storm as a fixed boundary.
 */
import { clamp, type Rng } from '../../common/math.ts';
import { dryLand, HALF, mapFor, terrainHeight, type ShooterMap } from '../map.ts';
import { groundAt, resolve, worldFor, type World } from '../collide.ts';
import { CLASSES, weaponStats, WEAPONS, type WeaponId } from '../weapons.ts';
import type { ShooterMode } from '../catalog.ts';
import {
  crown, emit, emptyAmmo, SLOTS, STEP, standing,
  type Fighter, type ShooterInput, type ShooterState,
} from '../sim.ts';
import { seedInfected, setupZombies, stepZombies, zombieBase } from '../zombies.ts';

export const isRoyale = (m: ShooterMode): boolean => m === 'royale' || m === 'apoc';
/** Arena modes: fighters respawn with a class loadout. */
export const respawns = (m: ShooterMode): boolean => m === 'tdm' || m === 'dom';
export const hasInfected = (m: ShooterMode): boolean => m === 'zombies' || m === 'apoc';

/** A Domination point: owner team (-1 neutral), capture progress 0..1 and the team it is going to. */
export interface CapturePoint { x: number; y: number; z: number; r: number; own: number; cap: number; by: number }
export const POINT_NAMES = ['A', 'B', 'C'] as const;
/** Zombies phases: between rounds, a round under way, the helicopter inbound, landed and boarding, over. */
export const WAVE_PRE = 0, WAVE_ON = 1, WAVE_HELI = 2, WAVE_BOARD = 3, WAVE_OVER = 4;

export interface ModeState {
  /** Team scores (TDM eliminations, Domination points), the limit, seconds left (0: no clock); ending (s since decided, -1 while playing). */
  score: [number, number]; limit: number; time: number; over: number;
  /** The arena / Zombies base: centre, boundary radius and the two team spawns. */
  ax: number; az: number; ar: number; spawns: [number, number][];
  points: CapturePoint[]; tick: number;
  /** Zombies: round, phase (WAVE_*), phase timer, infected still to spawn this round, spawn timer, the extraction round, landing zone, survivors aboard. */
  round: number; wave: number; timer: number; left: number; spawnT: number; lastRound: number; lzX: number; lzY: number; lzZ: number; aboard: number;
  /** Where the infected come from (host only). */
  zspawns: [number, number][];
}

export const emptyMode = (): ModeState => ({
  score: [0, 0], limit: 0, time: 0, over: -1, ax: 0, az: 0, ar: 0, spawns: [], points: [], tick: 0,
  round: 0, wave: WAVE_PRE, timer: 0, left: 0, spawnT: 0, lastRound: 8, lzX: 0, lzY: 0, lzZ: 0, aboard: 0, zspawns: [],
});

/** TDM: eliminations to win (scaled down for small lobbies) and the clock; Domination: points to win and its clock. */
export const TDM_TIME = 300;
export const DOM_LIMIT = 150;
export const DOM_TIME = 360;
export const CAP_TIME = 6;
const ARENA_R = 78;
const SPAWN_OFF = 50;
const POINT_OFF = 28;

/** The middle of the action: the first town (or the first landmark). */
export function arenaCentre(map: ShooterMap): [number, number] {
  const poi = map.pois.find(p => p.kind === 'town') ?? map.pois[0];
  return poi ? [poi.x, poi.z] : [0, 0];
}

/** A clear spot (dry, not inside a prop) within `spread` of (x, z), or the point itself. */
export function clearSpot(map: ShooterMap, world: World, x: number, z: number, spread: number, rng: Rng): [number, number] {
  const body = { x: 0, y: 0, z: 0, vx: 0, vy: 0, vz: 0, grounded: true };
  for (let i = 0; i < 24; i++) {
    const r = i === 0 ? 0 : spread * Math.sqrt(rng.next()), a = rng.range(0, Math.PI * 2);
    const px = clamp(x + Math.cos(a) * r, -HALF + 8, HALF - 8), pz = clamp(z + Math.sin(a) * r, -HALF + 8, HALF - 8);
    if (!dryLand(map, px, pz)) continue;
    body.x = px; body.z = pz; body.y = terrainHeight(map, px, pz);
    if (resolve(world, body)) continue;
    return [px, pz];
  }
  return [x, z];
}

/** Puts a fighter on its feet at (x, z) facing `yaw`. */
export function place(world: World, p: Fighter, x: number, z: number, yaw: number): void {
  p.x = x; p.z = z; p.y = groundAt(world, x, z, terrainHeight(world.map, x, z) + 0.5);
  p.vx = 0; p.vy = 0; p.vz = 0; p.yaw = yaw; p.pitch = 0; p.grounded = true; p.mode = 'ground';
}

/** Equips a fighter with a class (weapons, ammo, grenades, wood), full health and some shield. */
export function giveLoadout(p: Fighter, cls: number, shield = 50): void {
  const c = CLASSES[clamp(cls, 0, CLASSES.length - 1)];
  p.cls = CLASSES.indexOf(c);
  p.weapons = Array.from({ length: SLOTS }, () => null);
  p.ammo = emptyAmmo();
  c.guns.forEach(([w, rarity, att], i) => {
    const def = WEAPONS[w], mag = weaponStats(def, att).mag;
    p.weapons[i] = { w, rarity, mag, att };
    p.ammo[def.ammo] = Math.min(p.ammo[def.ammo] + (w === 'rocket' ? 4 : mag * 3), 240);
  });
  p.current = 0;
  p.lethal = c.lethal; p.lethalN = 1; p.tacKind = c.tactical; p.tacN = 2;
  p.mats = [c.mats, 0, 0];
  p.hp = 100; p.shield = shield;
  resetCombat(p);
}

/** Clears transient combat state (after a respawn or a mode change). */
export function resetCombat(p: Fighter): void {
  p.reload = 0; p.cooldown = 0.4; p.useT = 0; p.ads = 0; p.kick = 0; p.kickYaw = 0; p.shots = 0; p.bloom = 0; p.crouch = false; p.slideT = 0; p.mantleT = 0;
  p.tactical = false; p.tacT = 4; p.blind = 0; p.building = false; p.interact = 0; p.revive = 0; p.knocker = -1; p.stormT = 0; p.hurtT = 0; p.killedBy = -1;
}

/** A pistol and a frag: what a Zombies survivor starts (and respawns) with. */
export function zombieKit(p: Fighter): void {
  p.weapons = Array.from({ length: SLOTS }, () => null);
  p.weapons[0] = { w: 'pistol' as WeaponId, rarity: 1, mag: WEAPONS.pistol.mag, att: 0 };
  p.ammo = emptyAmmo(); p.ammo.light = 64;
  p.current = 0;
  p.lethal = 0; p.lethalN = 2; p.tacKind = 3; p.tacN = 1;
  p.hp = 100; p.shield = 0;
  resetCombat(p);
}

/**
 * Lays out the mode at match start: the arena (spawns, capture points,
 * boundary) or the Zombies base, seats everyone on the ground with their gear,
 * or seeds Apocalypse Royale's horde. Called once by createMatch.
 */
export function setupMode(state: ShooterState, map: ShooterMap, rng: Rng, rounds?: number): ModeState {
  const obj = emptyMode();
  const world = worldOf(state);
  if (state.mode === 'royale') return obj;
  if (state.mode === 'apoc') { seedInfected(state, map, world, rng); obj.spawnT = 3; return obj; }
  state.phase = 'play';
  if (state.mode === 'zombies') {
    setupZombies(state, map, world, rng, obj, rounds);
    return obj;
  }
  // Arena: the town in the middle, the two spawns on opposite sides along the first axis with dry ground at both ends.
  const [cx, cz] = arenaCentre(map);
  let axis = rng.range(0, Math.PI * 2);
  for (let k = 0; k < 16; k++) {
    const a = axis + k * (Math.PI / 8), dx = Math.sin(a), dz = Math.cos(a);
    if ([-SPAWN_OFF, SPAWN_OFF, -POINT_OFF, POINT_OFF].every(o => dryLand(map, cx + dx * o, cz + dz * o))) { axis = a; break; }
  }
  const dx = Math.sin(axis), dz = Math.cos(axis);
  obj.ax = cx; obj.az = cz; obj.ar = ARENA_R;
  obj.spawns = [clearSpot(map, world, cx - dx * SPAWN_OFF, cz - dz * SPAWN_OFF, 6, rng), clearSpot(map, world, cx + dx * SPAWN_OFF, cz + dz * SPAWN_OFF, 6, rng)];
  if (state.mode === 'dom') {
    for (const o of [-POINT_OFF, 0, POINT_OFF]) {
      const [x, z] = clearSpot(map, world, cx + dx * o, cz + dz * o, 5, rng);
      obj.points.push({ x, y: groundAt(world, x, z, terrainHeight(map, x, z) + 0.5), z, r: 6, own: -1, cap: 0, by: -1 });
    }
    obj.limit = DOM_LIMIT; obj.time = DOM_TIME;
  } else {
    obj.limit = clamp(state.players.length * 2, 10, 30); obj.time = TDM_TIME;
  }
  bounds(state, cx, cz, ARENA_R);
  for (const p of state.players) spawnArena(state, world, obj, p, rng);
  return obj;
}

/** The storm as a fixed wall around the play area (it hurts outside, never moves). */
export function bounds(state: ShooterState, x: number, z: number, r: number): void {
  const s = state.storm;
  s.stage = 0; s.cx = s.fromX = s.toX = x; s.cz = s.fromZ = s.toZ = z; s.r = s.fromR = s.toR = r; s.timer = 0; s.shrinking = false; s.damage = 10;
}

/** Respawns a fighter at its team's side, at the candidate spot farthest from the enemy. */
function spawnArena(state: ShooterState, world: World, obj: ModeState, p: Fighter, rng: Rng): void {
  const [sx, sz] = obj.spawns[p.team] ?? obj.spawns[0];
  let best: [number, number] = [sx, sz], score = -Infinity;
  for (let k = 0; k < 5; k++) {
    const spot = clearSpot(world.map, world, sx, sz, 9, rng);
    let near = 200;
    for (const q of state.players) if (q.team !== p.team && standing(q) && q.mode !== 'dead') near = Math.min(near, Math.hypot(q.x - spot[0], q.z - spot[1]));
    if (near > score) { score = near; best = spot; }
  }
  place(world, p, best[0], best[1], Math.atan2(obj.ax - best[0], obj.az - best[1]));
  giveLoadout(p, p.cls);
}

/** Per tick: respawns, the clock, capture points and scores; Zombies runs its rounds. Decides and ends the match. */
export function stepMode(state: ShooterState, map: ShooterMap, world: World, rng: Rng, inputs: (ShooterInput | undefined)[]): void {
  const obj = state.obj;
  if (isRoyale(state.mode) || state.phase !== 'play') return;
  if (obj.over >= 0) {
    obj.over += STEP;
    if (obj.over > 3) state.phase = 'done';
    return;
  }
  if (state.mode === 'zombies') { stepZombies(state, map, world, rng, obj); return; }
  // Respawn modes: the dead come back after the wait, in the class they picked.
  for (const p of state.players) {
    if (p.mode !== 'dead' || p.respawn <= 0) continue;
    if (p.human) { const want = inputs[p.id]?.cls; if (Number.isInteger(want) && want! >= 0 && want! < CLASSES.length) p.cls = want!; }
    p.respawn -= STEP;
    if (p.respawn > 0) continue;
    p.respawn = 0;
    if (!p.human) p.cls = rng.chance(0.7) ? p.cls : rng.int(CLASSES.length);
    spawnArena(state, world, obj, p, rng);
    emit(state, 'spawn', p.id, p.x, p.y, p.z, { value: p.cls });
  }
  obj.time = Math.max(0, obj.time - STEP);
  if (state.mode === 'dom') stepPoints(state, obj);
  const [a, b] = obj.score;
  if (a >= obj.limit || b >= obj.limit || obj.time <= 0) finish(state, a === b ? -1 : a > b ? 0 : 1);
}

/** Domination: capture (faster with more people, contested points hold), and every two seconds a point per held point. */
function stepPoints(state: ShooterState, obj: ModeState): void {
  for (let i = 0; i < obj.points.length; i++) {
    const pt = obj.points[i];
    let n0 = 0, n1 = 0;
    for (const p of state.players) {
      if (p.mode !== 'ground' || Math.abs(p.y - pt.y) > 4 || Math.hypot(p.x - pt.x, p.z - pt.z) > pt.r) continue;
      if (p.team === 0) n0++; else n1++;
    }
    if (n0 > 0 && n1 > 0) continue;
    if (n0 + n1 === 0) { if (pt.own < 0 && pt.cap > 0) pt.cap = Math.max(0, pt.cap - STEP * 0.05); continue; }
    const t = n0 > 0 ? 0 : 1, rate = (1 + 0.35 * (Math.min(3, n0 + n1) - 1)) / CAP_TIME * STEP;
    if (pt.own === t) pt.cap = Math.min(1, pt.cap + rate);
    else if (pt.own >= 0) { pt.cap -= rate; if (pt.cap <= 0) { pt.own = -1; pt.cap = 0; pt.by = t; } }
    else if (pt.by !== t && pt.cap > 0) { pt.cap -= rate; if (pt.cap <= 0) { pt.cap = 0; pt.by = t; } }
    else {
      pt.by = t; pt.cap += rate;
      if (pt.cap >= 1) { pt.cap = 1; pt.own = t; emit(state, 'capture', -1, pt.x, pt.y, pt.z, { value: i, flag: t }); }
    }
  }
  obj.tick += STEP;
  if (obj.tick >= 2) {
    obj.tick -= 2;
    for (const pt of obj.points) if (pt.own >= 0) obj.score[pt.own]++;
  }
}

/** Ends an arena match: the winning team places first (a draw: nobody wins). */
export function finish(state: ShooterState, team: number): void {
  const obj = state.obj;
  obj.over = 0;
  if (team >= 0) {
    crown(state, team);
    for (const p of state.players) if (p.team !== team) p.place = 2;
    const w = state.players[state.winner];
    emit(state, 'win', state.winner, w.x, w.y, w.z, { value: team });
  } else for (const p of state.players) p.place = 2;
  for (const p of state.players) { p.building = false; if (p.mode === 'dead') p.respawn = 0; }
}

export { zombieBase };
/** The match's collision world (cached per map in collide.ts). */
const worldOf = (state: ShooterState): World => worldFor(mapFor(state.map));
