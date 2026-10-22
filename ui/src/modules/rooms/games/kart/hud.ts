/**
 * Kart HUD logic (pure, no DOM): the art each item shows in the item box and
 * roulette, the position badge's medal tier, and the on-screen call-out for
 * a race event as the viewer sees it — lightning is good news for whoever
 * fired it and bad news only for the racers it struck.
 */
import { ITEM_KINDS, ITEMS, type ItemKind } from './items.ts';
import type { KartEvent, Racer } from './sim.ts';

/** Portrait URL for an item icon (public/room-games/portraits). */
export const portrait = (icon: string): string => `/room-games/portraits/item-${icon}.webp`;
/** How the HUD draws an item: a portrait (gold-toned for the golden mushroom), a shell in its colour, or the bolt. */
export type ItemArt = { kind: 'portrait'; src: string; gold: boolean; count: number } | { kind: 'shell'; color: 'green' | 'red' | 'blue' } | { kind: 'bolt' };
export function itemArt(item: ItemKind): ItemArt {
  switch (item) {
    case 'green': case 'red': case 'blue': return { kind: 'shell', color: item };
    case 'bolt': return { kind: 'bolt' };
    default: return { kind: 'portrait', src: portrait(ITEMS[item].icon), gold: item === 'golden', count: item === 'triple' ? 3 : 1 };
  }
}
/** The roulette spins through every item. */
export const ROULETTE: readonly ItemKind[] = ITEM_KINDS;

/** Medal tier of the position badge. */
export function placeTier(place: number): 'gold' | 'silver' | 'bronze' | 'pack' {
  return place === 1 ? 'gold' : place === 2 ? 'silver' : place === 3 ? 'bronze' : 'pack';
}

/** A call-out: text, tone (count / good / small / bad / info) and how long it stays. */
export interface Notice { text: string; kind: 'count' | 'good' | 'small' | 'bad' | 'info'; ms: number }
const ORD = (n: number) => { const s = n % 100; return s >= 11 && s <= 13 ? `${n}th` : `${n}${['th', 'st', 'nd', 'rd'][n % 10] ?? 'th'}`; };
/**
 * The call-out for event `e` as the racer at index `mine` sees it, or null
 * for none. Lightning: the user's own bolt is a positive "Lightning!" (with
 * how many it struck); someone else's is neutral news; being struck (a `zap`
 * on the viewer) is the bad one.
 */
export function noticeFor(e: KartEvent, mine: number, racers: Pick<Racer, 'name' | 'place'>[]): Notice | null {
  const own = e.racer === mine, r = racers[e.racer], v = e.value ?? 0;
  switch (e.type) {
    case 'count': return { text: String(v), kind: 'count', ms: 900 };
    case 'go': return { text: 'GO!', kind: 'count', ms: 900 };
    case 'turbo': return own && v >= 2 ? { text: v >= 3 ? 'Ultra mini-turbo!' : 'Super mini-turbo!', kind: 'small', ms: 900 } : null;
    case 'boost': return own && v === 2 ? { text: 'Rocket start!', kind: 'good', ms: 1100 } : own && v === 3 ? { text: 'Golden Mushroom!', kind: 'good', ms: 1100 } : null;
    case 'burnout': return own ? { text: 'Burnout! Too early', kind: 'bad', ms: 1100 } : null;
    case 'draft': return own ? { text: 'Slipstream!', kind: 'small', ms: 800 } : null;
    case 'spin': return own ? { text: 'Spun out!', kind: 'bad', ms: 900 } : null;
    case 'star': return own ? { text: 'Super Star!', kind: 'good', ms: 1400 } : null;
    case 'bolt':
      if (own) return { text: v > 0 ? `Lightning! ${v} struck` : 'Lightning!', kind: 'good', ms: 1400 };
      return { text: `${r?.name ?? 'Someone'} called down Lightning`, kind: 'info', ms: 1400 };
    case 'zap': return own ? { text: 'Struck by lightning!', kind: 'bad', ms: 1200 } : null;
    case 'trick': return own ? { text: 'Trick!', kind: 'small', ms: 700 } : null;
    case 'respawn': return own ? { text: 'Rescued', kind: 'small', ms: 900 } : null;
    case 'splash': return own && v !== 1 ? { text: v === 2 ? 'Too hot!' : 'Splash!', kind: 'bad', ms: 900 } : null;
    case 'glide': return own ? { text: 'Glider!', kind: 'good', ms: 900 } : null;
    case 'crush': return own ? { text: 'Squashed!', kind: 'bad', ms: 900 } : null;
    case 'lap': return own ? { text: `Lap ${v + 1}`, kind: 'good', ms: 1400 } : null;
    case 'final': return own ? { text: 'Final lap!', kind: 'good', ms: 1800 } : null;
    case 'finish': return own ? { text: `Finished ${ORD(r?.place ?? 1)}!`, kind: 'good', ms: 3000 } : null;
    default: return null;
  }
}
