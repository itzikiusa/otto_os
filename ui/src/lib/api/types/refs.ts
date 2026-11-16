// Contract mirror: agent reference discovery (`/refs/directory`, `/refs/resolve`).
// Part of lib/api/types (see ../types.ts, the barrel).

import type { Id } from './core';

/** Kinds the agent discovery routes (`GET /refs/directory`, `GET /refs/resolve`)
 *  know — the same set every agent tool's friendly id argument resolves. */
export type AgentRefKind =
  | 'workspace'
  | 'workflow'
  | 'connection'
  | 'broker_cluster'
  | 'swarm'
  | 'scheduled_task'
  | 'goal_loop'
  | 'agent_room'
  | 'canvas_scene'
  | 'product_story'
  | 'vault'
  | 'api_request'
  | 'api_automation'
  | 'api_environment'
  | 'issue_account'
  | 'aws_account'
  | 'k8s_cluster'
  | 'design_artifact';

/** `GET /refs/directory?kind=&workspace_id=&prefer_workspace_id=` — every
 *  object of `kind` across the workspaces the caller can read (a token's
 *  workspace pin applied). Rows are the kind's own list rows (projected for
 *  heavy kinds — no workflow graph, no connection params); per-workspace kinds
 *  add `workspace_id`, `workspace_name` and `current`, current workspace first. */
export interface AgentRefDirectory {
  kind: AgentRefKind;
  items: Array<Record<string, unknown>>;
  current_workspace_id: Id | null;
  workspace_count: number;
}

/** `GET /refs/resolve?kind=&ref=&arg=&workspace_id=&prefer_workspace_id=` —
 *  one reference (id, or name / title / Jira key / label) resolved. 404 lists
 *  near misses or what IS available, 409 the ambiguous candidates (both in the
 *  Problem `message`). `matched_by` is `id`, the matched field name, or
 *  `only_candidate` (an omitted issue account with exactly one account). */
export interface AgentRefResolveResp {
  kind: AgentRefKind;
  id: string;
  label: string;
  workspace_id: Id | null;
  workspace_name: string | null;
  matched_by: string;
  item: Record<string, unknown>;
}
