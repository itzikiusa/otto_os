// How much of the Git page's repo tab strip can show the branch beside the
// repo name. The name is the tab's identity, the branch is context: when the
// strip has room every tab shows its branch (as it always did); when it would
// overflow, only the ACTIVE tab keeps its branch if that still fits; otherwise
// every tab is name-only (the branch stays in its tooltip + accessible name).
// Measured from the real strip (GitTabs.svelte, ResizeObserver), never a fixed
// repo count. Pure so unit/repoTabFit.test.ts runs it under node --test.

export type BranchMode = 'all' | 'active' | 'none';

export interface TabMeasure {
  /** The tab's width with its branch shown (px, incl. its share of the gap). */
  full: number;
  /** What hiding the branch saves (its chip + the gap before it); 0 = none. */
  branch: number;
}

/** `available`: the width the tab row may use. `active`: index or -1. */
export function branchMode(tabs: readonly TabMeasure[], available: number, active: number): BranchMode {
  const full = tabs.reduce((sum, t) => sum + t.full, 0);
  if (full <= available + 0.5) return 'all';
  const bare = full - tabs.reduce((sum, t) => sum + t.branch, 0);
  const activeBranch = active >= 0 && active < tabs.length ? tabs[active].branch : 0;
  if (activeBranch > 0 && bare + activeBranch <= available + 0.5) return 'active';
  return 'none';
}

/** A tab's accessible name / tooltip: "repo — branch" (the branch may be hidden). */
export function repoTabLabel(name: string, branch: string | null): string {
  return branch ? `${name} — ${branch}` : name;
}
