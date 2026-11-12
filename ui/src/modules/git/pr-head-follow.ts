// A push moved the PR's head while it was open (PrDetail's background poll).
// The file list and the diff FOLLOW the new head — they used to stay on the
// old one until a manual Refresh, while the PR list's stats and every lazily
// loaded file were already read at the new head (one PR, two heads on one
// screen). What changed is said in the header pill (PrHeadPill): how many
// commits arrived, once the commit list for the new head is in.
import { plural } from '../../lib/plural.ts';

interface Sha {
  sha: string;
}

/** The commits in `next` that `before` didn't hold; null when either list is
 *  unknown (a failed commits read must not claim "0 new commits"). A
 *  force-push that REWROTE history counts every rewritten commit as new —
 *  they are, to the reader. */
export function newCommitCount(before: readonly Sha[] | null, next: readonly Sha[] | null): number | null {
  if (!before || !next) return null;
  const seen = new Set(before.map((c) => c.sha));
  return next.reduce((n, c) => n + (seen.has(c.sha) ? 0 : 1), 0);
}

/** The pill text after the files followed a new head. */
export function headMovedLabel(count: number | null): string {
  if (count == null || count <= 0) return 'New commits · files updated';
  return `${plural(count, 'new commit')} · files updated`;
}

/** Whether the background copy `next` of the PR is at another head than the
 *  one on screen (`shown`; '' = not known yet, which never counts as moved). */
export function headMoved(shown: string | null | undefined, next: string | null | undefined): boolean {
  const a = (shown ?? '').trim();
  const b = (next ?? '').trim();
  return a !== '' && b !== '' && a !== b;
}
