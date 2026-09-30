// Keyboard glue shared by the JSON document modals (DocEditor, RawDocViewer):
// each hosts a CodeEditor inside the shared Modal, whose Esc listener sits on
// `window` and whose content the page-wide find overlay can't search.
import { keyContext } from '../../lib/keys';

/**
 * Route ⌘F to `open` (the modal editor's search panel) while the modal is up.
 * The global key map claims ⌘F in the capture phase and preventDefaults it, so
 * CodeMirror's own Mod-f never runs; `keyContext.openFind` is the one hook.
 * Returns the release, for an `$effect` cleanup. Releases only its own opener:
 * an editor refocused under the closing modal may already have claimed it.
 */
export function claimFind(open: () => void): () => void {
  keyContext.openFind = open;
  return () => {
    if (keyContext.openFind === open) keyContext.openFind = null;
  };
}

/**
 * Keydown handler for the element wrapping the CodeEditor: an Esc CodeMirror
 * already consumed (closing its search panel or a completion popup — it
 * preventDefaults but lets the event bubble) must not reach Modal's `window`
 * listener and close the whole dialog too.
 */
export function keepEditorEsc(e: KeyboardEvent): void {
  if (e.key === 'Escape' && e.defaultPrevented) e.stopPropagation();
}
