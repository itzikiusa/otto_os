// Global keyboard map (spec §7.4). One window-level keydown listener which
// translates chords into named actions; App.svelte supplies the dispatcher.
//
// ⌘K palette · ⌘I ask Otto (plain English) · ⌘⇧B broadcast · ⌘⇧R hard reload · ⌘1 rail ·
// ⌘J right panel · ⌘T new session · ⌘W close tab (⌃⇧T / ⌃⇧W in a browser tab,
// which reserves the ⌘ pair for itself) ·
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
  | 'navForward'
  | 'snip';

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

    // Bare Backspace outside an editable element: WKWebView's legacy default
    // is "navigate back", which silently loses page state when the user just
    // missed a text field (or a grid/canvas owns the key). Kill the default;
    // component handlers (canvas node delete, chip removal…) still run.
    if (e.key === 'Backspace' && !mod && !e.altKey && !term) {
      const el = document.activeElement as HTMLElement | null;
      const editable =
        !!el &&
        (el.tagName === 'INPUT' ||
          el.tagName === 'TEXTAREA' ||
          el.tagName === 'SELECT' ||
          el.isContentEditable ||
          !!el.closest('.cm-editor, .xterm'));
      if (!editable) e.preventDefault();
      return;
    }

    // ⌃Tab cycling (ctrl specifically, also when meta absent; shift = previous).
    if (e.ctrlKey && !e.metaKey && !e.altKey && e.key === 'Tab') {
      e.preventDefault();
      dispatch(e.shiftKey ? 'prevTab' : 'nextTab', e);
      return;
    }

    // ⌃⇧T / ⌃⇧W → new session / close tab. These are ALIASES for ⌘T / ⌘W,
    // which a browser TAB reserves for itself (new tab / close tab) at a level
    // above the page: the keydown never reaches us, so preventDefault can't
    // help. The ⌘ chords stay bound — they do arrive in the desktop shell and
    // in an installed PWA / `--app=` window — and these give a keyboard route
    // when Otto is just a tab, which is how it's reached remotely.
    // (macOS browsers leave ⌃⇧T / ⌃⇧W free. On Windows/Linux Chrome, Ctrl+Shift+T
    // is "reopen closed tab" — the same reservation, and no chord escapes it
    // there; Otto's own shell is macOS-only.)
    if (e.ctrlKey && e.shiftKey && !e.metaKey && !e.altKey) {
      const k = e.key.toLowerCase();
      if (k === 't') {
        e.preventDefault();
        dispatch('newSession', e);
        return;
      }
      if (k === 'w') {
        e.preventDefault();
        dispatch('closeTab', e);
        return;
      }
    }

    // ⌃1…⌃9 → jump straight to the Nth session tab (ctrl specifically, so it
    // doesn't collide with ⌘1 = toggle rail). Handled before the meta switch.
    if (e.ctrlKey && !e.metaKey && !e.shiftKey && !e.altKey && e.key >= '1' && e.key <= '9') {
      e.preventDefault();
      dispatch('jumpSession', e, Number(e.key));
      return;
    }

    if (!mod) return;
    // No global chord uses ⌥ as a modifier — match exactly so an ⌥-augmented
    // combo never triggers the plain-⌘ action (e.g. ⌥⌘T must not fire ⌘T's
    // "new session"; the DB editor binds ⌥⌘T for a new query tab).
    if (e.altKey) return;

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
        if (e.shiftKey) return;
        e.preventDefault();
        dispatch('palette', e);
        return;
      case ',':
        // ⌘, → Settings (works even if the native menu bridge isn't attached)
        if (e.shiftKey) return;
        e.preventDefault();
        dispatch('settings', e);
        return;
      case 'i':
        // ⌘I → straight to the plain-English "Ask Otto" box.
        if (e.shiftKey) return;
        e.preventDefault();
        dispatch('askOtto', e);
        return;
      case 'u':
        // ⌘U / ⌘⇧U → update all agent CLIs (spawns the Update CLIs session).
        // Both chords intentionally: ⌘⇧U predates the exact-modifier pass and
        // stays supported; nothing else binds it.
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
      case 's':
        if (e.shiftKey) {
          // ⌘⇧S → snip: interactive screen capture → annotation editor.
          // (Plain ⌘S stays free — browsers claim it for "save page".)
          e.preventDefault();
          dispatch('snip', e);
          return;
        }
        return;
      case '1':
        if (e.shiftKey) return;
        e.preventDefault();
        dispatch('toggleRail', e);
        return;
      case 'j':
        if (e.shiftKey) return;
        e.preventDefault();
        dispatch('toggleRight', e);
        return;
      case 't':
        // ⌘T → new session. ⇧⌘T / ⌥⌘T are NOT this (the DB editor uses ⌥⌘T).
        if (e.shiftKey) return;
        e.preventDefault();
        dispatch('newSession', e);
        return;
      case 'w':
        if (e.shiftKey) return;
        e.preventDefault();
        dispatch('closeTab', e);
        return;
      case 'd':
        // ⌘D vertical split, ⌘⇧D horizontal split — both intended.
        e.preventDefault();
        dispatch(e.shiftKey ? 'splitHorizontal' : 'splitVertical', e);
        return;
      case 'f':
        // ⌘F → find. ⇧⌘F is NOT find (the DB editor uses it for Format).
        if (e.shiftKey) return;
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
      { keys: '⌘U / ⌘⇧U', label: 'Update all agent CLIs' },
      { keys: '⌘⇧S', label: 'Snip — capture screen region & annotate' },
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
