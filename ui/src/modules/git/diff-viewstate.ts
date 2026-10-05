import type { ComposerAt } from './diff-model';

// A PR diff is re-fetched on every push, and the new DiffResp object used to
// reset everything the reviewer had toggled (viewed marks, expansions, the
// open inline composer). When the caller names a stable identity (`stateKey`,
// e.g. repo + PR number) that state is CARRIED to the new diff for every path
// that still exists. Fetched file bodies / errors are not carried: the content
// may have changed with the push.

export interface CarriedState {
  overrides: Record<string, boolean>;
  composer: ComposerAt | null;
  uncapped: Map<string, Set<number>>;
  full: Set<string>;
}

export function carryViewState(prev: CarriedState, paths: ReadonlySet<string>): CarriedState {
  const overrides: Record<string, boolean> = {};
  for (const [p, v] of Object.entries(prev.overrides)) if (paths.has(p)) overrides[p] = v;
  const uncapped = new Map<string, Set<number>>();
  for (const [p, v] of prev.uncapped) if (paths.has(p)) uncapped.set(p, v);
  return {
    overrides,
    composer: prev.composer && paths.has(prev.composer.path) ? prev.composer : null,
    uncapped,
    full: new Set([...prev.full].filter((p) => paths.has(p))),
  };
}

/** Viewed marks survive a reload for the paths still in the diff. */
export function carryViewed(prev: ReadonlySet<string>, paths: ReadonlySet<string>): Set<string> {
  return new Set([...prev].filter((p) => paths.has(p)));
}
