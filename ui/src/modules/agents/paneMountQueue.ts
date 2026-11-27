// Priority-first progressive mounting for multi-terminal views (TiledView,
// Splits). Mounting 11–15 <SessionView>/<Terminal> instances in one synchronous
// flush creates 10 WebGL2 contexts + 1–5 DOM xterm trees + 11–15 concurrent
// /ws/term scrollback snapshots in a single frame, stalling the main thread
// for 1.1–3.2 s before the focused terminal becomes usable.
//
// Instead, Frame 0 mounts ONLY the priority (focused/active) pane so it gets
// uncontested CPU, GPU and daemon snapshot bandwidth (~80–180 ms regardless of
// N). Remaining visible panes hydrate one at a time on paced timers; leaving
// the page or switching workspaces cancels pending mounts so hidden panes cost
// nothing. Pure and rune-free so unit/paneMountQueue.test.ts drives every rule.

/** Grace before the first background pane mounts, giving the focused terminal
 *  time to open its socket, apply its snapshot and paint uncontested. */
export const INITIAL_MOUNT_DELAY_MS = 200;
/** Spacing between subsequent background pane mounts so each paints in its own
 *  frame instead of stacking WebGL commits into one multi-second longtask. */
export const STEP_MOUNT_DELAY_MS = 60;

type TimerId = ReturnType<typeof setTimeout>;

export interface PaneMountQueueOpts {
  initialDelayMs?: number;
  stepDelayMs?: number;
  setT?: (fn: () => void, ms: number) => TimerId;
  clearT?: (id: TimerId) => void;
  /** Return false while the page is hidden to pause background hydration. */
  isVisible?: () => boolean;
}

/** Decide which tiles in a tiled grid qualify for a live terminal, in priority
 *  order: active first, then explicitly pinned tiles, then live visible tiles
 *  (skipping threads and inactive suspended/exited sessions that hold no live
 *  PTY), capped at `maxLive`. */
export function selectLiveTiles<T extends { id: string }>(
  ordered: readonly T[],
  opts: {
    activeId: string | null;
    pinned: ReadonlySet<string>;
    visible: ReadonlySet<string>;
    isInactive: (item: T) => boolean;
    isThread: (item: T) => boolean;
    maxLive: number;
  },
): string[] {
  const out: string[] = [];
  const seen = new Set<string>();
  const add = (id: string): void => {
    if (seen.has(id) || out.length >= opts.maxLive) return;
    seen.add(id);
    out.push(id);
  };
  if (opts.activeId) {
    const active = ordered.find((s) => s.id === opts.activeId && !opts.isThread(s));
    if (active) add(active.id);
  }
  // Explicit attaches next — honor deliberate user clicks even for suspended tiles.
  for (const s of ordered) {
    if (out.length >= opts.maxLive) break;
    if (!opts.isThread(s) && opts.pinned.has(s.id)) add(s.id);
  }
  // Auto-attach only live visible tiles: suspended/exited sessions have no live
  // PTY and stay lightweight placeholders until clicked.
  for (const s of ordered) {
    if (out.length >= opts.maxLive) break;
    if (!opts.isThread(s) && opts.visible.has(s.id) && !opts.isInactive(s)) add(s.id);
  }
  return out;
}

export class PaneMountQueue {
  private admitted = new Set<string>();
  private wanted: string[] = [];
  private timer: TimerId | null = null;
  private readonly onChange: (admitted: ReadonlySet<string>) => void;
  private readonly initialDelayMs: number;
  private readonly stepDelayMs: number;
  private readonly setT: (fn: () => void, ms: number) => TimerId;
  private readonly clearT: (id: TimerId) => void;
  private readonly isVisible: () => boolean;

  constructor(onChange: (admitted: ReadonlySet<string>) => void, opts: PaneMountQueueOpts = {}) {
    this.onChange = onChange;
    this.initialDelayMs = opts.initialDelayMs ?? INITIAL_MOUNT_DELAY_MS;
    this.stepDelayMs = opts.stepDelayMs ?? STEP_MOUNT_DELAY_MS;
    this.setT = opts.setT ?? ((fn, ms) => setTimeout(fn, ms));
    this.clearT = opts.clearT ?? ((id) => clearTimeout(id));
    this.isVisible = opts.isVisible ?? (() => typeof document === 'undefined' || document.visibilityState !== 'hidden');
  }

  /** Sync the desired set of panes and the priority (focused) pane.
   *  The priority pane (or first wanted pane if none is focused) is admitted
   *  synchronously; already-admitted panes still in `wanted` stay mounted;
   *  any remaining panes hydrate one per paced tick. */
  sync(wanted: readonly string[], priorityId: string | null): void {
    const wantedSet = new Set(wanted);
    this.wanted = [...wantedSet];
    let changed = false;

    for (const id of [...this.admitted]) {
      if (!wantedSet.has(id)) {
        this.admitted.delete(id);
        changed = true;
      }
    }

    const immediate = priorityId && wantedSet.has(priorityId) ? priorityId : (this.wanted[0] ?? null);
    if (immediate && !this.admitted.has(immediate)) {
      this.admitted.add(immediate);
      changed = true;
    }

    if (changed) this.onChange(new Set(this.admitted));
    this.scheduleNext();
  }

  /** Resume background hydration when the tab/window becomes visible again. */
  wake(): void {
    this.scheduleNext();
  }

  /** Clear all admitted panes and cancel pending mounts (workspace switch). */
  reset(): void {
    this.cancelTimer();
    this.wanted = [];
    if (this.admitted.size > 0) {
      this.admitted.clear();
      this.onChange(new Set());
    }
  }

  destroy(): void {
    this.cancelTimer();
    this.wanted = [];
    this.admitted.clear();
  }

  private nextPending(): string | null {
    for (const id of this.wanted) {
      if (!this.admitted.has(id)) return id;
    }
    return null;
  }

  private cancelTimer(): void {
    if (this.timer !== null) {
      this.clearT(this.timer);
      this.timer = null;
    }
  }

  private scheduleNext(): void {
    const next = this.nextPending();
    if (!next) {
      this.cancelTimer();
      return;
    }
    if (this.timer !== null || !this.isVisible()) return;
    const delay = this.admitted.size <= 1 ? this.initialDelayMs : this.stepDelayMs;
    this.timer = this.setT(() => {
      this.timer = null;
      if (!this.isVisible()) return;
      const pick = this.nextPending();
      if (!pick) return;
      this.admitted.add(pick);
      this.onChange(new Set(this.admitted));
      this.scheduleNext();
    }, delay);
  }
}
