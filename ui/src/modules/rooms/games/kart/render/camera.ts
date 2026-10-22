/**
 * Chase-camera maths for the kart renderer (no three.js, allocation-free):
 * a critically damped spring for the lagging follow, decaying screen-shake
 * "trauma" with smooth deterministic noise, and the speed / boost field of
 * view kick.
 */
import { clamp } from '../../common/math.ts';

/** One spring-driven value and its velocity. */
export interface Spring { x: number; v: number }
export const spring = (x = 0): Spring => ({ x, v: 0 });
/**
 * Advances `s` toward `target` as a critically damped spring of angular
 * frequency `omega` (higher = stiffer) over `dt`, exactly (frame-rate
 * independent, never overshoots from rest). Returns the new value.
 */
export function springTo(s: Spring, target: number, omega: number, dt: number): number {
  const x = s.x - target, e = Math.exp(-omega * dt), t = (s.v + omega * x) * dt;
  s.v = (s.v - omega * t) * e;
  s.x = target + (x + t) * e;
  return s.x;
}

/**
 * Screen shake as trauma (0..1) that drains over time; the visible shake is
 * trauma² so small knocks stay subtle and big hits jolt.
 */
export class Shake {
  trauma = 0;
  /** Adds a knock (landing ~0.25, a nearby blast ~0.6). */
  add(amount: number): void { this.trauma = clamp(this.trauma + amount, 0, 1); }
  update(dt: number): void { this.trauma = Math.max(0, this.trauma - dt * 1.8); }
  /** Offset along one axis (0, 1, 2…) at time `t`, in units of `scale`. */
  offset(t: number, axis: number, scale: number): number {
    const k = this.trauma * this.trauma;
    if (k === 0) return 0;
    const a = axis * 1.7;
    return k * scale * (Math.sin(t * 37 + a) * 0.6 + Math.sin(t * 23.3 + a * 2.1) * 0.4);
  }
}

/** Chase-camera field of view: widens with speed, kicks with a boost (harder the stronger it is) and on the glider. */
export function chaseFov(speed: number, boost: number, boostPower: number, glide: boolean): number {
  const kick = boost > 0 ? 6 + clamp(boostPower - 1.25, 0, 0.35) * 30 : 0;
  return 66 + clamp(speed - 18, 0, 26) * 0.32 + kick + (glide ? 6 : 0);
}
