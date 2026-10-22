// Loot: chests, floor pickups, supply drops, launch pads and weapon swaps,
// plus the nearest-item queries the HUD prompts use.
import { clamp, Rng } from '../../common/math.ts';
import { chestY, HALF, mapFor, waterLevel, dryLand, type ShooterMap } from '../map.ts';
import { groundAt, type World } from '../collide.ts';
import { AMMO_MAX, AMMO_PACK, ATTACHMENTS, attachFits, CONSUMABLE_PACK, CONSUMABLES, isConsumable, isWeapon, MAX_MATS, MAX_NADES, NADES, rollAttach, rollConsumable, rollRarity, rollWeapon, WEAPONS, type NadeKind, type Rarity } from '../weapons.ts';
import { STEP, SLOTS, MAX_PADS, MAX_DROPS, DROP_HEIGHT, DROP_SPEED, heldItem, type Held, type Fighter, type Loot, type ShooterState } from './model.ts';
import { emit, dropLoot, dropItem, dropMats, dropAttach, dropNade, dropHeld, takeLoot, fwd } from './core.ts';

// ------------------------------------------------------------ loot

export function nearestChest(state: ShooterState, map: ShooterMap, p: Fighter, range: number): number {
  let best = -1, bd = range;
  map.chests.forEach((c, i) => {
    if (state.chests[i] || Math.abs(chestY(map, c) - p.y) > 2.2) return;
    const d = Math.hypot(c.x - p.x, c.z - p.z);
    if (d < bd) { bd = d; best = i; }
  });
  return best;
}
export function openChest(state: ShooterState, map: ShooterMap, index: number, rng: Rng): void {
  const c = map.chests[index];
  state.chests[index] = true;
  state.lootRev++;
  const y = chestY(map, c);
  const weapon = rollWeapon(rng.next());
  const ox = Math.sin(c.yaw) * 1.6, oz = Math.cos(c.yaw) * 1.6;
  dropItem(state, 'weapon', weapon, rollRarity(rng.next(), 1.2), WEAPONS[weapon].mag, c.x + ox, y, c.z + oz);
  dropLoot(state, 'ammo', WEAPONS[weapon].ammo, AMMO_PACK[WEAPONS[weapon].ammo] * 2, c.x + ox + 1, y, c.z + oz - 0.6);
  const item = rollConsumable(rng.next());
  dropItem(state, 'item', item, CONSUMABLES[item].rarity, CONSUMABLE_PACK[item], c.x + ox - 1, y, c.z + oz + 0.6);
  dropMats(state, 0, 30, c.x + ox, y, c.z + oz + 1.2);
  if (rng.chance(0.45)) dropNade(state, rng.int(4) as NadeKind, c.x + ox + 1.1, y, c.z + oz + 1.1);
  emit(state, 'chest', -1, c.x, y + 0.6, c.z, { value: index });
}

export function pickupAuto(state: ShooterState, p: Fighter): void {
  for (let i = state.loot.length - 1; i >= 0; i--) {
    const l = state.loot[i];
    const dx = l.x - p.x, dz = l.z - p.z;
    if (dx * dx + dz * dz > 1.6 * 1.6 || Math.abs(l.y - p.y) > 2) continue;
    if (l.kind === 'ammo' && l.ammo) {
      const room = AMMO_MAX[l.ammo] - p.ammo[l.ammo];
      if (room <= 0) continue;
      const take = Math.min(room, l.amount);
      p.ammo[l.ammo] += take; l.amount -= take; state.lootRev++;
      if (l.amount <= 0) takeLoot(state, i);
      emit(state, 'pickup', p.id, l.x, l.y, l.z, { value: 1 });
    } else if (l.kind === 'mats' && l.mat !== null) {
      const room = MAX_MATS - p.mats[l.mat];
      if (room <= 0) continue;
      const take = Math.min(room, l.amount);
      p.mats[l.mat] += take; l.amount -= take; state.lootRev++;
      if (l.amount <= 0) takeLoot(state, i);
      emit(state, 'pickup', p.id, l.x, l.y, l.z, { value: 4 });
    } else if (l.kind === 'heal' && p.hp < 100) {
      p.hp = Math.min(100, p.hp + l.amount); takeLoot(state, i);
      emit(state, 'pickup', p.id, l.x, l.y, l.z, { value: 2 });
    } else if (l.kind === 'shield' && p.shield < 100) {
      p.shield = Math.min(100, p.shield + l.amount); takeLoot(state, i);
      emit(state, 'pickup', p.id, l.x, l.y, l.z, { value: 3 });
    } else if (l.kind === 'item' && isConsumable(l.weapon)) {
      // Top up a stack of the same item, then use a free slot.
      const stack = CONSUMABLES[l.weapon].stack, before = l.amount;
      for (const h of p.weapons) {
        if (!h || h.w !== l.weapon || h.mag >= stack || l.amount <= 0) continue;
        const take = Math.min(stack - h.mag, l.amount);
        h.mag += take; l.amount -= take;
      }
      const free = p.weapons.findIndex(w => w === null);
      if (l.amount > 0 && free >= 0) { p.weapons[free] = { w: l.weapon, rarity: l.rarity, mag: l.amount, att: 0 }; l.amount = 0; }
      if (l.amount === before) continue;
      state.lootRev++;
      emit(state, 'pickup', p.id, l.x, l.y, l.z, { value: 5 });
      if (l.amount <= 0) takeLoot(state, i);
    } else if (l.kind === 'nade') {
      // Into the matching pocket: lethal (frag / molotov) or tactical (smoke / flash); a new kind replaces an empty pocket's.
      const kind = l.amount as NadeKind, lethal = NADES[kind]?.lethal;
      if (!NADES[kind]) continue;
      if (lethal) { if (p.lethalN >= MAX_NADES || (p.lethalN > 0 && p.lethal !== kind)) continue; p.lethal = kind; p.lethalN++; }
      else { if (p.tacN >= MAX_NADES || (p.tacN > 0 && p.tacKind !== kind)) continue; p.tacKind = kind; p.tacN++; }
      takeLoot(state, i);
      emit(state, 'pickup', p.id, l.x, l.y, l.z, { value: 7, flag: kind });
    } else if (l.kind === 'attach') {
      // Onto the weapon in hand if it takes it, else the first weapon that takes it and lacks it.
      const bit = ATTACHMENTS[l.amount]?.bit ?? 0;
      const fits = (h: Held | null) => !!h && isWeapon(h.w) && attachFits(h.w, bit) && !(h.att & bit);
      const h = fits(heldItem(p)) ? heldItem(p) : p.weapons.find(fits) ?? null;
      if (!h || !bit) continue;
      h.att |= bit;
      takeLoot(state, i);
      emit(state, 'pickup', p.id, l.x, l.y, l.z, { value: 6, flag: l.amount });
    } else if (l.kind === 'weapon' && isWeapon(l.weapon)) {
      const free = p.weapons.findIndex(w => w === null);
      if (free < 0 || p.weapons.some(w => w?.w === l.weapon && w.rarity >= l.rarity)) continue;
      p.weapons[free] = { w: l.weapon, rarity: l.rarity, mag: l.amount, att: 0 };
      // Holding the pickaxe or an empty slot: draw the new weapon.
      if (!heldItem(p) && !p.building) p.current = free;
      takeLoot(state, i);
      emit(state, 'pickup', p.id, l.x, l.y, l.z, { value: 0 });
    }
  }
}

/** Nearest weapon or consumable on the floor within reach, for the "E to pick up" prompt. */
export function nearestWeaponLoot(state: ShooterState, p: Fighter, range = 2.3): Loot | null {
  let best: Loot | null = null, bd = range;
  for (const l of state.loot) if (l.kind === 'weapon' || l.kind === 'item') { const d = Math.hypot(l.x - p.x, l.z - p.z); if (d < bd && Math.abs(l.y - p.y) < 2) { bd = d; best = l; } }
  return best;
}
export function nearestClosedChest(state: ShooterState, p: Fighter, range = 2.4): number { return nearestChest(state, mapFor(state.map), p, range); }

/** Nearest knocked teammate within reach (index into players, or -1). */
export function nearestDowned(state: ShooterState, p: Fighter, range: number): number {
  let best = -1, bd = range;
  for (const q of state.players) {
    if (q.mode !== 'down' || q.team !== p.team || q.id === p.id || Math.abs(q.y - p.y) > 2) continue;
    const d = Math.hypot(q.x - p.x, q.z - p.z);
    if (d < bd) { bd = d; best = q.id; }
  }
  return best;
}
/** Nearest landed supply drop within reach (index into drops, or -1). */
export function nearestDrop(state: ShooterState, p: Fighter, range = 2.6): number {
  let best = -1, bd = range;
  state.drops.forEach((d, i) => {
    if (!d.landed || Math.abs(d.y - p.y) > 2.2) return;
    const r = Math.hypot(d.x - p.x, d.z - p.z);
    if (r < bd) { bd = r; best = i; }
  });
  return best;
}

/** A supply drop over a dry spot inside the next safe circle. */
export function spawnDrop(state: ShooterState, map: ShooterMap, world: World, rng: Rng): void {
  if (state.drops.length >= MAX_DROPS) return;
  const s = state.storm;
  for (let tries = 0; tries < 20; tries++) {
    const a = rng.range(0, Math.PI * 2), r = rng.range(0, Math.max(4, s.toR * 0.6));
    const x = clamp(s.toX + Math.cos(a) * r, -HALF + 12, HALF - 12), z = clamp(s.toZ + Math.sin(a) * r, -HALF + 12, HALF - 12);
    if (!dryLand(map, x, z) && tries < 19) continue;
    const ground = Math.max(groundAt(world, x, z, DROP_HEIGHT, state.builds), waterLevel(map, x, z));
    state.drops.push({ id: state.nextId++, x, y: DROP_HEIGHT, z, ground, landed: false });
    emit(state, 'supply', -1, x, DROP_HEIGHT, z, { value: state.drops[state.drops.length - 1].id, flag: 0 });
    return;
  }
}
export function stepDrops(state: ShooterState): void {
  for (const d of state.drops) {
    if (d.landed) continue;
    d.y = Math.max(d.ground, d.y - DROP_SPEED * STEP);
    if (d.y <= d.ground) { d.landed = true; emit(state, 'supply', -1, d.x, d.y, d.z, { value: d.id, flag: 1 }); }
  }
}
/** Opens a supply drop: an epic-or-better weapon with ammo, shield potions, metal and sometimes a launch pad. */
export function openDrop(state: ShooterState, index: number, by: number, rng: Rng): void {
  const d = state.drops[index];
  state.drops.splice(index, 1);
  const weapon = rollWeapon(rng.next());
  dropItem(state, 'weapon', weapon, Math.max(3, rollRarity(rng.next(), 2)) as Rarity, WEAPONS[weapon].mag, d.x + 1.4, d.y, d.z);
  dropLoot(state, 'ammo', WEAPONS[weapon].ammo, AMMO_PACK[WEAPONS[weapon].ammo] * 2, d.x + 1.2, d.y, d.z + 1.2);
  dropItem(state, 'item', 'shieldpot', 2, 2, d.x - 1.3, d.y, d.z + 0.4);
  dropMats(state, 2, 60, d.x - 0.6, d.y, d.z - 1.3);
  if (rng.chance(0.4)) dropItem(state, 'item', 'launchpad', 3, 1, d.x + 0.4, d.y, d.z - 1.5);
  dropAttach(state, rollAttach(rng.next()), d.x + 0.2, d.y, d.z + 1.6);
  dropNade(state, 0, d.x - 1.5, d.y, d.z - 0.2);
  // Whoever opens it is handed a killstreak (a UAV, sometimes an airstrike or a drone).
  const opener = state.players[by];
  if (opener) { const k = rng.chance(0.55) ? 0 : rng.chance(0.6) ? 1 : 2; opener.ready |= 1 << k; emit(state, 'streak', by, d.x, d.y, d.z, { value: k, flag: 0 }); }
  state.lootRev++;
  emit(state, 'supply', by, d.x, d.y, d.z, { value: d.id, flag: 2 });
}
/** Deploys a launch pad on open ground just ahead (false when there is no room). */
export function placePad(state: ShooterState, world: World, p: Fighter): boolean {
  if (state.pads.length >= MAX_PADS) return false;
  const [fx, fz] = fwd(p.yaw);
  const x = p.x + fx * 2.2, z = p.z + fz * 2.2;
  const y = groundAt(world, x, z, p.y + 0.6, state.builds);
  if (Math.abs(y - p.y) > 1.2 || y < waterLevel(mapFor(state.map), x, z)) return false;
  for (const pad of state.pads) if (Math.hypot(pad.x - x, pad.z - z) < 3) return false;
  state.pads.push({ id: state.nextId++, x, y, z });
  emit(state, 'use', p.id, x, y, z, { value: 2, flag: 1 });
  return true;
}

export function swapWeapon(state: ShooterState, p: Fighter): void {
  const l = nearestWeaponLoot(state, p);
  if (!l || !l.weapon) return;
  const free = p.weapons.findIndex(w => w === null);
  const slot = free >= 0 ? free : p.current < SLOTS ? p.current : 0;
  const old = p.weapons[slot];
  p.weapons[slot] = { w: l.weapon, rarity: l.rarity, mag: l.amount, att: 0 };
  p.current = slot; p.reload = 0; p.useT = 0;
  takeLoot(state, state.loot.indexOf(l));
  if (old) dropHeld(state, old, p.x - Math.sin(p.yaw) * 1.2, p.y, p.z - Math.cos(p.yaw) * 1.2);
  emit(state, 'pickup', p.id, p.x, p.y, p.z, { value: 0 });
}
