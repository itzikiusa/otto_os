// Global keyboard map (spec §7.4). One window-level keydown listener which
// translates chords into named actions; App.svelte supplies the dispatcher.
//
// ⌘K palette · ⌘I ask Otto (plain English) · ⌘⇧B broadcast · ⌘⇧R hard reload · ⌘1 rail ·
// ⌘J right panel · ⌘T new session · ⌘W close tab ·
// ⌃Tab / ⌃⇧Tab cycle tabs · ⌘[ / ⌘] prev/next session · ⌃1…⌃9 jump to session N ·
// ⌘D / ⌘⇧D splits · ⌘F find (terminal) ·
// ⌘+ / ⌘- / ⌘0 zoom (app zoom, or terminal font-size when a terminal is focused)

export type KeyAction =
  | 'palette'
  | 'askOtto'
  | 'broadcast'
  | 'hardReload'
  | 'settings'
  | 'updateCLIs'
  | 'toggleRail'
  | 'toggleRight'
  | 'newSession'
  | 'closeTab'
  | 'nextTab'
  | 'prevTab'
  | 'nextSession'
  | 'prevSession'
  | 'jumpSession'
  | 'splitVertical'
  | 'splitHorizontal'
  | 'find'
  | 'appZoomIn'
  | 'appZoomOut'
  | 'appZoomReset'
  | 'termZoomIn'
  | 'termZoomOut'
  | 'termZoomReset'
  | 'navBack'
  | 'navForward';

/** Mutable context the Terminal component updates on focus/blur. */
export const keyContext: {
  terminalFocused: boolean;
  /** focused terminal registers its find-bar opener here */
  openFind: (() => void) | null;
} = {
  terminalFocused: false,
  openFind: null,
};

/** `index` is the 1-based session number for the `jumpSession` action. */
export type KeyDispatcher = (action: KeyAction, e: KeyboardEvent, index?: number) => void;

/** Install the global key map. Returns an uninstall fn. */
export function installKeyMap(dispatch: KeyDispatcher): () => void {
  const handler = (e: KeyboardEvent) => {
    const mod = e.metaKey || e.ctrlKey;
    const term = keyContext.terminalFocused;

    // ⌃Tab cycling (ctrl specifically, also when meta absent)
    if (e.ctrlKey && !e.metaKey && e.key === 'Tab') {
      e.preventDefault();
      dispatch(e.shiftKey ? 'prevTab' : 'nextTab', e);
      return;
    }

    // ⌃1…⌃9 → jump straight to the Nth session tab (ctrl specifically, so it
    // doesn't collide with ⌘1 = toggle rail). Handled before the meta switch.
    if (e.ctrlKey && !e.metaKey && !e.shiftKey && e.key >= '1' && e.key <= '9') {
      e.preventDefault();
      dispatch('jumpSession', e, Number(e.key));
      return;
    }

    if (!mod) return;

    // ⌘⇧← / ⌘⇧→ → navigate back / forward through page history. Skip when an
    // editable element (input/textarea/contenteditable/CodeMirror) is focused,
    // since that chord selects text there.
    if (e.shiftKey && (e.key === 'ArrowLeft' || e.key === 'ArrowRight')) {
      const el = document.activeElement as HTMLElement | null;
      const editable =
        !!el &&
        (el.tagName === 'INPUT' ||
          el.tagName === 'TEXTAREA' ||
          el.isContentEditable ||
          !!el.closest('.cm-editor'));
      if (editable) return;
      e.preventDefault();
      dispatch(e.key === 'ArrowLeft' ? 'navBack' : 'navForward', e);
      return;
    }

    switch (e.key.toLowerCase()) {
      case 'k':
        e.preventDefault();
        dispatch('palette', e);
        return;
      case ',':
        // ⌘, → Settings (works even if the native menu bridge isn't attached)
        e.preventDefault();
        dispatch('settings', e);
        return;
      case 'i':
        // ⌘I → straight to the plain-English "Ask Otto" box.
        e.preventDefault();
        dispatch('askOtto', e);
        return;
      case 'u':
        // ⌘U → update all agent CLIs (spawns the Update CLIs session).
        e.preventDefault();
        dispatch('updateCLIs', e);
        return;
      case 'b':
        if (e.shiftKey) {
          // ⌘⇧B → plain-English box pre-filled to broadcast.
          e.preventDefault();
          dispatch('broadcast', e);
          return;
        }
        return;
      case '1':
        e.preventDefault();
        dispatch('toggleRail', e);
        return;
      case 'j':
        e.preventDefault();
        dispatch('toggleRight', e);
        return;
      case 't':
        e.preventDefault();
        dispatch('newSession', e);
        return;
      case 'w':
        e.preventDefault();
        dispatch('closeTab', e);
        return;
      case 'd':
        e.preventDefault();
        dispatch(e.shiftKey ? 'splitHorizontal' : 'splitVertical', e);
        return;
      case 'f':
        e.preventDefault();
        dispatch('find', e);
        return;
      case 'r':
        // ⌘⇧R → hard-reload the UI (like a browser refresh). All sessions live
        // in the daemon, so they survive — this just re-fetches fresh state and
        // clears any stale in-memory UI. Requires Shift (plain ⌘R is left alone).
        if (e.shiftKey) {
          e.preventDefault();
          dispatch('hardReload', e);
        }
        return;
    }

    // ⌘[ / ⌘] → previous / next session tab.
    if (e.key === '[' || e.key === ']') {
      e.preventDefault();
      dispatch(e.key === '[' ? 'prevSession' : 'nextSession', e);
      return;
    }

    // zoom chords — '=' is the unshifted '+' key
    if (e.key === '=' || e.key === '+') {
      e.preventDefault();
      dispatch(term ? 'termZoomIn' : 'appZoomIn', e);
      return;
    }
    if (e.key === '-') {
      e.preventDefault();
      dispatch(term ? 'termZoomOut' : 'appZoomOut', e);
      return;
    }
    if (e.key === '0') {
      e.preventDefault();
      dispatch(term ? 'termZoomReset' : 'appZoomReset', e);
    }
  };

  window.addEventListener('keydown', handler, { capture: true });
  return () => window.removeEventListener('keydown', handler, { capture: true });
}

// ---------------------------------------------------------------------------
// Cheat-sheet data — the single source of truth for the `?` overlay
// (ShortcutsOverlay.svelte). Keep these rows in sync with the chords handled
// above so the overlay stays accurate; the overlay derives entirely from this.
// ---------------------------------------------------------------------------

export interface ShortcutBinding {
  /** display chord, e.g. "⌘K" or "⌃1…⌃9" */
  keys: string;
  /** what it does */
  label: string;
}

export interface ShortcutGroup {
  category: string;
  bindings: ShortcutBinding[];
}

export const KEYMAP: ShortcutGroup[] = [
  {
    category: 'General',
    bindings: [
      { keys: '⌘K', label: 'Command palette' },
      { keys: '⌘I', label: 'Ask Otto (plain English)' },
      { keys: '⌘⇧B', label: 'Broadcast to sessions' },
      { keys: '⌘U', label: 'Update all agent CLIs' },
      { keys: '⌘⇧R', label: 'Hard reload — refresh UI (sessions kept)' },
      { keys: '⌘,', label: 'Settings' },
      { keys: '?', label: 'Keyboard shortcuts (this sheet)' },
    ],
  },
  {
    category: 'Sessions',
    bindings: [
      { keys: '⌘T', label: 'New session' },
      { keys: '⌘W', label: 'Close tab' },
      { keys: '⌃Tab', label: 'Next tab' },
      { keys: '⌃⇧Tab', label: 'Previous tab' },
      { keys: '⌘]', label: 'Next session' },
      { keys: '⌘[', label: 'Previous session' },
      { keys: '⌃1…⌃9', label: 'Jump to session N' },
      { keys: '⌘D', label: 'Split vertically' },
      { keys: '⌘⇧D', label: 'Split horizontally' },
      { keys: '⌘F', label: 'Find (terminal / page)' },
    ],
  },
  {
    category: 'View',
    bindings: [
      { keys: '⌘1', label: 'Toggle sidebar' },
      { keys: '⌘J', label: 'Toggle right panel' },
      { keys: '⌘⇧←', label: 'Navigate back' },
      { keys: '⌘⇧→', label: 'Navigate forward' },
    ],
  },
  {
    category: 'Zoom',
    bindings: [
      { keys: '⌘+', label: 'Zoom in (app / terminal font)' },
      { keys: '⌘-', label: 'Zoom out (app / terminal font)' },
      { keys: '⌘0', label: 'Reset zoom' },
    ],
  },
];
