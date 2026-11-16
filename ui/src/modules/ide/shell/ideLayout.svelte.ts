// IDE layout mode, decided by the IDE page's OWN measured size — not by the
// viewport bucket. A phone on its side is ~900 px wide but under 400 px tall;
// classing it by width gave it the desktop workbench (explorer rail + editor
// + bottom dock + agent dock) in a strip a few lines high, and rotating the
// phone swapped between two different component trees.
//
//   stack     — one pane at a time (Files · Editor · Tools · Agent): any page
//               narrower than STACK_MAX_W or shorter than STACK_MAX_H.
//   workbench — the multi-pane desktop layout.
//
// The stack pane is this layout's own state (kept per tab in sessionStorage);
// it never writes the workbench's persisted preferences (dock open, explorer
// visible, pane widths), so a rotation can't leave a desktop dock open over
// the editor.

export type IdeStackPane = 'files' | 'editor' | 'tools' | 'agent';

/** Below this page width the IDE stacks its panes. */
export const STACK_MAX_W = 700;
/** Below this page height the IDE stacks its panes (phone landscape). */
export const STACK_MAX_H = 520;

const PANE_KEY = 'otto.ide.stackPane';
const PANES: readonly IdeStackPane[] = ['files', 'editor', 'tools', 'agent'];

/** Pure: does a page of this size stack? An unmeasured page (0×0) keeps
 *  the workbench so a desktop first paint never flashes the phone layout. */
export function isStackLayout(width: number, height: number): boolean {
  if (width <= 0 || height <= 0) return false;
  return width < STACK_MAX_W || height < STACK_MAX_H;
}

/** Pure: the pane to show after a file was opened from the given pane — a
 *  tap on a file in Files goes to the editor; every other pane stays put. */
export function paneAfterFileOpen(pane: IdeStackPane): IdeStackPane {
  return pane === 'files' ? 'editor' : pane;
}

/** Pure: the tallest the workbench's bottom dock may be. Measured against
 *  the column it shares with the editor (keep ~140 px of editor), falling
 *  back to the window before the column has been measured. */
export function dockMaxFor(columnH: number, windowH: number, min: number): number {
  if (columnH > 0) return Math.max(min, Math.round(columnH - 140));
  return Math.max(520, Math.round(windowH * 0.85));
}

/** Pure: the stacked Tools pane lists the terminal first — on a phone it is
 *  the tool you open most, and the last tab sat off-screen past the strip. */
export function stackDockOrder<T extends { id: string }>(tabs: readonly T[]): T[] {
  const term = tabs.filter((t) => t.id === 'terminal');
  return [...term, ...tabs.filter((t) => t.id !== 'terminal')];
}

function readPane(): IdeStackPane {
  try {
    const v = sessionStorage.getItem(PANE_KEY);
    return PANES.includes(v as IdeStackPane) ? (v as IdeStackPane) : 'editor';
  } catch {
    return 'editor';
  }
}

class IdeLayoutStore {
  /** The IDE page's measured box (bound by IdePage). */
  width = $state(0);
  height = $state(0);
  #pane = $state<IdeStackPane>(readPane());

  get stack(): boolean {
    return isStackLayout(this.width, this.height);
  }

  get pane(): IdeStackPane {
    return this.#pane;
  }

  set pane(next: IdeStackPane) {
    if (this.#pane === next) return;
    this.#pane = next;
    try {
      sessionStorage.setItem(PANE_KEY, next);
    } catch {
      /* private mode: the pane just isn't remembered across reloads */
    }
  }
}

export const ideLayout = new IdeLayoutStore();
