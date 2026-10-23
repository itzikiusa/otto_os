/** Menu metadata for the two room games (no three.js; safe for the chooser). */
import type { GameRoomKind, GameRoomMap } from '../../../lib/api/game-room-types';
import { TRACK_DEFS, type TrackId } from './kart/tracks';
import { RACERS } from './kart/roster';
import { SHOOTER_MAPS, FIGHTERS } from './shooter/catalog';
// The ONE catalog the daemon also reads (otto-rooms games/types.rs include_str!s it):
// map ids, their order and each game's capacity come from here, so the
// chooser can never offer a map the relay refuses.
import CATALOG from './catalog.json';

export type Difficulty = 'easy' | 'normal' | 'hard';
export interface MapChoice { id: GameRoomMap; name: string; blurb: string }
export interface GameChoice {
  kind: GameRoomKind; title: string; tagline: string; cover: string; coverAlt: string; summary: string;
  maxPlayers: number; maps: MapChoice[];
}
export const GAMES: GameChoice[] = [
  {
    kind: 'kart', title: 'Kart Grand Prix', tagline: 'Drift, boost and bump your way to the podium.',
    cover: '/room-games/cover-kart.webp', coverAlt: 'Karts racing through a sunny meadow circuit',
    summary: '8 racers · 3 laps · route choices, gliders, drawbridges & coins', maxPlayers: CATALOG.games.kart.capacity,
    maps: (CATALOG.games.kart.maps as TrackId[]).map(id => ({ id, name: TRACK_DEFS[id].name, blurb: TRACK_DEFS[id].blurb })),
  },
  {
    kind: 'shooter', title: 'Storm Royale', tagline: 'Battle royale, team gunfights and zombie survival.',
    cover: '/room-games/cover-shooter.webp', coverAlt: 'Third-person survivors fighting across an island town',
    summary: 'Royale · Team Deathmatch · Domination · Zombies · Apocalypse · up to 16', maxPlayers: CATALOG.games.shooter.capacity,
    maps: CATALOG.games.shooter.maps.map(id => {
      const m = SHOOTER_MAPS.find(x => x.id === id)!;
      return { id: m.id, name: m.name, blurb: m.blurb };
    }),
  },
];
export const gameFor = (kind: GameRoomKind): GameChoice => GAMES.find(g => g.kind === kind)!;
export { RACERS, FIGHTERS };
