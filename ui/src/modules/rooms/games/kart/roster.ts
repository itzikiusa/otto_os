/**
 * Kart Grand Prix racers. Karts and the astronaut drivers are Kenney Car Kit
 * models; Kai, Rosa and Dex are Kenney Mini Characters seated in Car Kit karts.
 * Stats are 1–5 (3 = balanced).
 */
export type RacerId = 'oobi' | 'oodi' | 'ooli' | 'oopi' | 'oozi' | 'kai' | 'rosa' | 'dex';
export interface RacerDef {
  id: RacerId; name: string; tagline: string;
  kart: string; driver: string | null; colour: string;
  speed: number; accel: number; handling: number; weight: number;
}
export const RACERS: RacerDef[] = [
  { id: 'oobi', name: 'Oobi', tagline: 'All-rounder', kart: 'kart-kart-oobi', driver: null, colour: '#9b8cf2', speed: 3, accel: 3, handling: 3, weight: 3 },
  { id: 'oodi', name: 'Oodi', tagline: 'Top speed', kart: 'kart-kart-oodi', driver: null, colour: '#f27fa6', speed: 4, accel: 2, handling: 3, weight: 3 },
  { id: 'ooli', name: 'Ooli', tagline: 'Quick off the line', kart: 'kart-kart-ooli', driver: null, colour: '#f2c84b', speed: 2, accel: 5, handling: 3, weight: 2 },
  { id: 'oopi', name: 'Oopi', tagline: 'Sharp handling', kart: 'kart-kart-oopi', driver: null, colour: '#4bc49a', speed: 2, accel: 3, handling: 5, weight: 2 },
  { id: 'oozi', name: 'Oozi', tagline: 'Heavyweight', kart: 'kart-kart-oozi', driver: null, colour: '#e8b48c', speed: 4, accel: 2, handling: 2, weight: 5 },
  { id: 'kai', name: 'Officer Kai', tagline: 'Steady drifter', kart: 'kart-kart-oodi', driver: 'male-c', colour: '#5b7bd6', speed: 3, accel: 3, handling: 4, weight: 3 },
  { id: 'rosa', name: 'Rosa', tagline: 'Boost hunter', kart: 'kart-kart-oozi', driver: 'female-f', colour: '#e07a4f', speed: 3, accel: 4, handling: 3, weight: 2 },
  { id: 'dex', name: 'Dex', tagline: 'Bruiser', kart: 'kart-kart-ooli', driver: 'male-a', colour: '#3fae5a', speed: 4, accel: 3, handling: 2, weight: 4 },
];
export const RACER_IDS = RACERS.map(r => r.id);
export const racerDef = (id: string): RacerDef => RACERS.find(r => r.id === id) ?? RACERS[0];
