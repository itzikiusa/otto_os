/**
 * Item roulette: odds by race position, Mario Kart style. The leader draws
 * defence (bananas, green shells, the odd bomb); the middle of the pack gets
 * shells and mushrooms; the back gets the catch-up items (triple and golden
 * mushrooms, stars, lightning, the blue shell). `icon` names a portrait under
 * /room-games/portraits (item-<icon>.webp) the HUD can show for each item.
 */
import type { Rng } from '../common/math.ts';

/** Wire order: new kinds are only ever appended (the snapshot sends the index). */
export type ItemKind = 'mushroom' | 'triple' | 'banana' | 'bomb' | 'star' | 'bolt' | 'golden' | 'green' | 'red' | 'blue';
export const ITEMS: Record<ItemKind, { name: string; hint: string; icon: string }> = {
  mushroom: { name: 'Turbo Mushroom', hint: 'A long, strong burst of speed — blasts through grass and mud', icon: 'mushroom' },
  triple: { name: 'Triple Mushroom', hint: 'Three turbo bursts, one per press', icon: 'mushroom' },
  golden: { name: 'Golden Mushroom', hint: 'Unlimited turbo bursts for 7.5 seconds after the first press', icon: 'mushroom' },
  banana: { name: 'Banana', hint: 'Dropped behind you; spins out whoever hits it', icon: 'banana' },
  green: { name: 'Green Shell', hint: 'Fired straight ahead (hold brake: behind); bounces off walls', icon: 'bomb' },
  red: { name: 'Red Shell', hint: 'Homes in on the racer ahead of you', icon: 'bomb' },
  blue: { name: 'Blue Shell', hint: 'Flies over the pack and blasts whoever is leading', icon: 'bomb' },
  bomb: { name: 'Fuse Bomb', hint: 'Lobbed ahead (hold brake: behind); blows up on contact or when the fuse runs out', icon: 'bomb' },
  star: { name: 'Super Star', hint: 'Seven seconds of invincibility and extra speed; off-road cannot slow you', icon: 'star' },
  bolt: { name: 'Lightning', hint: 'Strikes every opponent — never you — spinning, shrinking and slowing them', icon: 'star' },
};
export const ITEM_KINDS: ItemKind[] = ['mushroom', 'triple', 'banana', 'bomb', 'star', 'bolt', 'golden', 'green', 'red', 'blue'];

/** Weights per item for the front, middle and back of the pack. */
const TABLE: Record<ItemKind, [number, number, number]> = {
  mushroom: [10, 22, 10],
  triple: [0, 10, 22],
  golden: [0, 3, 12],
  banana: [40, 12, 2],
  green: [35, 18, 4],
  red: [5, 18, 10],
  blue: [0, 4, 8],
  bomb: [10, 8, 3],
  star: [0, 3, 14],
  bolt: [0, 2, 15],
};

/** Weighted item for `place` (1-based) out of `count` racers. */
export function itemWeights(place: number, count: number): Record<ItemKind, number> {
  const t = count <= 1 ? 0 : (place - 1) / (count - 1);
  const out = {} as Record<ItemKind, number>;
  for (const kind of ITEM_KINDS) {
    const [front, mid, back] = TABLE[kind];
    out[kind] = t < 0.5 ? front + (mid - front) * (t / 0.5) : mid + (back - mid) * ((t - 0.5) / 0.5);
  }
  return out;
}

export function rollItem(rng: Rng, place: number, count: number): ItemKind {
  const weights = itemWeights(place, count);
  const total = ITEM_KINDS.reduce((sum, k) => sum + weights[k], 0);
  let pick = rng.next() * total;
  for (const kind of ITEM_KINDS) {
    pick -= weights[kind];
    if (pick <= 0) return kind;
  }
  return 'mushroom';
}
