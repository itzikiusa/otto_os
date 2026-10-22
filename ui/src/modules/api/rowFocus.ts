// A11y-5-08: removing a row in an index-keyed list editor (headers, query
// params, environment variables) destroys the focused "Remove" button when the
// last row goes, dropping focus to <body> mid-form. After the list re-renders,
// move focus to the Remove button now at the same index (else the previous
// row's), or to the list's fallback control (its "Add" button, or the blank
// row's input) when no Remove button is left.

import { tick } from 'svelte';

/** The list element around a Remove button — read from the click event
 *  BEFORE the row is removed (the button is detached afterwards). */
export function rowListOf(e: Event, listSelector: string): HTMLElement | null {
  return (e.currentTarget as HTMLElement | null)?.closest<HTMLElement>(listSelector) ?? null;
}

export async function focusAfterRowRemoval(list: HTMLElement | null, index: number, fallbackSelector: string): Promise<void> {
  await tick();
  if (!list) return;
  const removes = list.querySelectorAll<HTMLElement>('[data-remove-row]');
  const next = removes[Math.min(index, removes.length - 1)];
  const fallbacks = list.querySelectorAll<HTMLElement>(fallbackSelector);
  (next ?? fallbacks[fallbacks.length - 1])?.focus();
}
