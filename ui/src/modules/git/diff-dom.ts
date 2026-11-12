// Event helpers DiffViewer hands its DOM events to: keyboard file stepping,
// the touch line-tap that opens a comment, keeping the composer above the
// soft keyboard, and the stage/discard error toasts. Kept out of the
// renderer so DiffViewer.svelte stays about the windowed row model.
import { scrollBehavior } from '../../lib/motion';
import { loadErrorText } from '../../lib/loadError';
import { ApiError } from '../../lib/api/client';
import { toasts } from '../../lib/toast.svelte';

/** Keyboard file navigation: ] / n = next file, [ / N = previous, 0 = not
 *  ours. Never while typing in a field, and bare keys only (⌘[ / ⌘] are
 *  back / forward, Ctrl+N and friends are not ours either). */
export function fileNavDelta(e: KeyboardEvent): -1 | 0 | 1 {
  const tag = (e.target as HTMLElement).tagName.toLowerCase();
  if (tag === 'input' || tag === 'textarea' || tag === 'select') return 0;
  if (e.metaKey || e.ctrlKey || e.altKey) return 0;
  if (e.key === ']' || e.key === 'n') return 1;
  if (e.key === '[' || e.key === 'N') return -1;
  return 0;
}

/** Touch: a tap on a diff line opens the comment composer for that line —
 *  the line-number gutter is a 30 px × 20 px target, too small for a thumb,
 *  and making every row 44 px tall would halve how much code fits a phone.
 *  Returns the tapped line row when the click is such a tap: a coarse
 *  pointer, nothing selected (a long-press selection never counts) and not
 *  on a control. A pan or fling fires no click, so scrolling never opens a
 *  composer. */
export function lineTapRow(e: Event): Element | null {
  if (typeof matchMedia !== 'function' || !matchMedia('(pointer: coarse)').matches) return null;
  const sel = window.getSelection();
  if (sel && !sel.isCollapsed) return null;
  const t = e.target instanceof Element ? e.target : null;
  if (!t || t.closest('button, a, input, textarea, select, label')) return null;
  return t.closest('.vrow.dline');
}

/** Touch: the soft keyboard can cover the composer — centre it. Mouse: only
 *  bring the whole box (buttons too) into view, at once; a smooth re-centre
 *  on each focus slid Comment away under the pointer. */
export function revealComposer(e: FocusEvent): void {
  const el = e.currentTarget as HTMLElement | null;
  if (!el) return;
  const touch = matchMedia('(pointer: coarse)').matches;
  const box = touch ? el : (el.closest('.composer') ?? el);
  requestAnimationFrame(() => box.scrollIntoView(touch ? { block: 'center', behavior: scrollBehavior() } : { block: 'nearest' }));
}

/** The toast for a failed stage / unstage / discard of a hunk or lines. */
export function stageErrorToast(e: unknown): void {
  const raw = (e as { message?: unknown } | null)?.message;
  const msg = typeof raw === 'string' ? raw : '';
  const status = e instanceof ApiError ? e.status : 0;
  if (status === 400 && msg.includes('stage the whole file')) {
    toasts.error(
      'Partial staging unavailable',
      "This hunk can’t be staged partially (renamed/binary file) — stage the whole file.",
    );
  } else if (status === 409 && msg.includes('changed since the diff')) {
    toasts.error(
      'Hunk out of date',
      'The hunk no longer applies — the file changed since the diff was shown; refresh and retry.',
    );
  } else {
    toasts.error('Couldn’t stage', loadErrorText(e));
  }
}
