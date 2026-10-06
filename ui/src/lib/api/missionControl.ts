// Mission Control (work graph) API helpers. Read-mostly: a unified, traceable
// view of every agentic activity. Writes are human annotation (risk/goal/
// result), manual edges, approvals, and a re-derive backfill.

import { api } from './client';
import type {
  BackfillResp,
  GraphView,
  MissionFilterQuery,
  MissionSummary,
  WorkApproval,
  WorkEdge,
  WorkItem,
  WorkItemDetail,
} from './types';

function qs(f?: MissionFilterQuery): string {
  if (!f) return '';
  const p = new URLSearchParams();
  if (f.kind) p.set('kind', f.kind);
  if (f.status) p.set('status', f.status);
  if (f.risk) p.set('risk', f.risk);
  if (f.q && f.q.trim()) p.set('q', f.q.trim());
  if (f.limit != null) p.set('limit', String(f.limit));
  const s = p.toString();
  return s ? `?${s}` : '';
}

/** Home's Today panel and the Mission Control box both read the summary on a
 *  cold boot, a moment apart: one request serves both — shared while in
 *  flight and reused for SUMMARY_REUSE_MS after it settles. Every
 *  `work_graph_updated` event (and a resync) drops it, so a change is never
 *  served stale (`invalidateMissionSummary`, called by the event bus). */
const SUMMARY_REUSE_MS = 2_000;
const summaryCache = new Map<string, { p: Promise<MissionSummary>; at: number }>();

export function invalidateMissionSummary(ws?: string): void {
  if (ws) summaryCache.delete(ws);
  else summaryCache.clear();
}

export const missionControlApi = {
  /** Header summary: counts by kind/status/risk, total cost, active, needs-approval. */
  summary: (ws: string): Promise<MissionSummary> => {
    const hit = summaryCache.get(ws);
    // `at` is 0 while in flight; set when the request settles.
    if (hit && (hit.at === 0 || Date.now() - hit.at < SUMMARY_REUSE_MS)) return hit.p;
    const entry = { p: api.get<MissionSummary>(`/workspaces/${ws}/workgraph/summary`), at: 0 };
    summaryCache.set(ws, entry);
    entry.p.then(
      () => (entry.at = Date.now()),
      () => summaryCache.get(ws) === entry && summaryCache.delete(ws),
    );
    return entry.p;
  },

  /** Filtered work-item list, newest-updated first. */
  items: (ws: string, f?: MissionFilterQuery) =>
    api.get<WorkItem[]>(`/workspaces/${ws}/workgraph/items${qs(f)}`),

  /** Nodes + edges for the graph view. */
  graph: (ws: string, f?: MissionFilterQuery) =>
    api.get<GraphView>(`/workspaces/${ws}/workgraph/graph${qs(f)}`),

  /** Full detail of one item (events / artifacts / edges / approvals). */
  item: (ws: string, id: string) =>
    api.get<WorkItemDetail>(`/workspaces/${ws}/workgraph/items/${id}`),

  /** Annotate the human-governable fields (risk / goal / result). */
  patch: (ws: string, id: string, body: { risk_level?: string; goal?: string; result_summary?: string }) =>
    api.patch<WorkItem>(`/workspaces/${ws}/workgraph/items/${id}`, body),

  /** Manually link two items. */
  addEdge: (ws: string, id: string, body: { to_item_id: string; relation: string }) =>
    api.post<WorkEdge>(`/workspaces/${ws}/workgraph/items/${id}/edges`, body),

  /** Open a human-approval gate on an item. */
  requestApproval: (ws: string, id: string, body: { reason?: string }) =>
    api.post<WorkApproval>(`/workspaces/${ws}/workgraph/items/${id}/approvals`, body),

  /** Decide a pending approval. */
  decideApproval: (ws: string, aid: string, body: { decision: 'approved' | 'rejected'; note?: string }) =>
    api.post<WorkApproval>(`/workspaces/${ws}/workgraph/approvals/${aid}/decide`, body),

  /** Re-derive the graph from the source repos (idempotent). */
  backfill: (ws: string) => api.post<BackfillResp>(`/workspaces/${ws}/workgraph/backfill`),

  /** Stop an item through its OWNER's cancel endpoint (`stopPath` in
   *  modules/mission-control/sourceLinks.ts — goal loop stop, workflow-run
   *  cancel, review cancel). The projector picks the new status up from the
   *  owner's event; the work graph itself has no lifecycle of its own. */
  stopSource: (path: string) => api.post<unknown>(path, {}),

  /** Source lookups an "Open in <module>" needs before it can route. */
  sourceProject: (pid: string) =>
    api.get<{ id: string; swarm_id: string | null }>(`/projects/${encodeURIComponent(pid)}`),
  sourceWorkflowRun: (rid: string) =>
    api.get<{ id: string; workflow_id: string }>(`/workflow-runs/${encodeURIComponent(rid)}`),
  sourceReview: (rid: string) =>
    api.get<{ id: string; repo_id: string; pr_number: number }>(`/reviews/${encodeURIComponent(rid)}`),
};
