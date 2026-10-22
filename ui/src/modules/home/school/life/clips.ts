// Otto School — the life director's vocabulary: animation clips, phase
// timings and speeds, kid / headmaster modes, the per-frame ActorView, the
// seeded RNG and the seated / cheer clip pickers. Pure; re-exported from
// ../life.ts, which the scene and the tests import.

import type { Pose } from '../model.ts';

export type KidClip = 'Sit_Type' | 'Sit_Idle' | 'Sit_RaiseHand' | 'Sit_Slump' | 'Stand_Up' | 'Sit_Down' | 'Walk' | 'Idle_Stand' | 'Talk' | 'Look_Around' | 'Wave';
export type HeadClip = 'Idle' | 'Walk' | 'Nod' | 'Scold' | 'Wave' | 'ThumbsUp' | 'Inspect_Screen' | 'Point_Board';

export const KID_CLIPS: readonly KidClip[] = ['Sit_Type', 'Sit_Idle', 'Sit_RaiseHand', 'Sit_Slump', 'Stand_Up', 'Sit_Down', 'Walk', 'Idle_Stand', 'Talk', 'Look_Around', 'Wave'];
export const HEAD_CLIPS: readonly HeadClip[] = ['Idle', 'Walk', 'Nod', 'Scold', 'Wave', 'ThumbsUp', 'Inspect_Screen', 'Point_Board'];
/** Clips that play once (the scene clamps them on their last frame). */
export const ONCE_CLIPS: ReadonlySet<string> = new Set(['Stand_Up', 'Sit_Down', 'Nod', 'Scold']);

/** Timings (ms) and speeds (m/s). Exported so the scene can fit one-shot
 *  clips to their phase and tests can reason about durations. */
export const STAND_MS = 900;
export const SIT_MS = 900;
export const KID_SPEED = 0.95;
export const HURRY_SPEED = 1.5;
export const HEAD_SPEED = 1.1;
export const INSPECT_MS = 3600;
export const NOD_MS = 1300;
export const SCOLD_MS = 1600;
export const POINT_MS = 3200;
/** Kid cheer (Wave, standing) after its session stops working. */
export const CHEER_MS = 2400;
/** Headmaster turns to you and waves at a needs-you kid's desk. */
export const CALL_MS = 2400;
/** Headmaster's thumbs-up for a cheering kid. */
export const THUMBS_MS = 1800;
/** An unanswered "needs you" this old (ms since the session was last
 *  active) frees the kid to wander. */
export const BORED_MS = 45_000;
/** Facing the board (kids at desks). */
export const FACE_BOARD = Math.PI;
/** Facing the class (kids on the detention bench). */
export const FACE_CLASS = 0;

export type KidMode =
  | 'seated'
  | 'standing' // Stand_Up at the seat, then `after`
  | 'walking' // out to a spot (or a new seat / the door / the bench)
  | 'visiting' // dwelling at a spot
  | 'returning'
  | 'sitting' // Sit_Down at the seat
  | 'celebrating' // Stand_Up → Wave → Sit_Down, at the desk
  | 'leaving'
  | 'gone'
  | 'bench';

export type HeadMode = 'lectern' | 'pointing' | 'walking' | 'inspecting' | 'nodding' | 'scolding' | 'calling' | 'cheering';

export interface ActorView {
  id: string;
  x: number; // room-local
  z: number;
  heading: number;
  clip: KidClip | HeadClip;
  /** Mode name, for the e2e hook and the overlay ("wandering", …). */
  mode: KidMode | HeadMode;
  /** Draw it (false = empty chair / walked out). */
  visible: boolean;
  /** At the desk (the scene keeps the chair tucked in). */
  seated: boolean;
}

/** mulberry32 — small, fast, deterministic. */
export function rng(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

export function seatedClip(pose: Pose, asleep = false): KidClip {
  switch (pose) {
    case 'working':
      return 'Sit_Type';
    case 'needs-you':
      return 'Sit_RaiseHand';
    case 'error':
      return 'Sit_Slump';
    case 'idle':
      return asleep ? 'Sit_Slump' : 'Sit_Idle';
    default:
      return 'Sit_Idle';
  }
}

/** Celebration phase clip at `left` ms remaining of the whole cheer. */
export function cheerClip(left: number): KidClip {
  const elapsed = STAND_MS + CHEER_MS + SIT_MS - left;
  return elapsed < STAND_MS ? 'Stand_Up' : elapsed < STAND_MS + CHEER_MS ? 'Wave' : 'Sit_Down';
}
