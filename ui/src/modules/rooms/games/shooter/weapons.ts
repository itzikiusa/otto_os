/** Storm Royale arsenal (Kenney Blaster Kit models). Damage is per bullet/pellet. */
export type WeaponId = 'pistol' | 'smg' | 'rifle' | 'shotgun' | 'sniper' | 'rocket';
export type AmmoKind = 'light' | 'medium' | 'shells' | 'heavy' | 'rockets';
export type Rarity = 0 | 1 | 2 | 3 | 4;
export interface WeaponDef {
  id: WeaponId; name: string; model: string; ammo: AmmoKind;
  damage: number; head: number; rate: number; auto: boolean; mag: number; reload: number;
  pellets: number; spread: number; adsSpread: number; range: number; falloff: [number, number]; minFalloff: number;
  /** Rockets fly as projectiles and explode with splash damage. */
  projectile?: { speed: number; radius: number };
  zoom: number; recoil: number;
}
export const WEAPONS: Record<WeaponId, WeaponDef> = {
  pistol: { id: 'pistol', name: 'Sidearm', model: 'gun-blaster-a', ammo: 'light', damage: 24, head: 1.8, rate: 6.5, auto: false, mag: 16, reload: 1.3, pellets: 1, spread: 0.028, adsSpread: 0.012, range: 90, falloff: [30, 90], minFalloff: 0.65, zoom: 58, recoil: 0.012 },
  smg: { id: 'smg', name: 'Buzz SMG', model: 'gun-blaster-j', ammo: 'light', damage: 15, head: 1.6, rate: 12, auto: true, mag: 30, reload: 1.9, pellets: 1, spread: 0.05, adsSpread: 0.03, range: 70, falloff: [20, 70], minFalloff: 0.55, zoom: 58, recoil: 0.009 },
  rifle: { id: 'rifle', name: 'Assault Blaster', model: 'gun-blaster-d', ammo: 'medium', damage: 29, head: 1.6, rate: 6.5, auto: true, mag: 30, reload: 2.2, pellets: 1, spread: 0.034, adsSpread: 0.009, range: 160, falloff: [60, 160], minFalloff: 0.7, zoom: 46, recoil: 0.014 },
  shotgun: { id: 'shotgun', name: 'Pump Scatter', model: 'gun-blaster-l', ammo: 'shells', damage: 10, head: 1.5, rate: 1.1, auto: false, mag: 5, reload: 3.4, pellets: 10, spread: 0.11, adsSpread: 0.085, range: 40, falloff: [8, 32], minFalloff: 0.25, zoom: 58, recoil: 0.05 },
  sniper: { id: 'sniper', name: 'Longshot Rail', model: 'gun-blaster-e', ammo: 'heavy', damage: 98, head: 2.5, rate: 0.55, auto: false, mag: 4, reload: 2.8, pellets: 1, spread: 0.07, adsSpread: 0, range: 320, falloff: [200, 320], minFalloff: 0.85, zoom: 20, recoil: 0.06 },
  rocket: { id: 'rocket', name: 'Boom Tube', model: 'gun-blaster-h', ammo: 'rockets', damage: 95, head: 1, rate: 0.75, auto: false, mag: 1, reload: 2.6, pellets: 1, spread: 0.01, adsSpread: 0.004, range: 200, falloff: [200, 200], minFalloff: 1, projectile: { speed: 48, radius: 5 }, zoom: 52, recoil: 0.04 },
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
