/** Storm Royale menu data: islands and survivors (Kenney Mini Characters, CC0). */
export type ShooterMapId = 'isle' | 'mesa' | 'deadcity';
export const SHOOTER_MAPS: { id: ShooterMapId; name: string; blurb: string }[] = [
  { id: 'isle', name: 'Palm Isle', blurb: 'Eight landmarks: Harbor Town, the Rustworks factory, Whispering Woods, Loot Lake and its river, a hilltop lookout, farm and port.' },
  { id: 'mesa', name: 'Red Mesa', blurb: 'Desert plateaus: Dust Town, the Oil Works, an oasis lake, Cactus Grove, Mesa Top and long canyon sightlines.' },
  { id: 'deadcity', name: 'Dead City', blurb: 'A burning, ash-choked downtown: two collapsed towers, Mercy Hospital, Eastgate Mall, the Line 3 subway, Checkpoint Bravo and an overgrown park.' },
];
/** Every shooter map id (wire validation, the room config and tests). */
export const SHOOTER_MAP_IDS: readonly ShooterMapId[] = SHOOTER_MAPS.map(m => m.id);
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
/**
 * Game modes (see modes/index.ts). The list order is the wire value: append
 * only. `teams` says whether the solo / duos / squads picker applies; `bots`
 * is a sensible bot count to suggest when the mode is picked.
 */
export type ShooterMode = 'royale' | 'tdm' | 'dom' | 'zombies' | 'apoc';
export const SHOOTER_MODES: { id: ShooterMode; name: string; short: string; blurb: string; teams: boolean; bots: number }[] = [
  { id: 'royale', name: 'Storm Royale', short: 'Royale', blurb: 'Drop in, loot weapons and attachments, build, outlast the storm. Last one (or team) standing.', teams: true, bots: 11 },
  { id: 'tdm', name: 'Team Deathmatch', short: 'TDM', blurb: 'Two teams, class loadouts, instant respawns. First to the score limit wins.', teams: false, bots: 11 },
  { id: 'dom', name: 'Domination', short: 'Domination', blurb: 'Capture and hold A, B and C. Held points score every two seconds.', teams: false, bots: 11 },
  { id: 'zombies', name: 'Zombies', short: 'Zombies', blurb: 'Survive rounds of walkers, runners, brutes and spitters. Earn points, buy from walls, barricade, then make the helicopter.', teams: false, bots: 3 },
  { id: 'apoc', name: 'Apocalypse Royale', short: 'Apocalypse', blurb: 'The royale under a toxic cloud, with the infected roaming the island.', teams: true, bots: 11 },
];
export const SHOOTER_MODE_IDS: readonly ShooterMode[] = SHOOTER_MODES.map(m => m.id);
export const modeName = (id: ShooterMode): string => SHOOTER_MODES.find(m => m.id === id)?.name ?? 'Storm Royale';
