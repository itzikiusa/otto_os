// A workflow run's context files go through the run-scoped, RBAC-checked
// routes (`/workflow-runs/{id}/context/{browse,read}`), whose paths are
// relative to the run's context dir — not through `/fs/*`, which refuses the
// Otto data dir to everyone but root.

/** A FileTree path → the run-context route's relative path. The tree is rooted
 *  at the absolute context dir (its header shows that folder), so that root is
 *  `''`, a path under it drops the prefix, and the route's own entries (already
 *  relative) pass through unchanged. */
export function runContextPath(contextDir: string, path: string): string {
  const root = contextDir.replace(/\/+$/, '');
  if (path === root || path === `${root}/`) return '';
  if (root && path.startsWith(`${root}/`)) return path.slice(root.length + 1);
  return path;
}
