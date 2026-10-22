// Shared shooter-sim helpers: event emission, floor-loot drops and the
// forward vector (used by the match loop, loot, combat and building).
import { ATTACHMENTS, isWeapon, WEAPONS, type AmmoKind, type ItemId, type Material, type NadeKind, type Rarity } from '../weapons.ts';
import { MAX_LOOT, type Held, type LootKind, type ShooterEventType, type ShooterEvent, type ShooterState } from './model.ts';

export function emit(state: ShooterState, type: ShooterEventType, player: number, x: number, y: number, z: number, extra: Partial<ShooterEvent> = {}): void {
  state.events.push({ id: ++state.eventSeq, type, player, target: -1, value: 0, flag: 0, x, y, z, x2: 0, y2: 0, z2: 0, ...extra });
  if (state.events.length > 64) state.events.shift();
}
export function dropLoot(state: ShooterState, kind: LootKind, ammo: AmmoKind | null, amount: number, x: number, y: number, z: number): void {
  if (state.loot.length >= MAX_LOOT) return;
  state.loot.push({ id: state.nextId++, kind, weapon: null, rarity: 0, ammo, mat: null, amount, x, y, z });
  state.lootRev++;
}
export function dropItem(state: ShooterState, kind: 'weapon' | 'item', item: ItemId, rarity: Rarity, amount: number, x: number, y: number, z: number): void {
  if (state.loot.length >= MAX_LOOT) return;
  state.loot.push({ id: state.nextId++, kind, weapon: item, rarity, ammo: null, mat: null, amount, x, y, z });
  state.lootRev++;
}
export function dropMats(state: ShooterState, mat: Material, amount: number, x: number, y: number, z: number): void {
  if (state.loot.length >= MAX_LOOT || amount <= 0) return;
  state.loot.push({ id: state.nextId++, kind: 'mats', weapon: null, rarity: 0, ammo: null, mat, amount, x, y, z });
  state.lootRev++;
}
/** An attachment on the floor (`index` into ATTACHMENTS, carried in `amount`). */
export function dropAttach(state: ShooterState, index: number, x: number, y: number, z: number): void {
  if (state.loot.length >= MAX_LOOT) return;
  state.loot.push({ id: state.nextId++, kind: 'attach', weapon: null, rarity: 2, ammo: null, mat: null, amount: index, x, y, z });
  state.lootRev++;
}
/** A grenade on the floor (`kind`: frag, molotov, smoke, flash; carried in `amount`). */
export function dropNade(state: ShooterState, kind: NadeKind, x: number, y: number, z: number): void {
  if (state.loot.length >= MAX_LOOT) return;
  state.loot.push({ id: state.nextId++, kind: 'nade', weapon: null, rarity: 1, ammo: null, mat: null, amount: kind, x, y, z });
  state.lootRev++;
}
/** Drops a hotbar item as floor loot (a weapon keeps its attachments only while held; they fall beside it). */
export function dropHeld(state: ShooterState, h: Held, x: number, y: number, z: number): void {
  dropItem(state, isWeapon(h.w) ? 'weapon' : 'item', h.w, h.rarity, isWeapon(h.w) ? Math.min(h.mag, WEAPONS[h.w].mag) : h.mag, x, y, z);
  for (let i = 0; i < ATTACHMENTS.length; i++) if (h.att & ATTACHMENTS[i].bit) dropAttach(state, i, x + 0.5 + i * 0.3, y, z - 0.5);
}
export function takeLoot(state: ShooterState, index: number): void { state.loot.splice(index, 1); state.lootRev++; }
export const fwd = (yaw: number): [number, number] => [Math.sin(yaw), Math.cos(yaw)];
