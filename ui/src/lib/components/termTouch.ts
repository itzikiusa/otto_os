import type { Terminal } from '@xterm/xterm';

// Touch gestures are independent of terminal rendering (DOM or WebGL) and
// viewport width: a phone in landscape is still operated with a finger.
export interface TouchPoint {
  pointerId: number;
  clientX: number;
  clientY: number;
  timeStamp: number;
}

interface TouchActions {
  scroll: (dx: number, dy: number, x: number, y: number) => void;
  tap: () => void;
  reset?: () => void;
  canCoast: () => boolean;
  frame?: (cb: FrameRequestCallback) => number;
  cancelFrame?: (id: number) => void;
}

/** A drag never focuses the keyboard; only a completed tap does. */
export class TerminalTouch {
  private gesture: (TouchPoint & { startX: number; startY: number; moved: boolean; vx: number; vy: number }) | null = null;
  private frameId: number | null = null;
  private readonly frame: (cb: FrameRequestCallback) => number;
  private readonly cancelFrame: (id: number) => void;

  private readonly actions: TouchActions;

  constructor(actions: TouchActions) {
    this.actions = actions;
    this.frame = actions.frame ?? ((cb) => requestAnimationFrame(cb));
    this.cancelFrame = actions.cancelFrame ?? ((id) => cancelAnimationFrame(id));
  }

  start(p: TouchPoint): void {
    this.cancel();
    this.gesture = { pointerId: p.pointerId, clientX: p.clientX, clientY: p.clientY, timeStamp: p.timeStamp,
      startX: p.clientX, startY: p.clientY, moved: false, vx: 0, vy: 0 };
  }

  move(p: TouchPoint): void {
    const g = this.gesture;
    if (!g || g.pointerId !== p.pointerId) return;
    if (!g.moved && Math.hypot(p.clientX - g.startX, p.clientY - g.startY) < 7) return;
    g.moved = true;
    const dx = g.clientX - p.clientX;
    const dy = g.clientY - p.clientY;
    const elapsed = Math.max(1, p.timeStamp - g.timeStamp);
    g.vx = dx / elapsed;
    g.vy = dy / elapsed;
    this.actions.scroll(dx, dy, p.clientX, p.clientY);
    g.clientX = p.clientX;
    g.clientY = p.clientY;
    g.timeStamp = p.timeStamp;
  }

  end(p: TouchPoint): void {
    const g = this.gesture;
    if (!g || g.pointerId !== p.pointerId) return;
    this.gesture = null;
    if (!g.moved) {
      this.actions.tap();
      return;
    }
    if (!this.actions.canCoast() || p.timeStamp - g.timeStamp > 80) return;
    let vx = Math.max(-2, Math.min(2, g.vx));
    let vy = Math.max(-2, Math.min(2, g.vy));
    let last: number | null = null;
    let duration = 0;
    const coast: FrameRequestCallback = (now) => {
      this.frameId = null;
      if (!this.actions.canCoast()) return;
      const dt = last === null ? 16 : Math.min(32, now - last);
      last = now;
      duration += dt;
      vx *= Math.exp(-dt / 140);
      vy *= Math.exp(-dt / 140);
      if (duration > 600 || Math.hypot(vx, vy) < 0.025) return;
      this.actions.scroll(vx * dt, vy * dt, p.clientX, p.clientY);
      this.frameId = this.frame(coast);
    };
    this.frameId = this.frame(coast);
  }

  /** Cancellation, a new session or unmount never counts as a tap. */
  cancel(): void {
    this.gesture = null;
    this.actions.reset?.();
    if (this.frameId !== null) this.cancelFrame(this.frameId);
    this.frameId = null;
  }
}

/** Map touch movement to scrollback or the active terminal application. */
export function createTerminalTouch(getTerminal: () => Terminal | null | undefined, readOnly: () => boolean): TerminalTouch {
  let touchScrollAccum = 0;
  return new TerminalTouch({
    reset: () => { touchScrollAccum = 0; },
    tap: () => { if (!readOnly()) getTerminal()?.focus(); },
    canCoast: () => { const term = getTerminal(); return !!term && term.buffer.active.type === 'normal'
      && term.modes.mouseTrackingMode === 'none'
      && !window.matchMedia('(prefers-reduced-motion: reduce)').matches; },
    scroll: (dx, dy, x, y) => {
      const term = getTerminal();
      if (!term) return;
      const host = term.element?.parentElement;
      if (host) host.scrollLeft += dx;
      // Let xterm translate wheel movement for interactive full-screen CLIs.
      // A read-only viewer must never turn a gesture into PTY input.
      if (term.buffer.active.type === 'alternate' || term.modes.mouseTrackingMode !== 'none') {
        if (!readOnly()) term.element?.dispatchEvent(new WheelEvent('wheel', {
          deltaY: dy, clientX: x, clientY: y, bubbles: true, cancelable: true,
        }));
        return;
      }
      const height = term.element?.querySelector('.xterm-screen')?.getBoundingClientRect().height;
      const lineHeight = height && term.rows ? height / term.rows : 20;
      touchScrollAccum += dy;
      const lines = Math.trunc(touchScrollAccum / lineHeight);
      if (lines) {
        touchScrollAccum -= lines * lineHeight;
        term.scrollLines(lines);
      }
    },
  });

}
