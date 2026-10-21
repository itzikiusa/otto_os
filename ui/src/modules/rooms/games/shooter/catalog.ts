/** Storm Royale menu data: islands and survivors (Kenney Mini Characters, CC0). */
export type ShooterMapId = 'isle' | 'mesa';
export const SHOOTER_MAPS: { id: ShooterMapId; name: string; blurb: string }[] = [
  { id: 'isle', name: 'Palm Isle', blurb: 'Eight landmarks: Harbor Town, the Rustworks factory, Whispering Woods, Loot Lake and its river, a hilltop lookout, farm and port.' },
  { id: 'mesa', name: 'Red Mesa', blurb: 'Desert plateaus: Dust Town, the Oil Works, an oasis lake, Cactus Grove, Mesa Top and long canyon sightlines.' },
];
export type FighterId = 'male-a' | 'male-b' | 'male-c' | 'male-d' | 'male-e' | 'male-f' | 'female-a' | 'female-b' | 'female-c' | 'female-d' | 'female-e' | 'female-f';
export const FIGHTERS: { id: FighterId; name: string }[] = [
  { id: 'male-a', name: 'Dex' }, { id: 'female-b', name: 'Sunny' }, { id: 'male-c', name: 'Officer Kai' }, { id: 'female-d', name: 'Ivy' },
  { id: 'male-b', name: 'Bruno' }, { id: 'female-a', name: 'Nova' }, { id: 'male-d', name: 'Milo' }, { id: 'female-c', name: 'Rex' },
  { id: 'male-e', name: 'Doc' }, { id: 'female-e', name: 'Juno' }, { id: 'male-f', name: 'Theo' }, { id: 'female-f', name: 'Rosa' },
];
export const FIGHTER_IDS = FIGHTERS.map(f => f.id);
export const fighterName = (id: string): string => FIGHTERS.find(f => f.id === id)?.name ?? 'Survivor';
/** Team colours (duos / squads): markers, map dots, HUD and results. */
export const TEAM_COLOURS = [0x4aa8ff, 0xff6b5a, 0x5fd35f, 0xffb02e, 0xc06bff, 0x37d2c8, 0xff7ad9, 0xd8d84a] as const;
export const teamColour = (team: number): number => TEAM_COLOURS[((team % TEAM_COLOURS.length) + TEAM_COLOURS.length) % TEAM_COLOURS.length];
export const TEAM_MODES = [{ size: 1, name: 'Solo' }, { size: 2, name: 'Duos' }, { size: 4, name: 'Squads' }] as const;
