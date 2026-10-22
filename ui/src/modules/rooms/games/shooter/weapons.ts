/**
 * Storm Royale arsenal (Kenney Blaster Kit models), healing/shield
 * consumables, building materials and the harvesting pickaxe. Damage is per
 * bullet/pellet. A hotbar slot holds either a weapon (`mag` = rounds loaded)
 * or a consumable stack (`mag` = count).
 */
export type WeaponId = 'pistol' | 'smg' | 'rifle' | 'shotgun' | 'sniper' | 'rocket';
export type AmmoKind = 'light' | 'medium' | 'shells' | 'heavy' | 'rockets';
export type Rarity = 0 | 1 | 2 | 3 | 4;
export interface WeaponDef {
  id: WeaponId; name: string; model: string; ammo: AmmoKind;
  damage: number; head: number; rate: number; auto: boolean; mag: number; reload: number;
  pellets: number; spread: number; adsSpread: number; range: number; falloff: [number, number]; minFalloff: number;
  /** Rockets fly as projectiles and explode with splash damage. */
  projectile?: { speed: number; radius: number };
  /** `recoil` is the hip-fire bloom each shot adds (it recovers on its own). */
  zoom: number; recoil: number;
  /**
   * Gunplay: seconds to raise the sights; the per-shot recoil pattern ([up,
   * sideways] kick in radians, cycled, so a spray climbs the same way every
   * time and can be learned and pulled down); how fast the kick recovers
   * once the trigger is released (per second).
   */
  ads: number; pattern: readonly (readonly [number, number])[]; recover: number;
}
/** Recoil patterns: a short climb with a sideways drift that reverses, like a learnable spray. */
const P_RIFLE = [[0.012, 0.002], [0.013, 0.003], [0.012, 0.001], [0.011, -0.002], [0.012, -0.004], [0.01, -0.003], [0.009, 0.002], [0.01, 0.004], [0.009, 0.003], [0.008, -0.002]] as const;
const P_SMG = [[0.007, -0.002], [0.007, 0.003], [0.008, 0.002], [0.006, -0.003], [0.007, -0.004], [0.006, 0.003], [0.006, 0.004], [0.007, -0.001]] as const;
export const WEAPONS: Record<WeaponId, WeaponDef> = {
  pistol: { id: 'pistol', name: 'Sidearm', model: 'gun-blaster-a', ammo: 'light', damage: 24, head: 1.8, rate: 6.5, auto: false, mag: 16, reload: 1.3, pellets: 1, spread: 0.028, adsSpread: 0.012, range: 90, falloff: [30, 90], minFalloff: 0.65, zoom: 58, recoil: 0.012, ads: 0.14, pattern: [[0.02, 0.003], [0.022, -0.004]], recover: 9 },
  smg: { id: 'smg', name: 'Buzz SMG', model: 'gun-blaster-j', ammo: 'light', damage: 15, head: 1.6, rate: 12, auto: true, mag: 30, reload: 1.9, pellets: 1, spread: 0.05, adsSpread: 0.03, range: 70, falloff: [20, 70], minFalloff: 0.55, zoom: 58, recoil: 0.009, ads: 0.18, pattern: P_SMG, recover: 8 },
  rifle: { id: 'rifle', name: 'Assault Blaster', model: 'gun-blaster-d', ammo: 'medium', damage: 29, head: 1.6, rate: 6.5, auto: true, mag: 30, reload: 2.2, pellets: 1, spread: 0.034, adsSpread: 0.009, range: 160, falloff: [60, 160], minFalloff: 0.7, zoom: 46, recoil: 0.014, ads: 0.24, pattern: P_RIFLE, recover: 6 },
  shotgun: { id: 'shotgun', name: 'Pump Scatter', model: 'gun-blaster-l', ammo: 'shells', damage: 10, head: 1.5, rate: 1.1, auto: false, mag: 5, reload: 3.4, pellets: 10, spread: 0.11, adsSpread: 0.085, range: 40, falloff: [8, 32], minFalloff: 0.25, zoom: 58, recoil: 0.05, ads: 0.2, pattern: [[0.05, 0.006]], recover: 5 },
  sniper: { id: 'sniper', name: 'Longshot Rail', model: 'gun-blaster-e', ammo: 'heavy', damage: 98, head: 2.5, rate: 0.55, auto: false, mag: 4, reload: 2.8, pellets: 1, spread: 0.07, adsSpread: 0, range: 320, falloff: [200, 320], minFalloff: 0.85, zoom: 20, recoil: 0.06, ads: 0.38, pattern: [[0.07, -0.01]], recover: 3.5 },
  rocket: { id: 'rocket', name: 'Boom Tube', model: 'gun-blaster-h', ammo: 'rockets', damage: 95, head: 1, rate: 0.75, auto: false, mag: 1, reload: 2.6, pellets: 1, spread: 0.01, adsSpread: 0.004, range: 200, falloff: [200, 200], minFalloff: 1, projectile: { speed: 48, radius: 5 }, zoom: 52, recoil: 0.04, ads: 0.32, pattern: [[0.04, 0]], recover: 4 },
};
export const WEAPON_IDS = Object.keys(WEAPONS) as WeaponId[];
export const AMMO_KINDS: AmmoKind[] = ['light', 'medium', 'shells', 'heavy', 'rockets'];
export const AMMO_PACK: Record<AmmoKind, number> = { light: 36, medium: 30, shells: 8, heavy: 6, rockets: 2 };
export const AMMO_MAX: Record<AmmoKind, number> = { light: 240, medium: 240, shells: 40, heavy: 24, rockets: 6 };
export const RARITY_NAMES = ['Common', 'Uncommon', 'Rare', 'Epic', 'Legendary'] as const;
export const RARITY_COLOURS = [0xb8c0c8, 0x5fd35f, 0x4aa8ff, 0xc06bff, 0xffb02e] as const;
const RARITY_DAMAGE = [1, 1.06, 1.12, 1.18, 1.25];

/** Damage for one bullet/pellet at `distance`, including rarity and headshots. */
export function damageAt(def: WeaponDef, rarity: Rarity, distance: number, head: boolean): number {
  const [near, far] = def.falloff;
  const t = far > near ? Math.min(1, Math.max(0, (distance - near) / (far - near))) : 0;
  const fall = 1 - t * (1 - def.minFalloff);
  return Math.round(def.damage * RARITY_DAMAGE[rarity] * fall * (head ? def.head : 1));
}

/** Shield absorbs first; returns the new [hp, shield] and how much hit the shield. */
export function applyDamage(hp: number, shield: number, amount: number): { hp: number; shield: number; absorbed: number } {
  const absorbed = Math.min(shield, amount);
  return { hp: Math.max(0, hp - (amount - absorbed)), shield: shield - absorbed, absorbed };
}

/**
 * Consumables: hold fire to use; `cap` is the most this item can heal / shield
 * up to. The launch pad is a utility in the same hotbar mechanics: using it
 * deploys a pad on the ground ahead instead of healing.
 */
export type ConsumableId = 'bandage' | 'medkit' | 'minishield' | 'shieldpot' | 'launchpad';
export interface ConsumableDef { id: ConsumableId; name: string; heal: number; shield: number; cap: number; time: number; stack: number; rarity: Rarity; pad?: boolean }
export const CONSUMABLES: Record<ConsumableId, ConsumableDef> = {
  bandage: { id: 'bandage', name: 'Bandages', heal: 15, shield: 0, cap: 75, time: 3, stack: 15, rarity: 0 },
  medkit: { id: 'medkit', name: 'Med Kit', heal: 100, shield: 0, cap: 100, time: 7, stack: 3, rarity: 1 },
  minishield: { id: 'minishield', name: 'Mini Shield', heal: 0, shield: 25, cap: 50, time: 2, stack: 6, rarity: 1 },
  shieldpot: { id: 'shieldpot', name: 'Shield Potion', heal: 0, shield: 50, cap: 100, time: 5, stack: 3, rarity: 2 },
  launchpad: { id: 'launchpad', name: 'Launch Pad', heal: 0, shield: 0, cap: 0, time: 0.6, stack: 2, rarity: 3, pad: true },
};
export const CONSUMABLE_IDS = Object.keys(CONSUMABLES) as ConsumableId[];
export type ItemId = WeaponId | ConsumableId;
/** Wire order: weapons first so the original weapon indices are unchanged. */
export const ITEM_IDS: ItemId[] = [...WEAPON_IDS, ...CONSUMABLE_IDS];
export const isWeapon = (id: ItemId | null | undefined): id is WeaponId => !!id && id in WEAPONS;
export const isConsumable = (id: ItemId | null | undefined): id is ConsumableId => !!id && id in CONSUMABLES;
/** Wire slots for an item index (+1 for "no item"): the loot code packs one of these. */
export const ITEM_SLOTS = ITEM_IDS.length + 1;
export const itemName = (id: ItemId): string => (isWeapon(id) ? WEAPONS[id].name : CONSUMABLES[id].name);
/** Would using `id` do anything for a fighter with this hp / shield? */
export function canUse(id: ConsumableId, hp: number, shield: number): boolean {
  const c = CONSUMABLES[id];
  if (c.pad) return true;
  return c.heal > 0 ? hp < c.cap : shield < c.cap;
}
/** Floor/chest consumable table (bandages and minis are common, pots rare, launch pads rarer). */
export function rollConsumable(r: number): ConsumableId {
  const table: [ConsumableId, number][] = [['bandage', 34], ['minishield', 32], ['shieldpot', 18], ['medkit', 16], ['launchpad', 7]];
  let pick = r * table.reduce((a, [, w]) => a + w, 0);
  for (const [id, w] of table) { pick -= w; if (pick <= 0) return id; }
  return 'bandage';
}
export const CONSUMABLE_PACK: Record<ConsumableId, number> = { bandage: 5, medkit: 1, minishield: 3, shieldpot: 1, launchpad: 1 };

/** Building materials: harvested with the pickaxe, spent on walls / floors / ramps. */
export type Material = 0 | 1 | 2;
export const MATERIAL_NAMES = ['Wood', 'Brick', 'Metal'] as const;
export const MATERIAL_COLOURS = [0xc8955a, 0xb5574a, 0x9aa6b2] as const;
/** Structure hp grows from `start` to `hp` over `time` seconds after placement. */
export const MATERIALS: { hp: number; start: number; time: number }[] = [
  { hp: 150, start: 0.35, time: 2.5 },
  { hp: 300, start: 0.3, time: 4 },
  { hp: 450, start: 0.25, time: 6 },
];
export const MAX_MATS = 500;
export const BUILD_COST = 10;
/** The pickaxe: always in hand when no hotbar slot is selected. */
export const PICKAXE = { damage: 20, structure: 60, rate: 2.2, range: 2.6, harvest: 9 };

/** Weighted rarity roll; chests roll `bonus` tiers higher on average. */
export function rollRarity(r: number, bonus = 0): Rarity {
  const weights = [40, 30, 18, 9, 3].map((w, i) => w * (1 + bonus * i * 0.6));
  const total = weights.reduce((a, b) => a + b, 0);
  let pick = r * total;
  for (let i = 0; i < weights.length; i++) { pick -= weights[i]; if (pick <= 0) return i as Rarity; }
  return 0;
}
/** Floor/chest weapon table. */
export function rollWeapon(r: number): WeaponId {
  const table: [WeaponId, number][] = [['pistol', 24], ['smg', 20], ['rifle', 24], ['shotgun', 18], ['sniper', 8], ['rocket', 6]];
  let pick = r * table.reduce((a, [, w]) => a + w, 0);
  for (const [id, w] of table) { pick -= w; if (pick <= 0) return id; }
  return 'pistol';
}

/**
 * Attachments (a bitmask on the held weapon), found as floor loot in the
 * royale modes and part of every class loadout: a scope zooms further and
 * tightens aimed spread, a grip tames the recoil pattern, an extended mag
 * holds half again as many rounds, a suppressor keeps the shooter off enemy
 * radar (unsuppressed fire pings the minimap) at a little range.
 */
export type AttachId = 'scope' | 'grip' | 'extmag' | 'suppressor';
export const ATTACHMENTS: { id: AttachId; name: string; bit: number }[] = [
  { id: 'scope', name: 'Scope', bit: 1 }, { id: 'grip', name: 'Grip', bit: 2 }, { id: 'extmag', name: 'Extended Mag', bit: 4 }, { id: 'suppressor', name: 'Suppressor', bit: 8 },
];
/** Which attachments a weapon takes (the launcher takes none, the shotgun no suppressor). */
export function attachFits(w: WeaponId, bit: number): boolean {
  if (w === 'rocket') return false;
  if (bit === 8 && w === 'shotgun') return false;
  if (bit === 1 && w === 'sniper') return false;
  return true;
}
/** Effective numbers for a weapon with attachments. Allocation-free (one shared object). */
const _ws = { mag: 0, zoom: 0, adsSpread: 0, kick: 1, ads: 0, range: 1, loud: true };
export function weaponStats(def: WeaponDef, att: number): typeof _ws {
  _ws.mag = att & 4 ? Math.round(def.mag * 1.5) : def.mag;
  _ws.zoom = att & 1 ? def.zoom * 0.62 : def.zoom;
  _ws.adsSpread = att & 1 ? def.adsSpread * 0.6 : def.adsSpread;
  _ws.ads = def.ads * (att & 1 ? 1.25 : 1);
  _ws.kick = att & 2 ? 0.62 : 1;
  _ws.range = att & 8 ? 0.88 : 1;
  _ws.loud = !(att & 8);
  return _ws;
}
/** A random attachment that fits (floor loot rolls this; the bit's index is the wire value). */
export const rollAttach = (r: number): number => Math.min(3, Math.floor(r * 4));

/**
 * Grenades: two lethal (frag, molotov) and two tactical (smoke, flash), thrown
 * in an arc; the spitter's acid glob flies the same way. `fuse` 0 bursts on
 * impact; `radius`/`damage` for the blast or the burning patch; `time` how
 * long a cloud or fire lingers.
 */
export type NadeKind = 0 | 1 | 2 | 3 | 4;
export const NADE_KINDS = 5;
export const NADES: { name: string; lethal: boolean; fuse: number; radius: number; damage: number; time: number }[] = [
  { name: 'Frag', lethal: true, fuse: 2.1, radius: 6.5, damage: 120, time: 0 },
  { name: 'Molotov', lethal: true, fuse: 0, radius: 4.5, damage: 9, time: 7 },
  { name: 'Smoke', lethal: false, fuse: 1.1, radius: 7.5, damage: 0, time: 12 },
  { name: 'Flash', lethal: false, fuse: 1.3, radius: 20, damage: 0, time: 0 },
  { name: 'Acid', lethal: true, fuse: 0, radius: 2.6, damage: 6, time: 3.5 },
];
export const MAX_NADES = 3;

/**
 * Class loadouts for the respawn modes (Team Deathmatch, Domination) and the
 * gear survivors start Zombies with: a primary and secondary (rarity,
 * attachments), a lethal and a tactical grenade and some wood to build with.
 */
export interface ClassDef { id: string; name: string; blurb: string; guns: [WeaponId, Rarity, number][]; lethal: NadeKind; tactical: NadeKind; mats: number }
export const CLASSES: ClassDef[] = [
  { id: 'assault', name: 'Assault', blurb: 'Gripped, scoped rifle and a sidearm · frag · flash', guns: [['rifle', 2, 3], ['pistol', 1, 0]], lethal: 0, tactical: 3, mats: 120 },
  { id: 'rusher', name: 'Rusher', blurb: 'Extended-mag SMG and a shotgun · molotov · smoke', guns: [['smg', 2, 6], ['shotgun', 2, 0]], lethal: 1, tactical: 2, mats: 150 },
  { id: 'marksman', name: 'Marksman', blurb: 'Rail rifle and a suppressed SMG · frag · smoke', guns: [['sniper', 3, 4], ['smg', 1, 8]], lethal: 0, tactical: 2, mats: 90 },
  { id: 'demo', name: 'Demolition', blurb: 'Rocket tube and a rifle · molotov · flash', guns: [['rocket', 2, 0], ['rifle', 1, 2]], lethal: 1, tactical: 3, mats: 200 },
];

/**
 * Killstreaks, earned by eliminations without dying (Zombies: four times as
 * many infected), kept until used: a UAV shows every enemy on the team's
 * radar, an airstrike carpets a line through the point under the crosshair,
 * an attack drone hovers over its owner and shoots whatever comes close.
 */
export const STREAKS: { id: 'uav' | 'airstrike' | 'drone'; name: string; kills: number; time: number }[] = [
  { id: 'uav', name: 'UAV', kills: 3, time: 22 },
  { id: 'airstrike', name: 'Airstrike', kills: 5, time: 0 },
  { id: 'drone', name: 'Attack Drone', kills: 7, time: 25 },
];
