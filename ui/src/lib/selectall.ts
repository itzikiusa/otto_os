// Context-aware "Select All" for the app's own widgets.
//
// WHY this exists: the native macOS Edit ▸ Select All used to be a
// `PredefinedMenuItem`, whose ⌘A key equivalent is resolved by AppKit BEFORE
// the key event ever reaches the webview. That native action runs `selectAll:`
// straight on the focused contenteditable — and both of our text surfaces are
// VIRTUALIZED: CodeMirror only renders the lines inside its viewport, xterm
// only the visible rows. The result was the bug users saw: ⌘A in a long query
// selected (and copied) just the on-screen part of the document.
//
// The menu item is now a custom one that emits `select-all`, which lands here,
// so the selection is made against the real MODEL (CM's document, xterm's
// buffer) instead of the rendered DOM subset.

import { EditorView } from '@codemirror/view';

/** Returns true when it handled the select-all for its element. */
type Handler = () => boolean;

const owners = new Map<HTMLElement, Handler>();

/**
 * Register a widget that owns its own text model (xterm, canvas-backed views).
 * `fn` is called when focus is inside `el`; returning false falls through to
 * the generic handling below. Returns an unregister fn for teardown.
 */
export function registerSelectAll(el: HTMLElement, fn: Handler): () => void {
  owners.set(el, fn);
  return () => owners.delete(el);
}

/** Select everything in whatever currently has focus. */
export function selectAllInFocus(): void {
  const el = (document.activeElement as HTMLElement | null) ?? null;

  // Registered model-owning widgets (terminals) come FIRST: xterm keeps focus
  // in a hidden helper <textarea>, so the form-field branch below would
  // otherwise "select" that empty proxy instead of the scrollback.
  if (el) {
    for (const [owner, fn] of owners) {
      if (owner.contains(el) && fn()) return;
    }
  }

  // Plain form fields select their whole value natively.
  if (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement) {
    // `select()` throws on input types that have no text selection (number,
    // color…); those simply have nothing to select.
    try {
      el.select();
    } catch {
      /* not a selectable input */
    }
    return;
  }

  // CodeMirror: ask the view, so the selection covers the whole document and
  // not just the lines the viewport happens to have rendered.
  const cm = el?.closest?.('.cm-editor') as HTMLElement | null;
  if (cm) {
    const view = EditorView.findFromDOM(cm);
    if (view) {
      view.dispatch({ selection: { anchor: 0, head: view.state.doc.length } });
      view.focus();
      return;
    }
  }

  // Anything else (read-only panes, plain markup): the native behavior.
  document.execCommand('selectAll');
}
