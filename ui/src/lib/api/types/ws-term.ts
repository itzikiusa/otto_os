// Contract mirror: ws-term. Part of lib/api/types (see ../types.ts, the barrel).
// Mirrors docs/contracts/api.md + ws.md — change them together.

// ---------------------------------------------------------------------------
// WS /ws/term/{id} — frame types (docs/contracts/ws.md)
// ---------------------------------------------------------------------------

/** One match returned by a server-side ring-buffer search. */
export interface TermSearchMatch {
  /** Absolute ring-buffer line index (oldest = 0, newest = ringLen-1). */
  line: number;
  /** ANSI-stripped plain text of the matching line. */
  text: string;
}

/** `{"type":"search_result"}` server→client frame (response to a Search frame). */
export interface WsSearchResultFrame {
  type: 'search_result';
  /** The query string that was searched. */
  query: string;
  /** Up to 200 matching lines, oldest → newest. */
  matches: TermSearchMatch[];
}

/** Client→server pause-mode flow-control frames (docs/contracts/ws.md §1
 *  "Flow control"): `pause` above the client's pending-bytes high watermark,
 *  `resume` below the low one. On resume the server sends one `scrollback`
 *  snapshot if it held output back; a paused stream auto-resumes 2 s after the
 *  last `pause`. The fallback when the daemon does not grant `credit`. */
export interface WsTermPauseFrame {
  type: 'pause' | 'resume';
}

/** `/ws/term` credit flow control (docs/contracts/ws.md §1 "Credit"). Client →
 *  server first thing on a socket: offer a `window` (bytes). Server → client:
 *  the grant (window clamped to 64 KB–8 MB); binary frames after it count
 *  against the window. A daemon that never answers stays in pause mode. */
export interface WsTermCreditFrame {
  type: 'credit';
  window: number;
  /** Client → server only (perf 01 N3): send snapshots in the binary form
   *  (`WsTermBinarySnapshotHeader` + one binary frame). Absent = base64 JSON. */
  binary_snapshots?: boolean;
}

/** Server → client `/ws/term` (perf 01 N3), only to a client that offered
 *  `binary_snapshots`: a `scrollback` snapshot whose bytes follow as the very
 *  NEXT frame — one binary frame of `len` bytes. That frame is the snapshot,
 *  not live output: it is never counted against the credit window. An empty
 *  snapshot still uses the JSON form (`data: ""`). */
export interface WsTermBinarySnapshotHeader {
  type: 'scrollback';
  epoch: number;
  binary: true;
  len: number;
}

/** Client → server: cumulative credited binary bytes consumed (parsed or
 *  dropped) since the `credit` grant; sent every ~64 KB. */
export interface WsTermAckFrame {
  type: 'ack';
  bytes: number;
}

/** Every flow-control frame a terminal client sends (termFlow.ts). */
export type WsTermFlowFrame = WsTermPauseFrame | WsTermCreditFrame | WsTermAckFrame;

/** Client → server `/ws/term` frame (docs/contracts/ws.md §1): the user typed
 *  while more than 256 KB was queued in front of the emulator; the client
 *  dropped that queue and asks for ONE snapshot of up to `lines` rows. The
 *  server discards this viewer's queued output (already in the snapshot),
 *  leaves any flow-control pause and always answers with a `scrollback`. */
export interface WsTermResyncFrame {
  type: 'resync';
  lines: number;
  /** The client's grid (perf F1): with both set, a viewer that may resize
   *  has the PTY + emulator resized to it BEFORE the snapshot is captured. */
  cols?: number;
  rows?: number;
}

/** Client → server `/ws/term` snapshot request (docs/contracts/ws.md §1):
 *  up to `lines` rows of history + the current screen, answered with one
 *  `scrollback`. Its `lines` is also the depth of every later server-pushed
 *  snapshot on this socket. Sent on open WITH the measured grid (`cols` /
 *  `rows`, perf F1) so a viewer that may resize gets the snapshot already at
 *  its own grid — one snapshot per attach, no follow-up compact. */
export interface WsTermScrollbackRequestFrame {
  type: 'scrollback';
  lines: number;
  cols?: number;
  rows?: number;
}

/** Client → server `/ws/term` size-authority claim (docs/contracts/ws.md §1
 *  "Size authority"): sent on focus, when the window comes back and on
 *  Redraw. With `cols` + `rows` the server also resizes the PTY to them. */
export interface WsTermClaimFrame {
  type: 'claim';
  cols?: number;
  rows?: number;
}

/** Server → client `/ws/term` grid feed ("Grid feed"): the PTY's real grid
 *  (0×0 while no process is live) and who sizes it — this socket (`self`),
 *  another attached one (`other`) or nobody attached (`none`). Sent after
 *  the attach `status` and on every change. */
export interface WsTermGridFrame {
  type: 'grid';
  cols: number;
  rows: number;
  owner: 'self' | 'other' | 'none';
}

/** Client → server `/ws/term` latency probe (docs/contracts/ws.md §1
 *  "Latency probe"): answered at once with a `probe_ack` echoing `id`. */
export interface WsTermProbeFrame {
  type: 'probe';
  id: number;
}

/** PTY keystroke-echo clock: input reaching the PTY → the child's first
 *  output after it, since spawn (`avg_ms` is an EWMA, α = 1/8). */
export interface TermEchoStats {
  last_ms: number;
  avg_ms: number;
  max_ms: number;
  samples: number;
}

/** Server → client reply to `probe`; `echo` is null with no live PTY. */
export interface WsTermProbeAckFrame {
  type: 'probe_ack';
  id: number;
  echo: TermEchoStats | null;
}
