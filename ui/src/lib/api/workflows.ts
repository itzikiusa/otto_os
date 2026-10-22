// Workflows API client — thin typed wrappers over the generic `api` helper:
// the typed partial update, the version-history surface and run progress.
// They mirror crates/otto-server/src/routes/workflows.rs.

import { api } from './client';
import type { ActiveWorkflowRun, FsBrowse, FsRead, UpdateWorkflowReq, Workflow, WorkflowRun, WorkflowVersionSummary } from './types';

/** Partial update (`PATCH /workflows/{id}`): only the fields present change; an unknown
 *  `on_restart` value answers 400. Returns the saved workflow. */
export function patchWorkflow(id: string, body: UpdateWorkflowReq): Promise<Workflow> {
  return api.patch<Workflow>(`/workflows/${encodeURIComponent(id)}`, body);
}

/** In-flight workflow runs (pending|running) across a workspace, newest first.
 *  Backs the "Running" sidebar list. */
export function listActiveWorkflowRuns(workspaceId: string): Promise<ActiveWorkflowRun[]> {
  return api.get<ActiveWorkflowRun[]>(`/workspaces/${workspaceId}/workflow-runs/active`);
}

/** Re-enter a FINISHED run in place. Default: retry a single errored step.
 *  With `includeDownstream`, re-run the step AND everything after it — the
 *  "run from here keeping this run's files/worktree" flow (any settled step
 *  is a valid entry then). Fails with 409 while the run is still active; on
 *  success the returned run has flipped back to `running` (WS keeps it live). */
export function retryRunNode(
  runId: string,
  nodeId: string,
  includeDownstream = false,
): Promise<WorkflowRun> {
  return api.post<WorkflowRun>(`/workflow-runs/${runId}/retry-node`, {
    node_id: nodeId,
    include_downstream: includeDownstream,
  });
}

/** One metadata-only history page, newest first; beforeVersion is exclusive.
 * A full page may have an older page; an empty next page marks the end. */
export function listWorkflowVersions(id: string, options: {limit?: number; beforeVersion?: number} = {}): Promise<WorkflowVersionSummary[]> {
  const requested = options.limit ?? 50;
  const limit = Number.isFinite(requested) ? Math.min(100, Math.max(1, Math.trunc(requested))) : 50;
  const query = new URLSearchParams({summary: 'true', limit: String(limit)});
  if (options.beforeVersion !== undefined) query.set('before_version', String(options.beforeVersion));
  return api.get<WorkflowVersionSummary[]>(`/workflows/${encodeURIComponent(id)}/versions?${query}`);
}

/** Restore a version's graph as a new version; returns the updated workflow. */
export function restoreWorkflowVersion(id: string, v: number, note?: string): Promise<Workflow> {
  return api.post<Workflow>(`/workflows/${id}/versions/${v}/restore`, note ? { note } : {});
}


export function workflowProgress(runId: string, afterRev?: number, signal?: AbortSignal): Promise<import('./types').WorkflowProgressResponse> {
  return api.get(`/workflow-runs/${encodeURIComponent(runId)}/progress${afterRev === undefined ? '' : `?after_rev=${afterRev}`}`, signal);
}
export function workflowCheckpointPage(runId: string, cursor?: string): Promise<import('./types').WorkflowCheckpointPage> {
  return api.get(`/workflow-runs/${encodeURIComponent(runId)}/checkpoints${cursor ? `?cursor=${encodeURIComponent(cursor)}` : ''}`);
}
export function workflowNodeDetail(runId: string, nodeId: string, signal?: AbortSignal): Promise<import('./types').WorkflowDetail<import('./types').NodeRunState>> {
  return api.get(`/workflow-runs/${encodeURIComponent(runId)}/nodes/${encodeURIComponent(nodeId)}`, signal);
}
export function workflowCheckpointDetail(runId: string, nodeId: string): Promise<import('./types').WorkflowDetail<import('./types').WorkflowCheckpoint>> {
  return api.get(`/workflow-runs/${encodeURIComponent(runId)}/checkpoints/${encodeURIComponent(nodeId)}`);
}

/** List one directory of a run's context dir (`GET /workflow-runs/{id}/context/browse`).
 *  `path` is relative to the run's context dir ('' = its root); entry paths come back relative
 *  too, so pass them straight back. Same `FsBrowse` shape as `/fs/browse?files=true`, but gated
 *  at workspace Viewer on the run, so non-root members can open their runs' context files.
 *  400 for an absolute path, `..` or a symlink escaping the dir; 404 for an unknown run/path. */
export function browseRunContext(runId: string, path: string, signal?: AbortSignal): Promise<FsBrowse> {
  return api.get<FsBrowse>(
    `/workflow-runs/${encodeURIComponent(runId)}/context/browse?path=${encodeURIComponent(path)}`,
    signal,
  );
}

/** Read one file of a run's context dir (`GET /workflow-runs/{id}/context/read`), with the
 *  same `FsRead` shape and 400 KiB bound as `/fs/read`. The run's deliverable is
 *  `readRunContext(id, 'final-output.md')`. */
export function readRunContext(runId: string, path: string, signal?: AbortSignal): Promise<FsRead> {
  return api.get<FsRead>(
    `/workflow-runs/${encodeURIComponent(runId)}/context/read?path=${encodeURIComponent(path)}`,
    signal,
  );
}
