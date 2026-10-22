// In-flight workflow runs (pending|running) for the current workspace: the
// "Running" sidebar list, the Workflows nav count chip and Today's run rows.
// Moved out of the shell's workspace store (architecture-2-03); `ws` keeps
// delegating members with the old names (`activeWorkflowRuns`,
// `refreshActiveWorkflowRuns`, `applyWorkflowRunEvent`) for existing callers.

import { listActiveWorkflowRuns } from '../api/workflows';
import type { ActiveWorkflowRun, Id } from '../api/types';

/** The `workflow_run_updated` fields the list needs. */
export interface WorkflowRunEvent {
  workspace_id: Id;
  run_id: Id;
  status: string;
  nodes_done?: number;
  nodes_total?: number;
  waiting_approval?: boolean;
}

const TERMINAL = new Set(['success', 'error', 'canceled']);

class WorkflowRunsStore {
  /** Runs of `workspaceId`, newest list from the daemon patched by WS events. */
  list: ActiveWorkflowRun[] = $state([]);
  /** The workspace `list` belongs to; a late response for another is dropped. */
  private workspaceId: Id | null = null;

  /** Forget the list (identity change, workspace switch, zero-workspace mode). */
  clear(workspaceId: Id | null = null): void {
    this.workspaceId = workspaceId;
    this.list = [];
  }

  /** Reload the runs of `workspaceId`. Cheap query; called on workspace switch
   *  and for a run the list does not know yet, so the sidebar list and nav
   *  count stay live without per-page polling. */
  async refresh(workspaceId: Id | null): Promise<void> {
    this.workspaceId = workspaceId;
    if (!workspaceId) {
      this.list = [];
      return;
    }
    try {
      const runs = await listActiveWorkflowRuns(workspaceId);
      // Guard against an out-of-order response after a workspace switch.
      if (this.workspaceId === workspaceId) this.list = runs;
    } catch {
      /* transient; the next event re-fetches */
    }
  }

  /** Apply a `workflow_run_updated` WS event IN PLACE: update a known run's
   *  status/progress/approval flag, drop it on a terminal status, and fall back
   *  to a full refetch only for runs the list doesn't know yet (or events from
   *  an older daemon without progress fields, signalled by `nodes_total` 0/absent). */
  apply(ev: WorkflowRunEvent, currentWorkspaceId: Id | null): void {
    if (ev.workspace_id !== currentWorkspaceId) return;
    const idx = this.list.findIndex((r) => r.run_id === ev.run_id);
    if (TERMINAL.has(ev.status)) {
      if (idx >= 0) this.list.splice(idx, 1);
      return;
    }
    if (idx < 0) {
      // A run the list doesn't know yet (started elsewhere) — one full refetch
      // brings it in with its workflow name.
      void this.refresh(currentWorkspaceId);
      return;
    }
    const r = this.list[idx];
    r.status = ev.status as ActiveWorkflowRun['status'];
    if (ev.nodes_total && ev.nodes_total > 0) {
      r.nodes_done = ev.nodes_done ?? r.nodes_done;
      r.nodes_total = ev.nodes_total;
    }
    if (ev.waiting_approval !== undefined) r.waiting_approval = ev.waiting_approval;
  }
}

export const workflowRuns = new WorkflowRunsStore();
