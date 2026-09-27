// Terminal flow control + repaint heuristics, kept out of Terminal.svelte so
// they are unit-testable (unit/termFlow.test.ts). See docs/contracts/ws.md §1
// "Flow control" and the SA-02 / SA-03 notes in Terminal.svelte.
import type { WsTermFlowFrame } from '../api/types';

/** xterm scrollback depth (lines) for a PRIMARY terminal — the one pane the
 *  user works in (SessionView: agents main/split panes, the maximized tile,
 *  swarm/loop session panes; the share page; the DB SSH shell). Each line
 *  costs ~12 B/cell, so 10k × 200 cols ≈ 24 MB of JS heap. */
export const PRIMARY_SCROLLBACK = 10_000;
/** Default depth for every other Terminal: grid tiles and the embedded
 *  previews that mount several terminals at once (review/docs/analysis/run
 *  agents, assistant panels, docks, exec views). 10k there was 150–360 MB
 *  across a busy grid (SA-05); the daemon keeps 4000 rows, so maximizing or
 *  reconnecting in a primary pane still restores depth. */
export const EMBED_SCROLLBACK = 2_000;

/** Pending (handed to xterm, not yet parsed) bytes above which the client
 *  asks the server to `pause` this stream. xterm parses 5.5–8 MB/s in WebKit,
 *  so this bounds the on-screen lag after ^C to a few hundred ms. */
export const FLOW_HIGH = 2 * 1024 * 1024;
/** Pending bytes below which a paused stream is resumed. */
export const FLOW_LOW = 256 * 1024;
/** Re-send `pause` this often while still paused and above LOW: the server
 *  auto-resumes a paused stream 2 s after the last `pause` (so a lost
 *  `resume` can never freeze a pane); a slow drain keeps it paused. */
export const FLOW_PAUSE_KEEPALIVE_MS = 1000;

/**
 * xterm's documented watermark pattern: `add(n)` when bytes are handed to
 * term.write(), `done(n)` from its parse callback. Sends `pause` above HIGH
 * and `resume` below LOW through `send`. `pending` is the local xterm backlog
 * itself, so it deliberately survives reconnects; `paused` is per socket
 * (`resetStream()` on every new connection — a fresh stream starts unpaused).
 */
export class TermFlow {
  pending = 0;
  paused = false;
  private lastPauseAt = 0;
  private readonly send: (frame: WsTermFlowFrame) => void;
  private readonly now: () => number;

  constructor(send: (frame: WsTermFlowFrame) => void, now: () => number = () => performance.now()) {
    this.send = send;
    this.now = now;
  }

  /** Count bytes handed to the emulator. `canSend` = the socket is open. */
  add(n: number, canSend: boolean): void {
    this.pending += n;
    if (this.pending > FLOW_HIGH && !this.paused && canSend) {
      this.paused = true;
      this.lastPauseAt = this.now();
      this.send({ type: 'pause' });
    }
  }

  /** Un-count bytes the emulator finished parsing (or dropped). */
  done(n: number): void {
    this.pending = Math.max(0, this.pending - n);
    if (!this.paused) return;
    if (this.pending < FLOW_LOW) {
      this.paused = false;
      this.send({ type: 'resume' });
    } else if (this.now() - this.lastPauseAt > FLOW_PAUSE_KEEPALIVE_MS) {
      this.lastPauseAt = this.now();
      this.send({ type: 'pause' });
    }
  }

  /** A new socket: its server stream starts unpaused. */
  resetStream(): void {
    this.paused = false;
  }
}

/** Largest slice handed to xterm in one `write()` (A3). */
export const WRITE_SLICE = 64 * 1024;
/** Bytes allowed INSIDE xterm (handed over, not yet parsed). The rest waits in
 *  [`WriteQueue`], where it can still be dropped: xterm has no API to cancel
 *  queued writes, so everything handed to it will parse — 2 MB of it after ^C
 *  was ~0.35 s of scrolling. Two slices keep its parser busy (one parses while
 *  the next waits), so throughput is unchanged. */
export const WRITE_INFLIGHT = 2 * WRITE_SLICE;
/** Queued (not yet handed to xterm) bytes above which a keystroke drops the
 *  queue and asks the server for a `resync` snapshot instead of making the
 *  user watch it scroll by. Below this the backlog drains in ≲ 40 ms anyway. */
export const RESYNC_ON_INPUT = 256 * 1024;

interface Pending {
  bytes: Uint8Array;
  /** Runs once this frame's LAST byte has been parsed. */
  onParsed?: () => void;
}

/**
 * A byte queue in front of xterm (A3). Frames are fed to `write` in slices of
 * at most WRITE_SLICE while at most WRITE_INFLIGHT bytes are inside the
 * emulator; each parse callback refills. Every byte is counted in `flow`
 * (TermFlow watermarks) from arrival until parsed or dropped, so pause/resume
 * see the whole backlog. `dropQueued()` discards what xterm has not been
 * handed yet — safe only when a snapshot that supersedes it follows (a
 * `resync` reply, or a snapshot frame already received).
 */
export class WriteQueue {
  private q: Pending[] = [];
  /** Bytes waiting here (droppable). */
  queued = 0;
  /** Bytes handed to xterm and not yet parsed. */
  inflight = 0;
  private readonly write: (bytes: Uint8Array, done: () => void) => void;
  private readonly flow: TermFlow;
  private readonly canSend: () => boolean;

  constructor(
    write: (bytes: Uint8Array, done: () => void) => void,
    flow: TermFlow,
    canSend: () => boolean = () => true,
  ) {
    this.write = write;
    this.flow = flow;
    this.canSend = canSend;
  }

  /** Whole local backlog (queued + inside xterm). */
  get backlog(): number {
    return this.queued + this.inflight;
  }

  /** Enqueue one received frame. `onParsed` fires after its last byte parses. */
  push(bytes: Uint8Array, onParsed?: () => void): void {
    const n = bytes.byteLength;
    if (n === 0) {
      onParsed?.();
      return;
    }
    this.flow.add(n, this.canSend());
    this.q.push({ bytes, onParsed });
    this.queued += n;
    this.pump();
  }

  /** Hand slices to xterm while there is room inside it. */
  private pump(): void {
    while (this.q.length && this.inflight < WRITE_INFLIGHT) {
      const head = this.q[0];
      let slice: Uint8Array;
      let hook: (() => void) | undefined;
      if (head.bytes.byteLength <= WRITE_SLICE) {
        this.q.shift();
        slice = head.bytes;
        hook = head.onParsed;
      } else {
        slice = head.bytes.subarray(0, WRITE_SLICE);
        head.bytes = head.bytes.subarray(WRITE_SLICE);
      }
      const n = slice.byteLength;
      this.queued -= n;
      this.inflight += n;
      let settled = false;
      const done = (): void => {
        if (settled) return;
        settled = true;
        this.inflight = Math.max(0, this.inflight - n);
        this.flow.done(n);
        hook?.();
        this.pump();
      };
      try {
        this.write(slice, done);
      } catch {
        // xterm throws (dropping the data) past its discard watermark; the
        // callback never fires — un-count it or the gate would stay shut.
        done();
      }
    }
  }

  /**
   * User input over a big queue (A3): when more than RESYNC_ON_INPUT bytes
   * wait here, call `requestResync` (send the `resync` frame) and THEN drop
   * the queue — dropping can un-pause the stream (`resume`), and the server
   * must see `resync` (which opens its gate itself) first, or a resume
   * snapshot and the resync snapshot would both rebuild. `true` = resynced.
   */
  resyncOnInput(requestResync: () => void): boolean {
    if (this.queued <= RESYNC_ON_INPUT) return false;
    requestResync();
    this.dropQueued();
    return true;
  }

  /** Discard everything not yet handed to xterm. Returns the bytes dropped. */
  dropQueued(): number {
    const n = this.queued;
    if (n === 0) return 0;
    this.q = [];
    this.queued = 0;
    this.flow.done(n);
    return n;
  }
}

/**
 * True when a frame repositions the cursor or erases — the redraws (↑
 * history, prompt rewrites, progress bars) that leave WebGL partial-update
 * ghosts. Plain appended lines (typing echo, `tail -f`, build logs) paint
 * correctly through xterm's dirty-row tracking and need no full repaint.
 * Scans for ESC M / ESC 8 / ESC c and CSI sequences (ESC [ params /
 * intermediates, then a cursor-move, erase, insert/delete or scroll final).
 */
export function hasCursorOrErase(bytes: Uint8Array): boolean {
  const n = bytes.byteLength;
  for (let i = 0; i < n - 1; i++) {
    if (bytes[i] !== 0x1b) continue;
    const next = bytes[i + 1];
    // ESC M (reverse index), ESC 8 (restore cursor), ESC c (reset).
    if (next === 0x4d || next === 0x38 || next === 0x63) return true;
    if (next !== 0x5b) continue;
    let j = i + 2;
    while (j < n && bytes[j] >= 0x20 && bytes[j] <= 0x3f) j++;
    if (j >= n) return false;
    switch (bytes[j]) {
      case 0x41: case 0x42: case 0x43: case 0x44: // A B C D — cursor moves
      case 0x45: case 0x46: case 0x47: // E F G — next/prev line, column
      case 0x48: case 0x66: case 0x64: // H f d — absolute position
      case 0x4a: case 0x4b: // J K — erase display / line
      case 0x4c: case 0x4d: case 0x50: case 0x58: // L M P X — insert/delete/erase
      case 0x53: case 0x54: // S T — scroll
        return true;
    }
    i = j;
  }
  return false;
}

/** Prefix a snapshot with RIS (ESC c) so the emulator resets IN ORDER, after
 *  output still queued inside it, instead of a synchronous term.reset() that
 *  lets the stale backlog parse on top of the rebuild. */
export function withInOrderReset(snapshot: Uint8Array): Uint8Array {
  const framed = new Uint8Array(snapshot.byteLength + 2);
  framed[0] = 0x1b;
  framed[1] = 0x63; // 'c'
  framed.set(snapshot, 2);
  return framed;
}
