// I1-15: the explorer steps aside when a side tool window (AI agent, Maven/Gradle) opens and
// the editor column would drop under IDE_EDITOR_MIN px — and comes back when that window
// closes, unless the person toggled the explorer in between (then their choice stands).
// Only the OPENING of a side window hides it: re-showing the explorer with the shortcut while
// the window is open is respected, also across window resizes.

import { toasts } from '../../lib/toast.svelte';
import { ideKeymapPresetOf, ideShortcutLabel } from './shell/ideShortcuts';

export const IDE_EDITOR_MIN = 720;

export interface IdeExplorerRoomFacts {
  /** Width of the IDE page's workbench row. */
  width: number;
  treeW: number;
  /** Summed width of the side tool windows that are open (0 when none). */
  sideW: number;
  sideOpen: boolean;
}

/** Would the editor column be narrower than IDE_EDITOR_MIN with the explorer AND the side
 *  window(s) beside it? */
export function ideEditorCramped(f: IdeExplorerRoomFacts): boolean {
  return f.sideOpen && f.width > 0 && f.width - f.treeW - f.sideW < IDE_EDITOR_MIN;
}

export class IdeExplorerRoom {
  private wasOpen = false;
  /** The explorer was hidden by us and the person has not touched it since. */
  private autoHid = false;

  /** Feed the current facts and the explorer's visibility; returns the visibility to apply
   *  (`hid` = it was just hidden to make room, the caller may say so). */
  step(visible: boolean, f: IdeExplorerRoomFacts): { visible: boolean; hid: boolean } {
    const opened = f.sideOpen && !this.wasOpen;
    const closed = !f.sideOpen && this.wasOpen;
    this.wasOpen = f.sideOpen;
    if (this.autoHid && visible) this.autoHid = false; // shown again by hand: theirs now
    if (opened && visible && ideEditorCramped(f)) {
      this.autoHid = true;
      return { visible: false, hid: true };
    }
    if (closed && this.autoHid) {
      this.autoHid = false;
      return { visible: true, hid: false };
    }
    return { visible, hid: false };
  }
}

const HINT_KEY = 'otto_ide_explorer_yield_hint';
let hintedThisSession = false;

/** Say once (ever, per browser) why the explorer went away and how to bring it back. */
export function hintExplorerYield(preset: unknown): void {
  if (hintedThisSession) return;
  hintedThisSession = true;
  try {
    if (localStorage.getItem(HINT_KEY)) return;
    localStorage.setItem(HINT_KEY, '1');
  } catch {
    /* private window: once this session is enough */
  }
  const key = ideShortcutLabel('toggleExplorer', ideKeymapPresetOf(preset));
  toasts.info('Explorer hidden to make room', key ? `Press ${key} to show it again.` : 'Show it again from ⋯.');
}
