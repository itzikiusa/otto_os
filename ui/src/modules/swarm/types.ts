// Agent Swarm DTOs — section-local mirror of the Rust contract (otto_state::swarm
// + otto_swarm types). snake_case fields, ULID string ids.

export type Id = string;

export type SwarmStatus = 'active' | 'paused' | 'aborted';
export type AgentStatus = 'active' | 'paused';
export type TaskStatus =
  | 'backlog'
  | 'todo'
  | 'in_progress'
  | 'in_review'
  | 'blocked'
  | 'done'
  | 'cancelled';
export type TaskPriority = 'low' | 'medium' | 'high' | 'urgent';
export type RunStatus = 'queued' | 'running' | 'waiting' | 'done' | 'error' | 'stopped';
export type MessageKind =
  | 'message'
  | 'idea'
  | 'review_request'
  | 'review'
  | 'decision'
  | 'status'
  | 'concern'
  | 'escalation'
  | 'handoff'
  | 'system';

export interface SwarmConfig {
  provider?: string;
  model?: string | null;
  max_parallel_sessions?: number;
  cwd_mode?: string;
  default_soul?: string;
  auto_submit?: boolean;
  /** Naming theme for recruited agents (e.g. "Famous footballers"). */
  naming_theme?: string;
  [k: string]: unknown;
}

export interface Swarm {
  id: Id;
  workspace_id: Id;
  name: string;
  description: string;
  preset_slug?: string | null;
  status: SwarmStatus;
  config: SwarmConfig;
  /** Budget guardrails (all null = unlimited). Enforced by the Coordinator. */
  max_total_runs?: number | null;
  max_cost_usd?: number | null;
  max_runtime_secs?: number | null;
  /** Per-task attempt ceiling (default 3). */
  max_attempts: number;
  /** When the swarm last went active — anchors the runtime budget. */
  run_started_at?: string | null;
  /** Why the Coordinator auto-paused (budget/limit reason), else null. */
  pause_reason?: string | null;
  created_by: Id;
  created_at: string;
  updated_at: string;
}

export interface AgentSkill {
  name: string;
  must_use: boolean;
}

export interface AgentSchedule {
  cadence: 'interval' | 'daily' | 'weekly';
  every_min?: number;
  at?: string;
  weekday?: number;
  directive: string;
  enabled: boolean;
  last_run?: string;
}

export interface SwarmAgent {
  id: Id;
  swarm_id: Id;
  workspace_id: Id;
  name: string;
  title: string;
  reports_to?: Id | null;
  provider: string;
  model?: string | null;
  soul_name?: string | null;
  soul_md?: string | null;
  specialization: string;
  scope_md: string;
  skills: AgentSkill[];
  schedule?: AgentSchedule | null;
  cwd_mode?: string | null;
  avatar: string;
  status: AgentStatus;
  order_idx: number;
  created_by: Id;
  created_at: string;
  updated_at: string;
}

export interface SwarmProject {
  id: Id;
  swarm_id: Id;
  workspace_id: Id;
  name: string;
  description: string;
  repo_path?: string | null;
  goal_md?: string | null;
  /** Source Product story this project was created from (Plan → Swarm), or null. */
  story_id?: Id | null;
  status: 'active' | 'archived';
  order_idx: number;
  created_by: Id;
  created_at: string;
  updated_at: string;
}

export interface SwarmTask {
  id: Id;
  project_id: Id;
  swarm_id: Id;
  workspace_id: Id;
  title: string;
  description: string;
  assignee_agent_id?: Id | null;
  status: TaskStatus;
  priority: TaskPriority;
  parent_task_id?: Id | null;
  depends_on: Id[];
  labels: string[];
  result_ref?: string | null;
  delegated: boolean;
  /** How many turns the Coordinator has queued for this task (attempt ceiling). */
  attempts: number;
  order_idx: number;
  created_by: Id;
  created_at: string;
  updated_at: string;
}

export interface SwarmRun {
  id: Id;
  swarm_id: Id;
  workspace_id: Id;
  project_id?: Id | null;
  task_id?: Id | null;
  agent_id: Id;
  session_id?: Id | null;
  kind: string;
  trigger: string;
  status: RunStatus;
  attempt: number;
  summary?: string | null;
  result?: Record<string, unknown> | null;
  error?: string | null;
  tokens_input?: number | null;
  tokens_output?: number | null;
  cost_usd?: number | null;
  enqueued_at: string;
  started_at?: string | null;
  finished_at?: string | null;
}

/** One artifact a turn produced — a file, PR, doc, or arbitrary URL. */
export interface TurnArtifact {
  type: string; // 'file' | 'pr' | 'doc' | 'url'
  path?: string | null;
  url?: string | null;
  label: string;
}

export interface TurnHandoff {
  to_role: string;
  brief: string;
}

export interface TurnConcern {
  severity: string;
  text: string;
}

/**
 * The parsed structured result an agent turn writes (`SwarmRun.result`),
 * mirrored from Rust `SwarmTurnResult`. The daemon also folds in `cwd` + the
 * `brief` it sent (see swarm_run::enrich_result) so the Run Inspector can show
 * them without a dedicated endpoint. All fields optional — older runs and
 * failed turns carry partial objects.
 */
export interface TurnResult {
  status?: string;
  summary?: string;
  artifacts?: TurnArtifact[];
  handoffs?: TurnHandoff[];
  reviews?: unknown[];
  subtasks?: unknown[];
  concerns?: TurnConcern[];
  /** Absolute cwd / worktree path the turn ran in (added server-side). */
  cwd?: string;
  /** The brief/prompt that was sent to the agent (added server-side). */
  brief?: string;
  [k: string]: unknown;
}

export interface SwarmMessage {
  id: Id;
  swarm_id: Id;
  workspace_id: Id;
  project_id?: Id | null;
  task_id?: Id | null;
  run_id?: Id | null;
  author_agent_id?: Id | null;
  author_user_id?: Id | null;
  to_agent_id?: Id | null;
  kind: MessageKind;
  body: string;
  meta: Record<string, unknown>;
  created_at: string;
}

export interface SwarmCounts {
  agents: number;
  projects: number;
  tasks: number;
  running_runs: number;
  /** Total runs ever enqueued (basis for the max_total_runs budget). */
  total_runs: number;
  /** Accumulated backfilled spend in USD (basis for the max_cost_usd budget). */
  cost_usd: number;
}

export interface SwarmDetail extends Swarm {
  agents: SwarmAgent[];
  projects: SwarmProject[];
  counts: SwarmCounts;
}

export interface GraphNode {
  id: string;
  kind: 'task' | 'run';
  label: string;
  status: string;
  agent_id?: Id | null;
  session_id?: Id | null;
  project_id?: Id | null;
}

export interface GraphEdge {
  from: string;
  to: string;
  kind: 'depends' | 'handoff' | 'review';
}

export interface SwarmGraph {
  nodes: GraphNode[];
  edges: GraphEdge[];
}

export interface PresetAgent {
  key: string;
  name: string;
  title: string;
  reports_to?: string | null;
  provider: string;
  specialization: string;
}

export interface SwarmPreset {
  slug: string;
  name: string;
  description: string;
  max_parallel_sessions: number;
  agents: PresetAgent[];
}

export interface RecruitedSkill {
  name: string;
  must_use: boolean;
  why: string;
}

export interface RecruitedAgent {
  name: string;
  title: string;
  reports_to_title?: string | null;
  specialization: string;
  soul_md: string;
  scope_md: string;
  skills: RecruitedSkill[];
  suggested_provider: string;
  suggested_model?: string | null;
  suggested_schedule?: AgentSchedule | null;
  avatar: string;
}

export interface CreateAgentReq {
  name: string;
  provider: string;
  title?: string;
  reports_to?: Id | null;
  model?: string | null;
  soul_name?: string | null;
  soul_md?: string | null;
  specialization?: string;
  scope_md?: string;
  skills?: AgentSkill[];
  schedule?: AgentSchedule | null;
  cwd_mode?: string | null;
  avatar?: string;
  order_idx?: number;
}

export type UpdateAgentReq = Partial<CreateAgentReq> & { status?: AgentStatus };

export interface RunFilters {
  swarm_id?: Id;
  project_id?: Id;
  agent_id?: Id;
  status?: RunStatus;
}

export const TASK_COLUMNS: TaskStatus[] = [
  'backlog',
  'todo',
  'in_progress',
  'in_review',
  'blocked',
  'done',
];

// ---------------------------------------------------------------------------
// Swarm↔Product closure types  (GET /swarm/tasks/{tid}/story)
// ---------------------------------------------------------------------------

/** Back-link from a swarm task to the Product story that originated it.
 *  Returned by `GET /swarm/tasks/{tid}/story`. */
export interface TaskStoryLink {
  /** Source Product story, null when the task was not created from a story plan. */
  story: import('../product/types').ProductStory | null;
  /** Acceptance criteria from the task description (convenience field). */
  acceptance: string | null;
}
