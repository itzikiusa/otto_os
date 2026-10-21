/**
 * Storm Royale bots. Each tick a bot produces the same input a human would.
 * Priorities: escape the storm, fight a visible enemy (reaction delay, aim
 * error and burst discipline scale with difficulty; wall up or ramp when
 * shot, duck behind cover to reload), heal when calm, loot when under-armed,
 * harvest materials when low, otherwise roam toward the safe zone. Heavy
 * checks (visibility, cover, trees) run on a think timer, not every tick.
 */
import { clamp, type Rng, wrapAngle } from '../common/math.ts';
import { dryLand, HALF, type ShooterMap } from './map.ts';
import { visible, type World } from './collide.ts';
import { BUILD_COST, canUse, CONSUMABLES, isConsumable, isWeapon, WEAPONS, type Material, type WeaponId } from './weapons.ts';
import { EYE, heldItem, nearestWeaponLoot, nearestClosedChest, idleShooterInput, PICKAXE_KEY, PICKAXE_SLOT, type Difficulty, type Fighter, type ShooterInput, type ShooterState } from './sim.ts';

interface Skill { reaction: number; error: number; sight: number; burst: number; pause: number; strafe: number; build: number; ramp: number; cover: number }
export const SKILL: Record<Difficulty, Skill> = {
  easy: { reaction: 0.95, error: 0.11, sight: 55, burst: 0.35, pause: 0.9, strafe: 0.4, build: 0.15, ramp: 0, cover: 0.3 },
  normal: { reaction: 0.55, error: 0.06, sight: 80, burst: 0.6, pause: 0.5, strafe: 0.75, build: 0.45, ramp: 0.25, cover: 0.65 },
  hard: { reaction: 0.3, error: 0.03, sight: 105, burst: 1.0, pause: 0.25, strafe: 1, build: 0.8, ramp: 0.5, cover: 1 },
};
const THINK = 0.25;
const DT = 1 / 60;

/** Best owned weapon slot for a distance (shotgun close, sniper far, rifle mid); consumables never count. */
export function weaponFor(p: Fighter, distance: number): number {
  let best = -1, score = -Infinity;
  p.weapons.forEach((w, i) => {
    if (!w || !isWeapon(w.w)) return;
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

/** Hotbar slot of a consumable worth using now (heals first when low on health), or -1. */
export function healFor(p: Fighter): number {
  let best = -1, score = 0;
  p.weapons.forEach((w, i) => {
    if (!w || !isConsumable(w.w) || !canUse(w.w, p.hp, p.shield)) return;
    const c = CONSUMABLES[w.w];
    const gain = c.heal ? Math.min(c.heal, Math.max(0, c.cap - p.hp)) * 1.2 : Math.min(c.shield, Math.max(0, c.cap - p.shield));
    if (gain > score) { score = gain; best = i; }
  });
  return score >= 12 ? best : -1;
}

/** The material a bot builds with: whatever it has most of. */
const richest = (p: Fighter): Material => (p.mats[2] >= p.mats[1] && p.mats[2] >= p.mats[0] ? 2 : p.mats[1] >= p.mats[0] ? 1 : 0);

export function botThink(state: ShooterState, map: ShooterMap, world: World, p: Fighter, rng: Rng): ShooterInput {
  const b = p.bot, skill = SKILL[state.difficulty];
  const input = idleShooterInput(p.yaw, p.pitch);
  if (p.mode === 'bus') return input;
  if (p.mode === 'sky' || p.mode === 'glide') {
    steerTo(input, p, b.goalX, b.goalZ);
    return input;
  }
  b.think -= DT;
  b.strafeT -= DT;
  b.jumpT -= DT;
  b.buildT -= DT;
  b.coverT -= DT;
  b.calm += DT;
  if (b.think <= 0) {
    b.think = THINK + rng.range(0, 0.1);
    // Stuck detection: barely moved while trying to.
    const moved = Math.hypot(p.x - b.lastX, p.z - b.lastZ);
    b.stuck = moved < 0.35 ? b.stuck + THINK : 0;
    b.lastX = p.x; b.lastZ = p.z;
    // Acquire / keep a visible target (structures block sight).
    let target = -1, bestD = skill.sight;
    for (const q of state.players) {
      if (q.id === p.id || q.mode === 'dead' || q.mode === 'bus' || q.mode === 'sky') continue;
      const d = Math.hypot(q.x - p.x, q.z - p.z);
      if (d > bestD) continue;
      if (visible(world, p.x, p.y + EYE, p.z, q.x, q.y + 1.2, q.z, state.builds)) { target = q.id; bestD = d; }
    }
    if (target >= 0 && target !== b.target) { b.seen = 0; b.errX = rng.range(-1, 1) * skill.error * 3; b.errY = rng.range(-1, 1) * skill.error * 2; }
    if (target < 0 && b.target >= 0 && b.seen > 0) b.seen = Math.max(0, b.seen - 0.5);
    b.target = target >= 0 ? target : b.seen > 0.3 ? b.target : -1;
    if (b.target >= 0) b.calm = 0;
    // Choose a goal when not fighting.
    if (b.target < 0) chooseGoal(state, map, p, rng);
  }
  const s = state.storm;
  const outside = Math.hypot(p.x - s.cx, p.z - s.cz) > s.r - 4;
  const target = b.target >= 0 ? state.players[b.target] : undefined;
  const armed = p.weapons.some(w => w && isWeapon(w.w) && (w.mag > 0 || p.ammo[WEAPONS[w.w].ammo] > 0));
  const mats = p.mats[0] + p.mats[1] + p.mats[2];

  if (target && target.mode !== 'dead' && armed && !(outside && s.damage >= 4)) {
    b.seen += DT;
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
    // Shot at and holding materials: throw up a wall toward the shooter, sometimes a ramp for height.
    if (p.hurtT > 0.3 && b.buildT <= 0 && p.cooldown <= 0 && !p.swimming && mats >= BUILD_COST && d > 6 && rng.chance(skill.build)) {
      input.build = true; input.mat = richest(p); input.fire = true;
      input.yaw = Math.atan2(dx, dz);
      input.piece = rng.chance(skill.ramp) && p.mats[input.mat] >= BUILD_COST * 2 ? 2 : 0;
      b.buildT = rng.range(2.5, 5);
      return input;
    }
    const onTarget = Math.abs(wrapAngle(aimYaw - p.yaw)) < 0.08 + 2 / Math.max(4, d);
    const held = heldItem(p);
    const def = held && isWeapon(held.w) ? WEAPONS[held.w] : null;
    // Reloading or hurting: break line of sight behind the nearest solid prop.
    if ((p.reload > 0 || p.hp + p.shield < 45) && rng.chance(skill.cover * 0.05) && b.coverT <= 0) {
      const spot = coverFrom(map, p, target);
      if (spot) { b.coverX = spot[0]; b.coverZ = spot[1]; b.coverT = 2.5; }
    }
    if (b.coverT > 0 && (p.reload > 0 || p.hp + p.shield < 45)) {
      const want = Math.atan2(b.coverX - p.x, b.coverZ - p.z);
      const rel = wrapAngle(want - p.yaw);
      input.mz = Math.cos(rel); input.mx = -Math.sin(rel);
      input.sprint = false;
      if (p.reload <= 0 && def) input.fire = def.auto && b.seen > skill.reaction && onTarget;
      return input;
    }
    // Burst discipline: fire for `burst` seconds, then pause.
    b.burst += DT;
    const cycle = skill.burst + skill.pause;
    const inBurst = b.burst % cycle < skill.burst;
    if (b.seen > skill.reaction && onTarget && held && def && held.mag > 0 && (inBurst || !def.auto)) input.fire = def.auto ? true : rng.chance(0.25);
    if (held && def && held.mag === 0) input.reload = true;
    input.aim = !!def && (def.id === 'sniper' || def.id === 'rifle') && d > 25;
    // Strafe and hold a preferred range.
    if (b.strafeT <= 0) { b.strafe = rng.chance(0.5) ? 1 : -1; b.strafeT = rng.range(0.5, 1.4); }
    const want = def?.id === 'shotgun' ? 6 : def?.id === 'sniper' ? 70 : def?.id === 'smg' ? 14 : 28;
    input.mx = b.strafe * skill.strafe;
    input.mz = d > want + 6 ? 0.9 : d < want - 6 ? -0.7 : 0;
    if (state.difficulty !== 'easy' && b.jumpT <= 0 && rng.chance(0.01)) { input.jump = true; b.jumpT = 2; }
    return input;
  }

  // Not fighting: storm first, then heal, harvest, loot and roam.
  if (outside) { b.goalX = s.toX; b.goalZ = s.toZ; b.harvest = -1; }
  const heal = healFor(p);
  if (heal >= 0 && b.calm > 1.2 && !(outside && s.damage >= 2)) {
    input.slot = heal + 1;
    input.fire = p.current === heal;
    if (outside) steerTo(input, p, b.goalX, b.goalZ);
    return input;
  }
  if (b.harvest >= 0 && !outside) {
    const c = map.cylinders[b.harvest];
    const d = Math.hypot(c.x - p.x, c.z - p.z);
    if (mats >= 120) b.harvest = -1;
    else if (d < c.r + 1.6) {
      input.yaw = p.yaw + clamp(wrapAngle(Math.atan2(c.x - p.x, c.z - p.z) - p.yaw), -0.3, 0.3);
      input.pitch = 0;
      if (p.current !== PICKAXE_SLOT) input.slot = PICKAXE_KEY;
      else input.fire = true;
      return input;
    } else { b.goalX = c.x; b.goalZ = c.z; }
  }
  // Put a gun back in hand (the pickaxe and heals come out only when needed).
  const gun = weaponFor(p, 30);
  if (gun >= 0 && (p.current === PICKAXE_SLOT || !isWeapon(heldItem(p)?.w)) && gun !== p.current) input.slot = gun + 1;
  steerTo(input, p, b.goalX, b.goalZ);
  const goalD = Math.hypot(b.goalX - p.x, b.goalZ - p.z);
  input.sprint = goalD > 10;
  if (goalD < 1.6) { input.mz = 0; input.mx = 0; }
  if (b.stuck > 0.6) { input.jump = true; input.mx = b.strafe; if (b.stuck > 1.6) { b.goalX = p.x + rng.range(-20, 20); b.goalZ = p.z + rng.range(-20, 20); b.stuck = 0; b.harvest = -1; } }
  // Swap into better floor weapons and open nearby chests.
  const floor = nearestWeaponLoot(state, p);
  if (floor && isWeapon(floor.weapon)) {
    const worst = p.weapons.reduce((m, w) => Math.min(m, w && isWeapon(w.w) ? w.rarity : -1), 9);
    if (worst < floor.rarity || p.weapons.some(w => !w)) input.interact = (state.clock * 4) % 2 < 1;
  }
  if (nearestClosedChest(state, p) >= 0) { input.interact = true; input.mz = 0; input.mx = 0; }
  const held = heldItem(p);
  if (held && isWeapon(held.w) && held.mag < WEAPONS[held.w].mag * 0.5) input.reload = true;
  return input;
}

function steerTo(input: ShooterInput, p: Fighter, x: number, z: number): void {
  const want = Math.atan2(x - p.x, z - p.z);
  input.yaw = p.yaw + clamp(wrapAngle(want - p.yaw), -0.12, 0.12);
  input.pitch = p.pitch * 0.9;
  input.mz = Math.abs(wrapAngle(want - p.yaw)) < 1.2 ? 1 : 0.3;
}

/** A spot behind the nearest tall prop, on the far side from `from`. */
export function coverFrom(map: ShooterMap, p: Fighter, from: { x: number; z: number }): [number, number] | null {
  let best: [number, number] | null = null, bd = 14;
  for (const b of map.boxes) {
    const cx = (b.minX + b.maxX) / 2, cz = (b.minZ + b.maxZ) / 2;
    if (Math.abs(cx - p.x) > bd || Math.abs(cz - p.z) > bd || b.maxY - p.y < 1.4 || b.minY > p.y + 0.5) continue;
    const ax = cx - from.x, az = cz - from.z, al = Math.hypot(ax, az) || 1;
    const reach = Math.hypot(b.maxX - b.minX, b.maxZ - b.minZ) / 2 + 1;
    const x = cx + (ax / al) * reach, z = cz + (az / al) * reach;
    const d = Math.hypot(x - p.x, z - p.z);
    if (d < bd) { bd = d; best = [x, z]; }
  }
  return best;
}

function chooseGoal(state: ShooterState, map: ShooterMap, p: Fighter, rng: Rng): void {
  const b = p.bot;
  const s = state.storm;
  const guns = p.weapons.filter(w => w && isWeapon(w.w)).length;
  const free = p.weapons.filter(w => !w).length;
  const needsAmmo = p.weapons.some(w => w && isWeapon(w.w) && w.mag === 0 && p.ammo[WEAPONS[w.w].ammo] === 0);
  const mats = p.mats[0] + p.mats[1] + p.mats[2];
  // Loot the nearest useful item within reach of the safe zone (other floors count as farther away).
  let best = -1, bd = guns === 0 ? 90 : 45;
  state.loot.forEach((l, i) => {
    const useful = l.kind === 'weapon' ? (guns < 3 && free > 0) || p.weapons.some(w => w && isWeapon(w.w) && w.rarity < l.rarity)
      : l.kind === 'ammo' ? needsAmmo || p.weapons.some(w => w && isWeapon(w.w) && WEAPONS[w.w].ammo === l.ammo)
        : l.kind === 'item' ? free > 0 || p.weapons.some(w => w?.w === l.weapon)
          : l.kind === 'mats' ? mats < 300
            : l.kind === 'heal' ? p.hp < 100 : p.shield < 100;
    if (!useful) return;
    const d = Math.hypot(l.x - p.x, l.z - p.z) + Math.abs(l.y - p.y) * 6;
    if (d < bd && Math.hypot(l.x - s.toX, l.z - s.toZ) < Math.max(s.toR, 20) + 30) { bd = d; best = i; }
  });
  if (best >= 0) { b.goalX = state.loot[best].x; b.goalZ = state.loot[best].z; b.loot = state.loot[best].id; b.harvest = -1; return; }
  map.chests.forEach((c, i) => {
    if (state.chests[i]) return;
    const d = Math.hypot(c.x - p.x, c.z - p.z) + Math.abs((c.y ?? p.y) - p.y) * 6;
    if (d < bd) { bd = d; best = i; }
  });
  if (best >= 0) { b.goalX = map.chests[best].x + 1.4; b.goalZ = map.chests[best].z; b.harvest = -1; return; }
  // Low on materials with time to spare: go chop the nearest tree.
  if (mats < 60 && b.harvest < 0 && s.stage < 4) {
    let tree = -1, td = 30;
    map.cylinders.forEach((c, i) => { const d = Math.hypot(c.x - p.x, c.z - p.z); if (d < td && Math.hypot(c.x - s.toX, c.z - s.toZ) < s.toR) { td = d; tree = i; } });
    if (tree >= 0) { b.harvest = tree; b.goalX = map.cylinders[tree].x; b.goalZ = map.cylinders[tree].z; return; }
  }
  // Roam inside the next safe circle (on dry land).
  if (Math.hypot(b.goalX - p.x, b.goalZ - p.z) < 4 || Math.hypot(b.goalX - s.toX, b.goalZ - s.toZ) > s.toR) {
    for (let t = 0; t < 8; t++) {
      const a = rng.range(0, Math.PI * 2), r = rng.range(0, Math.max(6, s.toR * 0.8));
      const x = clamp(s.toX + Math.cos(a) * r, -HALF + 10, HALF - 10), z = clamp(s.toZ + Math.sin(a) * r, -HALF + 10, HALF - 10);
      if (dryLand(map, x, z)) { b.goalX = x; b.goalZ = z; break; }
    }
  }
}
