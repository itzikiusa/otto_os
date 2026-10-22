// Otto School — the life director's internal actor records (a kid's walk /
// dwell state and the headmaster's patrol state). Only ../life.ts uses them.

import type { Kid, P2 } from '../model.ts';
import type { HeadClip, HeadMode, KidClip, KidMode } from './clips.ts';

export type Goal = 'spot' | 'seat' | 'leave' | 'bench';

export interface KidActor {
  kid: Kid;
  mode: KidMode;
  pos: P2;
  heading: number;
  clip: KidClip;
  path: P2[];
  dist: number;
  speed: number;
  timer: number;
  /** ms until the next "should I get up?" roll (seated + idle). */
  next: number;
  goal: Goal;
  /** Dwell at the spot: facing + clip + ms. */
  spot: { face: number; clip: KidClip; ms: number } | null;
  /** Where the kid sits (its desk, or its bench slot). */
  home: P2;
  /** Lane z of `home`'s row (the walkway the kid leaves its desk by). */
  homeLane: number;
}

export interface HeadActor {
  mode: HeadMode;
  pos: P2;
  lane: number;
  heading: number;
  clip: HeadClip;
  path: P2[];
  dist: number;
  timer: number;
  target: string | null;
  /** The mode to resume after a scold. */
  resume: { mode: HeadMode; clip: HeadClip; timer: number } | null;
}
