// Where `open {placement: "window"}` puts a module — pure, so the rules are
// unit-tested (unit/uiCommandsWindowPlacement.test.ts); nav.ts acts on it.

export type WindowOpenPlan = 'main' | 'detach' | 'popout' | 'side' | 'refuse';

export function windowOpenPlan(p: {
  /** The desktop app (native windows exist). */
  tauri: boolean;
  /** This document is a side pane. */
  embedded: boolean;
  /** The side pane is a native view that can detach into a window. */
  detachable: boolean;
  /** This window's side pane already shows the module. */
  sideShows: boolean;
  /** This window's main pane already shows the module. */
  mainShows: boolean;
  /** No native windows → the side pane instead of an error. */
  fallback: boolean;
}): WindowOpenPlan {
  if (p.mainShows) return 'main';
  // The pane already showing it moves out with its document.
  if (p.embedded || p.sideShows) return p.detachable ? 'detach' : p.fallback ? 'side' : 'refuse';
  // A new window of its own: the side pane keeps what it shows.
  if (p.tauri) return 'popout';
  return p.fallback ? 'side' : 'refuse';
}
