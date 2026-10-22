// Combat: the pickaxe swing, gunfire and hit tests, damage, knocks,
// eliminations, killstreak credit, rockets and blasts.
import { clamp, Rng } from '../../common/math.ts';
import type { ShooterMap } from '../map.ts';
import { raycast, type World } from '../collide.ts';
import { AMMO_KINDS, applyDamage, damageAt, MAX_MATS, PICKAXE, STREAKS, weaponStats, WEAPONS, WEAPON_IDS, type Material, type Rarity, type WeaponId } from '../weapons.ts';
import { maxHp, type Build } from '../builds.ts';
import { isRoyale, respawns } from '../modes/index.ts';
import { hurtZombie, rayZombie } from '../zombies.ts';
import { STEP, MAX_ROCKETS, SLOTS, PICKAXE_SLOT, KILL_STORM, KILL_PICKAXE, KILL_BLEED, FROM_STORM, DOWN_HP, MAX_NODES, REGROW, HARVEST_LEFT, emptyAmmo, teamUp, teamsAlive, EYE, type Held, type FighterMode, type Fighter, type Rocket, type HarvestNode, type ShooterState, type DamageSource } from './model.ts';
import { emit, dropLoot, dropMats, dropHeld, fwd } from './core.ts';
import { damageBuild, dropNode } from './building.ts';

/** Pickaxe: hurts a fighter, chips a structure, or harvests the prop in front. */
export function swing(state: ShooterState, map: ShooterMap, world: World, p: Fighter): void {
  p.cooldown = 1 / PICKAXE.rate;
  p.swingT = 0.35;
  const [fx, fz] = fwd(p.yaw), cp = Math.cos(p.pitch);
  const ox = p.x, oy = p.y + EYE, oz = p.z, dx = fx * cp, dy = Math.sin(p.pitch), dz = fz * cp;
  const hit = raycast(world, ox, oy, oz, dx, dy, dz, PICKAXE.range, true, state.builds);
  let victim = -1, tEnd = hit.t;
  for (const q of state.players) {
    if (q.id === p.id || (q.mode !== 'ground' && q.mode !== 'down') || q.team === p.team) continue;
    const h = rayFighter(ox, oy, oz, dx, dy, dz, q, tEnd);
    if (h.t >= 0 && h.t < tEnd) { tEnd = h.t; victim = q.id; }
  }
  // The infected take a pickaxe too (a last resort when the mag runs dry).
  let zv = -1;
  for (let i = 0; i < state.infected.length; i++) {
    const z = state.infected[i];
    if (z.hp <= 0) continue;
    const h = rayZombie(ox, oy, oz, dx, dy, dz, z, tEnd);
    if (h.t >= 0 && h.t < tEnd) { tEnd = h.t; zv = i; victim = -1; }
  }
  emit(state, 'swing', p.id, ox + dx * tEnd, oy + dy * tEnd, oz + dz * tEnd, { value: victim >= 0 || zv >= 0 ? 1 : hit.kind === 'none' ? 0 : 2 });
  if (victim >= 0) { hurt(state, state.players[victim], PICKAXE.damage, p.id, false, true, 'pickaxe'); return; }
  if (zv >= 0) { hurtZombie(state, state.infected[zv], PICKAXE.damage * 2, p.id, false, KILL_PICKAXE); return; }
  if (hit.kind === 'build') {
    const b = state.builds[hit.index];
    // Zombies: swinging at your team's barricade repairs it (and earns points) instead of breaking it.
    if (state.mode === 'zombies' && (b.owner < 0 || state.players[b.owner]?.team === p.team)) {
      if (b.hp < maxHp(b) && b.grow <= 0) {
        b.hp = Math.min(maxHp(b), b.hp + PICKAXE.structure);
        state.buildRev++;
        p.points += 10;
        emit(state, 'repair', p.id, ox + dx * tEnd, oy + dy * tEnd, oz + dz * tEnd, { value: b.kind, flag: b.mat });
      }
      return;
    }
    damageBuild(state, world, b, PICKAXE.structure);
    return;
  }
  if (hit.kind !== 'prop') return;
  const mat = hit.cyl ? 0 : map.boxMats[hit.index];
  if (mat < 0) return;
  // Every prop holds a limited amount: trees fall when spent, rocks and cars run dry; all regrow later.
  const key = hit.cyl ? -1 - hit.index : hit.index;
  let node: HarvestNode | null = null;
  for (const n of state.nodes) if (n.key === key) { node = n; break; }
  if (!node) {
    if (state.nodes.length >= MAX_NODES) dropNode(state, 0);
    node = { key, left: HARVEST_LEFT[mat], regrow: REGROW };
    state.nodes.push(node);
  }
  node.regrow = REGROW;
  const before = p.mats[mat];
  const ex = ox + dx * tEnd, ey = oy + dy * tEnd, ez = oz + dz * tEnd;
  if (node.left > 0) {
    const take = Math.min(PICKAXE.harvest, node.left);
    node.left -= take;
    p.mats[mat] = Math.min(MAX_MATS, before + take);
    if (node.left <= 0 && hit.cyl) {
      state.felled = [...state.felled, hit.index];
      const c = map.cylinders[hit.index];
      emit(state, 'fell', p.id, c.x, c.y0 + 1, c.z, { value: hit.index });
    }
  }
  emit(state, 'harvest', p.id, ex, ey, ez, { value: mat, flag: p.mats[mat] - before });
}
// ------------------------------------------------------------ combat

const _buildHits: Build[] = [];
const _buildDmg: number[] = [];
export function fire(state: ShooterState, map: ShooterMap, world: World, p: Fighter, held: Held, w: WeaponId, rng: Rng): void {
  const def = WEAPONS[w], st = weaponStats(def, held.att);
  held.mag--;
  p.cooldown = 1 / def.rate;
  p.shootT = 0.25;
  // Unsuppressed fire shows on the enemy radar for a moment.
  if (st.loud) p.spotted = 1.6;
  // The bullet goes where the sights are after the kick so far; then this shot's step of the pattern kicks.
  const yaw = p.yaw + p.kickYaw, pitch = clamp(p.pitch + p.kick, -1.5, 1.5);
  const step = def.pattern[p.shots % def.pattern.length];
  p.shots++;
  const kickMul = st.kick * (p.crouch ? 0.8 : 1);
  p.kick += step[0] * kickMul; p.kickYaw += step[1] * kickMul;
  const [fx, fz] = fwd(yaw);
  // Muzzle sits over the right shoulder; right of facing (fx, fz) is (-fz, fx).
  const ox = p.x - fz * 0.3, oy = p.y + (p.crouch ? EYE - 0.45 : EYE), oz = p.z + fx * 0.3;
  const cp = Math.cos(pitch);
  const baseX = fx * cp, baseY = Math.sin(pitch), baseZ = fz * cp;
  // Hip fire blooms; raised sights tighten towards the aimed spread; crouching steadies, moving / airborne / sliding loosen.
  const cone = def.spread + (st.adsSpread - def.spread) * p.ads;
  const spread = cone * (p.moving ? 1.4 : 1) * (p.grounded ? 1 : 2) * (p.crouch && p.slideT <= 0 ? 0.75 : 1) * (p.slideT > 0 ? 1.3 : 1) + p.bloom * (1 - 0.7 * p.ads) + (p.blind > 0 ? 0.06 : 0);
  p.bloom = Math.min(0.05, p.bloom + def.recoil * 0.5);
  if (def.projectile) {
    if (state.rockets.length < MAX_ROCKETS) {
      const s = def.projectile.speed;
      state.rockets.push({ id: state.nextId++, owner: p.id, rarity: held.rarity, x: ox + baseX, y: oy + baseY, z: oz + baseZ, vx: baseX * s, vy: baseY * s, vz: baseZ * s, life: 5 });
    }
    emit(state, 'shot', p.id, ox, oy, oz, { value: 5, x2: ox + baseX * 3, y2: oy + baseY * 3, z2: oz + baseZ * 3 });
    return;
  }
  const range = def.range * st.range;
  let firstEnd: [number, number, number] | null = null;
  _buildHits.length = 0; _buildDmg.length = 0;
  for (let k = 0; k < def.pellets; k++) {
    // Random direction inside the spread cone.
    const a = rng.range(0, Math.PI * 2), r = Math.sqrt(rng.next()) * spread;
    const ux = -fz, uz = fx; // horizontal perpendicular
    let dx = baseX + (ux * Math.cos(a) + 0) * r, dy = baseY + Math.sin(a) * r, dz = baseZ + uz * Math.cos(a) * r;
    const l = Math.hypot(dx, dy, dz); dx /= l; dy /= l; dz /= l;
    const wall = raycast(world, ox, oy, oz, dx, dy, dz, range, true, state.builds);
    let tEnd = wall.t, victim = -1, head = false, zv = -1;
    const struct = wall.kind === 'build' ? state.builds[wall.index] : null;
    for (const q of state.players) {
      // Shots pass through teammates.
      if (q.id === p.id || q.mode === 'dead' || q.mode === 'bus' || q.team === p.team) continue;
      const t = rayFighter(ox, oy, oz, dx, dy, dz, q, tEnd);
      if (t.t >= 0 && t.t < tEnd) { tEnd = t.t; victim = q.id; head = t.head; }
    }
    for (let i = 0; i < state.infected.length; i++) {
      const z = state.infected[i];
      if (z.hp <= 0) continue;
      const t = rayZombie(ox, oy, oz, dx, dy, dz, z, tEnd);
      if (t.t >= 0 && t.t < tEnd) { tEnd = t.t; zv = i; victim = -1; head = t.head; }
    }
    const ex = ox + dx * tEnd, ey = oy + dy * tEnd, ez = oz + dz * tEnd;
    if (!firstEnd) firstEnd = [ex, ey, ez];
    // A suppressor costs a little reach: falloff starts sooner.
    const far = tEnd / st.range;
    if (victim >= 0) {
      const q = state.players[victim];
      hurt(state, q, damageAt(def, held.rarity, far, head), p.id, head, true, w);
    } else if (zv >= 0) hurtZombie(state, state.infected[zv], damageAt(def, held.rarity, far, head), p.id, head, WEAPON_IDS.indexOf(w));
    else if (struct) {
      const at = _buildHits.indexOf(struct);
      const dmg = damageAt(def, held.rarity, far, false);
      if (at < 0) { _buildHits.push(struct); _buildDmg.push(dmg); } else _buildDmg[at] += dmg;
    }
  }
  for (let i = 0; i < _buildHits.length; i++) if (state.builds.includes(_buildHits[i])) damageBuild(state, world, _buildHits[i], _buildDmg[i]);
  const e = firstEnd!;
  emit(state, 'shot', p.id, ox, oy, oz, { value: WEAPON_IDS.indexOf(w), flag: st.loud ? 0 : 1, x2: e[0], y2: e[1], z2: e[2] });
  void map;
}

const _fh = { t: -1, head: false };
const SPHERES: [number, number, boolean][] = [[1.62, 0.3, true], [1.15, 0.46, false], [0.55, 0.42, false]];
/** Ray vs fighter: head sphere + two body spheres (squashed low while knocked). */
export function rayFighter(ox: number, oy: number, oz: number, dx: number, dy: number, dz: number, q: { x: number; y: number; z: number; mode?: FighterMode }, max: number): { t: number; head: boolean } {
  _fh.t = -1; _fh.head = false;
  const low = q.mode === 'down' ? 0.3 : 1;
  for (const [h, r, head] of SPHERES) {
    const cx = q.x - ox, cy = q.y + h * low + (low < 1 ? 0.2 : 0) - oy, cz = q.z - oz;
    const b = cx * dx + cy * dy + cz * dz;
    if (b < 0) continue;
    const c = cx * cx + cy * cy + cz * cz - r * r;
    const disc = b * b - c;
    if (disc < 0) continue;
    const t = b - Math.sqrt(disc);
    if (t >= 0 && t < max && (_fh.t < 0 || t < _fh.t)) { _fh.t = t; _fh.head = head; }
  }
  return _fh;
}

/** Kill / knock event weapon code for a source and weapon. */
const killCode = (from: number, src: DamageSource): number => (typeof src === 'number' ? src : from === FROM_STORM ? KILL_STORM : src === 'pickaxe' ? KILL_PICKAXE : WEAPON_IDS.indexOf(src));

/**
 * Damages a fighter. `from` is the fighter who dealt it (or FROM_STORM,
 * FROM_INFECTED, -1 for a fall); `src` names the weapon / kill code.
 */
export function hurt(state: ShooterState, q: Fighter, amount: number, from: number, head: boolean, useShield: boolean, src: DamageSource): void {
  // Once a winner is crowned nothing can take it away (storm, splash, falls); no friendly fire.
  if (q.mode === 'dead' || amount <= 0 || state.phase === 'done' || state.winner >= 0) return;
  if (from >= 0 && from !== q.id && state.players[from]?.team === q.team) return;
  const res = useShield ? applyDamage(q.hp, q.shield, amount) : { hp: Math.max(0, q.hp - amount), shield: q.shield, absorbed: 0 };
  q.hp = res.hp; q.shield = res.shield; q.hurtT = 0.4; q.useT = 0;
  if (from >= 0 && from !== q.id) {
    state.players[from].damage += amount;
    if (!q.human && q.mode !== 'down') { q.bot.target = from; q.bot.seen = 0.6; q.bot.calm = 0; q.bot.pushT = 0; q.bot.zt = -1; q.bot.seenX = state.players[from].x; q.bot.seenZ = state.players[from].z; q.bot.lost = 0; }
  }
  emit(state, 'hit', from, q.x, q.y + (q.mode === 'down' ? 0.5 : head ? 1.7 : 1.1), q.z, { target: q.id, value: amount, flag: (res.absorbed > 0 ? 1 : 0) | (head ? 2 : 0) });
  if (q.hp > 0) return;
  // Royale: the last team in it cannot be wiped out in the same instant as the one before it (storm ticks, splash): it wins instead.
  if (isRoyale(state.mode) && teamsAlive(state) <= 1) { q.hp = 1; return; }
  // Team play: knocked while a teammate still stands; finishing a knocked survivor eliminates them.
  const knocks = (isRoyale(state.mode) && state.squad > 1) || state.mode === 'zombies';
  if (q.mode !== 'down' && knocks && teamUp(state, q.team, q.id)) knock(state, q, from, src);
  else if (q.mode === 'down' && from < 0 && q.knocker >= 0) eliminate(state, q, q.knocker, KILL_BLEED);
  else eliminate(state, q, from, killCode(from, src));
}

function knock(state: ShooterState, q: Fighter, from: number, src: DamageSource): void {
  q.mode = 'down'; q.hp = DOWN_HP; q.shield = 0; q.knocker = from; q.revive = 0;
  q.building = false; q.useT = 0; q.reload = 0; q.aiming = false; q.sprinting = false; q.interact = 0; q.crouch = false; q.slideT = 0; q.ads = 0; q.mantleT = 0;
  emit(state, 'knock', from, q.x, q.y, q.z, { target: q.id, value: killCode(from, src) });
}

/** How long a fighter waits to respawn in the arena modes (seconds). */
export const RESPAWN = 4;

export function eliminate(state: ShooterState, q: Fighter, from: number, code: number): void {
  q.mode = 'dead'; q.killedBy = from; q.hp = 0; q.shield = 0; q.building = false; q.useT = 0; q.revive = 0;
  q.crouch = false; q.slideT = 0; q.ads = 0; q.kick = 0; q.kickYaw = 0; q.mantleT = 0; q.tactical = false; q.reload = 0; q.blind = 0;
  q.deaths++; q.streak = 0;
  const killer = from >= 0 && from !== q.id ? state.players[from] : null;
  if (killer) { killer.kills++; if (killer.team !== q.team) creditStreak(state, killer, 1); }
  emit(state, 'kill', from, q.x, q.y, q.z, { target: q.id, value: code });
  // The arena modes: back in after a short wait (the loadout comes back with the respawn); TDM scores the elimination.
  if (respawns(state.mode)) {
    q.respawn = RESPAWN;
    q.weapons = Array.from({ length: SLOTS }, () => null); q.current = PICKAXE_SLOT;
    if (state.mode === 'tdm' && (!killer || killer.team !== q.team) && state.obj.over < 0) state.obj.score[1 - q.team]++;
    return;
  }
  // Zombies: out until the next round starts (respawned with a pistol), or for good if nobody is left.
  if (state.mode === 'zombies') {
    q.weapons = Array.from({ length: SLOTS }, () => null); q.current = PICKAXE_SLOT; q.ammo = emptyAmmo(); q.lethalN = 0; q.tacN = 0;
    q.respawn = -1;
    return;
  }
  // Drop the inventory in a little ring.
  let k = 0;
  const ring = (i: number): [number, number] => [q.x + Math.cos(i * 1.1) * (1.3 + i * 0.12), q.z + Math.sin(i * 1.1) * (1.3 + i * 0.12)];
  const y = q.y;
  for (const w of q.weapons) if (w) { const [x, z] = ring(k++); dropHeld(state, w, x, y, z); }
  for (const kind of AMMO_KINDS) if (q.ammo[kind] > 0) { const [x, z] = ring(k++); dropLoot(state, 'ammo', kind, q.ammo[kind], x, y, z); }
  for (let m = 0; m < 3; m++) if (q.mats[m] > 0) { const [x, z] = ring(k++); dropMats(state, m as Material, q.mats[m], x, y, z); }
  // Every elimination leaves a small heal for whoever won the fight.
  { const [x, z] = ring(k++); dropLoot(state, 'heal', null, 25, x, y, z); }
  q.weapons = Array.from({ length: SLOTS }, () => null);
  q.current = PICKAXE_SLOT;
  q.ammo = emptyAmmo();
  q.mats = [0, 0, 0];
  q.lethalN = 0; q.tacN = 0;
  // Nobody left standing on the team: the knocked go too (credited to whoever knocked them).
  if (!teamUp(state, q.team)) for (const m of state.players) if (m.team === q.team && m.mode === 'down') eliminate(state, m, m.knocker, KILL_BLEED);
  // Team wiped: every member takes the team's place (solo: the fighter's own).
  if (state.players.some(m => m.team === q.team && m.mode !== 'dead')) return;
  const place = teamsAlive(state) + 1;
  for (const m of state.players) if (m.team === q.team && m.place === 0) m.place = place;
}

/**
 * Killstreak progress: eliminations since the last death (`weight` per kill;
 * an infected counts a quarter). Crossing a streak's count earns it (kept
 * until called in).
 */
export function creditStreak(state: ShooterState, p: Fighter, weight: number): void {
  const before = p.streak;
  p.streak += weight;
  for (let k = 0; k < STREAKS.length; k++) {
    const need = STREAKS[k].kills;
    if (before < need && p.streak >= need && !(p.ready & (1 << k))) { p.ready |= 1 << k; emit(state, 'streak', p.id, p.x, p.y, p.z, { value: k, flag: 0 }); }
  }
}

export function stepRockets(state: ShooterState, world: World): void {
  for (let i = state.rockets.length - 1; i >= 0; i--) {
    const r = state.rockets[i];
    r.life -= STEP;
    r.vy -= 2 * STEP;
    const sp = Math.hypot(r.vx, r.vy, r.vz), dx = r.vx / sp, dy = r.vy / sp, dz = r.vz / sp, step = sp * STEP;
    const hit = raycast(world, r.x, r.y, r.z, dx, dy, dz, step, true, state.builds);
    let t = hit.kind === 'none' ? -1 : hit.t;
    const team = state.players[r.owner]?.team ?? -1;
    for (const q of state.players) {
      if (q.id === r.owner || q.mode === 'dead' || q.mode === 'bus' || q.team === team) continue;
      const h = rayFighter(r.x, r.y, r.z, dx, dy, dz, q, step);
      if (h.t >= 0 && (t < 0 || h.t < t)) t = h.t;
    }
    if (t >= 0 || r.life <= 0) {
      const tt = Math.max(0, t);
      const ex = r.x + dx * tt, ey = r.y + dy * tt, ez = r.z + dz * tt;
      explode(state, world, r, ex, ey, ez);
      state.rockets.splice(i, 1);
    } else { r.x += r.vx * STEP; r.y += r.vy * STEP; r.z += r.vz * STEP; }
  }
}
function explode(state: ShooterState, world: World, r: Rocket, x: number, y: number, z: number): void {
  blast(state, world, r.owner, r.rarity, WEAPONS.rocket.projectile!.radius, damageAt(WEAPONS.rocket, r.rarity, 0, false), 320, 'rocket', x, y, z);
}

/**
 * An explosion (rockets, frags, airstrike bombs): splash damage to fighters
 * and the infected (half to the thrower), heavy damage to every structure in
 * the radius.
 */
export function blast(state: ShooterState, world: World, owner: number, rarity: Rarity, radius: number, damage: number, structure: number, src: DamageSource, x: number, y: number, z: number): void {
  void rarity;
  emit(state, 'explode', owner, x, y, z, { value: radius });
  for (const q of state.players) {
    if (q.mode === 'dead' || q.mode === 'bus') continue;
    const d = Math.hypot(q.x - x, q.y + 1 - y, q.z - z);
    if (d > radius) continue;
    const amount = damage * (1 - (d / radius) * 0.7) * (q.id === owner ? 0.5 : 1);
    hurt(state, q, Math.round(amount), owner, false, true, src);
  }
  const code = killCode(owner, src);
  for (const zb of state.infected) {
    if (zb.hp <= 0) continue;
    const d = Math.hypot(zb.x - x, zb.y + 1 - y, zb.z - z);
    if (d <= radius) hurtZombie(state, zb, Math.round(damage * (1 - (d / radius) * 0.6)), owner, false, code);
  }
  // Rockets are the structure breakers: heavy damage to every piece in the blast.
  _buildHits.length = 0; _buildDmg.length = 0;
  for (const b of state.builds) {
    const a = b.aabb;
    const d = Math.hypot(clamp(x, a.minX, a.maxX) - x, clamp(y, a.minY, a.maxY) - y, clamp(z, a.minZ, a.maxZ) - z);
    if (d <= radius) { _buildHits.push(b); _buildDmg.push(Math.round(structure * (1 - (d / radius) * 0.6))); }
  }
  const hits = _buildHits.slice(), dmg = _buildDmg.slice();
  for (let i = 0; i < hits.length; i++) if (state.builds.includes(hits[i])) damageBuild(state, world, hits[i], dmg[i]);
}
