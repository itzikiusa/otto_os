// Where a work item lives, and how to stop it. Mission Control is a projection:
// every item is backed by a row in its owning module (`source_id`), so "open"
// means going to that module's page on that row, and "stop" means calling the
// owner's own cancel endpoint — Mission Control never invents a lifecycle.
// Pure (no Svelte, no fetch) so the mapping is unit-testable.

import type { WorkKind, WorkStatus } from '../../lib/api/types';

/** The module a kind opens in — the button reads "Open in <label>". */
export const SOURCE_MODULE: Record<WorkKind, string> = {
  session: 'Agents',
  external_trigger: 'Agents',
  swarm: 'Swarm',
  goal_loop: 'Goal Loops',
  workflow: 'Workflows',
  review: 'Git',
  product_story: 'Product',
  pr: 'Git',
};

/** A `pr` item's source id is `<repo_id>:<pr_number>` (workgraph_projector's
 *  `pr_source_key`). Split on the LAST colon; null when it doesn't parse. */
export function prRoute(sourceId: string): string | null {
  const i = sourceId.lastIndexOf(':');
  if (i <= 0) return null;
  const repo = sourceId.slice(0, i);
  const n = sourceId.slice(i + 1);
  if (!/^\d+$/.test(n) || n === '0') return null;
  return `git/${encodeURIComponent(repo)}/pr/${n}`;
}

/** The route of a review's PR (or its repo, for a LOCAL review — pr 0). */
export function reviewRoute(repoId: string, prNumber: number): string {
  const repo = encodeURIComponent(repoId);
  return prNumber > 0 ? `git/${repo}/pr/${prNumber}` : `git/${repo}`;
}

/** Routes that need only the source id (no lookup). Kinds absent here need a
 *  fetch first (swarm → its swarm, workflow run → its workflow, review → repo). */
export function directRoute(kind: WorkKind, sourceId: string): string | null {
  const id = encodeURIComponent(sourceId);
  switch (kind) {
    case 'goal_loop':
      return `loops/${id}`;
    case 'product_story':
      return `product/${id}`;
    case 'pr':
      return prRoute(sourceId);
    default:
      return null;
  }
}

/** The owner's cancel endpoint for a kind, or null when the kind has no
 *  per-item stop (a session is stopped from its own pane; a swarm PROJECT has
 *  no stop of its own — aborting would stop the whole swarm; a PR / story isn't
 *  a running process). */
export function stopPath(kind: WorkKind, sourceId: string): string | null {
  const id = encodeURIComponent(sourceId);
  switch (kind) {
    case 'goal_loop':
      return `/goal-loops/${id}/stop`;
    case 'workflow':
      return `/workflow-runs/${id}/cancel`;
    case 'review':
      return `/reviews/${id}/cancel`;
    default:
      return null;
  }
}

/** Statuses a stop makes sense for (in flight, not settled). */
export function isActive(s: WorkStatus): boolean {
  return s === 'pending' || s === 'running' || s === 'waiting' || s === 'blocked';
}
