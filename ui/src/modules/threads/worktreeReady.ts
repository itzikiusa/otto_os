// The words for a new worktree's preparation (`POST /repos/{id}/worktrees`
// `prepared`, `…/worktrees/setup`). Pure (unit/threadsWorktreeReady.test.ts).
import type { WorktreePrepared, WorktreeSetupResult } from '../../lib/api/types';

/** What the person should know about what the worktree got, or null when
 *  everything named was copied or cloned. */
export function preparedNote(p: WorktreePrepared | null | undefined): string | null {
  const parts: string[] = [];
  if (p?.linked.length) parts.push(`Linked to the main checkout (this disk can’t clone): ${p.linked.join(', ')}`);
  if (p?.skipped.length) parts.push(`Left out: ${p.skipped.join('; ')}`);
  return parts.length ? parts.join(' · ') : null;
}

/** A failed setup in one line: its name, exit code and last output line. */
export function setupFailure(r: WorktreeSetupResult): string {
  const last = r.output.trim().split('\n').pop()?.trim() ?? '';
  return `Setup “${r.name}” failed${r.exit_code !== null ? ` (exit ${r.exit_code})` : ''}${last ? `: ${last}` : ''}`;
}
