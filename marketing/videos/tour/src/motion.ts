// Motion vocabulary shared by every component: springs tuned for "energetic
// but premium" (fast settle, a hint of overshoot, never bouncy), and beat helpers.
import { Easing, spring } from 'remotion';
import { BEAT, FPS } from './layout';

export const clamp = { extrapolateLeft: 'clamp', extrapolateRight: 'clamp' } as const;

/** Snappy arrival: settles in ~14 frames with ~3% overshoot. */
export const SNAP = { damping: 15, stiffness: 190, mass: 0.7 } as const;
/** Camera moves: a confident glide, ~20 frames, no visible overshoot. */
export const GLIDE = { damping: 22, stiffness: 120, mass: 0.9 } as const;
/** Text drops: a little more spring in the step. */
export const POP = { damping: 12, stiffness: 210, mass: 0.6 } as const;

/** Spring 0→1 starting at frame `at` (relative to the current sequence). */
export function sp(frame: number, at: number, config: { damping: number; stiffness: number; mass: number } = SNAP): number {
  if (frame < at) return 0;
  return spring({ frame: frame - at, fps: FPS, config });
}

/** Beats → frames. */
export const beats = (b: number) => Math.round(b * BEAT);

/** 1 on each beat decaying to 0 before the next (for gentle pulses). */
export function beatPulse(frame: number, decay = 5): number {
  const phase = (((frame % BEAT) + BEAT) % BEAT) / BEAT;
  return Math.exp(-phase * decay);
}

export const easeOut = Easing.out(Easing.cubic);
export const easeIn = Easing.in(Easing.cubic);
export const easeInOut = Easing.bezier(0.65, 0, 0.35, 1);
