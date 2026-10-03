// Graph reload splice (G5). A background reload (a ref moved: a commit, a
// fetch) used to re-read `max(PAGE, loaded)` commits — after paging to 50k
// that was ~10 MB of JSON and a full relayout per throttled reload. History
// older than the reloaded first page is immutable, so the reload reads ONE
// page and splices it onto the tail the client already holds.
import type { CommitInfo } from '../../lib/api/types';

/** `page` (the fresh first page of `log --all --date-order`) + the part of
 *  `old` that follows the page's last commit, or `null` when a splice is not
 *  provably safe and the caller must re-read everything it had:
 *   - the page's last sha is not in `old` (history was rewritten under it);
 *   - some commit `old` held above that boundary is missing from the page
 *     (a branch was deleted / force-pushed within the page window);
 *   - a commit of the page also sits in the old tail (order changed). */
export function spliceHistory(old: readonly CommitInfo[], page: readonly CommitInfo[]): CommitInfo[] | null {
  if (page.length === 0 || old.length <= page.length) return null;
  const lastSha = page[page.length - 1].sha;
  let k = -1;
  for (let i = 0; i < old.length; i++) {
    if (old[i].sha === lastSha) {
      k = i;
      break;
    }
  }
  if (k < 0) return null;
  const inPage = new Set(page.map((c) => c.sha));
  for (let i = 0; i <= k; i++) if (!inPage.has(old[i].sha)) return null;
  for (let i = k + 1; i < old.length; i++) if (inPage.has(old[i].sha)) return null;
  return page.concat(old.slice(k + 1));
}
