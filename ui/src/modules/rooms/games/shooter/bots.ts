/**
 * Storm Royale bots. Each tick a bot produces the same input a human would.
 * Priorities: escape the storm, revive a knocked teammate, fight a visible
 * enemy or infected (reaction delay, aim error, recoil control and burst
 * discipline scale with difficulty; wall up or ramp when shot, ramp-rush
 * toward enemies holding high ground, duck behind cover to reload, grenade
 * an enemy that broke line of sight, flash before a push, smoke a revive,
 * chase or flank to where an enemy was last seen, kite the infected), call
 * in killstreaks, heal when calm, then play the mode: loot (supply drops
 * first), harvest and roam toward the safe zone (royale), hold or take
 * capture points (Domination), hunt toward the enemy side (TDM), hold the
 * base, buy from walls and make the helicopter (Zombies). Goals on upper
 * floors are reached by the stairs (`climbStep`). Heavy checks (visibility,
 * smoke, cover, trees) run on a think timer, not every tick.
 */
import { clamp, type Rng, wrapAngle } from '../common/math.ts';
import { climbStep, dryLand, HALF, inClimb, mapFor, type ShooterMap } from './map.ts';
import { visible, type World } from './collide.ts';
import { ATTACHMENTS, attachFits, BUILD_COST, canUse, CONSUMABLES, isConsumable, isWeapon, MAX_NADES, NADES, STREAKS, WEAPONS, type Material, type WeaponId } from './weapons.ts';
import { cardinal } from './builds.ts';
import {
  EYE, heldItem, nearestDowned, nearestDrop, nearestWeaponLoot, nearestClosedChest, idleShooterInput, PICKAXE_KEY, PICKAXE_SLOT, REVIVE_RANGE, standing,
  type Difficulty, type Fighter, type ShooterInput, type ShooterState,
} from './sim.ts';
import { isRoyale, WAVE_BOARD, WAVE_HELI } from './modes/index.ts';
import { smokeBetween } from './modes/gear.ts';
import { buyWallsFor, nearestWall, ZOMBIE_KINDS, type Zombie } from './zombies.ts';

/** `recoil`: how much of the weapon's kick the bot pulls down; `nade`: grenade appetite; `flank`: chance to flank a lost enemy (team modes). */
interface Skill { reaction: number; error: number; sight: number; burst: number; pause: number; strafe: number; build: number; ramp: number; cover: number; recoil: number; nade: number; flank: number }
export const SKILL: Record<Difficulty, Skill> = {
  easy: { reaction: 0.95, error: 0.11, sight: 55, burst: 0.35, pause: 0.9, strafe: 0.4, build: 0.15, ramp: 0, cover: 0.3, recoil: 0.3, nade: 0.2, flank: 0.1 },
  normal: { reaction: 0.55, error: 0.06, sight: 80, burst: 0.6, pause: 0.5, strafe: 0.75, build: 0.45, ramp: 0.25, cover: 0.65, recoil: 0.6, nade: 0.5, flank: 0.35 },
  hard: { reaction: 0.3, error: 0.03, sight: 105, burst: 1.0, pause: 0.25, strafe: 1, build: 0.8, ramp: 0.5, cover: 1, recoil: 0.85, nade: 0.8, flank: 0.6 },
};
/** Something to shoot at: a fighter or an infected. */
type Foe = { x: number; y: number; z: number; vx: number; vz: number; mode?: Fighter['mode'] };
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
  input.cls = p.cls;
  if (p.mode === 'bus') return input;
  if (p.mode === 'sky' || p.mode === 'glide') {
    steerTo(input, p, b.goalX, b.goalZ);
    return input;
  }
  if (p.mode === 'down') {
    // Knocked: crawl toward the nearest teammate still standing.
    const mate = nearestMate(state, p, 200);
    if (mate) { steerTo(input, p, mate.x, mate.z); if (Math.hypot(mate.x - p.x, mate.z - p.z) < 1.4) input.mz = 0; }
    return input;
  }
  b.think -= DT;
  b.strafeT -= DT;
  b.jumpT -= DT;
  b.buildT -= DT;
  b.coverT -= DT;
  b.nadeT -= DT;
  b.flankT -= DT;
  b.lost += DT;
  b.calm += DT;
  if (b.think <= 0) {
    b.think = THINK + rng.range(0, 0.1);
    // Stuck detection: barely moved while trying to.
    const moved = Math.hypot(p.x - b.lastX, p.z - b.lastZ);
    b.stuck = moved < 0.35 ? b.stuck + THINK : 0;
    b.lastX = p.x; b.lastZ = p.z;
    // Acquire / keep a visible target (structures and smoke block sight; a flash blinds).
    let target = -1, bestD = p.blind > 0 ? 0 : skill.sight;
    const ey = p.y + EYE;
    for (const q of state.players) {
      if (q.id === p.id || q.team === p.team || q.mode === 'dead' || q.mode === 'bus' || q.mode === 'sky') continue;
      // Knocked enemies are worth finishing, but a standing threat comes first.
      const d = Math.hypot(q.x - p.x, q.z - p.z) * (q.mode === 'down' ? 1.6 : 1);
      if (d > bestD) continue;
      if (visible(world, p.x, ey, p.z, q.x, q.y + 1.2, q.z, state.builds) && !smokeBetween(state, p.x, ey, p.z, q.x, q.y + 1.2, q.z)) { target = q.id; bestD = d; }
    }
    if (target >= 0 && target !== b.target) { b.seen = 0; b.errX = rng.range(-1, 1) * skill.error * 3; b.errY = rng.range(-1, 1) * skill.error * 2; }
    if (target >= 0) { const q = state.players[target]; b.seenX = q.x; b.seenZ = q.z; b.lost = 0; }
    if (target < 0 && b.target >= 0 && b.seen > 0) b.seen = Math.max(0, b.seen - 0.5);
    b.target = target >= 0 ? target : b.seen > 0.3 ? b.target : -1;
    // The infected: the nearest one in sight (Zombies: the only enemy; otherwise only when no fighter is, or when one is close).
    b.zt = -1;
    if (state.infected.length && p.blind <= 0) {
      let zd = state.mode === 'zombies' ? skill.sight : b.target >= 0 ? 10 : 35;
      for (const z of state.infected) {
        if (z.hp <= 0) continue;
        const d = Math.hypot(z.x - p.x, z.z - p.z);
        if (d < zd && visible(world, p.x, ey, p.z, z.x, z.y + 1.2, z.z, state.builds)) { zd = d; b.zt = z.id; }
      }
      if (b.zt >= 0) { b.target = -1; b.calm = 0; }
    }
    if (b.target >= 0) b.calm = 0;
    // Choose a goal when not fighting (unless taking a detour around whatever it got stuck on).
    b.detour -= THINK;
    if (b.target < 0 && b.zt < 0 && b.detour <= 0) chooseGoal(state, map, p, rng);
    // In and out of buildings by the doors, between floors by the stairs.
    b.climb = -1;
    if (b.target < 0 && b.zt < 0) for (let i = 0; i < map.climbs.length; i++) {
      const c = map.climbs[i];
      const goalIn = inClimb(c, b.goalX, b.goalZ, 0.3), meIn = inClimb(c, p.x, p.z) && p.y > c.base - 0.4;
      if ((goalIn && (!meIn || Math.abs(b.goalY - p.y) > 1.5)) || (meIn && !goalIn)) { b.climb = i; break; }
    }
  }
  const s = state.storm;
  const outside = Math.hypot(p.x - s.cx, p.z - s.cz) > s.r - 4;
  const zombie = b.zt >= 0 ? zombieById(state, b.zt) : null;
  const fighter = b.target >= 0 ? state.players[b.target] : undefined;
  const target: Foe | undefined = zombie ?? (fighter && fighter.mode !== 'dead' ? fighter : undefined);
  const armed = p.weapons.some(w => w && isWeapon(w.w) && (w.mag > 0 || p.ammo[WEAPONS[w.w].ammo] > 0));
  const mats = p.mats[0] + p.mats[1] + p.mats[2];
  // Killstreaks: a UAV or a drone at once; an airstrike on an enemy in sight at range.
  if (p.ready) {
    const k = p.ready & 1 ? 0 : p.ready & 2 ? 1 : 2;
    if (STREAKS[k].id !== 'airstrike') input.streak = (state.clock * 2) % 1 < 0.5;
    else if (target && Math.hypot(target.x - p.x, target.z - p.z) > 18) {
      input.yaw = Math.atan2(target.x - p.x, target.z - p.z); input.pitch = Math.atan2(target.y - (p.y + EYE), Math.hypot(target.x - p.x, target.z - p.z));
      input.streak = true;
      return input;
    }
  }

  if (target && armed && !(outside && s.damage >= 4)) {
    b.seen += DT;
    const dx = target.x - p.x, dz = target.z - p.z, d = Math.hypot(dx, dz);
    const slot = weaponFor(p, d);
    if (slot >= 0 && slot !== p.current) input.slot = slot + 1;
    // Aim with an error that shrinks the longer the target stays in view, pulling down against the recoil.
    const settle = Math.max(0.15, 1 - b.seen / (skill.reaction * 3));
    const lead = !target.mode || target.mode === 'ground' ? 0.12 : 0;
    const ax = dx + target.vx * lead * d * 0.05, az = dz + target.vz * lead * d * 0.05;
    const aimYaw = Math.atan2(ax, az) + b.errX * settle - p.kickYaw * skill.recoil;
    const height = zombie ? 1.15 * ZOMBIE_KINDS[zombie.kind].scale : 1.15;
    const aimPitch = Math.atan2(target.y + height - (p.y + EYE), Math.max(0.5, d)) + b.errY * settle - p.kick * skill.recoil;
    input.yaw = p.yaw + clamp(wrapAngle(aimYaw - p.yaw), -0.22, 0.22);
    input.pitch = clamp(p.pitch + clamp(aimPitch - p.pitch, -0.12, 0.12), -1.3, 1.3);
    // Grenades: a frag / molotov at a group of infected or an enemy at mid range; a flash before closing in.
    if (b.nadeT <= 0 && p.cooldown <= 0 && !p.building && d > 9 && d < 30 && rng.chance(skill.nade * 0.02)) {
      const crowd = zombie ? state.infected.reduce((n, z) => n + (z.hp > 0 && Math.hypot(z.x - target.x, z.z - target.z) < 6 ? 1 : 0), 0) : 0;
      const lethal = p.lethalN > 0 && (!zombie || crowd >= 3);
      const flash = !zombie && !lethal && p.tacN > 0 && p.tacKind === 3 && d < 20;
      if (lethal || flash) {
        input.yaw = Math.atan2(dx, dz);
        input.pitch = clamp(Math.atan2(target.y - p.y, d) * 0.5 + 0.12 + (d - 10) * 0.011, -0.2, 0.75);
        if (lethal) input.lethal = true; else input.tactical = true;
        b.nadeT = rng.range(7, 13);
        return input;
      }
    }
    // Kite the infected: back off from anything in arm's reach (brutes from farther).
    if (zombie) {
      const keep = zombie.kind === 2 ? 7 : zombie.kind === 3 ? 0 : 4.5;
      const def = heldItem(p) && isWeapon(heldItem(p)!.w) ? WEAPONS[heldItem(p)!.w as WeaponId] : null;
      if (heldItem(p)?.mag === 0) input.reload = true;
      if (b.seen > skill.reaction * 0.6 && def && (heldItem(p)?.mag ?? 0) > 0) input.fire = def.auto ? true : rng.chance(0.35);
      input.aim = d > 12;
      if (b.strafeT <= 0) { b.strafe = rng.chance(0.5) ? 1 : -1; b.strafeT = rng.range(0.6, 1.5); }
      input.mx = b.strafe * 0.6;
      input.mz = d < keep ? -1 : d > 18 && state.mode !== 'zombies' ? 0.6 : 0;
      if (d < keep && b.stuck > 0.6) input.jump = true;
      return input;
    }
    const foe = fighter!;
    // Ramp rush: push toward an enemy holding high ground (or, sometimes, any enemy at mid range).
    b.pushT -= DT;
    if (b.pushT <= 0 && b.buildT <= 0 && skill.ramp > 0 && !p.swimming && p.grounded && mats >= BUILD_COST * 4 && d > 7 && d < 45
      && (foe.y - p.y > 2.2 || rng.chance(skill.ramp * 0.004))) { b.pushT = rng.range(2.2, 3.4); b.pushN = 0; }
    if (b.pushT > 0) {
      if (mats < BUILD_COST || d < 5 || p.swimming) { b.pushT = 0; b.buildT = rng.range(2, 4); }
      else {
        // Square to the grid so each ramp continues the last; a wall every third piece covers the climb.
        input.yaw = cardinal(Math.atan2(dx, dz)) * (Math.PI / 2);
        input.pitch = 0;
        input.build = true; input.mat = richest(p); input.mz = 1; input.mx = 0;
        input.piece = b.pushN % 3 === 2 ? 0 : 2;
        if (p.cooldown <= 0) { input.fire = true; b.pushN++; }
        if (b.stuck > 0.5) input.jump = true;
        if (b.pushT <= DT) b.buildT = rng.range(2.5, 5);
        return input;
      }
    }
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
    // Reloading or hurting: break line of sight behind the nearest solid prop (sliding into it when sprinting).
    if ((p.reload > 0 || p.hp + p.shield < 45) && rng.chance(skill.cover * 0.05) && b.coverT <= 0) {
      const spot = coverFrom(map, p, foe);
      if (spot) { b.coverX = spot[0]; b.coverZ = spot[1]; b.coverT = 2.5; }
    }
    if (b.coverT > 0 && (p.reload > 0 || p.hp + p.shield < 45)) {
      const want = Math.atan2(b.coverX - p.x, b.coverZ - p.z);
      const rel = wrapAngle(want - p.yaw);
      input.mz = Math.cos(rel); input.mx = -Math.sin(rel);
      input.sprint = false;
      // Nearly there and still exposed: crouch behind it.
      input.crouch = Math.hypot(b.coverX - p.x, b.coverZ - p.z) < 1.5;
      if (p.reload <= 0 && def) input.fire = def.auto && b.seen > skill.reaction && onTarget;
      return input;
    }
    // Burst discipline: fire for `burst` seconds, then pause.
    b.burst += DT;
    const cycle = skill.burst + skill.pause;
    const inBurst = b.burst % cycle < skill.burst;
    if (b.seen > skill.reaction && onTarget && held && def && held.mag > 0 && (inBurst || !def.auto)) input.fire = def.auto ? true : rng.chance(0.25);
    if (held && def && held.mag === 0) input.reload = true;
    // Aim down sights past close range (snipers and rifles always), crouch to steady a long shot.
    input.aim = !!def && (def.id === 'sniper' || def.id === 'rifle' ? d > 18 : d > 26);
    input.crouch = !!def && def.id === 'sniper' && d > 45;
    // Strafe and hold a preferred range.
    if (b.strafeT <= 0) { b.strafe = rng.chance(0.5) ? 1 : -1; b.strafeT = rng.range(0.5, 1.4); }
    const want = def?.id === 'shotgun' ? 6 : def?.id === 'sniper' ? 70 : def?.id === 'smg' ? 14 : 28;
    input.mx = b.strafe * skill.strafe;
    input.mz = d > want + 6 ? 0.9 : d < want - 6 ? -0.7 : 0;
    if (state.difficulty !== 'easy' && b.jumpT <= 0 && rng.chance(0.01)) { input.jump = true; b.jumpT = 2; }
    return input;
  }

  // Not fighting: storm first, then revive, heal, harvest, loot and roam.
  if (outside) { b.goalX = s.toX; b.goalZ = s.toZ; b.goalY = NaN; b.climb = -1; b.harvest = -1; }
  // A knocked teammate in reach: hold interact until they are up (chooseGoal brings the bot here), smoking the spot first.
  const downed = state.squad > 1 ? nearestDownedMate(state, p, REVIVE_RANGE) : null;
  if (downed && Math.hypot(downed.x - p.x, downed.z - p.z) < REVIVE_RANGE - 0.4 && nearestDowned(state, p, REVIVE_RANGE) >= 0 && !(outside && s.damage >= 6)) {
    if (p.tacN > 0 && p.tacKind === 2 && b.nadeT <= 0 && p.interact === 0 && b.lost < 6) { input.pitch = -0.6; input.tactical = true; b.nadeT = 10; return input; }
    input.interact = true;
    return input;
  }
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
    if (mats >= 120 || spent(state, -1 - b.harvest)) b.harvest = -1;
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
  // Upstairs (or back down): the stairs route, else straight at the goal.
  const via = b.climb >= 0 ? climbStep(map.climbs[b.climb], p.x, p.y, p.z, b.goalX, b.goalY, b.goalZ) : null;
  if (via) steerTo(input, p, via[0], via[1], 0.25);
  else steerTo(input, p, b.goalX, b.goalZ);
  const goalD = Math.hypot(b.goalX - p.x, b.goalZ - p.z);
  input.sprint = goalD > 10 && !via;
  // Long runs: tactical sprint (it refills while walking).
  input.tac = input.sprint && goalD > 35 && p.tacT > 1.5;
  if (goalD < 1.6 && !via) { input.mz = 0; input.mx = 0; }
  if (b.stuck > 0.6) { input.jump = true; input.mx = b.strafe; if (b.stuck > 1.6 && !downed) { b.goalX = p.x + rng.range(-20, 20); b.goalZ = p.z + rng.range(-20, 20); b.goalY = NaN; b.climb = -1; b.stuck = 0; b.harvest = -1; b.detour = 2.5; } }
  // Swap into better floor weapons and open nearby chests; Zombies: buy from a wall when it is an upgrade.
  const floor = nearestWeaponLoot(state, p);
  if (floor && isWeapon(floor.weapon)) {
    const worst = p.weapons.reduce((m, w) => Math.min(m, w && isWeapon(w.w) ? w.rarity : -1), 9);
    if (worst < floor.rarity || p.weapons.some(w => !w)) input.interact = (state.clock * 4) % 2 < 1;
  }
  if (nearestClosedChest(state, p) >= 0 || nearestDrop(state, p) >= 0) { input.interact = true; input.mz = 0; input.mx = 0; }
  const wall = nearestWall(state, p);
  if (wall >= 0 && wantWall(state, p, wall)) input.interact = (state.clock * 4) % 2 < 1;
  const held = heldItem(p);
  if (held && isWeapon(held.w) && held.mag < WEAPONS[held.w].mag * 0.5) input.reload = true;
  return input;
}

/** The infected with this id (null once it is gone). */
function zombieById(state: ShooterState, id: number): Zombie | null {
  for (const z of state.infected) if (z.id === id) return z.hp > 0 ? z : null;
  return null;
}
/** Zombies: is this buy-wall worth the points (a gun the bot lacks and can afford, keeping a little in reserve)? */
function wantWall(state: ShooterState, p: Fighter, i: number): boolean {
  const w = buyWallsFor(mapFor(state.map))[i];
  if (!w || p.points < w.price + 150) return false;
  const owned = p.weapons.find(h => h?.w === w.w);
  return owned ? p.ammo[WEAPONS[w.w].ammo] < WEAPONS[w.w].mag && p.points >= w.price / 2 : p.weapons.filter(h => h && isWeapon(h.w)).length < 3 || w.price >= 1500;
}

/** Turns toward (x, z) and walks; `turn` is the most it turns per tick (stairs need tighter steering). */
function steerTo(input: ShooterInput, p: Fighter, x: number, z: number, turn = 0.12): void {
  const want = Math.atan2(x - p.x, z - p.z);
  input.yaw = p.yaw + clamp(wrapAngle(want - p.yaw), -turn, turn);
  input.pitch = p.pitch * 0.9;
  input.mz = Math.abs(wrapAngle(want - p.yaw)) < (turn > 0.12 ? 0.5 : 1.2) ? 1 : turn > 0.12 ? 0 : 0.3;
}

/** Has this prop (box index, or -1 - trunk index) been harvested dry? */
function spent(state: ShooterState, key: number): boolean {
  for (const n of state.nodes) if (n.key === key) return n.left <= 0;
  return false;
}
/** Nearest standing teammate within `range` (null in solo or when nobody is up). */
function nearestMate(state: ShooterState, p: Fighter, range: number): Fighter | null {
  let best: Fighter | null = null, bd = range;
  for (const q of state.players) {
    if (q.team !== p.team || q.id === p.id || !standing(q) || q.mode === 'bus') continue;
    const d = Math.hypot(q.x - p.x, q.z - p.z);
    if (d < bd) { bd = d; best = q; }
  }
  return best;
}
function nearestDownedMate(state: ShooterState, p: Fighter, range: number): Fighter | null {
  let best: Fighter | null = null, bd = range;
  for (const q of state.players) {
    if (q.team !== p.team || q.id === p.id || q.mode !== 'down') continue;
    const d = Math.hypot(q.x - p.x, q.z - p.z) + Math.abs(q.y - p.y) * 3;
    if (d < bd) { bd = d; best = q; }
  }
  return best;
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
  b.goalY = NaN;
  // A knocked teammate comes first (by the stairs if need be).
  const downed = state.squad > 1 ? nearestDownedMate(state, p, 90) : null;
  if (downed && !(Math.hypot(p.x - s.cx, p.z - s.cz) > s.r && s.damage >= 6)) { b.goalX = downed.x; b.goalZ = downed.z; b.goalY = downed.y; b.harvest = -1; return; }
  if (!isRoyale(state.mode)) { modeGoal(state, map, p, rng); return; }
  // Team play: stay with the team's lead (a human teammate first, else the lowest seat); loot near them.
  let lead: Fighter | null = null;
  if (state.squad > 1) for (const q of state.players) if (q.team === p.team && q.id !== p.id && standing(q) && q.mode !== 'bus' && (!lead || (q.human && !lead.human) || (q.human === lead.human && q.id < lead.id))) lead = q;
  if (lead && !lead.human && lead.id > p.id) lead = null;
  if (lead && Math.hypot(lead.x - p.x, lead.z - p.z) > 30) { b.goalX = lead.x + rng.range(-5, 5); b.goalZ = lead.z + rng.range(-5, 5); b.goalY = lead.y; b.harvest = -1; return; }
  // Supply drops (landed or on the way down) inside the safe zone are worth a detour.
  let drop = -1, dd = 90;
  state.drops.forEach((d, i) => {
    const r = Math.hypot(d.x - p.x, d.z - p.z);
    if (r < dd && Math.hypot(d.x - s.toX, d.z - s.toZ) < Math.max(s.toR, 20) + 10) { dd = r; drop = i; }
  });
  if (drop >= 0) { const d = state.drops[drop]; b.goalX = d.x + 1.2; b.goalZ = d.z; b.goalY = d.ground; b.harvest = -1; return; }
  // Loot the nearest useful item within reach of the safe zone (other floors count as farther away).
  let best = -1, bd = guns === 0 ? 90 : 45;
  state.loot.forEach((l, i) => {
    const useful = l.kind === 'weapon' ? (guns < 3 && free > 0) || p.weapons.some(w => w && isWeapon(w.w) && w.rarity < l.rarity)
      : l.kind === 'ammo' ? needsAmmo || p.weapons.some(w => w && isWeapon(w.w) && WEAPONS[w.w].ammo === l.ammo)
        : l.kind === 'item' ? free > 0 || p.weapons.some(w => w?.w === l.weapon)
          : l.kind === 'mats' ? mats < 300
            : l.kind === 'attach' ? p.weapons.some(w => w && isWeapon(w.w) && attachFits(w.w, ATTACHMENTS[l.amount]?.bit ?? 0) && !(w.att & (ATTACHMENTS[l.amount]?.bit ?? 0)))
              : l.kind === 'nade' ? (NADES[l.amount]?.lethal ? p.lethalN < MAX_NADES && (p.lethalN === 0 || p.lethal === l.amount) : p.tacN < MAX_NADES && (p.tacN === 0 || p.tacKind === l.amount))
                : l.kind === 'heal' ? p.hp < 100 : p.shield < 100;
    if (!useful) return;
    const d = Math.hypot(l.x - p.x, l.z - p.z) + Math.abs(l.y - p.y) * 6;
    if (d < bd && Math.hypot(l.x - s.toX, l.z - s.toZ) < Math.max(s.toR, 20) + 30) { bd = d; best = i; }
  });
  if (best >= 0) { const l = state.loot[best]; b.goalX = l.x; b.goalZ = l.z; b.goalY = l.y; b.loot = l.id; b.harvest = -1; return; }
  map.chests.forEach((c, i) => {
    if (state.chests[i]) return;
    const d = Math.hypot(c.x - p.x, c.z - p.z) + Math.abs((c.y ?? p.y) - p.y) * 6;
    if (d < bd) { bd = d; best = i; }
  });
  if (best >= 0) { const c = map.chests[best]; b.goalX = c.x + 1.4; b.goalZ = c.z; b.goalY = c.y ?? NaN; b.harvest = -1; return; }
  if (lead && Math.hypot(lead.x - p.x, lead.z - p.z) > 14) { b.goalX = lead.x + rng.range(-4, 4); b.goalZ = lead.z + rng.range(-4, 4); b.goalY = lead.y; b.harvest = -1; return; }
  // Low on materials with time to spare: go chop the nearest tree.
  if (mats < 60 && b.harvest < 0 && s.stage < 4) {
    let tree = -1, td = 30;
    map.cylinders.forEach((c, i) => { const d = Math.hypot(c.x - p.x, c.z - p.z); if (d < td && Math.hypot(c.x - s.toX, c.z - s.toZ) < s.toR && !spent(state, -1 - i)) { td = d; tree = i; } });
    if (tree >= 0) { b.harvest = tree; b.goalX = map.cylinders[tree].x; b.goalZ = map.cylinders[tree].z; return; }
  }
  // Late game: take the high ground on a rooftop inside the next circle.
  if (s.stage >= 3) {
    let roof = -1, rd = 70;
    map.climbs.forEach((c, i) => { const d = Math.hypot(c.cx - p.x, c.cz - p.z); if (d < rd && Math.hypot(c.cx - s.toX, c.cz - s.toZ) < s.toR - 4) { rd = d; roof = i; } });
    if (roof >= 0) { const c = map.climbs[roof]; b.goalX = c.cx; b.goalZ = c.cz; b.goalY = c.roof; return; }
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

/**
 * Goals in the arena modes and Zombies: chase (or flank) an enemy lost from
 * sight, hunt one the UAV shows, take the capture points the team does not
 * hold (spread across them), push toward the enemy side; Zombies: the
 * helicopter, a buy-wall worth the points, ammo, else hold the base.
 */
function modeGoal(state: ShooterState, map: ShooterMap, p: Fighter, rng: Rng): void {
  const b = p.bot, obj = state.obj, skill = SKILL[state.difficulty];
  const set = (x: number, z: number) => {
    // Stay inside the boundary.
    const dx = x - obj.ax, dz = z - obj.az, d = Math.hypot(dx, dz), max = Math.max(4, obj.ar - 6);
    b.goalX = d > max ? obj.ax + (dx / d) * max : x; b.goalZ = d > max ? obj.az + (dz / d) * max : z; b.goalY = NaN; b.harvest = -1;
  };
  if (state.mode === 'zombies') {
    if (obj.wave === WAVE_HELI || obj.wave === WAVE_BOARD) { set(obj.lzX + rng.range(-4, 4), obj.lzZ + rng.range(-4, 4)); return; }
    // The last few of a round: go and find them.
    if (obj.left <= 0 && state.infected.length && state.infected.length <= 3) {
      let best: Zombie | null = null, bd = Infinity;
      for (const z of state.infected) { const d = Math.hypot(z.x - p.x, z.z - p.z); if (z.hp > 0 && d < bd) { bd = d; best = z; } }
      if (best) { set(best.x, best.z); return; }
    }
    const walls = buyWallsFor(map);
    for (let i = 0; i < walls.length; i++) if (wantWall(state, p, i)) { b.goalX = walls[i].x; b.goalZ = walls[i].z; b.goalY = walls[i].y; return; }
    const held = heldItem(p);
    const kind = held && isWeapon(held.w) ? WEAPONS[held.w].ammo : null;
    if (kind && p.ammo[kind] < 30) {
      let best = -1, bd = 30;
      state.loot.forEach((l, i) => { const d = Math.hypot(l.x - p.x, l.z - p.z); if (l.kind === 'ammo' && l.ammo === kind && d < bd) { bd = d; best = i; } });
      if (best >= 0) { b.goalX = state.loot[best].x; b.goalZ = state.loot[best].z; b.goalY = state.loot[best].y; return; }
    }
    if (Math.hypot(b.goalX - obj.ax, b.goalZ - obj.az) > 14 || Math.hypot(b.goalX - p.x, b.goalZ - p.z) < 2) set(obj.ax + rng.range(-10, 10), obj.az + rng.range(-10, 10));
    return;
  }
  // An enemy lost from sight: to where it was last seen, sometimes around the side.
  if (b.lost < 5) {
    if (b.flankT <= 0 && rng.chance(skill.flank)) {
      const dx = b.seenX - p.x, dz = b.seenZ - p.z, d = Math.hypot(dx, dz) || 1, side = rng.chance(0.5) ? 1 : -1;
      b.flankX = b.seenX - (dx / d) * 6 + (-dz / d) * side * 14; b.flankZ = b.seenZ - (dz / d) * 6 + (dx / d) * side * 14; b.flankT = 7;
    }
    if (b.flankT > 0 && Math.hypot(b.flankX - p.x, b.flankZ - p.z) > 3) set(b.flankX, b.flankZ); else set(b.seenX, b.seenZ);
    return;
  }
  // The team's UAV is up: hunt the nearest enemy it shows.
  if ((state.uav[p.team] ?? 0) > 0) {
    let best: Fighter | null = null, bd = Infinity;
    for (const q of state.players) if (q.team !== p.team && standing(q)) { const d = Math.hypot(q.x - p.x, q.z - p.z); if (d < bd) { bd = d; best = q; } }
    if (best) { set(best.x, best.z); return; }
  }
  if (state.mode === 'dom' && obj.points.length) {
    // Points the team does not hold first, nearest first, each bot leaning toward a different one.
    let best = 0, score = Infinity;
    obj.points.forEach((pt, i) => {
      const v = Math.hypot(pt.x - p.x, pt.z - p.z) + (pt.own === p.team ? 70 : 0) + ((p.id + i) % 3) * 14;
      if (v < score) { score = v; best = i; }
    });
    const pt = obj.points[best];
    if (Math.hypot(b.goalX - pt.x, b.goalZ - pt.z) > pt.r || Math.hypot(b.goalX - p.x, b.goalZ - p.z) < 1.5) set(pt.x + rng.range(-1, 1) * pt.r * 0.6, pt.z + rng.range(-1, 1) * pt.r * 0.6);
    return;
  }
  // TDM: push through the middle toward the enemy's side.
  const [ex, ez] = obj.spawns[1 - p.team] ?? [obj.ax, obj.az];
  if (Math.hypot(b.goalX - p.x, b.goalZ - p.z) < 4 || rng.chance(0.02)) {
    const t = rng.range(0.2, 0.9);
    set(obj.ax + (ex - obj.ax) * t + rng.range(-18, 18), obj.az + (ez - obj.az) * t + rng.range(-18, 18));
  }
}
