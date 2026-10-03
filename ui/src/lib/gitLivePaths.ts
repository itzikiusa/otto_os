/**
 * Path bookkeeping for `repo_status_changed` (perf R2).
 *
 * The daemon lists the repo-relative paths a change burst touched (`paths`),
 * or omits them when it can't tell (an index/HEAD move, a rescan, more than
 * 64 paths). Events coalesce on the client too (one status read in flight
 * per repo, a hidden window waits), so the sets UNION until a refresh takes
 * them; `null` — "anything may have changed" — absorbs every later set.
 */

/** Touched paths, or `null` for "unknown — assume anything". */
export type TouchedPaths = Set<string> | null;

/** Past this many accumulated paths, track "anything" instead. */
const MAX_PENDING = 256;

/** Fold one event's `paths` (absent = unknown) into the pending set. */
export function mergeTouched(
  cur: TouchedPaths | undefined,
  paths: readonly string[] | null | undefined,
): TouchedPaths {
  if (cur === null || paths == null) return null;
  const next = cur ?? new Set<string>();
  for (const p of paths) next.add(p);
  return next.size > MAX_PENDING ? null : next;
}

/** Does a change set touch `path`? A listed directory covers what is under it;
 *  `null`/`undefined` (unknown) touches everything. */
export function touches(changed: readonly string[] | null | undefined, path: string): boolean {
  if (changed == null) return true;
  return changed.some((p) => p === path || path.startsWith(`${p}/`));
}
