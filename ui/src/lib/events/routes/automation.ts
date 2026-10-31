import { lazyModule } from '../../lazyModule';
import type { EventRoutes } from './types';

const loopsStore = lazyModule(() => import('../../stores/loops.svelte').then((m) => m.loops), 'loops');
const scheduledTasksStore = lazyModule(() => import('../../stores/scheduledTasks.svelte').then((m) => m.scheduledTasks), 'scheduledTasks');

export const automationRoutes: EventRoutes = {
  // Goal Loops: update the list row + bump the open detail's re-fetch tick.
  goal_loop_updated: (parsed) => loopsStore.peek()?.applyEvent(parsed),
  // Scheduled Tasks page refreshes the affected task's runs + list status.
  scheduled_task_run_updated: (parsed) => scheduledTasksStore.peek()?.applyEvent(parsed),
};

/** Reconnect / daemon-restart resync (data-integrity-5-02). Scheduled Tasks is
 *  fed only by run events, so a run the boot reaper marked interrupted while
 *  no client was connected stayed "running" until the next event. Only a
 *  store someone already loaded (`peek()`) refetches, for the current
 *  workspace: reloads the list and the runs of every task whose history is
 *  loaded (expanded). */
export function resyncAutomation(workspaceId: string | null): void {
  if (!workspaceId) return;
  const tasks = scheduledTasksStore.peek();
  if (tasks) {
    void tasks.loadList(workspaceId);
    for (const taskId of Object.keys(tasks.runsByTask)) void tasks.loadRuns(taskId);
  }
}
