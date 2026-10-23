// 3D embed — when its render loop has work (embed.ts re-arms rAF only then;
// a state tween, a setView, a loaded model, a resize, the pointer or the
// viewport coming back kick it). A still embed schedules no frame at all.

export interface EmbedLoopState {
  /** A change arrived after this frame's render. */
  needsFrame: boolean;
  /** A state tween is running. */
  tween: boolean;
  autoRotate: boolean;
  reduced: boolean;
  /** The hover tilt is still converging on the pointer. */
  tilting: boolean;
  /** OrbitControls damping is still gliding. */
  damping: boolean;
}

export function embedHasWork(s: EmbedLoopState): boolean {
  return s.needsFrame || s.tween || (s.autoRotate && !s.reduced) || s.tilting || s.damping;
}
