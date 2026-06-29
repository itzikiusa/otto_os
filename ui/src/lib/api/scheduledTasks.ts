// Scheduled Tasks API client — thin typed wrappers over the generic `api`
// helper. Mirrors docs/contracts/api.md (#135–#143).

import { api } from './client';
import type { ScheduledTask, ScheduledTaskPreset, ScheduledTaskRun } from './types';

export interface ScheduledTaskInput {
  name: string;
  prompt?: string;
  kind?: string;
  skill?: string | null;
  provider?: string;
  model?: string;
  cwd?: string;
  schedule?: Record<string, unknown>;
  destination?: Record<string, unknown>;
  enabled?: boolean;
  // v2
  timezone?: string;
  workflow_id?: string | null;
  sandbox?: string;
  max_retries?: number;
  notify_on_change?: boolean;
  attach_proof?: boolean;
}

export const scheduledTasksApi = {
  list: (ws: string) => api.get<ScheduledTask[]>(`/workspaces/${ws}/scheduled-tasks`),
  create: (ws: string, body: ScheduledTaskInput) =>
    api.post<ScheduledTask>(`/workspaces/${ws}/scheduled-tasks`, body),
  get: (id: string) => api.get<ScheduledTask>(`/scheduled-tasks/${id}`),
  update: (id: string, body: Partial<ScheduledTaskInput>) =>
    api.patch<ScheduledTask>(`/scheduled-tasks/${id}`, body),
  remove: (id: string) => api.del<{ ok: boolean }>(`/scheduled-tasks/${id}`),
  run: (id: string) => api.post<ScheduledTaskRun>(`/scheduled-tasks/${id}/run`, {}),
  runs: (id: string) => api.get<ScheduledTaskRun[]>(`/scheduled-tasks/${id}/runs`),
  presets: () => api.get<ScheduledTaskPreset[]>(`/scheduled-tasks/presets`),
  /** Materialize a scheduled task as a multi-step workflow (+ schedule trigger). */
  convertToWorkflow: (id: string, disable_task?: boolean) =>
    api.post<{ workflow_id: string; trigger_id?: string }>(
      `/scheduled-tasks/${id}/convert-to-workflow`,
      disable_task ? { disable_task } : {},
    ),
  /** The stored report path for a run (fetched as text/markdown). */
  reportPath: (runId: string) => `/scheduled-tasks/runs/${runId}/report`,
};
