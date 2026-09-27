// Terminal flow control + repaint heuristics, kept out of Terminal.svelte so
// they are unit-testable (unit/termFlow.test.ts). See docs/contracts/ws.md §1
// "Flow control" and the SA-02 / SA-03 notes in Terminal.svelte.
import type { WsTermFlowFrame } from '../api/types';

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
