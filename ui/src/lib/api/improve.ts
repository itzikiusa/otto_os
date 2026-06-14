// Self-improvement API helpers: per-workspace config, runs, and the edit
// version log (approve / reject / rollback).

import { api } from './client';
import type {
  ImprovementEdit,
  ImprovementRun,
  RunNowResp,
  SelfImprovementConfig,
  UpdateSelfImprovementReq,
} from './types';

export const improveApi = {
  getConfig: (wsId: string) =>
    api.get<SelfImprovementConfig>(`/workspaces/${wsId}/self-improvement`),
  putConfig: (wsId: string, body: UpdateSelfImprovementReq) =>
    api.put<SelfImprovementConfig>(`/workspaces/${wsId}/self-improvement`, body),
  runNow: (wsId: string) =>
    api.post<RunNowResp>(`/workspaces/${wsId}/self-improvement/run`),
  listRuns: (wsId: string) =>
    api.get<ImprovementRun[]>(`/workspaces/${wsId}/improvement/runs`),
  getRun: (runId: string) =>
    api.get<{ run: ImprovementRun; edits: ImprovementEdit[] }>(
      `/improvement/runs/${runId}`,
    ),
  listEdits: (wsId: string, status = 'pending') =>
    api.get<ImprovementEdit[]>(
      `/workspaces/${wsId}/improvement/edits?status=${status}`,
    ),
  approve: (eid: string) =>
    api.post<ImprovementEdit>(`/improvement/edits/${eid}/approve`),
  reject: (eid: string) =>
    api.post<ImprovementEdit>(`/improvement/edits/${eid}/reject`),
  rollback: (eid: string) =>
    api.post<ImprovementEdit>(`/improvement/edits/${eid}/rollback`),
};
