// Event helpers DiffViewer hands its DOM events to: keyboard file stepping,
// touch line-tap, composer reveal, and stage/discard error toasts.
import { scrollBehavior } from '../motion';
import { loadErrorText } from '../loadError';
import { ApiError } from '../api/client';
import { toasts } from '../toast.svelte';

/** Keyboard file navigation: ] / n = next file, [ / N = previous, 0 = not ours. */
export function fileNavDelta(e: KeyboardEvent): -1 | 0 | 1 {
  const tag = (e.target as HTMLElement).tagName.toLowerCase();
  if (tag === 'input' || tag === 'textarea' || tag === 'select') return 0;
  if (e.metaKey || e.ctrlKey || e.altKey) return 0;
  if (e.key === ']' || e.key === 'n') return 1;
  if (e.key === '[' || e.key === 'N') return -1;
  return 0;
}

export function lineTapRow(e: Event): Element | null {
  if (typeof matchMedia !== 'function' || !matchMedia('(pointer: coarse)').matches) return null;
  const sel = window.getSelection();
  if (sel && !sel.isCollapsed) return null;
  const t = e.target instanceof Element ? e.target : null;
  if (!t || t.closest('button, a, input, textarea, select, label')) return null;
  return t.closest('.vrow.dline');
}

export function revealComposer(e: FocusEvent): void {
  const el = e.currentTarget as HTMLElement | null;
  if (!el) return;
  const touch = matchMedia('(pointer: coarse)').matches;
  const box = el.closest('.composer') ?? el;
  requestAnimationFrame(() => box.scrollIntoView({ block: 'nearest', behavior: touch ? scrollBehavior() : 'auto' }));
}

export function stageErrorToast(e: unknown): void {
  const raw = (e as { message?: unknown } | null)?.message;
  const msg = typeof raw === 'string' ? raw : '';
  const status = e instanceof ApiError ? e.status : 0;
  if (status === 400 && msg.includes('stage the whole file')) {
    toasts.error(
      'Partial staging unavailable',
      'This hunk can’t be staged partially (renamed/binary file) — stage the whole file.',
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
