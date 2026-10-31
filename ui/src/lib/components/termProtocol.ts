// The `/ws/term` frame decoder and the snapshot rebuild rule, shared by the
// live Terminal and its parked engine (architecture-2-01). Both used to decode
// the protocol by hand — the parked copy with an inline anonymous frame type,
// the live one with an untyped JSON.parse — and each carried its own copy of
// the binary-snapshot handshake (a header frame, then the NEXT binary frame
// is its payload) and of the snapshot apply. A frame change now lands here,
// once, and is unit-testable without xterm (unit/termProtocol.test.ts).
import type {
  SessionStatus,
  WsSearchResultFrame,
  WsTermGridFrame,
  WsTermProbeAckFrame,
} from '../api/types';
import { base64ToBytes } from '../b64';
import { withInOrderReset } from './termFlow';

/** One decoded `/ws/term` frame. Binary frames are live PTY output or a
 *  snapshot payload; the JSON control frames keep the contract's own types
 *  (`lib/api/types`) where the contract defines one. */
export type TermFrame =
  /** Live PTY bytes (credited, painted as output). */
  | { type: 'output'; bytes: Uint8Array }
  /** A full rebuild: base64 `scrollback`, or a binary header's payload. A
   *  null/empty `bytes` still moves the snapshot epoch. */
  | { type: 'snapshot'; epoch: number | null; bytes: Uint8Array | null }
  /** Flow-control window grant (`window` undefined = the default). */
  | { type: 'credit'; window: number | undefined }
  | { type: 'status'; status: SessionStatus | undefined }
  | { type: 'exit'; code: number }
  | { type: 'error'; code: unknown; message: string }
  /** Room relay only: this terminal stream is over for good (the shared
   *  process exited or was replaced, or this member lost access). */
  | { type: 'ended'; reason: string }
  | WsTermGridFrame
  | WsTermProbeAckFrame
  | WsSearchResultFrame;

/** Binary snapshot headers (perf 01 N3) waiting for their payload — the NEXT
 *  binary frame on that socket. Keyed by socket, not by component: a
 *  park/adopt can swap the handler between the header and its payload. */
const binarySnapHeaders = new WeakMap<object, { epoch?: number }>();

/** Decode one message from `sock`. Null for a frame with nothing to apply: a
 *  binary-snapshot header (its payload is the next binary frame, recorded
 *  here), a malformed or unknown control frame, a non-string text payload. */
export function decodeTermFrame(ev: { data: unknown }, sock: object): TermFrame | null {
  if (ev.data instanceof ArrayBuffer) {
    const bytes = new Uint8Array(ev.data);
    // A binary snapshot's payload (N3): not live output, never credited.
    const hdr = binarySnapHeaders.get(sock);
    if (hdr) {
      binarySnapHeaders.delete(sock);
      return { type: 'snapshot', epoch: hdr.epoch ?? null, bytes };
    }
    return { type: 'output', bytes };
  }
  if (typeof ev.data !== 'string') return null;
  let msg: Record<string, unknown> & { type?: unknown };
  try {
    msg = JSON.parse(ev.data);
  } catch {
    return null; // malformed control frame
  }
  if (!msg || typeof msg !== 'object') return null;
  switch (msg.type) {
    case 'credit':
      return { type: 'credit', window: typeof msg.window === 'number' ? msg.window : undefined };
    case 'scrollback': {
      const epoch = msg.epoch as number | undefined;
      if (msg.binary) {
        // Header only; the bytes are the next (binary) frame.
        binarySnapHeaders.set(sock, { epoch });
        return null;
      }
      return { type: 'snapshot', epoch: epoch ?? null, bytes: msg.data ? base64ToBytes(msg.data as string) : null };
    }
    case 'status':
      return { type: 'status', status: msg.status as SessionStatus | undefined };
    case 'exit':
      return { type: 'exit', code: (msg.code as number | undefined) ?? 0 };
    case 'error':
      return { type: 'error', code: msg.code, message: (msg.message as string | undefined) ?? '' };
    case 'ended':
      return { type: 'ended', reason: typeof msg.reason === 'string' ? msg.reason : 'Terminal sharing ended' };
    case 'grid':
      return msg as unknown as WsTermGridFrame;
    case 'probe_ack':
      return msg as unknown as WsTermProbeAckFrame;
    case 'search_result':
      return msg as unknown as WsSearchResultFrame;
    default:
      return null;
  }
}

/** What a snapshot rebuilds: the live Terminal or a parked engine. */
export interface SnapshotTarget {
  /** Engine bookkeeping that runs for EVERY snapshot, empty or not (the
   *  epoch moves, a pending compact is answered). */
  begin(epoch: number | null): void;
  /** The engine's write queue (termFlow.WriteQueue). */
  writes: { dropQueued(): void; readonly inflight: number };
  /** The rebuild answers any pending lag resync. */
  clearResync(): void;
  /** Synchronous terminal reset (nothing still queued inside xterm). */
  reset(): void;
  /** Feed rebuild bytes to the terminal. */
  paint(bytes: Uint8Array): void;
}

/** The ONE snapshot rule. A snapshot fully reconstructs terminal state
 *  (history + the current screen + input modes), so it always resets and
 *  rebuilds — appending under kept scrollback stacked duplicate transcripts.
 *  Everything received before it is already in it: the queued part never
 *  needs to parse. With output still queued inside xterm the reset goes
 *  in-order (RIS) so the stale backlog can't parse above the rebuild. */
export function applySnapshot(target: SnapshotTarget, epoch: number | null, bytes: Uint8Array | null): void {
  target.begin(epoch);
  if (!bytes?.length) return;
  target.writes.dropQueued();
  target.clearResync();
  if (target.writes.inflight === 0) {
    target.reset();
    target.paint(bytes);
  } else {
    target.paint(withInOrderReset(bytes));
  }
}
