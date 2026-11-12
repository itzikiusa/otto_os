// Did the PR move since the diff you read? (pure — unit/prMergeGate.test.ts).
// The merge modal pins the merge to the head it reads when it opens, which
// stops a push DURING the dialog — but not one that landed after you finished
// reading and before you clicked Merge. That head is compared here with the
// head the PR's diff view was loaded at; when they differ, Merge waits for an
// explicit "including changes I have not reviewed".

import type { PrCommit } from '../../lib/api/types';
import { plural } from '../../lib/plural.ts';

/** Same commit? Bitbucket abbreviates shas, so a prefix of the other counts. */
export function sameHead(a: string, b: string): boolean {
  const x = a.trim().toLowerCase();
  const y = b.trim().toLowerCase();
  if (x === '' || y === '') return false;
  return x.length <= y.length ? y.startsWith(x) : x.startsWith(y);
}

export interface UnseenHead {
  /** The head you would merge is not the head your diff shows. */
  moved: boolean;
  /** Commits after the one you read, when the commit list can say (null =
   *  unknown: not loaded yet, or the branch was rewritten). */
  count: number | null;
}

/**
 * `live` is the head the merge would pin, `reviewed` the head the diff view was
 * loaded at ('' / null = never loaded — nothing was read, so nothing to warn
 * about), `commits` the PR's commits as of `live` (any order).
 */
export function unseenHead(live: string | null, reviewed: string | null, commits: readonly PrCommit[] | null): UnseenHead {
  if (!live || !reviewed || sameHead(live, reviewed)) return { moved: false, count: null };
  if (!commits) return { moved: true, count: null };
  const t = (c: PrCommit): number => {
    const v = Date.parse(c.date);
    return Number.isNaN(v) ? 0 : v;
  };
  const sorted = [...commits].sort((a, b) => t(a) - t(b));
  const at = sorted.findIndex((c) => sameHead(c.sha, reviewed) || sameHead(c.short_sha, reviewed));
  // The commit you read isn't on the branch any more: a force-push rewrote it.
  if (at < 0) return { moved: true, count: null };
  const after = sorted.length - at - 1;
  return { moved: true, count: after > 0 ? after : null };
}

/** The warning block's headline. */
export function unseenText(u: UnseenHead): string {
  if (u.count === null) return 'New commits since you opened this diff';
  return `${plural(u.count, 'new commit')} since you opened this diff`;
}
