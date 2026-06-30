// Workflows API client — thin typed wrappers over the generic `api` helper for
// the version-history surface. (The core workflow CRUD/run calls are issued
// inline from WorkflowsPage via `api.*`; these mirror the versioning routes in
// crates/otto-server/src/routes/workflows.rs.)

import { api } from './client';
import type { ActiveWorkflowRun, Workflow, WorkflowVersion } from './types';

/** In-flight workflow runs (pending|running) across a workspace, newest first.
 *  Backs the "Running" sidebar list. */
export function listActiveWorkflowRuns(workspaceId: string): Promise<ActiveWorkflowRun[]> {
  return api.get<ActiveWorkflowRun[]>(`/workspaces/${workspaceId}/workflow-runs/active`);
}

/** Version history for a workflow (newest first). */
export function listWorkflowVersions(id: string): Promise<WorkflowVersion[]> {
  return api.get<WorkflowVersion[]>(`/workflows/${id}/versions`);
}

/** A single version snapshot. */
export function getWorkflowVersion(id: string, v: number): Promise<WorkflowVersion> {
  return api.get<WorkflowVersion>(`/workflows/${id}/versions/${v}`);
}

/** Restore a version's graph as a new version; returns the updated workflow. */
export function restoreWorkflowVersion(id: string, v: number, note?: string): Promise<Workflow> {
  return api.post<Workflow>(`/workflows/${id}/versions/${v}/restore`, note ? { note } : {});
}
