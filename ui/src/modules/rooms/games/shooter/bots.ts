/**
 * Storm Royale bots. Each tick a bot produces the same input a human would.
 * Priorities: escape the storm, fight a visible enemy (reaction delay, aim
 * error and burst discipline scale with difficulty), loot when under-armed,
 * otherwise roam toward the safe zone. Heavy checks (visibility) run on a
 * think timer, not every tick.
 */
import { clamp, type Rng, wrapAngle } from '../common/math.ts';
import { HALF, terrainHeight, type ShooterMap } from './map.ts';
import { visible, type World } from './collide.ts';
import { WEAPONS, type WeaponId } from './weapons.ts';
import { EYE, nearestWeaponLoot, nearestClosedChest, idleShooterInput, type Difficulty, type Fighter, type ShooterInput, type ShooterState } from './sim.ts';

interface Skill { reaction: number; error: number; sight: number; burst: number; pause: number; strafe: number }
export const SKILL: Record<Difficulty, Skill> = {
  easy: { reaction: 0.95, error: 0.11, sight: 55, burst: 0.35, pause: 0.9, strafe: 0.4 },
  normal: { reaction: 0.55, error: 0.06, sight: 80, burst: 0.6, pause: 0.5, strafe: 0.75 },
  hard: { reaction: 0.3, error: 0.03, sight: 105, burst: 1.0, pause: 0.25, strafe: 1 },
};
const THINK = 0.25;

/** Best owned weapon slot for a distance (shotgun close, sniper far, rifle mid). */
export function weaponFor(p: Fighter, distance: number): number {
  let best = -1, score = -Infinity;
  p.weapons.forEach((w, i) => {
    if (!w) return;
    const def = WEAPONS[w.w];
    if (w.mag <= 0 && p.ammo[def.ammo] <= 0) return;
    const pref: Record<WeaponId, number> = {
      shotgun: distance < 12 ? 10 : distance < 22 ? 3 : -5,
      smg: distance < 25 ? 7 : 2,
      rifle: distance < 90 ? 6 : 3,
      pistol: 2,
      sniper: distance > 45 ? 9 : -2,
      rocket: distance > 10 && distance < 70 ? 5 : -8,
    };
    const s = pref[w.w] + w.rarity * 0.5;
    if (s > score) { score = s; best = i; }
  });
  return best;
}

export function botThink(state: ShooterState, map: ShooterMap, world: World, p: Fighter, rng: Rng): ShooterInput {
  const b = p.bot, skill = SKILL[state.difficulty];
  const input = idleShooterInput(p.yaw, p.pitch);
  if (p.mode === 'bus') return input;
  if (p.mode === 'sky' || p.mode === 'glide') {
    steerTo(input, p, b.goalX, b.goalZ);
    return input;
  }
  b.think -= 1 / 60;
  b.strafeT -= 1 / 60;
  b.jumpT -= 1 / 60;
  if (b.think <= 0) {
    b.think = THINK + rng.range(0, 0.1);
    // Stuck detection: barely moved while trying to.
    const moved = Math.hypot(p.x - b.lastX, p.z - b.lastZ);
    b.stuck = moved < 0.35 ? b.stuck + THINK : 0;
    b.lastX = p.x; b.lastZ = p.z;
    // Acquire / keep a visible target.
    let target = -1, bestD = skill.sight;
    for (const q of state.players) {
      if (q.id === p.id || q.mode === 'dead' || q.mode === 'bus' || q.mode === 'sky') continue;
      const d = Math.hypot(q.x - p.x, q.z - p.z);
      if (d > bestD) continue;
      if (visible(world, p.x, p.y + EYE, p.z, q.x, q.y + 1.2, q.z)) { target = q.id; bestD = d; }
    }
    if (target >= 0 && target !== b.target) { b.seen = 0; b.errX = rng.range(-1, 1) * skill.error * 3; b.errY = rng.range(-1, 1) * skill.error * 2; }
    if (target < 0 && b.target >= 0 && b.seen > 0) b.seen = Math.max(0, b.seen - 0.5);
    b.target = target >= 0 ? target : b.seen > 0.3 ? b.target : -1;
    // Choose a goal when not fighting.
    if (b.target < 0) chooseGoal(state, map, p, rng);
  }
  const s = state.storm;
  const outside = Math.hypot(p.x - s.cx, p.z - s.cz) > s.r - 4;
  const target = b.target >= 0 ? state.players[b.target] : undefined;
  const armed = p.weapons.some(w => w && (w.mag > 0 || p.ammo[WEAPONS[w.w].ammo] > 0));

  if (target && target.mode !== 'dead' && armed && !(outside && s.damage >= 4)) {
    b.seen += 1 / 60;
    const dx = target.x - p.x, dz = target.z - p.z, d = Math.hypot(dx, dz);
    const slot = weaponFor(p, d);
    if (slot >= 0 && slot !== p.current) input.slot = slot + 1;
    // Aim with an error that shrinks the longer the target stays in view.
    const settle = Math.max(0.15, 1 - b.seen / (skill.reaction * 3));
    const lead = target.mode === 'ground' ? 0.12 : 0;
    const ax = dx + target.vx * lead * d * 0.05, az = dz + target.vz * lead * d * 0.05;
    const aimYaw = Math.atan2(ax, az) + b.errX * settle;
    const aimPitch = Math.atan2(target.y + 1.15 - (p.y + EYE), Math.max(0.5, d)) + b.errY * settle;
    input.yaw = p.yaw + clamp(wrapAngle(aimYaw - p.yaw), -0.22, 0.22);
    input.pitch = clamp(p.pitch + clamp(aimPitch - p.pitch, -0.12, 0.12), -1.3, 1.3);
    const onTarget = Math.abs(wrapAngle(aimYaw - p.yaw)) < 0.08 + 2 / Math.max(4, d);
    const held = p.weapons[p.current];
    const def = held ? WEAPONS[held.w] : null;
    // Burst discipline: fire for `burst` seconds, then pause.
    b.burst += 1 / 60;
    const cycle = skill.burst + skill.pause;
    const inBurst = b.burst % cycle < skill.burst;
    if (b.seen > skill.reaction && onTarget && held && held.mag > 0 && (inBurst || !def?.auto)) input.fire = def?.auto ? true : rng.chance(0.25);
    if (held && held.mag === 0) input.reload = true;
    input.aim = !!def && (def.id === 'sniper' || def.id === 'rifle') && d > 25;
    // Strafe and hold a preferred range.
    if (b.strafeT <= 0) { b.strafe = rng.chance(0.5) ? 1 : -1; b.strafeT = rng.range(0.5, 1.4); }
    const want = def?.id === 'shotgun' ? 6 : def?.id === 'sniper' ? 70 : def?.id === 'smg' ? 14 : 28;
    input.mx = b.strafe * skill.strafe;
    input.mz = d > want + 6 ? 0.9 : d < want - 6 ? -0.7 : 0;
    if (state.difficulty !== 'easy' && b.jumpT <= 0 && rng.chance(0.01)) { input.jump = true; b.jumpT = 2; }
    return input;
  }

  // Not fighting: storm first, then loot, then roam.
  if (outside) { b.goalX = s.toX; b.goalZ = s.toZ; }
  steerTo(input, p, b.goalX, b.goalZ);
  const goalD = Math.hypot(b.goalX - p.x, b.goalZ - p.z);
  input.sprint = goalD > 10;
  if (goalD < 1.6) { input.mz = 0; input.mx = 0; }
  if (b.stuck > 0.6) { input.jump = true; input.mx = b.strafe; if (b.stuck > 1.6) { b.goalX = p.x + rng.range(-20, 20); b.goalZ = p.z + rng.range(-20, 20); b.stuck = 0; } }
  // Swap into better floor weapons and open nearby chests.
  const floor = nearestWeaponLoot(state, p);
  if (floor?.weapon) {
    const worst = p.weapons.reduce((m, w) => Math.min(m, w ? w.rarity : -1), 9);
    if (worst < floor.rarity || p.weapons.some(w => !w)) input.interact = (state.clock * 4) % 2 < 1;
  }
  if (nearestClosedChest(state, p) >= 0) { input.interact = true; input.mz = 0; input.mx = 0; }
  const held = p.weapons[p.current];
  if (held && held.mag < WEAPONS[held.w].mag * 0.5) input.reload = true;
  return input;
}

function steerTo(input: ShooterInput, p: Fighter, x: number, z: number): void {
  const want = Math.atan2(x - p.x, z - p.z);
  input.yaw = p.yaw + clamp(wrapAngle(want - p.yaw), -0.12, 0.12);
  input.pitch = p.pitch * 0.9;
  input.mz = Math.abs(wrapAngle(want - p.yaw)) < 1.2 ? 1 : 0.3;
}

function chooseGoal(state: ShooterState, map: ShooterMap, p: Fighter, rng: Rng): void {
  const b = p.bot;
  const s = state.storm;
  const armed = p.weapons.filter(Boolean).length;
  const needsAmmo = p.weapons.some(w => w && w.mag === 0 && p.ammo[WEAPONS[w.w].ammo] === 0);
  // Loot the nearest useful item within reach of the safe zone.
  let best = -1, bd = armed === 0 ? 90 : 45;
  state.loot.forEach((l, i) => {
    const useful = l.kind === 'weapon' ? armed < 3 || p.weapons.some(w => w && w.rarity < l.rarity)
      : l.kind === 'ammo' ? needsAmmo || p.weapons.some(w => w && WEAPONS[w.w].ammo === l.ammo)
      : l.kind === 'heal' ? p.hp < 100 : p.shield < 100;
    if (!useful) return;
    const d = Math.hypot(l.x - p.x, l.z - p.z);
    if (d < bd && Math.hypot(l.x - s.toX, l.z - s.toZ) < Math.max(s.toR, 20) + 30) { bd = d; best = i; }
  });
  if (best >= 0) { b.goalX = state.loot[best].x; b.goalZ = state.loot[best].z; b.loot = state.loot[best].id; return; }
  map.chests.forEach((c, i) => {
    if (state.chests[i]) return;
    const d = Math.hypot(c.x - p.x, c.z - p.z);
    if (d < bd) { bd = d; best = i; }
  });
  if (best >= 0) { b.goalX = map.chests[best].x + 1.4; b.goalZ = map.chests[best].z; return; }
  // Roam inside the next safe circle (on land).
  if (Math.hypot(b.goalX - p.x, b.goalZ - p.z) < 4 || Math.hypot(b.goalX - s.toX, b.goalZ - s.toZ) > s.toR) {
    for (let t = 0; t < 8; t++) {
      const a = rng.range(0, Math.PI * 2), r = rng.range(0, Math.max(6, s.toR * 0.8));
      const x = clamp(s.toX + Math.cos(a) * r, -HALF + 10, HALF - 10), z = clamp(s.toZ + Math.sin(a) * r, -HALF + 10, HALF - 10);
      if (map.theme !== 'isle' || terrainHeight(map, x, z) > 0.8) { b.goalX = x; b.goalZ = z; break; }
    }
  }
}
