// Per-page "last selection" memory for list/detail pages (Swarm, Proof,
// Workflows, Brokers …). A list/detail page should never open onto a big empty
// "pick one" pane when it has items: it restores the last selection, or falls
// back to the first item.
//
// Storage is a per-device convenience only (localStorage, every access wrapped:
// private windows / blocked storage just mean "no memory").

const PREFIX = 'otto.lastSelection.';

export function recallSelection(page: string): string | null {
  try {
    return localStorage.getItem(PREFIX + page);
  } catch {
    return null;
  }
}

export function rememberSelection(page: string, id: string | null | undefined): void {
  try {
    if (id) localStorage.setItem(PREFIX + page, id);
    else localStorage.removeItem(PREFIX + page);
  } catch {
    /* storage unavailable — selection just isn't remembered */
  }
}

/**
 * Pick what a list/detail page should open on: the remembered id if it is
 * still in the list, else the first item. `null` for an empty list.
 */
export function initialSelection<T>(page: string, items: readonly T[], idOf: (t: T) => string): string | null {
  if (items.length === 0) return null;
  const last = recallSelection(page);
  if (last && items.some((t) => idOf(t) === last)) return last;
  return idOf(items[0]);
}
