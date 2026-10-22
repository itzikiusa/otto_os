/** Small allocation-free math helpers shared by both simulations. Y is up. */
export const TAU = Math.PI * 2;
export const clamp = (v: number, lo: number, hi: number): number => (v < lo ? lo : v > hi ? hi : v);
export const lerp = (a: number, b: number, t: number): number => a + (b - a) * t;
/** Wraps an angle into (-PI, PI]. */
export function wrapAngle(a: number): number {
  a %= TAU;
  if (a > Math.PI) a -= TAU;
  else if (a <= -Math.PI) a += TAU;
  return a;
}
/** Frame-rate independent exponential approach (rate per second). */
export const approach = (current: number, target: number, rate: number, dt: number): number =>
  target + (current - target) * Math.exp(-rate * dt);
export const smoothstep = (e0: number, e1: number, x: number): number => {
  const t = clamp((x - e0) / (e1 - e0), 0, 1);
  return t * t * (3 - 2 * t);
};
export const finite = (v: unknown): v is number => typeof v === 'number' && Number.isFinite(v);
/** A snapshot row: exactly `n` finite numbers (the net decoders' shape check). */
export const nums = (row: unknown, n: number): row is number[] => Array.isArray(row) && row.length === n && row.every(finite);
/** Rounds for compact JSON snapshots. */
export const r2 = (v: number): number => Math.round(v * 100) / 100;
export const r3 = (v: number): number => Math.round(v * 1000) / 1000;

/** Deterministic PRNG (mulberry32); state is a plain uint32 so it can be snapshotted. */
export class Rng {
  state: number;
  constructor(seed: number) { this.state = seed >>> 0 || 0x9e3779b9; }
  next(): number {
    let t = (this.state = (this.state + 0x6d2b79f5) >>> 0);
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  }
  range(lo: number, hi: number): number { return lo + (hi - lo) * this.next(); }
  int(n: number): number { return Math.floor(this.next() * n); }
  pick<T>(items: readonly T[]): T { return items[this.int(items.length)]; }
  chance(p: number): boolean { return this.next() < p; }
}
/** Stable string hash for seeding per-map generation. */
export function hashString(text: string): number {
  let h = 2166136261;
  for (let i = 0; i < text.length; i++) h = Math.imul(h ^ text.charCodeAt(i), 16777619);
  return h >>> 0;
}
/** 2D value noise in [-1, 1], smooth, deterministic for a seed. */
export function makeNoise(seed: number): (x: number, z: number) => number {
  const perm = new Uint8Array(512);
  const rng = new Rng(seed);
  const base = Array.from({ length: 256 }, (_, i) => i);
  for (let i = 255; i > 0; i--) { const j = rng.int(i + 1); [base[i], base[j]] = [base[j], base[i]]; }
  for (let i = 0; i < 512; i++) perm[i] = base[i & 255];
  const value = (ix: number, iz: number) => perm[(perm[ix & 255] + iz) & 511] / 127.5 - 1;
  return (x, z) => {
    const ix = Math.floor(x), iz = Math.floor(z);
    const fx = x - ix, fz = z - iz;
    const u = fx * fx * (3 - 2 * fx), v = fz * fz * (3 - 2 * fz);
    const a = value(ix, iz), b = value(ix + 1, iz), c = value(ix, iz + 1), d = value(ix + 1, iz + 1);
    return lerp(lerp(a, b, u), lerp(c, d, u), v);
  };
}
/** Fractal noise built from makeNoise; octaves halve amplitude, double frequency. */
export function fbm(noise: (x: number, z: number) => number, x: number, z: number, octaves = 4): number {
  let sum = 0, amp = 1, freq = 1, norm = 0;
  for (let o = 0; o < octaves; o++) { sum += noise(x * freq, z * freq) * amp; norm += amp; amp *= 0.5; freq *= 2.03; }
  return sum / norm;
}
export const ordinal = (n: number): string => {
  const s = n % 100;
  if (s >= 11 && s <= 13) return `${n}th`;
  return `${n}${['th', 'st', 'nd', 'rd'][n % 10] ?? 'th'}`;
};
export function raceClock(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return '--:--.---';
  const m = Math.floor(seconds / 60), s = seconds - m * 60;
  return `${m}:${s.toFixed(3).padStart(6, '0')}`;
}
