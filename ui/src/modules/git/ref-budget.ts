// Row budget for the git sidebar's branch sections (perf r3-03-02).
//
// The old cap was per LIST (loose leaves, and each folder separately): a repo
// whose 2k branches sit under many prefix folders (`user/…`, `feature/…`, one
// per team) still mounted every row — ~50k DOM nodes next to the graph, on every
// open and every refs refresh. The budget here is per SECTION: loose leaves,
// then folder headers and the leaves of expanded folders, until `budget` rows
// are mounted. Everything after that is summarised by one "Show more" row.

export interface BudgetFolder<L> {
  name: string;
  leaves: L[];
}

export interface PlannedFolder<L> {
  folder: BudgetFolder<L>;
  /** Leaves of this folder to mount (0 when the folder is collapsed). */
  shown: number;
}

export interface SectionPlan<L> {
  /** Loose (unfoldered) leaves to mount. */
  loose: number;
  /** Folders whose header is mounted, in order, with their mounted leaves. */
  folders: PlannedFolder<L>[];
  /** Leaves (of expanded or not-yet-mounted folders, and loose) left out. */
  hidden: number;
}

/** Mount at most `budget` rows (leaves + folder headers) for one section.
 *  `collapsed(name)` reports a folder the user collapsed — its header costs a
 *  row, its leaves nothing and they don't count as hidden. */
export function planSection<L>(
  loose: L[],
  folders: BudgetFolder<L>[],
  budget: number,
  collapsed: (name: string) => boolean,
): SectionPlan<L> {
  const cap = Math.max(1, Math.floor(budget));
  let rows = Math.min(loose.length, cap);
  const plan: SectionPlan<L> = { loose: rows, folders: [], hidden: loose.length - rows };
  for (const folder of folders) {
    const isCollapsed = collapsed(folder.name);
    if (rows >= cap) {
      if (!isCollapsed) plan.hidden += folder.leaves.length;
      continue;
    }
    rows += 1; // the header
    const shown = isCollapsed ? 0 : Math.min(folder.leaves.length, Math.max(0, cap - rows));
    rows += shown;
    if (!isCollapsed) plan.hidden += folder.leaves.length - shown;
    plan.folders.push({ folder, shown });
  }
  return plan;
}
