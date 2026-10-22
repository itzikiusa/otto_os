// Contract mirror: mission-control. Part of lib/api/types (see ../types.ts, the barrel).
// Mirrors docs/contracts/api.md + ws.md — change them together.

import type {
  Id,
} from './core';

// ---------------------------------------------------------------------------
// Mission Control — the unified work graph (mirrors otto_state::workgraph).
// ---------------------------------------------------------------------------

export type WorkKind =
  | 'session'
  | 'swarm'
  | 'goal_loop'
  | 'workflow'
  | 'review'
  | 'product_story'
  | 'pr'
  | 'external_trigger'
  | 'otto_run';
export type WorkStatus =
  | 'pending'
  | 'running'
  | 'waiting'
  | 'blocked'
  | 'succeeded'
  | 'failed'
  | 'cancelled'
  | 'done';
export type WorkActor = 'user' | 'agent' | 'system' | 'integration';
export type RiskLevel = 'low' | 'medium' | 'high' | 'critical';
export type EdgeRelation =
  | 'spawned'
  | 'depends_on'
  | 'fixes'
  | 'reviews'
  | 'verifies'
  | 'blocks'
  | 'belongs_to';
export type ArtifactKind =
  | 'diff'
  | 'commit'
  | 'pr'
  | 'test_run'
  | 'report'
  | 'file'
  | 'link'
  | 'finding'
  | 'session';
export type ApprovalStatus = 'pending' | 'approved' | 'rejected';

export interface WorkItem {
  id: Id;
  workspace_id: Id;
  kind: WorkKind;
  source_id: string;
  title: string;
  goal: string | null;
  status: WorkStatus;
  owner: string | null;
  owner_kind: WorkActor;
  repo_id: string | null;
  branch: string | null;
  cost_so_far: number;
  risk_level: RiskLevel;
  result_summary: string | null;
  context_summary: string | null;
  started_by_id: string | null;
  last_event_at: string | null;
  created_at: string;
  updated_at: string;
}
export interface EdgeView {
  relation: EdgeRelation;
  direction: 'out' | 'in';
  peer_id: Id;
  peer_kind: WorkKind;
  peer_title: string;
  peer_status: WorkStatus;
}
export interface WorkEvent {
  id: Id;
  work_item_id: Id;
  workspace_id: Id;
  ts: string;
  actor: WorkActor;
  event_type: string;
  payload: unknown;
  created_at: string;
}
export interface WorkArtifact {
  id: Id;
  work_item_id: Id;
  workspace_id: Id;
  kind: ArtifactKind;
  title: string;
  ref: string | null;
  payload: unknown;
  created_at: string;
}
export interface WorkApproval {
  id: Id;
  work_item_id: Id;
  workspace_id: Id;
  status: ApprovalStatus;
  reason: string | null;
  requested_by: string;
  requested_at: string;
  decided_by: string | null;
  decided_at: string | null;
  decision_note: string | null;
}
export interface WorkItemDetail extends WorkItem {
  edges: EdgeView[];
  events: WorkEvent[];
  artifacts: WorkArtifact[];
  approvals: WorkApproval[];
  pending_approvals: number;
  needs_approval: boolean;
}
export interface WorkEdge {
  id: Id;
  workspace_id: Id;
  from_item_id: Id;
  to_item_id: Id;
  relation: EdgeRelation;
  created_at: string;
}
export interface CountBucket {
  key: string;
  count: number;
}
export interface MissionSummary {
  total: number;
  active: number;
  needs_approval: number;
  total_cost: number;
  by_kind: CountBucket[];
  by_status: CountBucket[];
  by_risk: CountBucket[];
}
export interface GraphNode {
  id: Id;
  kind: WorkKind;
  title: string;
  status: WorkStatus;
  risk_level: RiskLevel;
  cost_so_far: number;
  owner_kind: WorkActor;
  needs_approval: boolean;
}
export interface GraphEdge {
  from_item_id: Id;
  to_item_id: Id;
  relation: EdgeRelation;
}
export interface GraphView {
  nodes: GraphNode[];
  edges: GraphEdge[];
}
export interface BackfillResp {
  ok: boolean;
  summary: MissionSummary;
}
/** Query filters for the items / graph endpoints. */
export interface MissionFilterQuery {
  kind?: WorkKind;
  status?: WorkStatus;
  risk?: RiskLevel;
  q?: string;
  limit?: number;
}
