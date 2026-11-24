// Undo instead of confirm (T3 Code's thread undo notice): a reversible
// thread action — pin / unpin, Move to History / back, dismissing a side
// chat — acts at once and shows ONE notice with Undo for 5 s; a newer action
// replaces it. ⌘Z / Ctrl+Z runs the same Undo while focus is not in a text
// field, an editor or a terminal (they keep their own undo).
import { router } from '../../lib/router.svelte';
import { setAppUndo } from '../../lib/undoredo';
import { toasts } from '../../lib/toast.svelte';

const UNDO_MS = 5000;
let current: number | null = null;

/** Show "<title> · Undo"; running Undo calls `undo`. Replaces the last one. */
export function offerUndo(title: string, undo: () => void | Promise<void>): void {
  if (current !== null) toasts.dismiss(current);
  const id = toasts.push('info', title, undefined, UNDO_MS, {
    action: { label: 'Undo', run: undo },
    onClose: () => {
      if (current === id) current = null;
    },
  });
  current = id;
}

/** Run the notice's Undo; false when no notice is up. */
export function undoLast(): boolean {
  if (current === null) return false;
  void toasts.runAction(current);
  return true;
}

/** Focus where ⌘Z belongs to the field: inputs, editors, terminals. */
export function isTypingTarget(el: Element | null | undefined): boolean {
  return !!el?.closest?.('input, textarea, select, [contenteditable]:not([contenteditable="false"]), .xterm, .cm-editor');
}

export function onUndoKey(e: Pick<KeyboardEvent, 'key' | 'metaKey' | 'ctrlKey' | 'shiftKey' | 'altKey' | 'preventDefault'>, active: Element | null): boolean {
  if (router.module !== 'threads' || e.key.toLowerCase() !== 'z' || !(e.metaKey || e.ctrlKey) || e.shiftKey || e.altKey) return false;
  if (isTypingTarget(active) || !undoLast()) return false;
  e.preventDefault();
  return true;
}

if (typeof window !== 'undefined') window.addEventListener('keydown', (e) => void onUndoKey(e, document.activeElement));
// The desktop app's Edit ▸ Undo may fire before (and then swallow) the
// keydown: its route tries the notice's Undo first, under the same rule.
setAppUndo((focused) => router.module === 'threads' && !isTypingTarget(focused) && undoLast());
