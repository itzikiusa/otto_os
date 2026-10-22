import { lazyModule } from '../../lazyModule';
import type { EventRoutes } from './types';

const loopsStore = lazyModule(() => import('../../stores/loops.svelte').then((m) => m.loops), 'loops');
const scheduledTasksStore = lazyModule(() => import('../../stores/scheduledTasks.svelte').then((m) => m.scheduledTasks), 'scheduledTasks');
const runWithOttoStore = lazyModule(() => import('../../stores/runWithOtto.svelte').then((m) => m.runWithOtto), 'runWithOtto');

export const automationRoutes: EventRoutes = {
  // Goal Loops: update the list row + bump the open detail's re-fetch tick.
  goal_loop_updated: (parsed) => loopsStore.peek()?.applyEvent(parsed),
  // Scheduled Tasks page refreshes the affected task's runs + list status.
  scheduled_task_run_updated: (parsed) => scheduledTasksStore.peek()?.applyEvent(parsed),
  // Run with Otto page refreshes the affected run + the workspace list.
  otto_run_updated: (parsed) => runWithOttoStore.peek()?.applyEvent(parsed),
};
