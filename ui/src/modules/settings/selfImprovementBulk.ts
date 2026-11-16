// Copy for the every-workspace self-improvement actions (Settings ›
// Self-improvement ⋯): the confirm dialogs and the result toasts.
import type { BulkImprovementResp, SelfImprovementSummary } from '../../lib/api/types';
import { plural } from '../../lib/plural.ts';

export function rejectAllMessage(s: SelfImprovementSummary): string {
  return (
    `Reject ${plural(s.pending, 'pending suggestion')} in ${plural(s.pending_workspaces, 'workspace')}? ` +
    'Nothing on disk changes. Edits already applied stay applied.'
  );
}

export function turnOffAllMessage(s: SelfImprovementSummary): string {
  return (
    `Turn off self-improvement in ${plural(s.enabled, 'workspace')}? ` +
    'Scheduled and live runs stop. Pending suggestions and applied edits stay. ' +
    'Turn it back on per workspace here.'
  );
}

/** "4 suggestions rejected in 2 workspaces. 1 workspace skipped (needs Editor)." */
export function bulkResultText(
  r: BulkImprovementResp,
  kind: 'reject' | 'off',
): string {
  const done =
    kind === 'reject'
      ? `${plural(r.changed, 'suggestion')} rejected in ${plural(r.workspaces, 'workspace')}.`
      : `Turned off in ${plural(r.changed, 'workspace')}.`;
  if (r.skipped === 0) return done;
  const role = kind === 'reject' ? 'Editor' : 'Admin';
  return `${done} ${plural(r.skipped, 'workspace')} skipped (needs ${role}).`;
}
