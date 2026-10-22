// Grid feed + size-authority (claim) negotiation for one terminal viewer
// (docs/contracts/ws.md "Grid feed"), moved out of Terminal.svelte the way
// termFlow / termPark were (architecture-2-01). The decisions themselves are
// the pure `gridStep` (termGrid.ts); this controller owns the per-viewer state
// they read and acts through a small host — the component keeps the socket,
// the xterm and the overlay state.
//
// The daemon tells every viewer the PTY's REAL grid and who sizes it. Before
// it, `lastCols`/`lastRows` (what THIS socket last sent) were all a pane knew,
// so a remote browser that typed (and re-sized the PTY) left the local pane
// painting another grid's output: TUI frames on the wrong cells, and with
// fewer PTY rows a bottom strip the TUI never repaints. Only a session
// restart cleared it.

import type { WsTermClaimFrame, WsTermGridFrame } from '../api/types';
import { gridStep, type GridOwner } from './termGrid';

export interface Grid {
  cols: number;
  rows: number;
}

/** What the controller needs from the terminal component. */
export interface GridSyncHost {
  /** The live xterm grid, or null before mount / after dispose. */
  term(): Grid | null;
  readOnly(): boolean;
  /** The session's primary pane (claims on attach). */
  primary(): boolean;
  connected(): boolean;
  send(frame: WsTermClaimFrame): void;
  /** Record the grid this socket last sent to the PTY (`lastCols/lastRows`). */
  sent(g: Grid): void;
  /** Rebuild the screen from ONE server snapshot. */
  rebuild(): void;
  /** Show / clear "sized for another window" (the component's `$state`). */
  setForeign(g: Grid | null): void;
  foreign(): Grid | null;
}

/** Agent TUI panes measure while their box settles and resize xterm only
 *  once the grid is stability-confirmed (Terminal's `confirmResizeStep`): the
 *  passing sizes of a tab switch, a split animation or a window restore never
 *  touch the buffer. Shells (their scrollback reflows natively) and phones
 *  (the soft keyboard must not wait) resize immediately. */
export function shouldDeferLocalResize(s: { preferDom: boolean; connected: boolean; phone: boolean }): boolean {
  return s.preferDom && s.connected && !s.phone;
}

/** What one `sendResize` call does (architecture-2-01, the resize half):
 *  - `confirm-now`: a forced sync (connect, focus re-claim) skips the settle
 *    wait but still runs the stability confirmation at once — an attach can
 *    race a window-restore animation just like a drag can;
 *  - `skip`: no confirmation pending, the grid is what this socket last sent
 *    and xterm did not reflow on its own — nothing to tell the PTY;
 *  - `schedule`: (re)start the settle timer, then confirm. */
export type ResizeSendStep = 'confirm-now' | 'skip' | 'schedule';

export function resizeSendStep(s: {
  force: boolean;
  /** A settle / confirmation timer is already running. */
  timerPending: boolean;
  /** The grid to compare: the measured one for a deferring TUI pane, else xterm's. */
  grid: Grid;
  /** What this socket last sent (`lastCols` / `lastRows`). */
  last: Grid;
  localReflowed: boolean;
}): ResizeSendStep {
  if (s.force) return 'confirm-now';
  if (!s.timerPending && s.grid.cols === s.last.cols && s.grid.rows === s.last.rows && !s.localReflowed) return 'skip';
  return 'schedule';
}

export class TermGridSync {
  /** The PTY's real grid from the latest `grid` frame (null: none yet). */
  pty: Grid | null = null;
  owner: GridOwner = 'none';
  /** Output arrived laid out for another viewer's grid: rebuild from ONE
   *  snapshot as soon as the PTY is at our grid and ours again. */
  owed = false;
  /** A snapshot this socket asked for is still on its way: it IS the
   *  rebuild (the attach snapshot is taken after the attach resize). */
  snapshotAsked = false;

  constructor(private readonly host: GridSyncHost) {}

  reset(): void {
    this.pty = null;
    this.owner = 'none';
    this.owed = false;
    this.host.setForeign(null);
    this.snapshotAsked = false;
  }

  /** The PTY grid is known (a live process) and differs from this xterm's. */
  differs(): boolean {
    const t = this.host.term();
    return !!t && !!this.pty && this.pty.cols > 0 && (this.pty.cols !== t.cols || this.pty.rows !== t.rows);
  }

  /** Claim size authority AND resize to this pane's grid in one step. */
  claim(): void {
    const t = this.host.term();
    if (!t || this.host.readOnly()) return;
    this.host.send({ type: 'claim', cols: t.cols, rows: t.rows } satisfies WsTermClaimFrame);
    this.host.sent({ cols: t.cols, rows: t.rows });
  }

  /** Re-evaluate after a `grid` frame or a local grid change (termGrid.ts:
   *  owe / do a rebuild, offer the take-over, auto-claim). `autoClaim` only
   *  on a `grid` frame — a local refit never claims on its own. */
  settle(autoClaim = false): void {
    const step = gridStep({
      differs: this.differs(),
      owner: this.owner,
      owed: this.owed,
      snapshotAsked: this.snapshotAsked,
      primary: this.host.primary(),
      readOnly: this.host.readOnly(),
      visible: document.visibilityState === 'visible',
    });
    this.owed = step.owed;
    if (step.rebuild) this.host.rebuild();
    const next = step.foreign && this.pty ? this.pty : null;
    const cur = this.host.foreign();
    if (next?.cols !== cur?.cols || next?.rows !== cur?.rows) this.host.setForeign(next ? { ...next } : null);
    // Not while our own snapshot request is on its way: the attach already
    // claims (primary pane) and resizes with that request.
    if (autoClaim && step.claim && !this.snapshotAsked) this.claim();
  }

  onFrame(f: WsTermGridFrame): void {
    this.pty = { cols: f.cols, rows: f.rows };
    this.owner = f.owner;
    this.settle(true);
  }

  /** The window came back (focus / visible): the primary pane re-takes a
   *  grid another viewer changed meanwhile — the "looked at it from another
   *  computer, came back, terminal broken" case. An arrow property, so it is
   *  passed to add/removeEventListener as is. */
  onWindowBack = (): void => {
    if (document.visibilityState !== 'visible' || !this.host.primary() || this.host.readOnly() || !this.host.connected()) return;
    if (this.differs()) this.claim();
  };
}
