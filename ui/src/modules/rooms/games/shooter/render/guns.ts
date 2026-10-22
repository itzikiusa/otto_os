/**
 * Procedural weapon silhouettes for the soldiers' hands and the first-person
 * viewmodel: each item is a handful of boxes in its own frame (grip at the
 * origin, muzzle toward +z, y up, metres), colored from three swatches
 * (body polymer, bare metal, accent / furniture). Pure data so the crowd
 * instancer, the viewmodel and tests share it.
 */

/** [x, y, z centre, width, height, depth, swatch 0 body / 1 metal / 2 accent]. */
export type GunBox = [number, number, number, number, number, number, 0 | 1 | 2];
export interface GunSpec { parts: GunBox[]; /** Muzzle (flash, tracer start) and the sight line height for aiming down sights. */ muzzle: [number, number, number]; sight: number; colors: [number, number, number] }
/** The most boxes any item uses (instance slots per soldier). */
export const GUN_PARTS = 8;

const POLY = 0x2b2e31, STEEL = 0x55595e, TAN = 0x8b7a5a;
export const GUNS: Record<string, GunSpec> = {
  pistol: {
    parts: [[0, 0.03, 0.06, 0.032, 0.05, 0.19, 1], [0, -0.04, 0, 0.03, 0.1, 0.045, 0], [0, 0.0, 0.07, 0.028, 0.025, 0.13, 0]],
    muzzle: [0, 0.035, 0.16], sight: 0.065, colors: [POLY, STEEL, POLY],
  },
  smg: {
    parts: [[0, 0.03, 0.06, 0.05, 0.08, 0.26, 0], [0, 0.045, 0.24, 0.022, 0.022, 0.12, 1], [0, -0.07, 0.09, 0.03, 0.16, 0.04, 1], [0, -0.04, -0.01, 0.03, 0.09, 0.04, 0],
      [0, 0.03, -0.14, 0.03, 0.05, 0.16, 1], [0, 0.085, 0.07, 0.028, 0.03, 0.08, 1]],
    muzzle: [0, 0.045, 0.3], sight: 0.1, colors: [POLY, STEEL, TAN],
  },
  rifle: {
    parts: [[0, 0.035, 0.05, 0.055, 0.085, 0.3, 0], [0, 0.05, 0.31, 0.022, 0.022, 0.3, 1], [0, 0.045, 0.24, 0.06, 0.065, 0.2, 2], [0, -0.07, 0.11, 0.035, 0.15, 0.06, 1],
      [0, -0.04, -0.02, 0.032, 0.1, 0.045, 0], [0, 0.025, -0.21, 0.045, 0.09, 0.22, 2], [0, 0.105, 0.06, 0.035, 0.045, 0.13, 1]],
    muzzle: [0, 0.05, 0.46], sight: 0.125, colors: [POLY, STEEL, TAN],
  },
  shotgun: {
    parts: [[0, 0.035, 0.05, 0.055, 0.08, 0.3, 0], [0, 0.055, 0.33, 0.03, 0.03, 0.36, 1], [0, 0.015, 0.28, 0.045, 0.045, 0.2, 2], [0, -0.04, -0.02, 0.032, 0.1, 0.045, 0],
      [0, 0.02, -0.23, 0.05, 0.1, 0.26, 2], [0, 0.085, 0.1, 0.02, 0.02, 0.05, 1]],
    muzzle: [0, 0.055, 0.51], sight: 0.1, colors: [POLY, STEEL, 0x6b4a2e],
  },
  sniper: {
    parts: [[0, 0.035, 0.05, 0.055, 0.085, 0.34, 0], [0, 0.05, 0.4, 0.024, 0.024, 0.46, 1], [0, -0.06, 0.1, 0.035, 0.11, 0.06, 1], [0, -0.04, -0.03, 0.032, 0.1, 0.045, 0],
      [0, 0.02, -0.25, 0.05, 0.11, 0.28, 2], [0, 0.125, 0.07, 0.05, 0.05, 0.26, 1], [0, 0.125, 0.21, 0.065, 0.065, 0.04, 1], [0, 0.125, -0.06, 0.06, 0.06, 0.04, 1]],
    muzzle: [0, 0.05, 0.63], sight: 0.125, colors: [0x3a3f2c, STEEL, 0x4f5536],
  },
  rocket: {
    parts: [[0, 0.07, 0.12, 0.13, 0.13, 0.95, 2], [0, 0.07, 0.6, 0.15, 0.15, 0.06, 1], [0, 0.07, -0.36, 0.15, 0.15, 0.06, 1], [0, -0.04, 0, 0.032, 0.1, 0.045, 0],
      [0, -0.04, 0.2, 0.032, 0.1, 0.045, 0], [-0.085, 0.12, 0.15, 0.04, 0.06, 0.12, 1]],
    muzzle: [0, 0.07, 0.64], sight: 0.17, colors: [POLY, STEEL, 0x4f5536],
  },
  pickaxe: {
    parts: [[0, 0, 0.12, 0.035, 0.035, 0.75, 2], [0, 0, 0.47, 0.42, 0.06, 0.07, 1], [0, 0, -0.22, 0.045, 0.045, 0.08, 0]],
    muzzle: [0, 0, 0.5], sight: 0.05, colors: [0x222222, 0xb8c4cc, 0x8a5a2b],
  },
  heal: {
    parts: [[0, 0.02, 0.08, 0.16, 0.11, 0.12, 2], [0, 0.02, 0.08, 0.05, 0.115, 0.125, 0], [0, 0.02, 0.08, 0.165, 0.035, 0.125, 0]],
    muzzle: [0, 0.05, 0.15], sight: 0.08, colors: [0xd23a3a, STEEL, 0xf2f2f2],
  },
  shield: {
    parts: [[0, 0.04, 0.08, 0.07, 0.16, 0.07, 2], [0, 0.135, 0.08, 0.04, 0.03, 0.04, 1]],
    muzzle: [0, 0.1, 0.1], sight: 0.08, colors: [POLY, STEEL, 0x3fa6ff],
  },
};
/** The silhouette for a held item id (weapons by id, heals / shields / pads by kind); unknown guns look like rifles. */
export function gunFor(id: string | null | undefined): GunSpec | null {
  if (!id) return null;
  return GUNS[id] ?? (id === 'bandage' || id === 'medkit' ? GUNS.heal : id === 'minishield' || id === 'shieldpot' || id === 'launchpad' ? GUNS.shield : GUNS.rifle);
}
