import type { DiffResp, FileDiff } from '../../lib/api/types';
import type { ComposerAt } from './diff-model';

// A PR diff is re-fetched on every push, and the new DiffResp object used to
// reset everything the reviewer had toggled (viewed marks, expansions, the
// open inline composer). When the caller names a stable identity (`stateKey`,
// e.g. repo + PR number) that state is CARRIED to the new diff for every path
// whose CONTENT is unchanged (`unchangedPaths`). A path the push changed loses
// its viewed mark (the new code must be looked at — GitHub resets "Viewed" the
// same way), its expansions (hunk #3 is a different hunk now) and the open
// composer (line 120 holds different code now). Fetched file bodies / errors
// are never carried.

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

/** A file's content identity within a diff: the server's byte-exact patch
 *  fingerprint when present, else what summary mode still shows (status,
 *  rename source, line counts, hunk headers). Equal stamps ⇒ same change. */
export function fileStamp(f: FileDiff): string {
  if (f.fingerprint) return `fp:${f.fingerprint}`;
  return [
    'sum',
    f.status ?? '',
    f.old_path ?? '',
    f.added ?? '',
    f.deleted ?? '',
    f.is_binary ? 'bin' : '',
    f.hunks.map((h) => h.header).join('\u001f'),
  ].join('\u001e');
}

/** Paths present in both diffs with an unchanged stamp — the only ones whose
 *  view state may be carried across a reload. */
export function unchangedPaths(prev: DiffResp | null, next: DiffResp): Set<string> {
  const out = new Set<string>();
  if (!prev) return out;
  const before = new Map(prev.files.map((f) => [f.path, fileStamp(f)]));
  for (const f of next.files) if (before.get(f.path) === fileStamp(f)) out.add(f.path);
  return out;
}
