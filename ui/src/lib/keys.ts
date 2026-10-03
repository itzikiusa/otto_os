// Global keyboard map (spec §7.4). One window-level keydown listener which
// translates chords into named actions; App.svelte supplies the dispatcher.
//
// ⌘K palette (focuses the floating bar when it's mounted) · ⌘I ask Otto (plain English) · ⌘⇧B broadcast · ⌘⇧R hard reload · ⌘1 rail ·
// ⌘J right panel · ⌘T new session · ⌘W close tab (⌃⇧T / ⌃⇧W in a browser tab,
// which reserves the ⌘ pair for itself) ·
// ⌃Tab / ⌃⇧Tab cycle tabs · ⌘[ / ⌘] prev/next session · ⌃1…⌃9 jump to session N ·
// ⌘D / ⌘⇧D splits · ⌘F find (terminal) ·
// ⌘+ / ⌘- / ⌘0 zoom (app zoom, or terminal font-size when a terminal is focused) ·
// ⌘\ side-by-side pane (open a picker / close)

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
  | 'reopenTab'
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
  | 'toggleSidePane'
  | 'snip';

/** Mutable context the Terminal component updates on focus/blur. */
export const keyContext: {
  terminalFocused: boolean;
  /** focused terminal (or its find input) / CodeEditor registers its find
   *  opener here — see routeFind below */
  openFind: (() => void) | null;
  /** The in-app floating bar has focus: ⌃1–⌃4 switch ITS spaces. */
  barFocused: boolean;
  /** A mounted page may claim ⌘-chords before the global map sees them (the
   *  API client's ⌘T new request tab / ⌘D duplicate). Return true when handled;
   *  the page clears it on unmount. */
  pageChords: ((e: KeyboardEvent) => boolean) | null;
} = {
  terminalFocused: false,
  openFind: null,
  barFocused: false,
  pageChords: null,
};

// ── ⌘F routing ────────────────────────────────────────────────────────────
// Focus-owned find (`keyContext.openFind`: a focused terminal, its own find
// input, a focused CodeEditor) wins. Otherwise the active session pane's
// terminal still owns ⌘F while it is on screen: a click on the pane header,
// a side panel or empty chrome moves focus off the xterm, and the page-wide
// FindInPage walks DOM text — a WebGL terminal has none, a DOM one only its
// visible rows. Only then does ⌘F fall back to the page.
//
// "Still owns" is scoped to where the user last WAS: a session embedded next
// to other content (a loop's timeline, a swarm board, the Agents right panel)
// must not steal ⌘F from a click on that content — clicking plain text moves
// focus to <body>, so focus alone can't tell. The last pointerdown / focusin
// target (capture phase, installKeyMap) must lie inside the owner's pane; no
// interaction yet counts as "in the active pane".

/** A terminal that takes ⌘F without holding focus (Terminal `findRank`). */
export interface FindOwner {
  /** > 0 while it can take ⌘F (mounted, bound, visible); highest wins —
   *  the focused split pane over another view of the same session. */
  rank(): number;
  /** Its pane (header included): the last interaction must be inside it. */
  pane(): Element | null;
  open(): void;
}

const findOwners = new Set<FindOwner>();

/** Target of the last pointerdown / focusin anywhere in the document. */
let lastInteraction: Element | null = null;

/** Record where the user last pointed or focused (exported for the tests). */
export function noteInteraction(target: EventTarget | null): void {
  const el = target && typeof (target as Element).closest === 'function' ? (target as Element) : null;
  // Chrome that only relays ⌘F (the phone quick-action bar) keeps the last
  // real interaction.
  if (el?.closest('[data-find-neutral]')) return;
  lastInteraction = el;
}

/** The last interaction allows a pane owner inside `pane` to take ⌘F. */
function interactedIn(pane: Element | null): boolean {
  if (!lastInteraction) return true;
  if (!pane || lastInteraction.closest('.rpanel')) return false;
  return pane.contains(lastInteraction);
}

function shown(el: Element): boolean {
  const check = (el as Element & { checkVisibility?: (o?: object) => boolean }).checkVisibility;
  if (typeof check === 'function') return check.call(el, { visibilityProperty: true });
  return (el as HTMLElement).offsetParent !== null;
}

/** Register a pane find owner; returns the unregister (an `$effect` cleanup). */
export function registerFindOwner(o: FindOwner): () => void {
  findOwners.add(o);
  return () => {
    findOwners.delete(o);
  };
}

/** Focus sits where ⌘F means "search HERE", not the session pane: a text
 *  field / editor outside any terminal, or anything under an open modal. */
function focusClaimsFind(): boolean {
  // Visible ones only: the compact right-panel Drawer stays mounted, hidden,
  // with role=dialog + aria-modal (shell/Drawer.svelte).
  for (const d of document.querySelectorAll('[role="dialog"][aria-modal="true"], dialog[open]')) {
    if (shown(d)) return true;
  }
  const el = document.activeElement as HTMLElement | null;
  if (!el || el.closest('.xterm')) return false;
  return (
    el.tagName === 'INPUT' ||
    el.tagName === 'TEXTAREA' ||
    el.tagName === 'SELECT' ||
    el.isContentEditable ||
    !!el.closest('.cm-editor')
  );
}

/** ⌘F / the phone toolbar's find: focused owner → active session pane's
 *  terminal → `fallback` (the page-wide find). */
export function routeFind(fallback: () => void): void {
  if (keyContext.openFind) {
    keyContext.openFind();
    return;
  }
  if (!focusClaimsFind()) {
    let best: FindOwner | null = null;
    let bestRank = 0;
    for (const o of findOwners) {
      const r = o.rank();
      if (r > bestRank && interactedIn(o.pane())) {
        best = o;
        bestRank = r;
      }
    }
    if (best) {
      best.open();
      return;
    }
  }
  fallback();
}

/** Minimal shape of the focused element `editorOwnsChord` needs (kept
 *  structural so the unit test can pass a stub instead of a DOM node). */
export interface FocusLike {
  tagName?: string;
  isContentEditable?: boolean;
  closest?: (selector: string) => unknown;
}

/** True when a ⌘-chord belongs to the focused text editor rather than the
 *  global map: inside a CodeMirror `.cm-editor`, ⌘D, ⌘[ / ⌘], ⌘U / ⌘⇧U and
 *  ⌘I are CM's own commands; ⌘U / ⌘⇧U are also left alone in any plain
 *  input/textarea/contenteditable (a global install is never what a typist
 *  meant). ⌘K, ⌘W, ⌘T, ⌘J, ⌘1, ⌘F and zoom keep working everywhere. */
export function editorOwnsChord(
  e: Pick<KeyboardEvent, 'key' | 'shiftKey' | 'altKey'>,
  focused: FocusLike | Element | null | undefined,
): boolean {
  if (!focused || e.altKey) return false;
  const el = focused as FocusLike;
  const k = e.key.toLowerCase();
  const inEditor = !!el.closest?.('.cm-editor');
  if (k === 'u') {
    return (
      inEditor ||
      el.tagName === 'INPUT' ||
      el.tagName === 'TEXTAREA' ||
      el.isContentEditable === true
    );
  }
  if (!inEditor) return false;
  if (k === 'd' || k === 'i') return !e.shiftKey;
  return e.key === '[' || e.key === ']';
}

/** `index` is the 1-based session number for the `jumpSession` action. */
export type KeyDispatcher = (action: KeyAction, e: KeyboardEvent, index?: number) => void;

/** Install the global key map. Returns an uninstall fn. */
export function installKeyMap(dispatch: KeyDispatcher): () => void {
  const handler = (e: KeyboardEvent) => {
    // Ask the DOM too: a Terminal that focuses itself on mount does so before
    // its focus listener exists, so the flag alone can miss a focused xterm.
    const term =
      keyContext.terminalFocused ||
      !!(document.activeElement as HTMLElement | null)?.closest?.('.xterm');
    // ⌃ stands in for ⌘ (non-Mac remote clients) EXCEPT in a focused terminal:
    // there ⌃D/⌃K/⌃B/⌃F/… are the shell's (EOF, kill-line, readline motion),
    // so only a real ⌘ chord may reach the app map.
    const mod = e.metaKey || (e.ctrlKey && !term);

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
      // Inside the floating bar ⌃1–⌃4 pick a space (the bar's own handler).
      if (keyContext.barFocused && e.key <= '4') return;
      e.preventDefault();
      dispatch('jumpSession', e, Number(e.key));
      return;
    }

    if (!mod) return;
    // A focused CodeMirror owns ⌘D / ⌘[ / ⌘] / ⌘U / ⌘I (select next match,
    // indent, undo selection, select parent syntax). CM skips keydowns that
    // are already defaultPrevented, so the global map must not even touch
    // them — otherwise ⌘U in an editor launched "Update all CLIs".
    if (!term && editorOwnsChord(e, document.activeElement)) return;
    // Page-scoped chords (e.g. the API client's ⌘D) win over the global map
    // while that page is mounted — outside a terminal, which owns its keys.
    if (!term && !e.altKey && keyContext.pageChords?.(e)) return;
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
        // ⌘T → new session · ⌘⇧T → reopen closed tab (browser convention; only
        // reaches us in the desktop shell / PWA — a browser TAB reserves it).
        // ⌥⌘T is NOT ours (the DB editor uses it).
        e.preventDefault();
        dispatch(e.shiftKey ? 'reopenTab' : 'newSession', e);
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
        // ⌘F → find. ⇧⌘F is NOT find (the DB editor uses it for Format), nor
        // ⌃⌘F (the native View ▸ Enter Full Screen).
        if (e.shiftKey || (e.ctrlKey && e.metaKey)) return;
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

    // ⌘\ → open the side-by-side pane (a module picker) or close it.
    if (e.key === '\\' && !e.shiftKey) {
      e.preventDefault();
      dispatch('toggleSidePane', e);
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

  const note = (e: Event) => noteInteraction(e.target);
  window.addEventListener('keydown', handler, { capture: true });
  window.addEventListener('pointerdown', note, { capture: true });
  window.addEventListener('focusin', note, { capture: true });
  return () => {
    window.removeEventListener('keydown', handler, { capture: true });
    window.removeEventListener('pointerdown', note, { capture: true });
    window.removeEventListener('focusin', note, { capture: true });
  };
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
      { keys: '⌘K', label: 'Floating bar — commands & Ask Otto (palette on phone/tablet)' },
      { keys: '⌃1…⌃4', label: 'In the floating bar: switch space' },
      { keys: '⌘I', label: 'Ask Otto (plain English; not in a code editor)' },
      { keys: '⌘⇧B', label: 'Broadcast to sessions' },
      { keys: '⌘U / ⌘⇧U', label: 'Update all agent CLIs (asks first; not in a text field)' },
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
      { keys: '⌘⇧T', label: 'Reopen closed tab' },
      { keys: '⌃Tab', label: 'Next tab' },
      { keys: '⌃⇧Tab', label: 'Previous tab' },
      { keys: '⌘]', label: 'Next session (not in a code editor)' },
      { keys: '⌘[', label: 'Previous session (not in a code editor)' },
      { keys: '⌃1…⌃9', label: 'Jump to session N' },
      { keys: '⌘D', label: 'Split vertically (not in a code editor)' },
      { keys: '⌘⇧D', label: 'Split horizontally' },
      { keys: '⌘⌥←', label: 'Move pane left (in a Database pane: previous query tab)' },
      { keys: '⌘⌥→', label: 'Move pane right (in a Database pane: next query tab)' },
      { keys: '⌘⌥↑', label: 'Move pane up' },
      { keys: '⌘⌥↓', label: 'Move pane down' },
      { keys: '⌘⌥S', label: 'Swap pane with next' },
      { keys: '⌘F', label: 'Find (terminal / page)' },
    ],
  },
  {
    category: 'API client (on the API page)',
    bindings: [
      { keys: '⌘↵', label: 'Send the request' },
      { keys: '⌘S', label: 'Save the request' },
      { keys: '⌘T', label: 'New request tab (instead of a new session)' },
      { keys: '⌘D', label: 'Duplicate the request (instead of a split)' },
    ],
  },
  {
    category: 'View',
    bindings: [
      { keys: '⌘1', label: 'Toggle sidebar' },
      { keys: '⌘J', label: 'Toggle right panel' },
      { keys: '⌘⇧←', label: 'Navigate back' },
      { keys: '⌘⇧→', label: 'Navigate forward' },
      { keys: '⌘\\', label: 'Open or close the side-by-side pane' },
      { keys: '⌥-click', label: 'A sidebar item: open it side by side' },
      { keys: '⌥↑ / ⌥↓', label: 'Sidebar: move a Favorite (any row while customizing)' },
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
