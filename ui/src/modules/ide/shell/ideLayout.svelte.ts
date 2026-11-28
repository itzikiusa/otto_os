// IDE layout mode. The ONE phone signal decides it (design.md §2,
// viewport.navDrawer: a phone, or a phone on its side):
//
//   stack     — the phone: the explorer is the page (ListDetail pushes), a
//               file is pushed over it with "‹ Files"; the content is the
//               editor or, picked from ⋯ › a tool window, the Tools (the
//               bottom dock filling the content). The agent and the build
//               tools are Detail panes (full-width sheets).
//   workbench — the multi-pane layout (desktop, tablet, a narrow window).
//
// The stack pane is this layout's own state (kept per tab in sessionStorage);
// it never writes the workbench's persisted preferences (dock open, explorer
// hidden, pane widths). The page's measured size still drives the phone
// chrome inside the content (a short page hides the sub-bar).

import { viewport } from '../../../lib/stores/viewport.svelte';

export type IdeStackPane = 'files' | 'editor' | 'tools';
/** The explorer's views (its list pane header): the phone Back names the active one. */
export type IdeExplorerView = 'files' | 'structure' | 'breakpoints';
export const EXPLORER_VIEW_LABEL: Record<IdeExplorerView, string> = { files: 'Files', structure: 'Structure', breakpoints: 'Breakpoints' };

/** The phone chrome's "wide" page (a phone on its side): the file row fits the header. */
export const STACK_MAX_W = 700;
/** Below this page height the phone chrome is short (phone landscape). */
export const STACK_MAX_H = 520;
/** What the editor keeps beside the agent / build tool Detail pane before the
 *  pane becomes a right sheet over it (the tablet rule, at the IDE's width):
 *  the explorer is never hidden for it. */
export const IDE_EDITOR_MIN = 640;

const PANE_KEY = 'otto.ide.stackPane';
const PANES: readonly IdeStackPane[] = ['files', 'editor', 'tools'];

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
  /** The explorer's active view (Files | Structure | Breakpoints). */
  explorerView = $state<IdeExplorerView>('files');

  get stack(): boolean {
    return viewport.navDrawer;
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
