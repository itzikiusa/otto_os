// Per-query-tab results-grid view state (sort, search, column filters and
// un-applied cell edits). ONE ResultsGrid instance serves every query tab of a
// connection, so without this a tab switch silently discarded pending edits and
// two tabs with the same columns shared sort/search. Module-level (not
// component state) so it also survives the grid remounting — Query ↔
// Structure/Diagram, leaving the page. Keyed by the tab's stable uid; bounded.

export interface GridTabState<P = unknown> {
  /** Identity of the result the state belongs to: pending edits are keyed by
   *  row index, so they are only restored onto the SAME result object. */
  result: unknown;
  /** Column signature; sort/search/filters are restored only on a match. */
  colKey: string;
  search: string;
  sortCol: number | null;
  sortDir: 'asc' | 'desc' | null;
  colFilters: Record<number, string>;
  /** Un-applied cell edits (EditFlow.pending), row index → patch. */
  pending: Map<number, P>;
}

const MAX_TABS = 64;
const states = new Map<string, GridTabState>();

/** Park a tab's grid state (LRU: re-parking moves it to the newest slot). */
export function stashGridState<P>(tabKey: string, state: GridTabState<P>): void {
  states.delete(tabKey);
  states.set(tabKey, state as GridTabState);
  while (states.size > MAX_TABS) {
    const oldest = states.keys().next().value;
    if (oldest === undefined) break;
    states.delete(oldest);
  }
}

/** Take (and remove) a tab's parked grid state, or null. */
export function takeGridState<P>(tabKey: string): GridTabState<P> | null {
  const s = states.get(tabKey);
  if (!s) return null;
  states.delete(tabKey);
  return s as GridTabState<P>;
}

/** Number of un-applied edits parked for a tab (for a close prompt). */
export function parkedEditCount(tabKey: string): number {
  return states.get(tabKey)?.pending.size ?? 0;
}

/** Forget a tab's parked state (tab closed). */
export function dropGridState(tabKey: string): void {
  states.delete(tabKey);
}
