// Context-aware Edit ▸ Undo / Redo (⌘Z / ⌘⇧Z) for the desktop app.
//
// WHY this exists: the macOS Edit menu used `PredefinedMenuItem::undo/redo`,
// whose key equivalents AppKit resolves by sending `undo:` / `redo:` to the
// web view's NATIVE undo manager. That list never holds CodeMirror's edits
// (CM applies typing and pastes to its own document and keeps its own
// history), and it is reset once focus leaves the window — so ⌘Z "did
// nothing" after a paste or after switching windows. The items are now custom
// ones that emit `undo` / `redo` (lib/menu.ts), the same route Select All
// takes (lib/selectall.ts):
//   - focus in a CodeMirror editor → that editor's history (`@codemirror/commands`);
//   - focus in xterm → nothing (a shell has no undo; ⌘Z never reached it before);
//   - focus in another document (a side-pane <iframe>) → nothing, that pane owns it;
//   - anything else (inputs, textareas) → the native `execCommand`, as before.
// In a plain browser none of this runs: CodeMirror's own keymap handles ⌘Z.
//
// One keypress must not undo twice. Depending on the WebKit path, the page
// may see the ⌘Z keydown before the menu fires (CM's keymap then runs and
// calls preventDefault) or the menu may fire first and the keydown still
// arrive. `installUndoKeyGuard` watches ⌘Z keydowns so the menu skips a
// press the page already handled, and swallows a keydown the menu already
// handled.

// No static CodeMirror import: this module is in the shell's boot closure
// (lib/menu.ts). A `.cm-editor` on screen means CM is already loaded, so the
// dynamic imports below resolve from the module cache.

export type HistoryAction = 'undo' | 'redo';

/** Where an undo/redo for the focused element belongs. */
export type UndoRoute = 'editor' | 'terminal' | 'elsewhere' | 'native';

/** Classify the focused element (pure — unit-tested with DOM fakes). */
export function undoRoute(el: Element | null): UndoRoute {
  if (!el) return 'native';
  // Focus lives in another document (the side-by-side pane's iframe): its
  // own webview handles its own edits; undoing here would hit the wrong pane.
  if (el.tagName === 'IFRAME') return 'elsewhere';
  // xterm keeps focus in a hidden helper <textarea>; native undo there would
  // only scramble that proxy. Check before the editor: never nested, but cheap.
  if (el.closest?.('.xterm')) return 'terminal';
  if (el.closest?.('.cm-editor')) return 'editor';
  return 'native';
}

export interface UndoDeps {
  /** Run CM's history command on the editor whose root is `cm`. */
  editor(cm: HTMLElement, action: HistoryAction): void;
  /** The native editing command (`document.execCommand`). */
  exec(action: HistoryAction): void;
}

const liveDeps: UndoDeps = {
  editor(cm, action) {
    void Promise.all([import('@codemirror/view'), import('@codemirror/commands')]).then(
      ([{ EditorView }, commands]) => {
        const view = EditorView.findFromDOM(cm);
        if (!view) return void document.execCommand(action);
        (action === 'undo' ? commands.undo : commands.redo)(view);
      },
      () => document.execCommand(action),
    );
  },
  exec(action) {
    document.execCommand(action);
  },
};

/** ⌘Z keydowns closer than this to a menu event are the same keypress. */
const SAME_PRESS_MS = 400;

let lastKey: { e: KeyboardEvent; action: HistoryAction; at: number } | null = null;
let lastMenu: { action: HistoryAction; at: number } | null = null;

/** ⌘Z → undo, ⌘⇧Z → redo, anything else → null. */
export function historyKeyAction(e: Pick<KeyboardEvent, 'key' | 'metaKey' | 'ctrlKey' | 'altKey' | 'shiftKey'>): HistoryAction | null {
  if (!e.metaKey || e.ctrlKey || e.altKey) return null;
  if (e.key.toLowerCase() !== 'z') return null;
  return e.shiftKey ? 'redo' : 'undo';
}

/** Capture-phase keydown hook (see the header). Exported for tests. */
export function onHistoryKeydown(e: KeyboardEvent, now = Date.now()): void {
  const action = historyKeyAction(e);
  if (!action) return;
  if (lastMenu && lastMenu.action === action && now - lastMenu.at < SAME_PRESS_MS) {
    // The menu already ran this press: don't let CM's keymap run it again.
    lastMenu = null;
    e.preventDefault();
    e.stopImmediatePropagation();
    return;
  }
  lastKey = { e, action, at: now };
}

/** Install the keydown guard on `target` (Tauri only). Returns the uninstall fn. */
export function installUndoKeyGuard(target: Pick<Window, 'addEventListener' | 'removeEventListener'> = window): () => void {
  const fn = (e: Event) => onHistoryKeydown(e as KeyboardEvent);
  target.addEventListener('keydown', fn, true);
  return () => target.removeEventListener('keydown', fn, true);
}

/**
 * Edit ▸ Undo / Redo: act on whatever has focus. Returns the route taken
 * (`'handled'` when the page's own keydown handler already took this press).
 */
export function undoRedoInFocus(
  action: HistoryAction,
  el: Element | null = typeof document === 'undefined' ? null : document.activeElement,
  deps: UndoDeps = liveDeps,
  now = Date.now(),
): UndoRoute | 'handled' {
  const key = lastKey;
  lastKey = null;
  if (key && key.action === action && now - key.at < SAME_PRESS_MS && key.e.defaultPrevented) return 'handled';
  lastMenu = { action, at: now };
  const route = undoRoute(el);
  if (route === 'editor') deps.editor(el!.closest('.cm-editor') as HTMLElement, action);
  else if (route === 'native') deps.exec(action);
  return route;
}

/** Test hook: forget the keypress bookkeeping. */
export function resetUndoGuard(): void {
  lastKey = null;
  lastMenu = null;
}
