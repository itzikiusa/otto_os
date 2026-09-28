// Parking lot for live terminals (r3-09-05). Leaving the Agents module (or
// flipping a pane to chat, or switching the pane to another session) used to
// dispose the xterm AND close its socket; coming back rebuilt both and
// replayed up to 4000 rows of snapshot per pane — ~1–2 MB of VT parsing each,
// on the main thread, for the most frequent navigation in the app.
//
// A Terminal that opts in (`keepAlive`) now PARKS instead: the xterm, its
// socket and its flow state move here, keep receiving output (parsing is
// cheap; the renderer is paused while detached/offscreen), and the next
// Terminal mounted for the same session adopts them — no snapshot, no replay.
//
// Bounds (memory note):
//  - at most PARK_CAP entries, least-recently parked evicted first;
//  - each entry lives at most PARK_TTL_MS (the daemon's idle-suspend grace
//    is 5 min of idle + unattached; a parked socket counts as attached, so
//    the TTL caps how long parking can defer a suspend);
//  - a parked xterm is trimmed to the daemon's own history depth (4000 rows,
//    what a fresh attach would replay anyway) and holds no WebGL context.
//    At ~12 B/cell that is ≤ 4000 × 200 × 12 ≈ 9.6 MB worst case per entry,
//    ≤ ~115 MB for a full lot of very wide, very long sessions; typical
//    entries (≤ 150 cols, partly filled) are a few MB.
// Kept free of xterm/DOM imports so the LRU/TTL rules are unit-testable
// (unit/termPark.test.ts); Terminal.svelte owns what "dispose" means.

export const PARK_CAP = 12;
export const PARK_TTL_MS = 5 * 60 * 1000;
/** Scrollback kept by a parked xterm: the daemon emulator's depth
 *  (otto-pty EMULATOR_SCROLLBACK_LINES), i.e. no less than a re-attach. */
export const PARK_SCROLLBACK = 4000;

type TimerId = ReturnType<typeof setTimeout>;

interface Slot<T> {
  value: T;
  timer: TimerId;
}

export class TermPark<T> {
  private readonly slots = new Map<string, Slot<T>>();
  private readonly dispose: (value: T, key: string) => void;
  private readonly cap: number;
  private readonly ttl: number;
  private readonly setT: (fn: () => void, ms: number) => TimerId;
  private readonly clearT: (id: TimerId) => void;

  constructor(
    dispose: (value: T, key: string) => void,
    cap = PARK_CAP,
    ttl = PARK_TTL_MS,
    setT: (fn: () => void, ms: number) => TimerId = (fn, ms) => setTimeout(fn, ms),
    clearT: (id: TimerId) => void = (id) => clearTimeout(id),
  ) {
    this.dispose = dispose;
    this.cap = cap;
    this.ttl = ttl;
    this.setT = setT;
    this.clearT = clearT;
  }

  get size(): number {
    return this.slots.size;
  }

  has(key: string): boolean {
    return this.slots.has(key);
  }

  /** Park `value` under `key`. An entry already parked under the key is
   *  disposed (one parked engine per session); past the cap the oldest goes. */
  put(key: string, value: T): void {
    this.evict(key);
    const timer = this.setT(() => this.evict(key), this.ttl);
    this.slots.set(key, { value, timer });
    while (this.slots.size > this.cap) {
      const oldest = this.slots.keys().next().value as string;
      this.evict(oldest);
    }
  }

  /** Remove and return the parked value (the caller now owns it). */
  take(key: string): T | null {
    const slot = this.slots.get(key);
    if (!slot) return null;
    this.clearT(slot.timer);
    this.slots.delete(key);
    return slot.value;
  }

  /** Drop `key` if parked, disposing its value. Only disposes `value` when
   *  given and it is the one parked (a stale socket close must not evict a
   *  newer engine parked under the same session). */
  evict(key: string, value?: T): void {
    const slot = this.slots.get(key);
    if (!slot || (value !== undefined && slot.value !== value)) return;
    this.clearT(slot.timer);
    this.slots.delete(key);
    this.dispose(slot.value, key);
  }

  clear(): void {
    for (const key of [...this.slots.keys()]) this.evict(key);
  }
}
