// Otto School — when the render loop has work (scene.ts re-arms rAF only
// then; `kick()` restarts it on any input, update or door swing). An idle
// corridor, or a room under reduced motion, schedules no frame at all.

export interface SchoolLoopState {
  /** A camera flight is running. */
  tween: boolean;
  /** Movement keys held. */
  keys: number;
  /** A classroom is open (its kids walk and animate). */
  roomOpen: boolean;
  reduced: boolean;
  /** Kicked-out kids still walking to the door. */
  kicks: number;
  /** A door leaf moved this frame. */
  doorsMoving: boolean;
}

export function schoolHasWork(s: SchoolLoopState): boolean {
  return s.tween || s.keys > 0 || (s.roomOpen && !s.reduced) || s.kicks > 0 || s.doorsMoving;
}
