// Grid feed decisions for one terminal viewer (docs/contracts/ws.md "Grid
// feed"). The daemon tells every viewer the PTY's REAL grid and who sizes it;
// this decides what the viewer does about it. Pure — unit-tested
// (unit/termGrid.test.ts); Terminal.svelte owns the state and the sockets.

export type GridOwner = 'self' | 'other' | 'none';

export interface GridView {
  /** The PTY grid is known (a live process) and differs from this xterm's. */
  differs: boolean;
  owner: GridOwner;
  /** This buffer already holds output laid out for another viewer's grid. */
  owed: boolean;
  /** A snapshot this socket asked for is on its way (it is the rebuild). */
  snapshotAsked: boolean;
  /** The session's primary pane (claims on attach) — tiles and previews are
   *  supposed to show the pane's grid and never act on it. */
  primary: boolean;
  readOnly: boolean;
  /** The page is visible (an auto-claim from a hidden tab would steal the
   *  grid from the window the user is actually looking at). */
  visible: boolean;
}

export interface GridStep {
  /** Whether a rebuild is still owed after this step. */
  owed: boolean;
  /** Ask the daemon for ONE snapshot now. */
  rebuild: boolean;
  /** Claim the size (with this pane's grid) now. */
  claim: boolean;
  /** Show "sized for another window" with a take-over. */
  foreign: boolean;
}

export function gridStep(v: GridView): GridStep {
  // Only output PAINTED here at a foreign grid is owed a rebuild: while our own
  // snapshot is on its way (a fresh attach) nothing is painted yet, and that
  // snapshot rebuilds the screen anyway. Owing one then made a stray rebuild
  // land seconds later — and a rebuild resets the screen (selection gone).
  let owed = v.owed || (v.differs && v.owner !== 'self' && !v.snapshotAsked);
  let rebuild = false;
  if (!v.differs && v.owner === 'self' && owed && !v.snapshotAsked) {
    owed = false;
    rebuild = true;
  }
  const acts = v.primary && !v.readOnly;
  return {
    owed,
    rebuild,
    // The viewer that sized the PTY left: authority is sticky server-side,
    // so only a claim moves it — the visible primary pane takes it back.
    claim: acts && v.visible && v.differs && v.owner === 'none',
    foreign: acts && v.differs && v.owner === 'other',
  };
}
