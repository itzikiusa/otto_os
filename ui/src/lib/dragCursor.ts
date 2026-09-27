// Drag plumbing for resizers (sidebar, right panel, split dividers, module
// panes). Two costs it removes, both measured on heavy pages (a 100k-line diff,
// a DB grid):
//  - `document.body.style.cursor` / `userSelect` are INHERITED, so writing them
//    restyles every element in the document at drag start and again at drag
//    end (65–690 ms each). A fixed full-window overlay carries the cursor
//    instead: one new element, no inherited change, and it also keeps the
//    pointer (and text selection) off iframes and content under the drag.
//  - mousemove fires faster than frames; each width write relayouts the
//    content column. Moves are coalesced to one per animation frame.

import { rafCoalesce } from './rafCoalesce';

export { rafCoalesce };

/** Put a full-window overlay with `cursor` over the app until the returned
 *  function is called. Idempotent teardown. */
export function showDragOverlay(cursor = 'col-resize'): () => void {
  if (typeof document === 'undefined') return () => {};
  const el = document.createElement('div');
  el.className = 'otto-drag-overlay';
  el.setAttribute('aria-hidden', 'true');
  el.dataset.testid = 'drag-overlay';
  const s = el.style;
  s.position = 'fixed';
  s.inset = '0';
  s.cursor = cursor;
  s.zIndex = 'var(--z-overlay-max)';
  s.userSelect = 'none';
  s.setProperty('-webkit-user-select', 'none');
  s.background = 'transparent';
  document.body.appendChild(el);
  let done = false;
  return () => {
    if (done) return;
    done = true;
    el.remove();
  };
}

export interface MouseDragOptions {
  /** CSS cursor for the whole window while dragging. */
  cursor?: string;
  /** Called at most once per frame with the latest pointer event. */
  onMove: (ev: MouseEvent) => void;
  /** Called once on release, after the last move was applied (persist here). */
  onEnd?: (ev: MouseEvent) => void;
}

/** Drive a window-level mouse drag started by `e` (a mousedown): overlay
 *  cursor, rAF-coalesced moves, one `onEnd`. Returns a cancel function. */
export function startMouseDrag(e: MouseEvent, opts: MouseDragOptions): () => void {
  e.preventDefault();
  const hide = showDragOverlay(opts.cursor);
  const moves = rafCoalesce(opts.onMove);
  const onMove = (ev: MouseEvent): void => moves.push(ev);
  const stop = (): void => {
    window.removeEventListener('mousemove', onMove);
    window.removeEventListener('mouseup', onUp);
    hide();
  };
  const onUp = (ev: MouseEvent): void => {
    moves.flush();
    stop();
    opts.onEnd?.(ev);
  };
  window.addEventListener('mousemove', onMove);
  window.addEventListener('mouseup', onUp);
  return () => {
    moves.cancel();
    stop();
  };
}
