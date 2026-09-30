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

/** Open CodeMirror UI that Esc dismisses: the search panel, a completion popup. */
const ESC_CONSUMERS = '.cm-search, .cm-tooltip-autocomplete';

/**
 * Action for the element wrapping the CodeEditor: an Esc that dismisses open
 * CodeMirror UI (the search panel, a completion popup) must not ALSO reach
 * Modal's `window` listener and close the dialog. Whether such UI was open is
 * read in the capture phase — before CodeMirror handles the key and removes
 * it — and the bubble phase then stops the event. Any other Esc (including
 * one CodeMirror spends collapsing a selection) closes the modal as usual.
 */
export function editorEscGuard(node: HTMLElement): { destroy: () => void } {
  let swallow: Event | null = null;
  const capture = (e: KeyboardEvent) => {
    swallow = e.key === 'Escape' && node.querySelector(ESC_CONSUMERS) ? e : null;
  };
  const bubble = (e: KeyboardEvent) => {
    if (e === swallow) e.stopPropagation();
    swallow = null;
  };
  node.addEventListener('keydown', capture, true);
  node.addEventListener('keydown', bubble);
  return {
    destroy() {
      node.removeEventListener('keydown', capture, true);
      node.removeEventListener('keydown', bubble);
    },
  };
}
