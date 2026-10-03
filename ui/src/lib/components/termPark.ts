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
//  - at most PARK_CAP entries AND at most PARK_BUDGET_BYTES of estimated
//    xterm heap (perf 01 N2), least-recently parked evicted first;
//  - each entry lives at most PARK_TTL_MS (the daemon's idle-suspend grace
//    is 5 min of idle + unattached; a parked socket counts as attached, so
//    the TTL caps how long parking can defer a suspend);
//  - a parked xterm is trimmed to the daemon's own history depth (4000 rows,
//    what a fresh attach would replay anyway) and holds no WebGL context.
//    At ~12 B/cell that is ≤ 4000 × 200 × 12 ≈ 9.6 MB worst case per entry.
//    By count alone a full lot of very wide, very long sessions was ~115 MB;
//    the byte budget holds it to ~48 MB, while typical entries (≤ 150 cols,
//    partly filled, a few MB) still fill all PARK_CAP slots.
// Kept free of xterm/DOM imports so the LRU/TTL rules are unit-testable
// (unit/termPark.test.ts); Terminal.svelte owns what "dispose" means.

export const PARK_CAP = 12;
export const PARK_TTL_MS = 5 * 60 * 1000;
/** Scrollback kept by a parked xterm: the daemon emulator's depth
 *  (otto-pty EMULATOR_SCROLLBACK_LINES), i.e. no less than a re-attach. */
export const PARK_SCROLLBACK = 4000;
/** Estimated xterm heap per buffer cell (Terminal.svelte `parkedBytes`). */
export const PARK_CELL_BYTES = 12;
/** Estimated heap the whole lot may hold (perf 01 N2): ~5 worst-case
 *  (4000 × 200) engines, or all PARK_CAP typical ones. */
export const PARK_BUDGET_BYTES = 48 * 1024 * 1024;

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
  private readonly sizeOf: (value: T) => number;
  private readonly budget: number;

  constructor(
    dispose: (value: T, key: string) => void,
    cap = PARK_CAP,
    ttl = PARK_TTL_MS,
    setT: (fn: () => void, ms: number) => TimerId = (fn, ms) => setTimeout(fn, ms),
    clearT: (id: TimerId) => void = (id) => clearTimeout(id),
    /** Estimated bytes an entry holds (default: none — count bound only).
     *  Read at every `put`: a parked engine keeps growing to PARK_SCROLLBACK. */
    sizeOf: (value: T) => number = () => 0,
    budget = PARK_BUDGET_BYTES,
  ) {
    this.dispose = dispose;
    this.cap = cap;
    this.ttl = ttl;
    this.setT = setT;
    this.clearT = clearT;
    this.sizeOf = sizeOf;
    this.budget = budget;
  }

  /** Estimated bytes held by every parked entry right now. */
  get bytes(): number {
    let total = 0;
    for (const slot of this.slots.values()) total += this.sizeOf(slot.value);
    return total;
  }

  get size(): number {
    return this.slots.size;
  }

  has(key: string): boolean {
    return this.slots.has(key);
  }

  /** Park `value` under `key`. An entry already parked under the key is
   *  disposed (one parked engine per session); past the cap or the byte
   *  budget the oldest go. The entry just parked always stays. */
  put(key: string, value: T): void {
    this.evict(key);
    const timer = this.setT(() => this.evict(key), this.ttl);
    this.slots.set(key, { value, timer });
    while (this.slots.size > this.cap) {
      const oldest = this.slots.keys().next().value as string;
      this.evict(oldest);
    }
    let total = this.bytes;
    while (total > this.budget && this.slots.size > 1) {
      const oldest = this.slots.keys().next().value as string;
      total -= this.sizeOf(this.slots.get(oldest)!.value);
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
