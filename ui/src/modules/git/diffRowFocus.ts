// A diff line's "expand line" button unmounts with the expansion it asks for,
// so focus fell to <body> in a long diff (accessibility-responsive-5-14).
import { tick } from 'svelte';

/** After expanding the line `btn` sat on, focus that row's first control, or
 *  the row itself (made programmatically focusable). Rows are found again by
 *  `data-rk` after the re-render, since the virtual list may re-key nodes. */
export async function refocusRow(btn: HTMLElement, root: ParentNode | null | undefined): Promise<void> {
  const rk = btn.closest<HTMLElement>('[data-rk]')?.dataset.rk;
  await tick();
  if (!rk || !root) return;
  const row = root.querySelector<HTMLElement>(`[data-rk="${CSS.escape(rk)}"]`);
  if (!row) return;
  const target = row.querySelector<HTMLElement>('button, [href], [tabindex]') ?? row;
  if (target === row) row.tabIndex = -1;
  target.focus({ preventScroll: true });
}
