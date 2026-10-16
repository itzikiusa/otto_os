/** Item roulette: racers further back get stronger catch-up items. */
import type { Rng } from '../common/math.ts';

export type ItemKind = 'mushroom' | 'triple' | 'banana' | 'bomb' | 'star' | 'bolt';
export const ITEMS: Record<ItemKind, { name: string; hint: string }> = {
  mushroom: { name: 'Turbo Mushroom', hint: 'A burst of speed — cuts across grass' },
  triple: { name: 'Triple Mushroom', hint: 'Three turbo bursts' },
  banana: { name: 'Banana', hint: 'Dropped behind you; spins out whoever hits it' },
  bomb: { name: 'Homing Bomb', hint: 'Chases the racer ahead of you' },
  star: { name: 'Super Star', hint: 'Seven seconds of invincibility and extra speed' },
  bolt: { name: 'Thunderbolt', hint: 'Spins and slows every racer ahead of you' },
};
export const ITEM_KINDS = Object.keys(ITEMS) as ItemKind[];

/** Weights per item for the front, middle and back of the pack. */
const TABLE: Record<ItemKind, [number, number, number]> = {
  mushroom: [20, 26, 14],
  triple: [0, 14, 28],
  banana: [50, 18, 4],
  bomb: [30, 30, 16],
  star: [0, 9, 24],
  bolt: [0, 3, 14],
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
