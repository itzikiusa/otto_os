// Shared kart-sim helpers: scratch buffers, event emission, per-racer stats,
// boosts and spin-outs (used by the race loop, items and contacts).
import { newNear, type Near, type Spot } from '../tracks.ts';
import { racerDef } from '../roster.ts';
import { COIN_LOSS, type Racer, type KartEventType, type KartState } from './model.ts';

export const _near: Near = newNear();
export const _pt = [0, 0, 0, 0];
export const _ob = [0, 0, 0, 0];
export const _spot: Spot = { seg: 0, s: 0 };

export function emit(state: KartState, type: KartEventType, r: { id: number; x: number; y: number; z: number }, value?: number): void {
  state.events.push({ id: ++state.eventSeq, type, racer: r.id, x: r.x, y: r.y, z: r.z, value });
  if (state.events.length > 48) state.events.shift();
}

export const stats = (r: Racer) => {
  const d = racerDef(r.racer);
  return { top: 29 + (d.speed - 3) * 0.9, accel: 15 + (d.accel - 3) * 2.2, turn: 1.9 + (d.handling - 3) * 0.14, weight: 0.8 + d.weight * 0.1 };
};

/**
 * Starts (or extends) a boost: `seconds` at `power` × the racer's top speed,
 * with a shove of `kick` of the way there at once so it is felt straight away.
 * The strongest boost running wins; a later one can only lengthen it.
 */
export function giveBoost(r: Racer, seconds: number, power: number, kick = 0.3): void {
  const top = stats(r).top * power;
  r.boostPower = r.boost > 0 ? Math.max(r.boostPower, power) : power;
  r.boost = Math.max(r.boost, seconds);
  if (r.speed < top) r.speed += (top - r.speed) * kick;
}

export function spinOut(state: KartState, r: Racer, seconds = 1.0): boolean {
  if (r.star > 0 || r.ghost > 0 || r.respawn > 0 || r.finish !== null && r.speed < 1) return false;
  r.spin = Math.max(r.spin, seconds);
  r.boost = 0; r.drift = 0; r.driftCharge = 0; r.driftLevel = 0; r.draft = 0;
  r.coins = Math.max(0, r.coins - COIN_LOSS);
  emit(state, 'spin', r);
  return true;
}
