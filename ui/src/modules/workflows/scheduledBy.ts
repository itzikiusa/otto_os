// The second way a workflow is scheduled: a Scheduled task of kind `workflow`
// launches it on the task's own cadence, and nothing in the workflow's Triggers
// panel said so ("why did it run at 9?"). Pure, so the filter, the label and the
// permission gate are unit-tested without a DOM (unit/scheduledByTasks.test.ts).
import type { Capability, Feature, ScheduledTask } from '../../lib/api/types';
import { cadenceLabel } from '../../lib/cadence';

export interface ScheduledByRow {
  id: string;
  name: string;
  /** "Every day", "Mondays at 09:00 · UTC"…, plus " · paused" when disabled. */
  cadence: string;
  /** Where the row links: the task, expanded, on the Scheduled tasks tab. */
  route: string;
}

/** The scheduled tasks that launch `workflowId`, in name order. */
export function scheduledByTasks(tasks: readonly ScheduledTask[], workflowId: string): ScheduledByRow[] {
  return tasks
    .filter((t) => t.kind === 'workflow' && !!workflowId && t.workflow_id === workflowId)
    .map((t) => ({
      id: t.id,
      name: t.name,
      cadence: cadenceLabel(t.schedule, t.timezone, { cronWords: true }) + (t.enabled ? '' : ' · paused'),
      route: `scheduled-tasks/${encodeURIComponent(t.id)}`,
    }))
    .sort((a, b) => a.name.localeCompare(b.name));
}

/** Only a person who can open Scheduled tasks is told about them (and the list
 *  is never fetched for anyone else). */
export function canSeeScheduledBy(can: (feature: Feature, required: Capability) => boolean): boolean {
  return can('scheduled_tasks', 'view');
}
