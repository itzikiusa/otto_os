// Goals API client (goal.md §7.2): a goal is a workflow the server builds from
// a short GoalSpec and starts at once. Pause / Resume / steer / verify are the
// run page's own calls (modules/workflows/runs/runActions.ts).
import { api } from './client';
import type { GoalSpec, GoalStartResp } from './types';

/** `POST /workspaces/{wid}/goals` → the goal's workflow and its first run. */
export function startGoal(workspaceId: string, spec: GoalSpec): Promise<GoalStartResp> {
  return api.post<GoalStartResp>(`/workspaces/${encodeURIComponent(workspaceId)}/goals`, spec);
}

/** `POST /goal-loops/{id}/continue`: an older goal loop goes on as a goal in its
 *  kept work (the server resolves the directory from the loop, never from us). */
export function continueLoopAsGoal(loopId: string, spec: GoalSpec): Promise<GoalStartResp> {
  return api.post<GoalStartResp>(`/goal-loops/${encodeURIComponent(loopId)}/continue`, spec);
}
