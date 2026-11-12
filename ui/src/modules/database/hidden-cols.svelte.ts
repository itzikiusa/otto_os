// Columns hidden from a result grid, per query tab. Not persisted: a reload
// (or a result of another column shape) shows every column again. Keyed by tab
// + column signature so one tab's hidden columns never carry onto another tab
// or onto a different result in the same tab. Module-level so it outlives the
// grid component (a Grid ⇄ Vertical switch or a page visit unmounts it).
import { hideColumn } from './grid-format';

const byTab: Record<string, string[]> = $state({});
const key = (tab: string, colKey: string): string => `${tab}\u0001${colKey}`;

export const hiddenCols = {
  /** The hidden column names of `tab`'s result with column signature `colKey`. */
  of(tab: string | null | undefined, colKey: string): string[] {
    return tab ? (byTab[key(tab, colKey)] ?? []) : [];
  },
  /** Hide `name`; false when it is the last visible column (nothing changes). */
  hide(tab: string | null | undefined, colKey: string, name: string, all: string[]): boolean {
    if (!tab) return false;
    const next = hideColumn(this.of(tab, colKey), name, all);
    if (next) byTab[key(tab, colKey)] = next;
    return next !== null;
  },
  showAll(tab: string | null | undefined, colKey: string): void {
    if (tab) delete byTab[key(tab, colKey)];
  },
};
