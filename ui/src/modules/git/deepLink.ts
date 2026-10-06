// Deep links into a repo tab that carry a TARGET, not just a tab (S20-301):
// `#/git/<repo>/review/<review_id>` opens that review's findings read-only,
// `#/git/<repo>/graph/<branch>` jumps the graph to the branch tip. Run with
// Otto's approval gate links here so "Open findings" / "View branch diff"
// show THIS run's evidence instead of a generic repo page. Pure, so the
// parsing is unit-tested without mounting the panels.

/** The review id a `git/<repoId>/review/<id>` route points at (else null). */
export function routeReviewId(parts: readonly string[], repoId: string): string | null {
  if (parts[0] !== 'git' || parts[1] !== repoId || parts[2] !== 'review') return null;
  return parts[3] || null;
}

/** The branch a `git/<repoId>/graph/<branch>` route points at (else null). A
 *  branch name is one encoded segment (`otto%2Frun-x`) — the router decodes
 *  each segment — but an unencoded `a/b` link still resolves. */
export function routeGraphRef(parts: readonly string[], repoId: string): string | null {
  if (parts[0] !== 'git' || parts[1] !== repoId || parts[2] !== 'graph') return null;
  const ref = parts.slice(3).join('/');
  return ref || null;
}
