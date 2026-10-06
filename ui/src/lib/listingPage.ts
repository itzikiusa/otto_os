// A capped listing (`?meta=1` → `{items, truncated}`, S5-22) for the Jira
// project / Confluence space pickers. Tolerates a bare array (a daemon that
// predates `meta`). Pure — unit-tested (unit/listingPage.test.ts).

import type { ListingPage } from './api/types';

export function unwrapListing<T>(body: T[] | ListingPage<T> | null | undefined): ListingPage<T> {
  if (Array.isArray(body)) return { items: body, truncated: false };
  return { items: body?.items ?? [], truncated: body?.truncated === true };
}

/** The note a picker shows under a truncated list. */
export function truncatedNote(noun: string, shown: number): string {
  return `Showing the first ${shown} ${noun} — the list stopped at its size limit, so some aren’t listed.`;
}
