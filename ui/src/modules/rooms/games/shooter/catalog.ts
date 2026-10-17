/** Storm Royale menu data: islands and survivors (Kenney Mini Characters, CC0). */
export type ShooterMapId = 'isle' | 'mesa';
export const SHOOTER_MAPS: { id: ShooterMapId; name: string; blurb: string }[] = [
  { id: 'isle', name: 'Palm Isle', blurb: 'Green island with a seaside town, farm, forest hills and a busy container port.' },
  { id: 'mesa', name: 'Red Mesa', blurb: 'Desert plateaus, a dusty frontier town, an oil yard and long canyon sightlines.' },
];
export type FighterId = 'male-a' | 'male-b' | 'male-c' | 'male-d' | 'male-e' | 'male-f' | 'female-a' | 'female-b' | 'female-c' | 'female-d' | 'female-e' | 'female-f';
export const FIGHTERS: { id: FighterId; name: string }[] = [
  { id: 'male-a', name: 'Dex' }, { id: 'female-b', name: 'Sunny' }, { id: 'male-c', name: 'Officer Kai' }, { id: 'female-d', name: 'Ivy' },
  { id: 'male-b', name: 'Bruno' }, { id: 'female-a', name: 'Nova' }, { id: 'male-d', name: 'Milo' }, { id: 'female-c', name: 'Rex' },
  { id: 'male-e', name: 'Doc' }, { id: 'female-e', name: 'Juno' }, { id: 'male-f', name: 'Theo' }, { id: 'female-f', name: 'Rosa' },
];
export const FIGHTER_IDS = FIGHTERS.map(f => f.id);
export const fighterName = (id: string): string => FIGHTERS.find(f => f.id === id)?.name ?? 'Survivor';
