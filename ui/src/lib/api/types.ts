// TS mirror of crates/otto-core (domain.rs + api.rs). Keep in lockstep — the
// Rust side is the source of truth (docs/contracts/api.md).

export type Id = string;

// ---------------------------------------------------------------------------
// Domain
// ---------------------------------------------------------------------------

export type GlobalRole = 'root' | 'member';
export type WorkspaceRole = 'viewer' | 'editor' | 'admin';

export interface User {
  id: Id;
  username: string;
  display_name: string;
  is_root: boolean;
  disabled: boolean;
  created_at: string;
}

export interface Workspace {
  id: Id;
  name: string;
  root_path: string;
  settings: Record<string, unknown>;
  archived: boolean;
  created_at: string;
}

export type SessionKind = 'agent' | 'connection';
export type SessionStatus = 'running' | 'working' | 'idle' | 'exited' | 'reconnectable';

// ---------------------------------------------------------------------------
// WS /ws/term/{id} — frame types (docs/contracts/ws.md)
// ---------------------------------------------------------------------------

/** One match returned by a server-side ring-buffer search. */
export interface TermSearchMatch {
  /** Absolute ring-buffer line index (oldest = 0, newest = ringLen-1). */
  line: number;
  /** ANSI-stripped plain text of the matching line. */
  text: string;
}

/** `{"type":"search_result"}` server→client frame (response to a Search frame). */
export interface WsSearchResultFrame {
  type: 'search_result';
  /** The query string that was searched. */
  query: string;
  /** Up to 200 matching lines, oldest → newest. */
  matches: TermSearchMatch[];
}

export interface Session {
  id: Id;
  workspace_id: Id;
  kind: SessionKind;
  provider: string;
  title: string;
  status: SessionStatus;
  cwd: string;
  provider_session_id: string | null;
  connection_id: Id | null;
  created_by: Id;
  created_at: string;
  last_active_at: string;
  archived: boolean;
  meta: Record<string, unknown>;
}

export type ConnectionKind = 'ssh' | 'mysql' | 'redis' | 'mongodb' | 'clickhouse' | 'custom';

/** Deployment environment. `prod` is write-guarded in the DB Explorer. */
export type Environment = 'dev' | 'staging' | 'prod';

export interface Connection {
  id: Id;
  workspace_id: Id | null;
  name: string;
  kind: ConnectionKind;
  params: Record<string, unknown>;
  secret_ref: string | null;
  first_command: string | null;
  section_id: Id | null;
  /** Deployment environment (defaults to 'dev'). */
  environment: Environment;
  /** When true, writes/DDL are refused without an explicit confirmation. */
  read_only: boolean;
  created_by: Id;
  created_at: string;
  /** ISO timestamp of the last open; absent means never opened. */
  last_opened_at?: string | null;
  /** When true the connection floats to the top of the list regardless of recency. */
  pinned?: boolean;
}

export interface ConnectionSection {
  id: Id;
  workspace_id: Id;
  parent_id: Id | null;
  name: string;
  position: number;
  /** Which tree this section belongs to: 'connections' (Connections page) or 'db' (DB Explorer). */
  scope: string;
  created_by: Id;
  created_at: string;
}

/** A user-configured MCP server for a workspace. Enabled servers are merged into
 *  the workspace's `.mcp.json` (alongside Otto's managed entries) on agent spawn.
 *  Never auto-enabled. */
export interface McpServer {
  id: Id;
  workspace_id: Id;
  /** Key under `.mcp.json`'s mcpServers map; unique within the workspace. */
  name: string;
  command: string;
  args: string[];
  /** Extra env passed to the server. Stored in plaintext for now (like `.mcp.json`). */
  env: Record<string, string>;
  /** Off by default — only written to `.mcp.json` once enabled. */
  enabled: boolean;
  created_by: Id;
  created_at: string;
  updated_at: string;
}

export interface CreateMcpServerReq {
  name: string;
  command: string;
  args?: string[];
  env?: Record<string, string>;
  enabled?: boolean;
}

export interface UpdateMcpServerReq {
  name?: string;
  command?: string;
  args?: string[];
  env?: Record<string, string>;
  enabled?: boolean;
}

// ---------------------------------------------------------------------------
// MCP Control Plane — mirrors crates/otto-state/src/mcp_control.rs +
// crates/otto-mcp/src/types.rs (wire JSON is snake_case). The legacy McpServer /
// CreateMcpServerReq / UpdateMcpServerReq above stay intact — they drive the
// `.mcp.json` config CRUD. The richer governed registry uses the DTOs below.
// ---------------------------------------------------------------------------

export type McpTransport = 'stdio' | 'http';
export type McpRiskLabel = 'read' | 'write' | 'dangerous' | 'unknown';
export type McpInjectionRisk = 'low' | 'medium' | 'high';
export type McpHealthStatus = 'unknown' | 'healthy' | 'unhealthy' | 'disabled';
export type McpToolAccess = 'allow' | 'deny';
export type McpPolicyEffect = 'allow' | 'deny' | 'require_approval' | 'require_dry_run';
/** `decision` on a governed invoke / audit row. */
export type McpDecision =
  | 'allowed'
  | 'approved'
  | 'denied'
  | 'dry_run'
  | 'pending_approval'
  | 'error';
export type McpApprovalStatus =
  | 'pending'
  | 'approved'
  | 'denied'
  | 'expired'
  | 'cancelled'
  | 'consumed';

/** Full registry view of a control-plane MCP server (augmented `mcp_servers`
 *  row). Secret VALUES are never returned — only the key-name lists + has_secret. */
export interface McpServerDetail {
  id: Id;
  workspace_id: Id;
  name: string;
  transport: McpTransport;
  command: string;
  args: string[];
  env: Record<string, string>;
  url: string | null;
  description: string | null;
  headers: Record<string, string>;
  secret_env_keys: string[];
  secret_header_keys: string[];
  has_secret: boolean;
  injection_risk: McpInjectionRisk;
  managed: boolean;
  default_tool_access: McpToolAccess;
  enabled: boolean;
  health_status: McpHealthStatus;
  health_checked_at: string | null;
  health_latency_ms: number | null;
  health_error: string | null;
  tools_count: number;
  tools_discovered_at: string | null;
  created_by: Id;
  created_at: string;
  updated_at: string;
}

/** A discovered tool + its governance metadata (`McpTool` in Rust). */
export interface McpToolView {
  id: Id;
  server_id: Id;
  name: string;
  title: string | null;
  description: string | null;
  input_schema: unknown;
  annotations: unknown;
  risk_label: McpRiskLabel;
  injection_risk: McpInjectionRisk;
  mutating: boolean;
  supports_dry_run: boolean;
  enabled: boolean;
  require_approval: boolean;
  risk_overridden: boolean;
  created_at: string;
  updated_at: string;
}

/** `GET /mcp/servers/{id}` body. */
export interface McpServerWithTools {
  server: McpServerDetail;
  tools: McpToolView[];
}

/** `POST /workspaces/{ws}/mcp/servers` — secret_* values go to the keychain and
 *  are never returned (only their key names show up as secret_*_keys). */
export interface CreateMcpControlServerReq {
  name: string;
  transport?: McpTransport;
  command?: string;
  args?: string[];
  env?: Record<string, string>;
  url?: string | null;
  description?: string | null;
  headers?: Record<string, string>;
  secret_env?: Record<string, string>;
  secret_headers?: Record<string, string>;
  injection_risk?: McpInjectionRisk;
  default_tool_access?: McpToolAccess;
  enabled?: boolean;
}

/** `PATCH /mcp/servers/{id}` — all fields optional; absent = keep current. */
export interface UpdateMcpControlServerReq {
  name?: string;
  description?: string | null;
  command?: string;
  args?: string[];
  env?: Record<string, string>;
  url?: string | null;
  headers?: Record<string, string>;
  secret_env?: Record<string, string>;
  secret_headers?: Record<string, string>;
  injection_risk?: McpInjectionRisk;
  default_tool_access?: McpToolAccess;
  enabled?: boolean;
}

/** `PATCH /mcp/tools/{toolId}` — per-tool permission + risk override. */
export interface PatchMcpToolReq {
  enabled?: boolean;
  require_approval?: boolean;
  risk_label?: McpRiskLabel;
  injection_risk?: McpInjectionRisk;
}

/** `POST /mcp/servers/{id}/tools/{name}/invoke` body. */
export interface McpInvokeReq {
  arguments?: unknown;
  dry_run?: boolean;
  workspace_id?: string | null;
}

/** The governed-invoke result (`InvokeResp` in Rust). */
export interface McpInvokeResp {
  decision: McpDecision;
  executed: boolean;
  dry_run: boolean;
  reason?: string | null;
  approval_id?: string | null;
  /** Redacted, capped tool result content when executed. */
  content?: unknown;
  is_error?: boolean | null;
  /** The dry-run preview when `dry_run`. */
  preview?: unknown;
}

export interface McpAllowlistEntry {
  id: Id;
  workspace_id: Id;
  server_id: Id;
  /** null = whole server. */
  tool_name: string | null;
  mode: McpToolAccess;
  created_by: Id;
  created_at: string;
}

/** One entry in the bulk allowlist PUT body. */
export interface McpAllowlistEntryInput {
  server_id: Id;
  tool_name?: string | null;
  mode: McpToolAccess;
}

/** `PUT /workspaces/{ws}/mcp/allowlist`. */
export interface SetMcpAllowlistReq {
  entries: McpAllowlistEntryInput[];
}

/** A policy-as-code rule (`match` is an arbitrary matcher object). */
export interface McpPolicy {
  id: Id;
  /** null = global. */
  workspace_id: Id | null;
  name: string;
  enabled: boolean;
  priority: number;
  match: unknown;
  effect: McpPolicyEffect;
  reason: string | null;
  created_by: Id;
  created_at: string;
  updated_at: string;
}

export interface CreateMcpPolicyReq {
  workspace_id?: string | null;
  name: string;
  enabled?: boolean;
  priority?: number;
  match?: unknown;
  effect: McpPolicyEffect;
  reason?: string | null;
}

export interface UpdateMcpPolicyReq {
  name?: string;
  enabled?: boolean;
  priority?: number;
  match?: unknown;
  effect?: McpPolicyEffect;
  reason?: string | null;
}

/** `GET /mcp/policies/export`. */
export interface McpPolicyExport {
  version: number;
  policies: McpPolicy[];
}

/** `POST /mcp/policies/import`. */
export interface ImportMcpPoliciesReq {
  policies: CreateMcpPolicyReq[];
  replace?: boolean;
}

export interface ImportMcpPoliciesResp {
  imported: number;
  replaced: boolean;
}

/** `POST /mcp/policies/evaluate` request. */
export interface EvaluateMcpReq {
  server_id: string;
  tool: string;
  workspace_id?: string | null;
  arguments?: unknown;
}

/** `POST /mcp/policies/evaluate` response (preview). */
export interface McpEvaluatePreview {
  server: string;
  tool: string;
  risk_label: string;
  injection_risk: string;
  policy_decision: string;
  reason: string | null;
}

export interface McpApproval {
  id: Id;
  workspace_id: string | null;
  /** 'tool_call' | 'human_ask'. */
  kind: string;
  server_id: string | null;
  server_name: string | null;
  tool: string | null;
  title: string;
  detail: string | null;
  /** Redacted display copy of the arguments (never the full/secret values). */
  args_redacted_json: string;
  risk_label: string | null;
  status: McpApprovalStatus;
  requested_by: string | null;
  requested_by_kind: string | null;
  decided_by: string | null;
  decision_note: string | null;
  created_at: string;
  decided_at: string | null;
  consumed_at: string | null;
  expires_at: string | null;
}

/** `POST /mcp/approvals/{id}/decide`. */
export interface DecideMcpApprovalReq {
  approved: boolean;
  note?: string | null;
}

/** One row of the governed-call audit ledger. */
export interface McpCallLogRow {
  id: Id;
  workspace_id: string | null;
  server_id: string | null;
  server_name: string | null;
  tool: string;
  /** 'outbound' | 'inbound'. */
  direction: string;
  caller_user_id: string | null;
  caller_kind: string | null;
  args_redacted_json: string;
  decision: string;
  decision_reason: string | null;
  risk_label: string | null;
  injection_risk: string | null;
  dry_run: boolean;
  ok: boolean;
  error: string | null;
  latency_ms: number | null;
  bytes: number | null;
  rows: number | null;
  approval_id: string | null;
  created_at: string;
}

/** Filters for `GET /mcp/audit`. */
export interface McpAuditQuery {
  server_id?: string;
  tool?: string;
  decision?: string;
  limit?: number;
  offset?: number;
}

/** Per-tool aggregate stats (cost = bytes proxy; latency / error counts). */
export interface McpToolStats {
  server_id: string | null;
  server_name: string | null;
  tool: string;
  calls: number;
  errors: number;
  error_rate: number;
  avg_latency_ms: number;
  max_latency_ms: number;
  total_bytes: number;
  avg_bytes: number;
  last_called_at: string | null;
}

// --- Otto-as-MCP-server (outward server admin) -----------------------------
// Response shape per the design contract (§7/§10). The daemon admin route is
// not yet wired into the otto-mcp router, so this DTO mirrors the documented
// contract rather than a Rust struct.

export interface McpOttoToolInfo {
  name: string;
  description: string;
  mutating: boolean;
  enabled: boolean;
  /** Feature group (e.g. "Workflows", "Message Brokers") for UI grouping. Optional
   * for forward-compat with daemons that predate the categorised catalog. */
  category?: string | null;
}

/** `GET /mcp/otto-server` (+ `PATCH` reply, which may also carry `token` once). */
export interface McpOttoServerStatus {
  enabled: boolean;
  tools: McpOttoToolInfo[];
  has_token: boolean;
  token_prefix?: string | null;
  /** The freshly-minted token — returned ONCE on a mint/rotate, never again. */
  token?: string | null;
}

/** `PATCH /mcp/otto-server`. */
export interface UpdateMcpOttoServerReq {
  enabled?: boolean;
  tools?: string[];
  rotate_token?: boolean;
}

export type GitProviderKind = 'github' | 'bitbucket' | 'gitlab';

export interface GitAccount {
  id: Id;
  user_id: Id;
  provider: GitProviderKind;
  label: string;
  username: string;
  token_ref: string;
  api_base_url: string | null;
  /** Org / workspace / group to browse repos under (e.g. "your-org"). */
  namespace: string | null;
  /** Token expiry (auto-detected for GitHub/GitLab, user-entered for Bitbucket); null = unknown/none. */
  token_expires_at: string | null;
  created_at: string;
}

/** A repository in a git account's namespace, returned by remote-repos search. */
export interface RemoteRepoSummary {
  full_name: string;
  name: string;
  clone_url: string;
  ssh_url: string;
  description: string;
  private: boolean;
  updated_at: string;
}

export interface Repo {
  id: Id;
  workspace_id: Id;
  name: string;
  path: string;
  remote_url: string | null;
  provider: GitProviderKind | null;
  git_account_id: Id | null;
  created_at: string;
}

// ---------------------------------------------------------------------------
// Agent activity (live trail + task tracker) — mirrors otto_core::domain
// ---------------------------------------------------------------------------

export type TrailSource = 'user' | 'agent' | 'otto';
export type TrailKind =
  | 'session'
  | 'prompt'
  | 'skill'
  | 'command'
  | 'tool'
  | 'file'
  | 'web'
  | 'task'
  | 'note'
  | 'other';

/** One entry in a session's live activity trail. */
export type TrailLevel = 'info' | 'warn' | 'error';

export interface TrailEvent {
  id: Id;
  session_id: Id;
  workspace_id: Id;
  ts: string;
  source: TrailSource;
  kind: TrailKind;
  level: TrailLevel;
  summary: string;
  detail: unknown | null;
}

/** Per-session task roll-up for the multi-agent overview (sidebar chips). */
export interface SessionActivitySummary {
  session_id: Id;
  total: number;
  done: number;
  in_progress: string | null;
  last_ts: string | null;
}

export type TaskStatus = 'pending' | 'in_progress' | 'completed' | 'blocked' | 'cancelled';

/** One task in a session's normalized task tracker. */
export interface AgentTask {
  id: Id;
  session_id: Id;
  workspace_id: Id;
  ext_id: string | null;
  title: string;
  status: TaskStatus;
  position: number;
  created_at: string;
  updated_at: string;
}

export interface AppendTrailReq {
  source?: TrailSource;
  kind?: TrailKind;
  summary: string;
  detail?: unknown;
}

export interface TaskInput {
  ext_id?: string | null;
  title: string;
  status: TaskStatus;
}

export interface PutTasksReq {
  tasks: TaskInput[];
}

// ---------------------------------------------------------------------------
// Events (WS /ws/events)
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Goal Loops — mirror of crates/otto-core domain + api DTOs.
// ---------------------------------------------------------------------------

export type GoalLoopStatus =
  | 'draft'
  | 'running'
  | 'paused'
  | 'blocked'
  | 'succeeded'
  | 'exhausted'
  | 'failed'
  | 'stopped';

export type GoalLoopPhase =
  | 'planning'
  | 'executing'
  | 'evaluating'
  | 'digesting'
  | 'waiting'
  | 'done';

export interface AcceptanceCriterion {
  id: string;
  text: string;
  verify: string;
  verify_kind: 'command' | 'manual';
  verify_cmd?: string | null;
}

export interface GoalLoopDefinition {
  title: string;
  summary: string;
  objectives: string[];
  acceptance_criteria: AcceptanceCriterion[];
  constraints: string[];
  out_of_scope: string[];
  success_signal: string;
}

export interface GoalLoopLimits {
  max_iterations: number;
  max_runtime_secs: number;
  per_phase_timeout_secs: number;
  max_cost_usd?: number | null;
  max_attempts_per_executor: number;
}

export interface GoalLoopAgentCfg {
  name: string;
  provider: string;
  model: string;
  prompt_extra: string;
}

export interface GoalLoopRoleCfg {
  provider: string;
  model: string;
  prompt: string;
}

export interface GoalLoopConfig {
  executors: GoalLoopAgentCfg[];
  planner: GoalLoopRoleCfg;
  evaluator: GoalLoopRoleCfg;
  digester: GoalLoopRoleCfg;
  definer: GoalLoopRoleCfg;
}

export interface LoopAgentState {
  name: string;
  provider: string;
  model: string;
  status: 'pending' | 'running' | 'waiting' | 'done' | 'error';
  note: string;
  session_id?: string | null;
  output_summary?: string | null;
}

export interface EvalCriterion {
  id: string;
  met: boolean;
  evidence: string;
}

export interface GoalLoopEvaluation {
  progress_pct: number;
  verdict: 'achieved' | 'continue' | 'blocked';
  criteria: EvalCriterion[];
  feedback: string;
  rationale: string;
}

export interface GoalLoop {
  id: Id;
  workspace_id: Id;
  name: string;
  repo_path: string;
  definition: GoalLoopDefinition;
  limits: GoalLoopLimits;
  config: GoalLoopConfig;
  status: GoalLoopStatus;
  phase: GoalLoopPhase;
  iterations_started: number;
  current_iteration: number;
  progress_pct: number;
  context_digest: string;
  branch?: string | null;
  worktree_path?: string | null;
  base_commit?: string | null;
  summary?: string | null;
  error?: string | null;
  run_started_at?: string | null;
  elapsed_secs: number;
  cost_usd: number;
  created_by: Id;
  created_at: string;
  updated_at: string;
  finished_at?: string | null;
}

export interface GoalLoopIteration {
  id: Id;
  loop_id: Id;
  workspace_id: Id;
  idx: number;
  status: string;
  plan: string;
  agents: LoopAgentState[];
  evaluation?: GoalLoopEvaluation | null;
  context_in: string;
  context_out: string;
  tokens_input: number;
  tokens_output: number;
  cost_usd: number;
  started_at: string;
  finished_at?: string | null;
}

export interface GoalLoopDetail {
  loop: GoalLoop;
  iterations: GoalLoopIteration[];
}

export interface DefineGoalReq {
  seed: string;
  repo_path: string;
  context?: string | null;
  feedback?: string | null;
}

export interface GoalLoopDraft {
  definition: GoalLoopDefinition;
  suggested_limits: GoalLoopLimits;
  suggested_config: GoalLoopConfig;
}

export interface CreateGoalLoopReq {
  name: string;
  repo_path: string;
  definition: GoalLoopDefinition;
  limits: GoalLoopLimits;
  config: GoalLoopConfig;
  autostart: boolean;
}

export interface UpdateGoalLoopReq {
  name?: string;
  limits?: GoalLoopLimits;
  config?: GoalLoopConfig;
}

export type OttoEvent =
  | { type: 'session_status'; session_id: Id; workspace_id: Id; status: SessionStatus }
  | { type: 'session_created'; session: Session }
  | {
      type: 'session_meta_updated';
      session_id: Id;
      workspace_id: Id;
      meta: Record<string, unknown>;
    }
  | { type: 'session_removed'; session_id: Id; workspace_id: Id }
  | { type: 'notice'; level: 'info' | 'warn' | 'error'; title: string; body: string }
  | { type: 'notification'; notice: Notice; user_id?: string | null }
  | { type: 'trail_appended'; workspace_id: Id; session_id: Id; event: TrailEvent }
  | { type: 'tasks_updated'; workspace_id: Id; session_id: Id; tasks: AgentTask[] }
  | { type: 'swarm_run_updated'; workspace_id: Id; swarm_id: Id; run: Record<string, unknown> }
  | {
      type: 'swarm_task_updated';
      workspace_id: Id;
      swarm_id: Id;
      project_id: Id;
      task: Record<string, unknown>;
    }
  | {
      type: 'swarm_message_posted';
      workspace_id: Id;
      swarm_id: Id;
      message: Record<string, unknown>;
    }
  | {
      type: 'swarm_goal_updated';
      workspace_id: Id;
      swarm_id: Id;
      task_id?: Id | null;
      goal: Record<string, unknown>;
    }
  | { type: 'swarm_status'; workspace_id: Id; swarm_id: Id; status: string }
  /** Emitted after each metrics-sampler tick so the dashboard can refresh
   *  sparklines in near-real-time. `ts` is the sample timestamp (UTC ISO-8601).
   *  A9 — Usage/Insights cluster. */
  | { type: 'usage_metrics_tick'; ts: string }
  /** A PR review run changed status (started, agent-done, finished, error).
   *  A2 — Git/Review cluster. Supplement for visibility-gated poll fallback. */
  | {
      type: 'review_changed';
      workspace_id: Id;
      session_id?: Id | null;
      review_id: Id;
      status: string;
    }
  /** A goal loop advanced (status/phase/iteration change, after each evaluation,
   *  or when an executor's live state flips). Drives the Loops UI re-fetch. */
  | {
      type: 'goal_loop_updated';
      workspace_id: Id;
      loop_id: Id;
      status: GoalLoopStatus;
      phase: GoalLoopPhase;
      current_iteration: number;
      progress_pct: number;
    }
  /** A product story AI run (analysis/rewrite/plan/testcases) completed or
   *  changed. `section` ∈ "analysis" | "rewrite" | "plan" | "testcases".
   *  `status` mirrors run status ("done" | "error" | "partial").
   *  A3 — Product cluster. */
  | {
      type: 'product_changed';
      workspace_id: Id;
      story_id: Id;
      section: string;
      status: string;
    }
  /** A multi-agent plan generation kicked off N visible planning sessions (and,
   *  with >1 planner, a summarizer). The Plan tab tiles `session_ids`
   *  side-by-side so the user can watch them (and answer questions when
   *  `interactive`). Ids are in spawn order; the summarizer is appended when it
   *  starts. A3 — Product cluster. */
  | {
      type: 'plan_run';
      workspace_id: Id;
      story_id: Id;
      session_ids: Id[];
      interactive: boolean;
    }
  /** A self-improvement run finished or an approval became pending. Lets the
   *  Self-Improvement settings pane refresh on the event instead of guessing.
   *  `kind` is "run_finished" | "approval_pending".
   *  A8 — Skills/Improve/Channels cluster. */
  | { type: 'improvement_updated'; kind: string; id?: string | null }
  /** A workflow run advanced: a node started, finished (or was served from
   *  cache), or the run reached a terminal status. `node_id` is present when a
   *  specific node changed; absent when the overall run changed (start/terminal).
   *  A11 — Workflows cluster. Supplement for a capped 700ms poll fallback. */
  | {
      type: 'workflow_run_updated';
      workspace_id: Id;
      run_id: Id;
      status: string;
      node_id?: Id | null;
    }
  /** A skill-evaluation run reached a terminal state (done/error/cancelled).
   *  Lets the Skill-Eval UI replace its 2s×600 fixed poll with event-driven
   *  refresh. A11 — Workflows cluster. */
  | { type: 'skill_eval_updated'; workspace_id: Id; run_id: Id; status: string }
  /** An insights report became available for a cadence period.
   *  `period` is a human label ("daily 2026-06-20", "weekly 2026-W25").
   *  B8 — Skills/Improve/Channels. */
  | { type: 'insight_ready'; period: string; session_id?: Id | null }
  /** A usage budget cap was crossed (or recovered).
   *  Only emitted when `enforce = true`. `direction` is "exceeded"|"recovered".
   *  B8 — Skills/Improve/Channels. */
  | {
      type: 'budget_exceeded';
      workspace_id: Id;
      provider: string;
      spend_usd: number;
      cap_usd: number;
      direction: string;
    }
  /** A proof pack was created, (re)assembled, had an artifact added, or was
   *  waived — its derived status/risk may have changed. The Proof page + sidebar
   *  badges re-fetch the affected pack and refresh the workspace proof summary. */
  | {
      type: 'proof_pack_updated';
      workspace_id: Id;
      proof_pack_id: Id;
      work_item_kind: string;
      work_item_id: string;
      status: string;
      risk_score: number;
    }
  | {
      /** A Mission Control work item was created or its normalized status
       *  changed — the page re-fetches the workspace summary/list on a match. */
      type: 'work_graph_updated';
      workspace_id: Id;
      item_id: Id;
      kind: string;
      status: string;
    }
  | {
      /** A scheduled-task run started/finished/errored — the Scheduled Tasks page
       *  re-fetches the task's run history on a matching tick. */
      type: 'scheduled_task_run_updated';
      workspace_id: Id;
      task_id: Id;
      run_id: Id;
      status: string;
    }
  | {
      /** A Run with Otto run advanced through its stage machine (status changed,
       *  proof/review attached, or it reached approval/PR/terminal). The Run with
       *  Otto page re-fetches the affected run + the workspace list on a match. */
      type: 'otto_run_updated';
      workspace_id: Id;
      run_id: Id;
      status: string;
    }
  | {
      /** A canvas scene's source doc changed — pushed LIVE while an agent edits
       *  the backing file (per-poll) and once with the committed result. The
       *  Canvas page re-renders `doc` for the matching `scene_id`. */
      type: 'canvas_updated';
      workspace_id: Id;
      scene_id: Id;
      doc: unknown;
    }
  | {
      /** The canvas Ask-AI agent session became live (turn start) — the Canvas
       *  Assistant panel attaches its shell immediately for the matching scene. */
      type: 'canvas_session_started';
      workspace_id: Id;
      scene_id: Id;
      session_id: Id;
    }
  | {
      /** A product mockup's source changed — pushed LIVE while the mockup agent
       *  edits the backing file (per-poll) and once with the committed result. The
       *  Mockups Assistant panel re-renders the live preview for `attachment_id`. */
      type: 'mockup_updated';
      workspace_id: Id;
      story_id: Id;
      attachment_id: Id;
      format: string;
      content: string;
    }
  | {
      /** The mockup agent session became live (turn start) — the Mockups Assistant
       *  panel attaches its shell immediately for the matching attachment. */
      type: 'mockup_session_started';
      workspace_id: Id;
      story_id: Id;
      attachment_id: Id;
      session_id: Id;
    }
  | {
      /** The DB Assistant agent session became live (turn start) — the embedded
       *  DB Assistant panel (beside the query editor) attaches its live shell for
       *  the matching `assist_id`. The session is hidden from the Agents list
       *  (meta.source = 'db_assist'). */
      type: 'db_assist_session_started';
      workspace_id: Id;
      connection_id: Id;
      assist_id: Id;
      session_id: Id;
    }
  | {
      /** The DB Assistant's proposed SQL/note changed — pushed LIVE while the agent
       *  writes its ANSWER.sql, and once with the committed result. The panel shows
       *  `sql` in a read-only block with Insert/Run for the matching `assist_id`. */
      type: 'db_assist_updated';
      workspace_id: Id;
      connection_id: Id;
      assist_id: Id;
      sql: string;
      note: string;
    }
  | {
      /** A review finding's workflow `status` (or a tracked field) changed —
       *  emitted after every triage action / transition. The Findings board
       *  subscribes and refetches the matching review's findings.
       *  `status` is the new `FindingStatus` (snake_case). */
      type: 'finding_updated';
      workspace_id: Id;
      review_id: Id;
      finding_id: Id;
      status: string;
    }
  | {
      /** An agent-backed finding action spawned a live, openable session
       *  (fix / verify / regression-test). `action` is
       *  "fix" | "verify" | "regression_test". */
      type: 'finding_action_started';
      workspace_id: Id;
      review_id: Id;
      finding_id: Id;
      action: string;
      session_id?: Id | null;
    }
  | {
      /** A review's Proof Pack was exported (snapshot persisted + verified
       *  findings ingested into memory). */
      type: 'proof_pack_exported';
      workspace_id: Id;
      review_id: Id;
      proof_pack_id: Id;
    };

// ---------------------------------------------------------------------------
// Notifications (notification center)
// ---------------------------------------------------------------------------

export type NoticeKind = 'credential' | 'session' | 'system';
export type NoticeSeverity = 'info' | 'warn' | 'error';

/** What the UI does when a notice's action button is clicked. */
export type NoticeAction =
  | { type: 'open_url'; url: string }
  | { type: 'open_session'; session_id: Id }
  /** `target` e.g. "claude" | "codex" | "git:<id>" | "issue:<id>". */
  | { type: 'reauth'; target: string };

/** A persisted notification shown in the notification center. */
export interface Notice {
  id: Id;
  created_at: string;
  read: boolean;
  kind: NoticeKind;
  severity: NoticeSeverity;
  title: string;
  body: string;
  source_key: string | null;
  action: NoticeAction | null;
}

export interface NotificationSettings {
  expiry_threshold_days: number;
  native_enabled: boolean;
  session_events: boolean;
}

// ---------------------------------------------------------------------------
// RBAC — features + capabilities
// ---------------------------------------------------------------------------

/** The 22 protected features (snake_case, mirrors Rust Feature enum). */
export type Feature =
  | 'agents'
  | 'connections'
  | 'database'
  | 'git'
  | 'issues'
  | 'product'
  | 'swarm'
  | 'api_client'
  | 'workflows'
  | 'channels'
  | 'skill_eval'
  | 'skills'
  | 'insights'
  | 'usage'
  | 'self_improvement'
  | 'context'
  | 'settings'
  | 'users'
  | 'canvas'
  | 'proof_pack'
  | 'mcp'
  | 'mission_control'
  | 'scheduled_tasks'
  | 'run_with_otto';

/** Capability ladder (None < View < Edit < Admin). */
export type Capability = 'none' | 'view' | 'edit' | 'admin';

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
  | 'external_trigger';
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

/** One (feature, capability) pair — mirrors `GrantEntry` in api.rs. */
export interface GrantEntry {
  feature: string;
  capability: string;
}

/** `GET /api/v1/users/{id}/grants` response. */
export interface UserGrantsResp {
  grants: GrantEntry[];
}

/** `PUT /api/v1/users/{id}/grants` request body. */
export interface UserGrantsReq {
  grants: GrantEntry[];
}

/** `GET /api/v1/auth/capabilities` response — the caller's effective map.
 *  Root receives `admin` for every feature. */
export interface CapabilitiesResp {
  /** snake_case feature name → snake_case capability string. */
  capabilities: Record<string, string>;
}

// ---------------------------------------------------------------------------
// Admin active-sessions overview (RBAC Task 4.2/4.3)
// ---------------------------------------------------------------------------

/** One row of `GET /api/v1/admin/sessions` — mirrors `AdminSessionRow` in api.rs. */
export interface AdminSessionRow {
  /** Session id. */
  id: string;
  /** `created_by` — owning user's id. */
  owner_id: string;
  /** Owning user's username (falls back to owner_id when user no longer exists). */
  owner_username: string;
  /** Workspace the session belongs to. */
  workspace_id: string;
  /** `'agent'` | `'connection'`. */
  kind: string;
  /** CLI provider / connection driver (`'claude'`, `'codex'`, `'shell'`, `'mysql'`, …). */
  provider: string;
  /** Display title. */
  title: string;
  /** Persisted status (`'running'` | `'working'` | `'idle'` | `'reconnectable'` | `'exited'`). */
  status: string;
  /** True when the session has a live PTY in the daemon process. */
  live: boolean;
  /** Number of WS terminal viewers currently attached. */
  viewers: number;
}

/** `GET /api/v1/admin/sessions` response — mirrors `AdminSessionsResp` in api.rs. */
export interface AdminSessionsResp {
  sessions: AdminSessionRow[];
}

/**
 * `POST /api/v1/admin/impersonate/{user_id}` response — mirrors `ImpersonateResp`
 * in api.rs. The short-lived impersonation bearer token (returned once); the UI
 * swaps to it to act-as the target user and calls `/admin/impersonate/stop` to end.
 */
export interface ImpersonateResp {
  token: string;
}

// ---------------------------------------------------------------------------
// Meta / auth
// ---------------------------------------------------------------------------

export interface ToolStatus {
  name: string;
  found: boolean;
  version: string | null;
}

export interface MetaResp {
  version: string;
  api_version: number;
  needs_onboarding: boolean;
  network_listener: boolean;
  tools: ToolStatus[];
  providers: string[];
  /** Configured default agent (provider name); null when unset. */
  default_provider: string | null;
}

export interface OnboardRootReq {
  password: string;
  display_name?: string | null;
}

export interface LoginReq {
  username: string;
  password: string;
}

export interface LoginResp {
  token: string;
  user: User;
}

/**
 * GET /api/v1/auth/me — mirrors `MeResp` in api.rs.
 * `user` is the effective identity (what authorisation runs against).
 * `real_user` is the token owner (== `user` for normal sessions).
 * `impersonating` is true when the caller holds an impersonation token.
 */
export interface MeResp {
  /** Effective user — the identity the session currently acts as. */
  user: User;
  /** Real token owner — equals `user` for a normal (non-impersonation) session. */
  real_user: User;
  /** true when real_user.id !== user.id (i.e. an impersonation token is active). */
  impersonating: boolean;
}

/** POST /api/v1/auth/tokens — mint a long-lived API (personal access) token. */
export interface CreateApiTokenReq {
  label?: string | null;
}

/** Metadata for one API token. NEVER carries the secret (only its prefix). */
export interface ApiTokenInfo {
  id: string;
  label?: string | null;
  /** First 12 chars of the raw token, for identification in a list. */
  token_prefix: string;
  created_at: string;
  last_seen_at: string;
  expires_at: string;
}

/** Response for POST /api/v1/auth/tokens: raw secret (shown once) + metadata. */
export interface CreateApiTokenResp {
  token: string;
  info: ApiTokenInfo;
}

// ---------------------------------------------------------------------------
// Trust & Safety: audit log + security posture (root only)
// ---------------------------------------------------------------------------

/**
 * One append-only entry in the security audit log. Written best-effort by the
 * daemon at sensitive sites; never updated or deleted.
 * `action` is a stable snake_case verb, e.g. 'login.success', 'token.mint',
 * 'token.revoke', 'settings.change', 'network_listener.toggle',
 * 'login.failure', 'login.lockout', 'db.write_confirmed'.
 */
export interface AuditEntry {
  id: Id;
  ts: string;
  /** Acting user; null for an unauthenticated actor (e.g. a failed login). */
  user_id: Id | null;
  action: string;
  target: string | null;
  detail: unknown | null;
  ip: string | null;
}

/** Query for GET /api/v1/audit-log (root only). All filters optional. */
export interface AuditLogQuery {
  /** Lower bound on `ts` (RFC3339), inclusive. */
  from?: string;
  /** Upper bound on `ts` (RFC3339), inclusive. */
  to?: string;
  /** Exact `action` match. */
  action?: string;
  /** Exact acting-user match. */
  user_id?: Id;
  limit?: number;
  offset?: number;
}

/** Response for GET /api/v1/audit-log: a page plus the filtered total. */
export interface AuditLogResp {
  entries: AuditEntry[];
  /** Total rows matching the filters (ignores limit/offset). */
  total: number;
}

/** GET /api/v1/security-posture (root only) — a derived security snapshot. */
export interface SecurityPostureResp {
  network_listener: boolean;
  network_listener_port: number | null;
  loopback_only: boolean;
  active_api_tokens: number;
}

// ---------------------------------------------------------------------------
// Users / workspaces
// ---------------------------------------------------------------------------

export interface CreateUserReq {
  username: string;
  password: string;
  display_name?: string | null;
}

export interface UpdateUserReq {
  display_name?: string | null;
  password?: string | null;
  disabled?: boolean | null;
}

export interface CreateWorkspaceReq {
  name: string;
  root_path: string;
}

export interface UpdateWorkspaceReq {
  name?: string | null;
  root_path?: string | null;
  settings?: Record<string, unknown> | null;
  archived?: boolean | null;
}

export interface MemberEntry {
  user_id: Id;
  username: string;
  display_name: string;
  role: WorkspaceRole;
}

export interface SetMembersReq {
  members: { user_id: Id; role: WorkspaceRole }[];
}

export type WorkspaceWithRole = Workspace & { my_role: WorkspaceRole };

// ---------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------

export interface CreateSessionReq {
  kind: SessionKind;
  provider?: string | null;
  title?: string | null;
  cwd?: string | null;
  connection_id?: Id | null;
  meta?: Record<string, unknown> | null;
}

export interface UpdateSessionReq {
  title?: string | null;
  /** Shallow-merged into the session's existing meta (e.g. `extra_dirs`). */
  meta?: Record<string, unknown> | null;
}

/** POST /sessions/{id}/input — write text into a session's PTY server-side. */
export interface SendInputReq {
  /** Text to write into the PTY. */
  text: string;
  /** Append a newline so the agent runs it immediately. `null`/`true` submits;
   *  `false` sends verbatim so the user can inspect/edit before pressing Enter. */
  submit?: boolean | null;
}

// ---------------------------------------------------------------------------
// Orchestrator
// ---------------------------------------------------------------------------

export type Action =
  | { action: 'spawn_sessions'; provider: string; count: number }
  | { action: 'broadcast'; text: string }
  | { action: 'open_connection'; connection_id: Id }
  | { action: 'run_command'; session_id: Id; text: string };

export type ActionPlan = Action[];

/** POST /workspaces/{id}/broadcast — dedicated, AI-free relay to sessions. */
export interface BroadcastReq {
  text: string;
  /** Target sessions; omit/empty to hit every live agent session. */
  session_ids?: Id[];
}

export interface BroadcastResp {
  /** The sessions that actually received the message. */
  session_ids: Id[];
}

/** POST /workspaces/{id}/relay — deliver a NAME-ADDRESSED message
 *  ("ronaldo: do X", "ronaldo, messi: ship it", "all: stand down"). */
export interface RelayReq {
  text: string;
}

export interface RelayResp {
  /** Sessions the message was delivered to (empty when unaddressed). */
  session_ids: Id[];
  /** True when the address was a broadcast keyword (all/everyone). */
  broadcast: boolean;
  /** True when no session was named — the caller should fall back. */
  unaddressed: boolean;
  /** The message actually sent (address prefix stripped). */
  text: string;
}

// --- Session name themes (auto-naming new sessions: "Ronaldo", "Messi", …) ----

/** One selectable name theme (built-in or a user's custom list). */
export interface NameThemeInfo {
  /** Built-in id ("footballers") or a custom theme's id. */
  id: string;
  label: string;
  /** "builtin" | "custom". */
  kind: string;
  /** How many distinct names the theme can yield. */
  capacity: number;
  /** A few example names for the picker preview. */
  sample: string[];
}

/** GET /name-themes */
export interface NameThemesResp {
  themes: NameThemeInfo[];
  /** Active theme id: a built-in id, a custom id, or "none" (legacy numbering). */
  active: string;
}

/** PUT /name-themes/active */
export interface SetActiveThemeReq {
  theme_id: string;
}

/** POST /name-themes */
export interface CreateNameThemeReq {
  label: string;
  names: string[];
}

/** PUT /name-themes/{id} */
export interface UpdateNameThemeReq {
  label: string;
  names: string[];
}

/** A custom name theme as returned to its owner. */
export interface CustomThemeResp {
  id: Id;
  label: string;
  names: string[];
}

export interface OrchestrateReq {
  text: string;
  optimize: boolean;
  ai_fallback: boolean;
  focused_session_id?: Id | null;
}

export interface OrchestrateResp {
  plan: ActionPlan;
  optimized_text: string | null;
}

export interface ExecutePlanReq {
  plan: ActionPlan;
}

export interface ExecuteResult {
  action_index: number;
  ok: boolean;
  detail: string;
  session_ids: Id[];
}

// ---------------------------------------------------------------------------
// Connections
// ---------------------------------------------------------------------------

export interface UpsertConnectionReq {
  name: string;
  kind: ConnectionKind;
  params: Record<string, unknown>;
  secret?: string | null;
  first_command?: string | null;
  section_id?: Id | null;
  /** Deployment environment. On create, omit for 'dev'; on PATCH, omitting it
   *  KEEPS the stored value (never silently downgrades a 'prod' connection). */
  environment?: Environment;
  /** Lock the profile against writes/DDL. On create, omit for false; on PATCH,
   *  omitting it KEEPS the stored value (never silently un-locks read-only). */
  read_only?: boolean;
}

export interface TestConnectionResp {
  ok: boolean;
  latency_ms: number | null;
  message: string;
  warn_argv: boolean;
  /** Set when the connection's SSH private key file has insecure (group/other-
   *  readable) permissions; carries the full message incl. the `chmod 600 <path>`
   *  fix. Independent of `ok`. */
  warn_key_perms?: string | null;
}

// ---------------------------------------------------------------------------
// Import connections from other DB tools
// ---------------------------------------------------------------------------

/** A DB tool Otto can import saved connections from. The daemon runs locally
 *  and reads each tool's config from its default macOS location — the user
 *  picks a tool, never a file. */
export type ImportSource = 'mysql_workbench' | 'dbeaver' | 'datagrip' | 'nosqlbooster';

/** One tool's availability (GET …/connections/import/sources). */
export interface SourceStatus {
  source: ImportSource;
  label: string;
  /** True when a config file was found at the default location. */
  present: boolean;
  /** The resolved config path (first match), if any. */
  path?: string | null;
  /** How many connections were parsed out, when present (may be 0). */
  count?: number | null;
}

/** A single connection parsed from a tool's config. When `supported`, `params`
 *  is the ready-to-create Otto shape; otherwise `kind` is null and `note`
 *  explains why the engine was skipped. */
export interface ParsedConnection {
  source: ImportSource;
  name: string;
  /** null for an engine Otto doesn't support (still listed so the user sees why). */
  kind?: ConnectionKind | null;
  params: Record<string, unknown>;
  supported: boolean;
  /** True when the source had a username but no recoverable password — the user
   *  must add it after import (MongoDB uses a `{secret}` placeholder). */
  needs_password: boolean;
  note?: string | null;
}

/** Result of scanning a single tool (POST …/connections/import/scan). */
export interface ImportScanResult {
  source: ImportSource;
  path?: string | null;
  connections: ParsedConnection[];
  warnings: string[];
}

/** One connection the user chose to create. */
export interface ImportCreateItem {
  name: string;
  kind: ConnectionKind;
  params: Record<string, unknown>;
  environment?: Environment;
  read_only?: boolean;
}

/** POST …/connections/import/create body. */
export interface ImportCreateReq {
  connections: ImportCreateItem[];
  section_id?: Id | null;
}

/** Result of an import create batch (best-effort — partial success is fine). */
export interface ImportCreateResult {
  created: Connection[];
  failed: { name: string; error: string }[];
}

// ---------------------------------------------------------------------------
// SFTP file browser (over an SSH connection's existing auth)
// ---------------------------------------------------------------------------

/** One entry in a remote directory listing (GET /connections/{id}/sftp/list). */
export interface SftpEntry {
  name: string;
  kind: 'dir' | 'file' | 'symlink' | 'other';
  size: number;
  /** Raw date/time field from the listing (e.g. "Jun 20 12:00"), if present. */
  mtime: string | null;
  /** The 10-char permission string (e.g. "drwxr-xr-x"). */
  perms: string;
  /** For symlinks, the link target; null otherwise. */
  symlink_target: string | null;
}

export interface SftpListResp {
  /** Absolute remote path that was listed (resolved from pwd when omitted). */
  path: string;
  entries: SftpEntry[];
}

/** `POST /api/v1/connections/{id}/sftp/download`. */
export interface SftpDownloadReq {
  remote_path: string;
  /** Local destination (file path or dir). Leading `~` expands to daemon home. */
  local_path: string;
}

export interface SftpDownloadResp {
  local_path: string;
  bytes: number;
}

/** `POST /api/v1/connections/{id}/sftp/upload`. */
export interface SftpUploadReq {
  /** Local source (leading `~` expands to daemon home). */
  local_path: string;
  remote_path: string;
}

/** `POST /api/v1/connections/{id}/sftp/mkdir`. */
export interface SftpMkdirReq {
  path: string;
}

/** `POST /api/v1/connections/{id}/sftp/remove` — `dir:true` ⇒ rmdir, else rm. */
export interface SftpRemoveReq {
  path: string;
  dir?: boolean;
}

/** `POST /api/v1/connections/{id}/sftp/rename`. */
export interface SftpRenameReq {
  from: string;
  to: string;
}

/** `GET /api/v1/connections/{id}/sftp/read?path=` — text view of a small file. */
export interface SftpReadResp {
  text: string;
  /** True when the file exceeded the read cap (content is the capped prefix). */
  truncated: boolean;
}

// ---------------------------------------------------------------------------
// Git
// ---------------------------------------------------------------------------

export interface CreateGitAccountReq {
  provider: GitProviderKind;
  label: string;
  username: string;
  token: string;
  api_base_url?: string | null;
  namespace?: string | null;
  /** Optional user-entered token expiry (ISO); for providers that don't expose it. */
  token_expires_at?: string | null;
}

/** `PATCH /api/v1/git/accounts/{id}` — all fields optional; absent = keep current. */
export interface UpdateGitAccountReq {
  label?: string;
  username?: string;
  /** Empty string clears to null; non-empty sets; absent keeps current. */
  namespace?: string;
  /** Empty string clears to null; non-empty sets; absent keeps current. */
  api_base_url?: string;
  /** Non-empty rotates the Keychain secret; empty/absent keeps existing. */
  token?: string;
  /** Set the user-entered token expiry (ISO); absent keeps current. */
  token_expires_at?: string | null;
}

/** `PATCH /api/v1/issue/accounts/{id}` — all fields optional; absent = keep current. */
export interface UpdateIssueAccountReq {
  label?: string;
  email?: string;
  base_url?: string;
  /** Non-empty rotates the Keychain secret; empty/absent keeps existing. */
  token?: string;
  /** Set the user-entered token expiry (ISO); absent keeps current. */
  token_expires_at?: string | null;
}

/** Daemon-side directory listing for the folder picker (GET /fs/browse). */
export interface FsEntry {
  name: string;
  path: string;
  is_dir: boolean;
  is_git_repo: boolean;
}

export interface FsBrowse {
  path: string;
  parent: string | null;
  /** True when the browsed directory is itself a git repo. */
  is_git_repo: boolean;
  entries: FsEntry[];
}

export interface LogFileEntry {
  name: string;
  size: number;
  modified_ms: number;
}

export interface DaemonLogs {
  log_dir: string;
  files: LogFileEntry[];
  selected: string;
  mode: 'all' | 'tail' | 'since';
  content: string;
  offset: number;
  next_offset: number;
}

export interface AddRepoReq {
  path?: string | null;
  clone_url?: string | null;
  name?: string | null;
  git_account_id?: Id | null;
}

export interface FileChange {
  path: string;
  orig_path: string | null;
  kind: 'modified' | 'added' | 'deleted' | 'renamed' | 'untracked' | 'conflicted';
  staged: boolean;
  unstaged: boolean;
}

export interface RepoStatusResp {
  branch: string;
  upstream: string | null;
  ahead: number;
  behind: number;
  changes: FileChange[];
}

export interface BranchInfo {
  name: string;
  is_current: boolean;
  upstream: string | null;
}

export interface CommitInfo {
  sha: string;
  short_sha: string;
  author: string;
  date: string;
  subject: string;
  parents: string[];
  refs: string[];
}

export interface RefBranch {
  name: string;
  is_current: boolean;
  upstream: string | null;
  remote: boolean;
}

export interface RefTag {
  name: string;
}

export interface RefsResp {
  local: RefBranch[];
  remote: RefBranch[];
  tags: RefTag[];
}

/** One `git stash list` entry. `ref` is the `stash@{N}` selector; `parents` are
 *  `[base, index, (untracked)]`. Read-only — fed by GET /repos/{id}/stashes. */
export interface StashInfo {
  index: number;
  ref: string;
  sha: string;
  parents: string[];
  date: string;
  message: string;
  branch: string | null;
}

export type LineOrigin = 'context' | 'add' | 'del';

export interface DiffLine {
  origin: LineOrigin;
  content: string;
  old_line: number | null;
  new_line: number | null;
}

export interface Hunk {
  header: string;
  lines: DiffLine[];
}

export interface FileDiff {
  path: string;
  old_path: string | null;
  is_binary: boolean;
  hunks: Hunk[];
  too_large?: boolean | null;
  added?: number | null;
  deleted?: number | null;
  language?: string | null;
}

export interface DiffResp {
  files: FileDiff[];
}

export interface StagePathsReq {
  paths: string[];
}

export interface CommitReq {
  message: string;
  amend: boolean;
}

export interface CheckoutReq {
  branch: string;
  create: boolean;
}

// --- Local merge + conflict resolution (#4) ---

/** Strategy for a local branch merge. */
export type LocalMergeStrategy = 'merge_commit' | 'ff' | 'ff_only' | 'squash';

/** `POST /repos/{id}/merge` — merge `source` into `target`. */
export interface MergeBranchReq {
  source: string;
  target: string;
  strategy: LocalMergeStrategy;
  /** When true and the tree is dirty, stash → merge → pop. Default false. */
  auto_stash?: boolean;
}

/** Outcome of a local merge / merge-completion. Conflicts are a normal result. */
export interface MergeResult {
  status: 'merged' | 'conflicts' | 'up_to_date';
  commit: string | null;
  conflicted_files: string[];
  repo_status: RepoStatusResp;
  /** Optional note (e.g. auto-stash outcome) to surface as a toast. */
  note?: string | null;
}

/** `POST /repos/{id}/merge/preview` — dry-run conflict check (no tree mutation). */
export interface MergePreviewReq {
  source: string;
  target: string;
}

/** Result of a merge dry-run. */
export interface MergePreview {
  conflicts: boolean;
  conflicted_files: string[];
  up_to_date: boolean;
}

/** `GET /repos/{id}/merge/status` */
export interface MergeConflictStatus {
  merging: boolean;
  source: string | null;
  conflicted_files: string[];
}

/** One segment of a conflicted file. */
export type ConflictSegment =
  | { kind: 'context'; lines: string[] }
  | { kind: 'conflict'; ours: string[]; theirs: string[]; base: string[] };

/** `GET /repos/{id}/conflict?path=<p>` */
export interface ConflictFile {
  path: string;
  is_binary: boolean;
  segments: ConflictSegment[];
}

/** `POST /repos/{id}/conflict/resolve` */
export interface ResolveConflictReq {
  path: string;
  content: string;
}

/** `POST /repos/{id}/merge/commit` */
export interface MergeCommitReq {
  message: string | null;
}

export type PrState = 'open' | 'merged' | 'declined' | 'all';

export interface PrSummary {
  number: number;
  title: string;
  author: string;
  state: PrState;
  source_branch: string;
  target_branch: string;
  updated_at: string;
  url: string;
  draft?: boolean | null;
  ci_status?: string | null;
  labels?: string[];
}

export interface PrComment {
  id: string;
  author: string;
  body: string;
  path: string | null;
  line: number | null;
  created_at: string;
  replies: PrComment[];
}

/** A PR reviewer with approval state; avatar/timestamp are best-effort. */
export interface PrReviewer {
  name: string;
  approved: boolean;
  avatar_url: string | null;
  reviewed_at: string | null;
}

export type PrDetail = PrSummary & {
  description_md: string;
  comments: PrComment[];
  /** Approver display names (back-compat; prefer `reviewers`). */
  approved_by: string[];
  reviewers: PrReviewer[];
  mergeable: boolean | null;
};

export interface CreatePrReq {
  title: string;
  description: string;
  source_branch: string;
  target_branch: string;
}

export interface DraftPrReq {
  base: string;
}

export interface DraftPrResp {
  title: string;
  description: string;
  source_branch: string;
  target_branch: string;
}

export interface DraftCommitMessageResp {
  message: string;
  /** True when drafted from the staged diff; false when it fell back to the working diff. */
  from_staged: boolean;
}

export interface UpdatePrReq {
  title?: string | null;
  description?: string | null;
}

export interface NewPrCommentReq {
  body: string;
  path?: string | null;
  line?: number | null;
  in_reply_to?: string | null;
}

export type MergeStrategy = 'merge' | 'squash' | 'rebase';

export interface MergePrReq {
  strategy: MergeStrategy;
}

export interface Problem {
  code: string;
  message: string;
}

export interface PrCommit {
  sha: string;
  short_sha: string;
  author: string;
  date: string;
  subject: string;
}

export interface RequestChangesReq {
  body?: string | null;
}

// ---------------------------------------------------------------------------
// Provider updates
// ---------------------------------------------------------------------------

export interface UpdateProvidersReq {
  /** When set, only update that specific provider; omit to update all. */
  provider?: string | null;
}

// ---------------------------------------------------------------------------
// Proof Packs
// ---------------------------------------------------------------------------

export type ProofStatus = 'missing' | 'partial' | 'passed' | 'failed' | 'waived';
export type ProofArtifactKind =
  | 'command'
  | 'log'
  | 'screenshot'
  | 'video'
  | 'diff'
  | 'ci'
  | 'api'
  | 'db'
  | 'kafka'
  | 'review'
  | 'approval'
  | 'pr_check'
  | 'self_review';
export type ProofArtifactStatus = 'passed' | 'failed' | 'pending' | 'info';
export type WorkItemKind = 'session' | 'goal_loop' | 'review' | 'workflow_run' | 'task' | 'manual';
export type ProofBadge =
  | 'no_proof'
  | 'tests_passed'
  | 'tests_failed'
  | 'human_approved'
  | 'risky_change'
  | 'ci_missing'
  | 'ci_passed'
  | 'ci_failed'
  | 'ci_pending'
  | 'db_api_verified'
  | 'ui_verified'
  | 'pr_inconsistent'
  | 'review_unresolved'
  | 'waived';

export interface ProofPack {
  id: Id;
  workspace_id: Id;
  work_item_kind: WorkItemKind;
  work_item_id: string;
  title: string;
  status: ProofStatus;
  summary: string;
  risk_score: number;
  done_score: number;
  parent_pack_id?: string | null;
  repo_id?: string | null;
  pr_number?: number | null;
  waived_by?: string | null;
  waived_reason?: string | null;
  waived_at?: string | null;
  created_by: Id;
  created_at: string;
  updated_at: string;
}

export interface ProofArtifact {
  id: Id;
  proof_pack_id: Id;
  workspace_id: Id;
  kind: ProofArtifactKind;
  title: string;
  content_ref?: string | null;
  status: ProofArtifactStatus;
  metadata: unknown;
  content_sha256?: string | null;
  created_by: Id;
  created_at: string;
  updated_at: string;
}

/** A proof pack with its derived badge list + artifact count (list/summary row). */
export interface ProofPackResp extends ProofPack {
  badges: string[];
  artifact_count: number;
}

/** One artifact plus a capped inline preview for list/detail rendering. */
export interface ProofArtifactView extends ProofArtifact {
  preview?: string | null;
  truncated: boolean;
}

/** One line of the done-contract checklist. */
export interface ContractItem {
  key: string;
  label: string;
  required: boolean;
  satisfied: boolean;
  weight: number;
  detail: string;
}

/** The "done contract": an explainable readiness score + itemized checklist. */
export interface DoneContract {
  score: number;
  satisfied: number;
  required: number;
  items: ContractItem[];
}

/** Snapshot metadata (no bundle/reports). */
export interface ProofSnapshotMeta {
  id: string;
  proof_pack_id: string;
  seq: number;
  sha256: string;
  status: string;
  done_score: number;
  risk_score: number;
  note: string;
  created_by: string;
  created_at: string;
}

/** A full immutable snapshot. */
export interface ProofSnapshotResp extends ProofSnapshotMeta {
  bundle: unknown;
  report_md: string;
  report_html: string;
}

/** `GET /proof-packs/{id}` — the pack, its badges, artifacts, children, contract. */
export interface ProofPackDetail {
  pack: ProofPack;
  badges: string[];
  artifacts: ProofArtifactView[];
  children: ProofPackResp[];
  done_contract: DoneContract;
  snapshots: ProofSnapshotMeta[];
}

export interface ProofSummaryRow {
  work_item_kind: string;
  work_item_id: string;
  proof_pack_id: string;
  status: string;
  risk_score: number;
  done_score: number;
  badges: string[];
}

export interface ProofSummaryResp {
  rows: ProofSummaryRow[];
}

export interface CreateProofPackReq {
  work_item_kind: string;
  work_item_id: string;
  title?: string;
  parent_pack_id?: string;
  /** Link the pack to a registered repo so its proof policy applies (strengthen-only). */
  repo_id?: string;
}

/** `POST /proof-packs/{id}/waive` body. */
export interface WaiveReq {
  reason: string;
}

export interface AddArtifactReq {
  kind: string;
  title: string;
  content?: string;
  content_url?: string;
  status?: string;
  metadata?: unknown;
}

export interface AssembleReq {
  cwd?: string;
  base?: string;
  commands?: { cmd: string; kind?: string }[];
}

/** Per-repository proof requirements (R3 — strengthen-only). */
export interface RepoProofConfig {
  require_test?: boolean;
  test_cmd?: string | null;
  require_ci?: boolean;
  require_pr_consistency?: boolean;
  require_review?: boolean;
}

export interface RepoProofConfigResp extends RepoProofConfig {
  repo_id: string;
}

export interface CreateSnapshotReq {
  note?: string;
}

export interface AttachMediaReq {
  kind: 'screenshot' | 'video';
  title: string;
  mime: string;
  data_base64: string;
  metadata?: unknown;
}

export interface ApiEvidenceReq {
  title: string;
  method: string;
  url: string;
  status: number;
  duration_ms?: number;
  request?: string;
  response?: string;
  metadata?: unknown;
}

export interface DbEvidenceReq {
  title: string;
  engine?: string;
  query?: string;
  columns?: string[];
  row_count?: number;
  sample?: string;
  error?: string;
  metadata?: unknown;
}

export interface KafkaEvidenceReq {
  title: string;
  topic: string;
  message_count?: number;
  sample?: string;
  truncated?: boolean;
  error?: string;
  metadata?: unknown;
}

export interface PrCheckReq {
  title: string;
  description: string;
  base?: string;
  cwd?: string;
}

export interface CiRefreshReq {
  repo_id?: string;
  pr_number?: number;
}

// ---------------------------------------------------------------------------
// PR Review (AI agents)
// ---------------------------------------------------------------------------

export type ReviewStatus = 'running' | 'done' | 'error' | 'cancelled';
export type ReviewCommentState = 'draft' | 'approved' | 'declined';
export type ReviewAgentStatus = 'pending' | 'running' | 'waiting' | 'done' | 'error';

export interface ReviewFinding {
  path: string | null;
  line: number | null;
  severity: string; // 'info' | 'warn' | 'bug'
  body: string;
  /** Stable sha2 fingerprint for cross-run deduplication (added A1). */
  fingerprint?: string | null;
  /** Lifecycle state: open | fixing | resolved | regressed | declined (added A1). */
  state?: string | null;
}

/** A persistent finding row from /reviews/{id}/findings (A1 verified-review loop). */
export interface ReviewFindingRow {
  id: string;
  review_id: string;
  fingerprint: string;
  path: string | null;
  line: number | null;
  severity: string;
  category: string | null;
  body: string;
  state: string; // open | fixing | resolved | regressed | declined
  fix_session_id: string | null;
  updated_at: string;
}

export interface ReviewAgentState {
  name: string;
  provider: string;
  model: string;
  status: ReviewAgentStatus;
  note: string;
  comment_count: number;
  /** Openable live session running this agent. */
  session_id?: string | null;
  /** This agent's own findings (before summarization). */
  findings?: ReviewFinding[];
}

export interface ReviewComment {
  id: string;
  review_id: string;
  path: string | null;
  line: number | null;
  severity: 'info' | 'warn' | 'bug';
  body: string;
  state: ReviewCommentState;
  posted: boolean;
  created_at: string;
}

export interface Review {
  id: string;
  repo_id: string;
  pr_number: number;
  status: ReviewStatus;
  error: string | null;
  comments: ReviewComment[];
  agents: ReviewAgentState[];
  created_at: string;
  verdict?: string | null;
  blocker_count?: number | null;
  summary_md?: string | null;
}

export interface ReviewAgentCfg {
  name: string;
  /** "claude" | "codex" | "agy" — kept for back-compat; effective list is `providers` when non-empty */
  provider: string;
  /** List of CLIs to run this agent on. Expands to one run per entry. */
  providers?: string[];
  /** "haiku" | "sonnet" | "opus" | "" (empty = provider default) */
  model: string;
  /** The lens/instructions for this agent */
  prompt: string;
}

export interface ReviewConfig {
  agents: ReviewAgentCfg[];
  summarizer: ReviewAgentCfg;
  custom_presets?: ReviewAgentCfg[];
  /** Max total attempts per agent (initial + retries). Default 3. */
  max_attempts?: number | null;
  /** Per-agent timeout in seconds; overrides diff-size heuristic. */
  timeout_secs?: number | null;
}

export interface MergeReadiness {
  unresolved_blocker_count: number;
  unresolved_total: number;
  resolved_count: number;
  last_updated?: string | null;
  // A1 additions from the /merge-readiness endpoint:
  total_findings?: number | null;
  /** Aggregated CI state: "success" | "failure" | "pending" | "none" */
  ci_status?: string | null;
  /** Number of human approvals on the PR. */
  approvals?: number | null;
  /** Whether the PR is mergeable according to the provider (null = unknown). */
  mergeable?: boolean | null;
  conflicts?: boolean | null;
  branch_freshness?: string | null;
  unpushed?: boolean | null;
}

export interface StartReviewReq {
  issue_account_id?: string | null;
  issue_key?: string | null;
  /** Free-text guidance for the review agents (e.g. "what to focus on"). Optional. */
  context?: string | null;
}

export interface LocalReviewReq {
  /** The base git ref to diff against (e.g. "origin/develop", "main"). */
  base: string;
}

export interface HandoffReq {
  /** The provider to spawn ("claude" | "codex" | "agy"). */
  provider: string;
  /** Optional list of comment ids to include. When absent, all non-declined comments are sent. */
  comment_ids?: string[] | null;
}

/** Where a handover brief is delivered. */
export type HandoverTarget =
  | { kind: 'new_agent'; provider: string }
  | { kind: 'existing_session'; session_id: Id };

/** POST /sessions/{id}/handover — pass one agent's context into another agent. */
export interface HandoverReq {
  target: HandoverTarget;
  /** What the receiving agent should focus on (weights the generated brief). */
  focus?: string | null;
  /** Title for the new session (new_agent only). */
  title?: string | null;
  /** Pre-reviewed brief; when present the server skips summarization. */
  brief?: string | null;
  /** Include git state in the generated brief. Defaults to true. */
  include_git?: boolean | null;
  /** Summarize with a fast model. Defaults to false. */
  fast?: boolean | null;
  /** Archive the source session after handover. Defaults to false. */
  archive_source?: boolean | null;
}

/** POST /sessions/{id}/handover/brief — generate the brief for review. */
export interface HandoverBriefReq {
  focus?: string | null;
  include_git?: boolean | null;
  fast?: boolean | null;
}

export interface HandoverBriefResp {
  /** The generated brief (markdown); empty when there was no context. */
  brief: string;
  /** True when summarization was unavailable and `brief` is raw context. */
  fallback: boolean;
  /** True when some source context (transcript/scrollback/git) was found. */
  had_context: boolean;
}

// ---------------------------------------------------------------------------
// Review findings workflow (the 6-state finding lifecycle + Proof Pack)
// ---------------------------------------------------------------------------
// Mirrors `crates/otto-core/src/finding.rs`. A `Finding` is the tracked
// workflow record produced by the multi-agent code review; the action endpoints
// drive its `status` through six states and append an immutable `FindingEvent`
// audit trail (the "closing the loop with evidence" spine). See
// docs/superpowers/specs/2026-06-26-review-findings-workflow-design.md.

/** The 6-value workflow disposition of a finding. */
export type FindingStatus =
  | 'open'
  | 'accepted'
  | 'false_positive'
  | 'fixed'
  | 'verified'
  | 'waived';

/** Normalized finding severity (5-level scale). */
export type FindingSeverity = 'critical' | 'high' | 'medium' | 'low' | 'info';

/** A review finding as the workflow tracks it (`GET /reviews/{id}/findings`,
 *  the per-action responses). `finding_id` = `id`. */
export interface Finding {
  id: Id;
  review_id: Id;
  workspace_id: Id;
  repo_id: Id;
  pr_number: number | null;
  fingerprint: string;
  // --- the 11 required fields ---
  severity: FindingSeverity;
  category: string | null;
  path: string | null;
  /** Range start line (the legacy `line`). */
  line: number | null;
  /** Range end line. */
  line_end: number | null;
  title: string;
  body: string;
  evidence: string;
  agent_reasoning_summary: string;
  suggested_fix: string | null;
  status: FindingStatus;
  linked_commit: string | null;
  linked_test: string | null;
  /** The current disposition owner (producing agent at creation, triaging actor after). */
  reviewer: string;
  // --- workflow state / gates / artifacts ---
  /** Engine DETECTION lifecycle (read-only): open|fixing|resolved|regressed|declined. */
  state: string;
  /** Derived: the detection axis currently reads `regressed`. */
  regressed: boolean;
  requires_human_approval: boolean;
  approval_decision: string | null;
  approved_by: string | null;
  approved_at: string | null;
  jira_key: string | null;
  jira_url: string | null;
  produced_by_agent: string | null;
  repo_rule_id: Id | null;
  fix_session_id: Id | null;
  occurrence_count: number;
  created_at: string;
  updated_at: string;
}

/** One immutable audit-trail entry for a finding. */
export interface FindingEvent {
  id: Id;
  finding_id: Id;
  kind: string;
  actor: string;
  from_status: string | null;
  to_status: string | null;
  /** Free-form JSON payload (`{session_id?, commit?, test?, jira_key?, reason?, …}`). */
  detail: unknown;
  created_at: string;
}

/** A finding plus its full event timeline (`GET /findings/{id}`). */
export interface FindingDetail {
  finding: Finding;
  events: FindingEvent[];
}

/** The response of an action that may spawn a live agent session (fix / verify /
 *  regression-test). `session_id` is present when an openable session started. */
export interface FindingActionResp {
  finding: Finding;
  session_id?: Id | null;
}

/** A repo rule generalized from a finding, fed into the Context Engine. */
export interface RepoRule {
  id: Id;
  workspace_id: Id;
  title: string;
  body: string;
  category: string | null;
  severity: string | null;
  glob: string | null;
  source_finding_id: Id | null;
  enabled: boolean;
  created_by: Id;
  created_at: string;
  updated_at: string;
}

/** Aggregate counts for a Proof Pack. */
export interface ReviewProofPackSummary {
  total: number;
  by_status: Record<string, number>;
  by_severity: Record<string, number>;
  verified: number;
  fixed: number;
  open: number;
  with_commit: number;
  with_test: number;
}

/** One finding + its event timeline inside a Proof Pack. */
export interface ReviewProofPackEntry {
  finding: Finding;
  events: FindingEvent[];
}

/** The live-assembled evidence bundle for a review (`GET /reviews/{id}/proof-pack`). */
export interface ReviewProofPack {
  review_id: Id;
  workspace_id: Id;
  generated_at: string;
  summary: ReviewProofPackSummary;
  findings: ReviewProofPackEntry[];
  repo_rules: RepoRule[];
}

/** The persisted-snapshot response of `POST /reviews/{id}/proof-pack/export`. */
export interface ReviewProofPackExport {
  id: Id;
  review_id: Id;
  format: string;
  markdown: string;
  created_at: string;
}

// ---------------------------------------------------------------------------
// Issue tracking (Jira)
// ---------------------------------------------------------------------------

export type IssueProviderKind = 'jira';

export interface IssueAccount {
  id: Id;
  user_id: Id;
  provider: IssueProviderKind;
  label: string;
  email: string;
  base_url: string;
  /** User-entered token expiry; null = unknown/none. */
  token_expires_at: string | null;
  created_at: string;
}

export interface IssueProject {
  key: string;
  name: string;
}

export interface IssueSummary {
  key: string;
  summary: string;
  status: string;
  issue_type: string;
  url: string;
}

export interface IssueDetail {
  key: string;
  summary: string;
  status: string;
  issue_type: string;
  url: string;
  description: string | null;
  assignee: string | null;
}

export interface AttachedIssue {
  provider: 'jira';
  account_id: Id;
  key: string;
  summary: string;
  url: string;
  status: string;
}

// ---------------------------------------------------------------------------
// Integrations (Slack / Telegram)
// ---------------------------------------------------------------------------

export type Channel = 'slack' | 'telegram' | 'webhook';

export interface Integration {
  workspace_id: string;
  channel: Channel;
  enabled: boolean;
  allowed_users: string;     // comma-separated
  agent_reply: boolean;
  reply_instructions: string;
  channel_id: string;
  preferred_cli: string;       // '' = use the default agent
  has_bot_token: boolean;
  has_app_token: boolean;
  updated_at: string;
}

export interface UpsertIntegrationReq {
  enabled: boolean;
  bot_token?: string | null;   // omit/null to keep existing
  app_token?: string | null;   // slack only
  allowed_users: string;
  agent_reply: boolean;
  reply_instructions: string;
  channel_id: string;
  preferred_cli: string;       // '' = use the default agent
}

// ---------------------------------------------------------------------------
// Filesystem (GET /fs/read)
// ---------------------------------------------------------------------------

export interface FsRead {
  path: string;
  content: string;
  language: string;
  truncated: boolean;
}

// ---------------------------------------------------------------------------
// LSP capabilities (GET /lsp/capabilities)
// ---------------------------------------------------------------------------

export interface LspServerStatus {
  lang: string;
  available: boolean;
  command: string;
  install_command: string | null;
}

export interface LspCapabilities {
  servers: LspServerStatus[];
}

// ---------------------------------------------------------------------------
// Agent self-improvement
// ---------------------------------------------------------------------------

export type Autonomy = 'tiered' | 'propose' | 'auto';
export type ImprovementTrigger = 'scheduled' | 'manual';
export type ImprovementRunStatus = 'running' | 'done' | 'skipped' | 'failed';
export type ImprovementTarget = 'skill' | 'memory';
export type ImprovementEditKind = 'add' | 'modify' | 'remove';
export type ImprovementRisk = 'low' | 'structural';
export type ImprovementEditStatus =
  | 'pending'
  | 'applied'
  | 'rejected'
  | 'rolled_back'
  | 'conflict';

export interface SelfImprovementConfig {
  enabled: boolean;
  cadence_minutes: number;
  lookback_hours: number;
  skill_allowlist: string[];
  autonomy: Autonomy;
  /** Agent CLIs to run the analysis on — one set of suggestions per provider. */
  providers: string[];
  live_evolve: boolean;
  last_run_at: string | null;
  next_run_at: string | null;
}

export interface UpdateSelfImprovementReq {
  enabled: boolean;
  cadence_minutes: number;
  lookback_hours: number;
  skill_allowlist: string[];
  autonomy: Autonomy;
  providers: string[];
  live_evolve: boolean;
}

export interface ImprovementRun {
  id: string;
  workspace_id: string;
  trigger: ImprovementTrigger;
  status: ImprovementRunStatus;
  summary: string;
  sessions_reviewed: number;
  applied: number;
  pending: number;
  error: string | null;
  started_at: string;
  finished_at: string | null;
}

export interface ImprovementEdit {
  id: string;
  run_id: string;
  workspace_id: string;
  target: ImprovementTarget;
  target_ref: string;
  target_path: string;
  kind: ImprovementEditKind;
  risk: ImprovementRisk;
  status: ImprovementEditStatus;
  rationale: string;
  evidence: string[];
  before_content: string | null;
  after_content: string;
  applied_at: string | null;
  actor: string | null;
  created_at: string;
}

export interface RunNowResp {
  run_id: string;
}

// ---------------------------------------------------------------------------
// Context provisioning (library + per-workspace context/soul)
// ---------------------------------------------------------------------------

export interface LibrarySkill {
  name: string;
  description: string;
  body: string;
}

export interface LibrarySoul {
  name: string;
  body: string;
}

export interface LibraryContext {
  name: string;
  body: string;
}

export interface UpsertLibraryEntryReq {
  body: string;
}

// --- Bundled skills library (Settings → Skills) ----------------------------

/** Install state of a bundled skill vs. the installed library copy. */
export type BundledSkillState =
  | 'not_installed'
  | 'up_to_date'
  | 'update_available'
  | 'ahead';

/** A skill shipped with Otto, with its state relative to the installed copy. */
export interface BundledSkill {
  name: string;
  category: string;
  version: number;
  description: string;
  /** Version of the currently-installed copy, or null if not installed. */
  installed_version: number | null;
  state: BundledSkillState;
  /**
   * True iff the bundle is strictly newer than the installed copy
   * (`bundled > installed`) — the `update_available` state. Lets the UI show an
   * "Update" button. A hand-edited copy that is `ahead` is NOT an update.
   */
  update_available: boolean;
}

/** Result of installing a single bundled skill. */
export interface InstallBundledResp {
  name: string;
  installed: boolean;
  backed_up: boolean;
  /** Path of the backup taken before overwriting, when backed_up is true. */
  backup_path: string | null;
}

/** Result of installing every bundled skill (optionally a single category). */
export interface InstallAllBundledResp {
  installed: string[];
  backed_up: string[];
}

export interface GlobalSoulReq {
  name: string;
}

export interface GlobalSoulResp {
  name: string | null;
}

export interface WorkspaceContextConfig {
  skills: string[] | null; // null = all library skills
  soul: string | null; // null = global default
  extra_context_md: string;
  include_memory: boolean;
  include_repo_map?: boolean; // opt-in tree-sitter repo map
  repo_map_max_lines?: number | null;
}

export interface UpdateWorkspaceContextReq {
  skills: string[] | null;
  soul: string | null;
  extra_context_md: string;
  include_memory: boolean;
  include_repo_map?: boolean;
}

export interface MaterializeProviderResult {
  provider: string;
  files_written: string[];
  skipped: boolean;
}

export interface MaterializeResp {
  provider_results: MaterializeProviderResult[];
}

// --- Context preview (dry-run before spawn) --------------------------------

/**
 * How binding a planned artifact is on the agent:
 * - `advisory` — instruction files (AGENTS.md/CLAUDE.md) and skills: guidance
 *   the model reads and *may ignore*.
 * - `enforced` — hooks / runtime settings the daemon imposes regardless of what
 *   the model decides (e.g. activity-forwarding hooks).
 */
export type ContextEnforcement = 'advisory' | 'enforced';

/** A single artifact a spawn would write, described without writing it. */
export interface ContextPlanFile {
  /** Absolute destination path the file would be written to. */
  path: string;
  /** `instructions`, `skill`, `skill_asset`, `hooks`, or `manifest`. */
  kind: string;
  enforcement: ContextEnforcement;
  /** Size in bytes of the content that would be written. */
  size: number;
  /** First lines of the content (a short excerpt for the preview list). */
  first_lines: string;
  /** Whether content was elided from first_lines (file is larger). */
  truncated: boolean;
}

/** A skill that would be activated, summarized for the preview. */
export interface ContextPlanSkill {
  name: string;
  description: string;
  version: number;
}

/** What a session spawn would materialize for one provider (dry-run). */
export interface ContextPreviewProvider {
  provider: string;
  /** True for providers that materialize nothing (shell/agy/…). */
  skipped: boolean;
  skills: ContextPlanSkill[];
  /** The soul (persona) name that would apply, if any. */
  soul: string | null;
  files: ContextPlanFile[];
  /** The exact instruction-file bytes the model will read (Otto region merged
   * into AGENTS.md/CLAUDE.md). Empty when the provider writes no instructions. */
  generated_instructions: string;
  /** Name of the instruction file (`CLAUDE.md` or `AGENTS.md`), if any. */
  instructions_file_name: string | null;
  /** The hooks/settings JSON the runtime would impose (enforced), if any. */
  generated_hooks: string | null;
}

/** `POST /workspaces/{id}/context/preview` */
export interface ContextPreviewResp {
  providers: ContextPreviewProvider[];
}

/**
 * `POST /workspaces/{id}/context/preview` body. All fields optional: when
 * present they override the workspace's stored context selection so the UI can
 * preview a not-yet-saved choice (the same inputs a session spawn would use).
 */
export interface ContextPreviewReq {
  /** Provider to preview; omit for both `claude` and `codex`. */
  provider?: string;
  /**
   * Override the active skill allow-list. Omit the key to inherit the stored
   * config; send explicit `null` to override to *all* library skills; send a
   * list to use exactly those.
   */
  skills?: string[] | null;
  /**
   * Override the active soul. Omit the key to inherit the stored config; send
   * explicit `null` to override to the global default; send a name to use it.
   */
  soul?: string | null;
  /** Override the extra-context markdown (omit ⇒ use stored config). */
  extra_context_md?: string;
  /** Override the include-memory toggle (omit ⇒ use stored config). */
  include_memory?: boolean;
  /** Override the include-repo-map toggle (omit ⇒ use stored config). */
  include_repo_map?: boolean;
  /** Working directory the spawn would use (omit ⇒ the workspace root). */
  cwd?: string;
}

// ---------------------------------------------------------------------------
// API client ("Postman" section) — workspace-scoped
// ---------------------------------------------------------------------------

export interface ApiKeyVal {
  key: string;
  value: string;
  enabled?: boolean;
}

export type ApiAuth =
  | { type: 'none' }
  | { type: 'bearer'; token: string }
  | { type: 'basic'; username: string; password: string }
  | { type: 'api_key'; key: string; value: string; in: 'header' | 'query' }
  | {
      type: 'oauth2';
      grant: 'client_credentials' | 'password' | 'refresh_token';
      token_url: string;
      client_id: string;
      client_secret: string;
      scope: string;
      username: string;
      password: string;
      refresh_token: string;
      access_token: string;
      token_type: string;
    };

export type ApiBodyMode = 'none' | 'json' | 'raw' | 'form' | 'multipart' | 'graphql';

export interface ApiCollection {
  id: Id;
  workspace_id: Id;
  name: string;
  parent_id: Id | null;
  position: number;
  created_at: string;
}

export interface ApiRequest {
  id: Id;
  workspace_id: Id;
  collection_id: Id | null;
  name: string;
  method: string;
  url: string;
  headers: ApiKeyVal[];
  query: ApiKeyVal[];
  body_mode: ApiBodyMode;
  body: string;
  auth: ApiAuth;
  /** Optional `ssh`-kind connection id to tunnel executions through (null = direct). */
  ssh_connection_id?: Id | null;
  position: number;
  created_at: string;
  updated_at: string;
}

export interface ApiEnvironment {
  id: Id;
  workspace_id: Id;
  name: string;
  variables: Record<string, string>;
  is_active: boolean;
  created_at: string;
}

export interface ApiHistoryEntry {
  id: Id;
  workspace_id: Id;
  method: string;
  url: string;
  status: number | null;
  duration_ms: number | null;
  request: unknown;
  response: unknown;
  executed_at: string;
}

export interface UpsertApiCollectionReq {
  name: string;
  parent_id?: Id | null;
}

export interface UpsertApiRequestReq {
  collection_id?: Id | null;
  name: string;
  method: string;
  url: string;
  headers?: ApiKeyVal[];
  query?: ApiKeyVal[];
  body_mode?: ApiBodyMode;
  body?: string;
  auth?: ApiAuth;
  /** Optional `ssh`-kind connection id to tunnel executions through. */
  ssh_connection_id?: Id | null;
}

export interface UpsertApiEnvironmentReq {
  name: string;
  variables?: Record<string, string>;
}

export interface ExecuteApiReq {
  method: string;
  url: string;
  headers?: ApiKeyVal[];
  query?: ApiKeyVal[];
  body_mode?: ApiBodyMode;
  body?: string;
  auth?: ApiAuth;
  environment_id?: Id | null;
  timeout_ms?: number | null;
  follow_redirects?: boolean | null;
  verify_ssl?: boolean | null;
  vars?: Record<string, string> | null;
  /** Route the request through this `ssh`-kind connection (SOCKS5 over SSH). */
  ssh_connection_id?: Id | null;
}

export interface ApiResponse {
  status: number;
  status_text: string;
  headers: ApiKeyVal[];
  body: string;
  /** Full response bytes, base64 (binary preview + save to disk). Empty when too_large. */
  body_base64: string;
  /** `body` was truncated for display (full bytes still in body_base64). */
  truncated: boolean;
  /** Body exceeded the inline cap: body + body_base64 are empty, only save-from-server unavailable. */
  too_large: boolean;
  duration_ms: number;
  size_bytes: number;
  content_type: string | null;
  /** Per-phase execution trace for the response "Trace" tab. */
  trace: TraceStep[];
}

export interface TraceStep {
  label: string;
  detail: string;
  ms: number | null;
  /** info | timing | redirect | success | error */
  level: string;
}

export interface ImportCurlReq {
  curl: string;
}

export interface ParsedCurl {
  method: string;
  url: string;
  headers: ApiKeyVal[];
  query: ApiKeyVal[];
  body_mode: ApiBodyMode;
  body: string;
  auth: ApiAuth;
}

export interface ApiAssertion {
  kind: 'status' | 'json_path' | 'duration_ms';
  /** JSON path into the response body, for kind='json_path'. */
  path?: string;
  op: 'eq' | 'ne' | 'contains' | 'lt' | 'gt';
  value: string;
}

export interface ApiExtract {
  /** JSON path into the response body. */
  path: string;
  /** Environment variable to set from the extracted value (used by later steps). */
  var: string;
}

export interface ApiAutomationStep {
  request_id: Id;
  assertions: ApiAssertion[];
  extract: ApiExtract[];
}

export interface ApiAutomation {
  id: Id;
  workspace_id: Id;
  name: string;
  steps: ApiAutomationStep[];
  created_at: string;
}

export interface UpsertApiAutomationReq {
  name: string;
  steps?: ApiAutomationStep[];
}

export interface ApiRunStepResult {
  request_id: Id;
  name: string;
  status: number | null;
  duration_ms: number;
  ok: boolean;
  assertions: { desc: string; passed: boolean }[];
  error: string | null;
}

export interface ApiRunResult {
  automation_id: Id;
  steps: ApiRunStepResult[];
  passed: boolean;
}

// ---------------------------------------------------------------------------
// Skills Evaluator
// ---------------------------------------------------------------------------

export type SkillEvalStatus = 'running' | 'done' | 'error' | 'cancelled';
export type EvalAgentStatus = 'pending' | 'running' | 'waiting' | 'done' | 'error';
export type EvalIterStatus =
  | 'pending'
  | 'implementing'
  | 'validating'
  | 'improving'
  | 'done'
  | 'error';

/** One problem a validation found, with the concrete suggested fix. */
export interface EvalFinding {
  /** 'info' | 'warn' | 'fail' */
  severity: string;
  issue: string;
  suggestion: string;
  location?: string | null;
}

/** Live state of one validation agent (validation × provider) in an iteration. */
export interface EvalValidationState {
  validation: string;
  name: string;
  provider: string;
  model: string;
  status: EvalAgentStatus;
  note: string;
  passed: boolean;
  score: number;
  session_id?: string | null;
  findings: EvalFinding[];
}

/** One scored signal (tests, lint, review) on a 0–100 scale. */
export interface SignalScore {
  ran: boolean;
  score: number;
  detail: string;
}

/** The diff-quality signal: parsimony + risk of the produced change. */
export interface DiffScore {
  ran: boolean;
  files_changed: number;
  additions: number;
  deletions: number;
  risky: number;
  score: number;
  detail: string;
}

/** The human-rating signal. */
export interface HumanScore {
  rating?: number | null;
  note: string;
  rater: string;
  score: number;
}

/** Relative weights for the composite (renormalized over the signals that ran). */
export interface ScoreWeights {
  tests: number;
  lint: number;
  diff: number;
  review: number;
  human: number;
}

/** The full multi-signal score for one iteration's produced code. */
export interface EvalScore {
  tests: SignalScore;
  lint: SignalScore;
  diff: DiffScore;
  review: SignalScore;
  human: HumanScore;
  weights: ScoreWeights;
  /** Weighted mean over the signals that ran, 0–100. */
  composite: number;
  /** Proof pack status: missing|partial|passed|failed|waived. */
  proof_status: string;
  /** Proof done-contract score 0–100. */
  done_score: number;
}

/** One iteration (round) of a skill evaluation. */
export interface EvalIteration {
  id: Id;
  eval_id: Id;
  iter: number;
  base_iter?: number | null;
  skill_name: string;
  skill_before: string;
  skill_after?: string | null;
  impl_provider: string;
  impl_session_id?: string | null;
  impl_summary: string;
  worktree_path?: string | null;
  status: EvalIterStatus;
  note: string;
  score: number;
  agents: EvalValidationState[];
  improvement_summary: string;
  skill_diff: string;
  /** Multi-signal score (null until the scoring pipeline has run). */
  scoring?: EvalScore | null;
  /** The Proof Pack assembled for this iteration's evidence. */
  proof_pack_id?: string | null;
  /** Human quality rating 0–5. */
  human_rating?: number | null;
  human_note?: string;
  human_rater?: string;
  created_at: string;
}

/** A complete skill-evaluation run. */
export interface SkillEval {
  id: Id;
  workspace_id: Id;
  source_skill: string;
  task: string;
  impl_cli: string;
  target_iterations: number;
  status: SkillEvalStatus;
  error?: string | null;
  summary: string;
  best_iteration?: number | null;
  best_score?: number | null;
  iterations: EvalIteration[];
  /** The original StartSkillEvalReq JSON (for per-validation retry + display). */
  config?: unknown;
  /** 'generate' | 'score_only' */
  mode?: string;
  golden_task_id?: string | null;
  matrix_id?: string | null;
  dim_provider?: string | null;
  dim_skill?: string | null;
  dim_prompt?: string | null;
  /** Best iteration's composite score. */
  composite_score?: number | null;
  promoted?: boolean;
  promoted_at?: string | null;
  promoted_by?: string | null;
  created_at: string;
}

/** A reusable, per-repo evaluation task (golden corpus + regression cases). */
export interface GoldenTask {
  id: Id;
  workspace_id: Id;
  repo_key: string;
  name: string;
  prompt: string;
  skill: string;
  test_cmd: string;
  lint_cmd: string;
  build_cmd: string;
  rubric: string;
  tags: string[];
  /** 'manual' | 'regression' */
  origin: string;
  source_eval_id?: string | null;
  source_iter_id?: string | null;
  enabled: boolean;
  created_by: Id;
  created_at: string;
  updated_at: string;
}

export interface GoldenTaskReq {
  name: string;
  prompt: string;
  skill?: string;
  test_cmd?: string;
  lint_cmd?: string;
  build_cmd?: string;
  rubric?: string;
  tags?: string[];
  repo_key?: string | null;
  enabled?: boolean;
}

/** Where a score-only run (or matrix cell) finds the code to score. */
export interface EvalTarget {
  /** 'working' | 'branch' | 'path' */
  kind: string;
  git_ref?: string | null;
  path?: string | null;
}

export interface RunGoldenReq {
  /** 'generate' | 'score_only' (default) */
  mode?: string;
  provider?: string | null;
  target?: EvalTarget | null;
}

export interface RateIterationReq {
  /** 0–5 */
  rating: number;
  note?: string;
}

export interface RegressionReq {
  name?: string | null;
}

/** Whether a run's winning skill may be promoted, with the reasons if not. */
export interface PromoteGate {
  allowed: boolean;
  score: number;
  threshold: number;
  proof_status: string;
  require_proof: boolean;
  score_ok: boolean;
  proof_ok: boolean;
  reasons: string[];
}

/** One prompt column-input of a matrix. */
export interface MatrixPrompt {
  label: string;
  task: string;
  golden_task_id?: string | null;
}

/** One executed cell of a matrix (a single skill_eval run). */
export interface MatrixCell {
  eval_id: Id;
  provider: string;
  skill: string;
  prompt: string;
  status: string;
  composite_score?: number | null;
  proof_status: string;
  best_iteration?: number | null;
}

/** A provider × skill × prompt comparison run. */
export interface EvalMatrix {
  id: Id;
  workspace_id: Id;
  name: string;
  status: string;
  repo_key: string;
  mode: string;
  providers: string[];
  skills: string[];
  prompts: MatrixPrompt[];
  cells: MatrixCell[];
  created_at: string;
}

export interface StartMatrixReq {
  name: string;
  mode?: string;
  providers: string[];
  skills: SkillSourceReq[];
  prompts: MatrixPrompt[];
  target?: EvalTarget | null;
  test_cmd?: string | null;
  lint_cmd?: string | null;
  base_ref?: string | null;
  weights?: ScoreWeights | null;
  validations?: SkillEvalValidationCfg[];
  iterations?: number;
}

/** One configurable validation dimension. */
export interface SkillEvalValidationCfg {
  name: string;
  criteria: string;
  /** CLIs to run this validation on (one agent each). */
  providers: string[];
  model: string;
}

export interface SkillEvalImproverCfg {
  provider: string;
  model: string;
}

export interface SkillEvalConfig {
  validations: SkillEvalValidationCfg[];
  improver: SkillEvalImproverCfg;
  iterations: number;
  /** Validation passes to average (1–3) — reduces grader noise. */
  validator_passes: number;
  /** Default composite-score weights. */
  weights?: ScoreWeights;
  /** Minimum composite score (0–100) required to promote. */
  promote_min_score?: number;
  /** Whether promotion also requires the iteration's proof pack to pass. */
  require_proof_pass?: boolean;
  default_test_cmd?: string;
  default_lint_cmd?: string;
}

/** Where the skill under test comes from. */
export interface SkillSourceReq {
  /** 'library' | 'path' | 'provider' */
  kind: string;
  reference: string;
  provider?: string | null;
}

export interface StartSkillEvalReq {
  source: SkillSourceReq;
  task: string;
  impl_cli: string;
  validations: SkillEvalValidationCfg[];
  iterations: number;
  improver?: SkillEvalImproverCfg | null;
  base_ref?: string | null;
  /** Validation passes to average (1–3). */
  validator_passes?: number;
  /** 'generate' (default) | 'score_only' */
  mode?: string;
  golden_task_id?: string | null;
  target?: EvalTarget | null;
  test_cmd?: string | null;
  lint_cmd?: string | null;
  weights?: ScoreWeights | null;
}

export interface PromoteSkillReq {
  iteration_id: Id;
  /** 'tested' = the skill that iteration ran with; 'improved' = its edited version. */
  source: 'tested' | 'improved';
  name: string;
  /** Bypass the score+proof gate (root only; audited + waives proof). */
  force?: boolean;
}

export interface ImplDiffResp {
  diff: string;
  truncated: boolean;
}

export interface SkillSourceInfo {
  /** 'library' | 'provider' */
  kind: string;
  name: string;
  description: string;
  provider?: string | null;
}

export interface SkillSourcesResp {
  sources: SkillSourceInfo[];
}

// ---------------------------------------------------------------------------
// Workflow engine (mirrors otto_core::workflows)
// ---------------------------------------------------------------------------

/** Per-node retry policy (exponential backoff). */
export interface RetryPolicy {
  max_attempts: number;
  backoff_ms: number;
  factor: number;
}

export interface WorkflowNode {
  id: string;
  kind: string;
  name: string;
  x: number;
  y: number;
  params: unknown;
  /** Optional retry policy for this node (null/absent = run once). */
  retry?: RetryPolicy | null;
}

export interface WorkflowEdge {
  id: string;
  source: string;
  target: string;
  /** Optional expression gating this edge: the target runs only when truthy. */
  condition?: string | null;
}

export interface WorkflowGraph {
  nodes: WorkflowNode[];
  edges: WorkflowEdge[];
}

export interface Workflow {
  id: Id;
  workspace_id: Id;
  name: string;
  description: string;
  graph: WorkflowGraph;
  /** Monotonic version counter, bumped on each graph edit/restore. */
  version?: number;
  created_by: Id;
  created_at: string;
  updated_at: string;
}

// NOTE: named `WorkflowRunStatus` (not `RunStatus`) to avoid colliding with the
// Run-with-Otto `RunStatus` stage machine below. Rust namespaces these per crate
// module (`otto-core::workflows` vs `otto-core::run`); this flat TS file cannot.
export type WorkflowRunStatus = 'pending' | 'running' | 'success' | 'error' | 'canceled';
export type NodeStatus = 'pending' | 'running' | 'success' | 'error' | 'skipped';

export interface NodeRunState {
  node_id: string;
  status: NodeStatus;
  output?: unknown;
  error?: string | null;
  logs: string[];
  duration_ms?: number | null;
  /** Number of attempts made (>1 when a retry policy fired). */
  attempts?: number | null;
  /** Session ids this node drove (e.g. agent_prompt / review_run). */
  sessions?: string[];
}

export interface WorkflowRun {
  id: Id;
  workflow_id: Id;
  workspace_id: Id;
  status: WorkflowRunStatus;
  input: unknown;
  nodes: NodeRunState[];
  error?: string | null;
  started_at: string;
  finished_at?: string | null;
  /** True when the run is paused waiting for a human_approval node decision. */
  waiting_approval?: boolean;
  /** The node id of the human_approval node the run is paused at. */
  approval_node_id?: string | null;
  /** User id of the person who approved (null if rejected or pending). */
  approved_by?: string | null;
  /** Human note attached to the approval/rejection decision. */
  approval_note?: string | null;
  /** RFC-3339 timestamp when the decision was recorded. */
  approved_at?: string | null;
  /** The workflow version this run executed against. */
  workflow_version?: number | null;
  /** Proof pack assembled for this run, if any. */
  proof_pack_id?: string | null;
}

/** Lightweight summary of an in-flight run for the "Running" sidebar list.
 *  Returned by `GET /workspaces/{wid}/workflow-runs/active`. */
export interface ActiveWorkflowRun {
  run_id: Id;
  workflow_id: Id;
  workspace_id: Id;
  workflow_name: string;
  status: WorkflowRunStatus;
  started_at: string;
  /** Total nodes and how many have finished (success|skipped) — "3/5 steps". */
  nodes_total: number;
  nodes_done: number;
  /** Paused on a human-approval node. */
  waiting_approval?: boolean;
}

// ---------------------------------------------------------------------------
// Workflow triggers (schedule / webhook / event)
// ---------------------------------------------------------------------------

export type TriggerKind = 'schedule' | 'webhook' | 'event';

/** A workflow trigger row from the database. */
export interface WorkflowTrigger {
  id: Id;
  workflow_id: Id;
  kind: TriggerKind;
  /** Kind-specific configuration object. */
  spec: Record<string, unknown>;
  enabled: boolean;
  created_at: string;
}

export interface CreateTriggerReq {
  kind: TriggerKind;
  spec?: Record<string, unknown>;
  enabled?: boolean;
}

export interface UpdateTriggerReq {
  spec?: Record<string, unknown>;
  enabled?: boolean;
}

// NOTE: named `WorkflowApproveRunReq` (not `ApproveRunReq`) to avoid colliding
// with the Run-with-Otto `ApproveRunReq` below — same per-module-vs-flat-file
// reason as WorkflowRunStatus.
export interface WorkflowApproveRunReq {
  node_id: string;
  approved: boolean;
  note?: string | null;
}

export interface NodeTypeSpec {
  kind: string;
  label: string;
  category: string;
  description: string;
  inputs: number;
  outputs: number;
  color: string;
  icon: string;
  /** JSON schema describing this node's output shape (for downstream wiring). */
  output_schema?: unknown;
  /** JSON schema describing this node's accepted params. */
  params_schema?: unknown;
}

/** An immutable snapshot of a workflow's graph (append-only version history). */
export interface WorkflowVersion {
  id: string;
  workflow_id: string;
  version: number;
  name: string;
  description: string;
  graph: WorkflowGraph;
  note: string;
  created_by: string;
  created_at: string;
}

export interface CreateWorkflowReq {
  name: string;
  description?: string | null;
  graph?: WorkflowGraph | null;
}

export interface GenerateWorkflowReq {
  description: string;
  name?: string | null;
}

// ---------------------------------------------------------------------------
// Database Explorer (DB module) — mirrors crates/otto-* DB endpoints.
// ---------------------------------------------------------------------------

/** Database engines the explorer can talk to (subset of ConnectionKind). */
export type DbEngine = 'mysql' | 'redis' | 'mongodb' | 'clickhouse';

/** Schema-tree node taxonomy across SQL / Redis / Mongo. */
export type DbNodeKind =
  | 'database'
  | 'schema'
  | 'table'
  | 'view'
  | 'column'
  | 'index'
  | 'collection'
  | 'field'
  | 'keyspace'
  | 'key_namespace'
  | 'key'
  | 'folder';

/** One lazy node in the schema tree (databases → tables → columns, etc.). */
export interface SchemaNode {
  id: string;
  label: string;
  kind: DbNodeKind;
  detail?: string;
  has_children: boolean;
}

export interface DbColumnDef {
  name: string;
  data_type: string;
  nullable: boolean;
  default?: string | null;
  key?: string | null;
  extra?: string | null;
  comment?: string | null;
}

export interface DbIndexDef {
  name: string;
  columns: string[];
  unique: boolean;
  method?: string | null;
}

export interface DbForeignKey {
  name: string;
  columns: string[];
  ref_table: string;
  ref_columns: string[];
  ref_schema?: string | null;
}

/** Full detail for a selected object (table / view / collection / key). */
export interface ObjectDetail {
  name: string;
  kind: DbNodeKind;
  columns: DbColumnDef[];
  primary_key: string[];
  indexes: DbIndexDef[];
  foreign_keys: DbForeignKey[];
  ddl?: string | null;
  row_count?: number | null;
  extra?: unknown;
}

/** One column on an ERD diagram card (trimmed ColumnDef + PK/FK flags). */
export interface DbGraphColumn {
  name: string;
  data_type: string;
  nullable: boolean;
  primary_key: boolean;
  foreign_key: boolean;
}

/** A table/view/collection node in the schema graph (an ERD card). */
export interface DbGraphTable {
  /** Opaque NodePath id (e.g. `db:shop/table:orders`) for opening elsewhere. */
  id: string;
  schema: string;
  name: string;
  kind: DbNodeKind;
  columns: DbGraphColumn[];
}

/** A foreign-key relationship between two graph tables (an ERD edge). */
export interface DbGraphEdge {
  name: string;
  from_table: string;
  from_columns: string[];
  to_schema: string;
  to_table: string;
  to_columns: string[];
}

/** The relationship graph (ERD) for one schema: tables + FK edges. Engines
 *  without FK metadata (Redis/Mongo) return `relationships: false` + no edges. */
export interface DbSchemaGraph {
  schema: string;
  tables: DbGraphTable[];
  edges: DbGraphEdge[];
  relationships: boolean;
  truncated: boolean;
}

/** A column in a query result set. */
export interface DbColumn {
  name: string;
  type_hint?: string | null;
}

export interface QueryStats {
  duration_ms: number;
  row_count: number;
  bytes_read?: number | null;
}

/** Result of running a statement: tabular rows + stats. */
export interface QueryResult {
  columns: DbColumn[];
  rows: unknown[][];
  rows_affected?: number | null;
  stats: QueryStats;
  message?: string | null;
  truncated: boolean;
  /** True when the server ran cell values through `otto_core::redact` (QueryRequest.mask=true). */
  masked?: boolean;
}

/**
 * Body for the read-only MCP query endpoint
 * (`POST /api/v1/connections/{id}/db/mcp-query`) — the agent-facing query path used
 * by `ottod mcp-tools`. Writes/DDL are refused server-side (`mcp_read_only:`); rows
 * are hard-capped (200) and PII-masked. Mirrors `McpQueryReq` in
 * `crates/otto-dbviewer/src/http.rs`.
 */
export interface McpDbQueryReq {
  statement: string;
  max_rows?: number | null;
  node?: string | null;
}

/** Selectable output format for the streaming local-file export. */
export type DbExportFormat =
  | 'csv'
  | 'csv_with_names'
  | 'tsv'
  | 'tsv_with_names'
  | 'json'
  | 'ndjson';

/**
 * `POST /connections/{id}/db/export-to-path` — stream an uncapped result to a
 * local file on the daemon host (selectable format, configurable path). The
 * daemon streams the result row/chunk-by-chunk so its memory stays bounded
 * regardless of size.
 */
export interface ExportToPathReq {
  statement: string;
  /** Active database to scope unqualified names (same as a query's `node`). */
  node?: string | null;
  /** Output format; default `csv`. */
  format?: DbExportFormat;
  /**
   * Destination on the daemon host. A leading `~` expands to the daemon user's
   * home. An existing directory → `<dir>/export.<ext>`; otherwise a full file
   * path whose parent dir is created.
   */
  local_path: string;
  /** Optional row cap; blank/absent = all rows. */
  max_rows?: number | null;
}

/** `POST /connections/{id}/db/export-to-path` response. */
export interface ExportToPathResp {
  /** The absolute file path actually written. */
  local_path: string;
  /** Rows written. */
  rows: number;
  /** Bytes written to the file. */
  bytes: number;
  /** Wall-clock duration of the export, in milliseconds. */
  duration_ms: number;
}

/** Supported import file formats (the mirror of the export). Delimited formats
 *  take the first row as the header; JSON/NDJSON carry keys per object. */
export type ImportFormat = 'csv' | 'tsv' | 'ndjson' | 'json';

/**
 * `POST /connections/{id}/db/import` — import a local file (on the daemon host)
 * into an existing SQL table. Each batch runs through the same guarded write
 * path as a query, so a Prod/read-only connection refuses the import unless
 * `confirm_write` is set (the typed-confirmation flow re-sends it). v1 is
 * SQL-only (MySQL/ClickHouse); Mongo/Redis are explicit follow-ups.
 */
export interface ImportReq {
  /** Source file on the daemon host (a leading `~` expands to the daemon home). */
  local_path: string;
  /** File format. */
  format: ImportFormat;
  /** Destination table (must already exist). */
  table: string;
  /** Rows per INSERT; clamped 1..=5000 server-side (default 500). */
  batch_size?: number;
  /** Typed-confirmation acknowledgement for a guarded (Prod/read-only) connection. */
  confirm_write?: boolean;
}

/** One streamed NDJSON line from `POST …/db/import`. The final line carries
 *  `done` (with `rows`/`batches`) or `error` (text starting `write_blocked:`
 *  means a guarded connection needs the typed confirmation). */
export interface ImportResult {
  done?: boolean;
  rows?: number;
  batches?: number;
  error?: string;
}

/**
 * `POST /connections/{id}/db/nl-to-sql` — draft a read query from plain English,
 * validated with `EXPLAIN` against the live schema before it's returned. Never
 * emits a write/DDL. A 400 starting "NL-to-SQL is not configured" means no
 * drafter is wired; a 400 starting "could not produce a valid read query" means
 * the bounded retry loop was exhausted (its message carries the last engine error).
 */
export interface NlToSqlReq {
  /** The user's plain-English question. */
  question: string;
  /** Optional active-database node (same semantics as a query's `node`). */
  node?: string;
  /** Draft/validate retries; clamped 1..=4 server-side (default 3). */
  max_attempts?: number;
}

/** A validated read query: the SQL, its plan text (from EXPLAIN), how many
 *  drafting attempts it took, and any non-fatal notes. */
export interface NlToSqlOutcome {
  sql: string;
  plan: string;
  attempts: number;
  warnings: string[];
}

/** Entry mode for the embedded DB Assistant panel:
 *  - `nl`          — "Ask in English": produce a runnable query for the question.
 *  - `ask`         — "Ask AI": free-form question about the data/schema.
 *  - `investigate` — examine a result/object (seeded with the statement + a small
 *                    sample of the result columns/rows via `result_context`). */
export type DbAssistMode = 'nl' | 'ask' | 'investigate';

/**
 * `POST /connections/{id}/db/assist` — run ONE turn of the file-backed, embedded
 * DB Assistant. The agent runs as a managed Otto session in an ephemeral, trusted
 * working dir seeded with the full schema + a read-only `q` tool; it never touches
 * the DB directly. Returns the assist id (resume key), the live agent session id
 * (the panel mounts a `<Terminal>` on it), and the agent's proposed SQL + note.
 */
export interface DbAssistReq {
  /** The user's question / instruction for this turn. */
  question: string;
  /** Entry mode (defaults to `ask` server-side). */
  mode?: DbAssistMode;
  /** Active-database node to scope the schema/queries (same as a query's `node`). */
  node?: string;
  /** Agent CLI to run (claude/codex/…); defaults to the workspace/global default. */
  provider?: string;
  /** investigate-mode seed: the current statement + a small result sample. */
  result_context?: string;
  /** Resume an existing assist (its session) instead of starting a new one. */
  assist_id?: string;
}

/** The result of one DB Assistant turn. */
export interface DbAssistResp {
  /** Stable id for this assist (resume key; also the DELETE/summary path segment). */
  assist_id: Id;
  /** The live agent session id — the panel binds `<Terminal>` to `/ws/term/{id}`. */
  session_id: Id;
  /** The agent's current proposed SQL (its ANSWER.sql), or empty. */
  sql: string;
  /** A one-line explanation/note from the agent, or empty. */
  note: string;
}

/** `POST /connections/{id}/db/assist/{aid}/summary` — the rendered investigation
 *  summary (the panel downloads it as a `.md` file). */
export interface DbAssistSummaryResp {
  markdown: string;
}

export type DbCompletionKind =
  | 'keyword'
  | 'function'
  | 'table'
  | 'view'
  | 'column'
  | 'database'
  | 'collection'
  | 'field'
  | 'command'
  | 'operator';

export interface DbCompletionItem {
  label: string;
  kind: DbCompletionKind;
  detail?: string | null;
  insert_text?: string | null;
  /**
   * Ranking hint mapped to CodeMirror's `boost`. Higher sorts earlier among
   * equally-matching options — index columns/fields out-rank plain ones, and
   * tables out-rank keywords in a slot where they're expected.
   */
  score?: number | null;
}

/** What a given engine supports — drives the UI affordances. */
export interface DbCapabilities {
  engine: DbEngine;
  sql: boolean;
  joins: boolean;
  transactions: boolean;
  multi_statement: boolean;
  default_port: number;
  schema_levels: string[];
  query_language: 'sql' | 'redis' | 'mongo';
}

export interface DbTestResult {
  ok: boolean;
  latency_ms?: number | null;
  message: string;
  server_version?: string | null;
}

export interface DbSavedQuery {
  id: string;
  workspace_id: string;
  connection_id?: string | null;
  name: string;
  statement: string;
  created_by: string;
  created_at: string;
}

export interface DbHistoryEntry {
  id: string;
  connection_id: string;
  statement: string;
  ok: boolean;
  duration_ms: number;
  row_count: number;
  error?: string | null;
  created_at: string;
}

/** Supported widget visualizations. */
export type DbViz = 'table' | 'line' | 'bar' | 'area' | 'pie' | 'number';

/** Maps result columns onto a chart's axes/series. */
export interface DbWidgetMapping {
  x?: string;
  y?: string[];
  category?: string;
  value?: string;
}

export interface DbWidget {
  id: string;
  workspace_id: string;
  dashboard_id?: string | null;
  connection_id: string;
  title: string;
  statement: string;
  viz: DbViz;
  mapping: DbWidgetMapping;
  options: Record<string, unknown>;
  created_at: string;
  updated_at: string;
}

/** One tile placement on a dashboard grid. */
export interface DbLayoutItem {
  widget_id: string;
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface DbDashboard {
  id: string;
  workspace_id: string;
  name: string;
  layout: DbLayoutItem[];
  refresh_secs?: number | null;
  created_at: string;
  updated_at: string;
}

/** A ready-made example workflow (e.g. a game pipeline: agent design + engine). */
export interface WorkflowTemplate {
  id: string;
  name: string;
  description: string;
  icon: string;
  graph: WorkflowGraph;
}

// ---------------------------------------------------------------------------
// Insights — scheduled (opt-in, catch-up) HTML reports.
// ---------------------------------------------------------------------------

/** Which scheduled insight reports are turned on. All default to false (opt-in). */
export interface InsightsConfig {
  /** Previous day, generated the next morning the app is open. */
  daily: boolean;
  /** Previous week (Sunday), catch-up if the app was closed. */
  weekly: boolean;
  /** Previous month (1st), catch-up if the app was closed. */
  monthly: boolean;
}

export type InsightKind = 'daily' | 'weekly' | 'monthly' | 'adhoc';

/** One generated insight report. `html_path` points at the rendered HTML. */
export interface InsightReport {
  kind: InsightKind;
  /** ISO start of the period the report covers. */
  period_start: string;
  /** ISO end of the period the report covers. */
  period_end: string;
  /** On-disk path to the rendered HTML report. */
  html_path: string;
  /** A ≤10-sentence plain-text summary of the report. */
  summary: string;
  /** ISO timestamp the report was created. */
  created_at: string;
}

/** The period to run an ad-hoc insights report for. */
export type InsightRunPeriod = 'day' | 'week' | 'month';

export interface RunInsightsReq {
  period: InsightRunPeriod;
  /** How many periods back (0 = the most recent complete period). */
  offset?: number;
}

export interface RunInsightsResp {
  started: boolean;
  /** Session id of the spawned insights run (when started === true). */
  run_id?: string | null;
  /** Human-readable explanation when started === false (e.g. skill not installed). */
  reason?: string | null;
}

// ---------------------------------------------------------------------------
// Share-link tokens (Phase 3 — remote/mobile access)
// Mirrors crates/otto-core/src/api.rs ShareInfo / CreateShareReq / CreateShareResp.
// ---------------------------------------------------------------------------

/** Metadata for one scoped share-link token. The raw token is never present
 *  after the initial mint response (only the first 12 chars are stored). */
export interface ShareInfo {
  id: Id;
  /** The single session this token may reach. */
  session_id: Id;
  /** Capped role on that session: 'viewer' (read-only) or 'editor' (read + input). */
  role: WorkspaceRole;
  /** First 12 chars of the raw token, for display/identification. */
  token_prefix: string;
  label: string | null;
  created_at: string;
  /** FIXED expiry (created_at + ttl); never slid for share tokens. */
  expires_at: string;
}

/** `POST /api/v1/sessions/{id}/share` request body. */
export interface CreateShareReq {
  /** 'viewer' (read-only) or 'editor' (read + input). Never 'admin'. */
  role: string;
  /** Fixed TTL in seconds (plain share only). Absent → 3600. Clamped to [60, 86400]. */
  ttl_secs?: number;
  /** Human-friendly label (e.g. "for Alice"). Optional. */
  label?: string;
  /** Email-OTP gate (mobile plan Task 7.2). When set, the recipient must redeem a
   *  6-digit code emailed to THIS address (POST /share/verify) before attaching —
   *  a leaked link alone is useless. The address is LOCKED for the share's life.
   *  Requires a verified email sender; absent → a plain share with no OTP gate. */
  recipient_email?: string;
  /** Session window (seconds) for an OTP-gated share — how long the guest may stay
   *  attached once verified. Clamped server-side to (0, 43200] (≤12h). Only
   *  meaningful with `recipient_email`; absent → default 1h. */
  duration_secs?: number;
}

/** `POST /api/v1/share/verify` — redeem an emailed OTP for a share token (mobile
 *  plan Task 7.3). Public/Exempt: the `token` (the share link) is the auth. */
export interface VerifyShareReq {
  /** The raw share token (from the `#/s/<session>/<token>` link). */
  token: string;
  /** The 6-digit one-time code the recipient received by email. */
  otp: string;
}

/** Response for `POST /api/v1/share/verify`. */
export interface VerifyShareResp {
  /** `true` once the code matched and the share is verified for attach. */
  verified: boolean;
}

/** `POST /api/v1/share/extend` — re-issue a FRESH OTP for an existing OTP share,
 *  emailed to the LOCKED original recipient ONLY (mobile plan Task 7.4).
 *  Public/Exempt: the `token` (the share link) is the auth. The request carries
 *  NO email field by design — the destination is read from the share row, never
 *  the request — so access can never be redirected to a different mailbox. The
 *  fresh code re-pends the share and opens a fresh ≤12h window; the guest then
 *  re-verifies via `POST /api/v1/share/verify`. Returns `{ ok: true }`. */
export interface ExtendShareReq {
  /** The raw share token (from the `#/s/<session>/<token>` link). */
  token: string;
}

/** Response for `POST /api/v1/sessions/{id}/share`. The raw token is returned
 *  exactly once — store or copy it immediately. */
export interface CreateShareResp {
  /** The raw share token (shown exactly once). */
  token: string;
  /** Ready-to-use share URL (`<origin>/#/s/<session_id>/<token>`). */
  url: string;
  /** Metadata for the newly-minted share. */
  info: ShareInfo;
}

// ---------------------------------------------------------------------------
// Email sender (Gmail App Password → Keychain; mobile plan Task 7.1).
// Mirrors crates/otto-core/src/api.rs SetEmailSenderReq / EmailSenderResp.
// ---------------------------------------------------------------------------

/** `PUT /api/v1/email-sender` request body. The app password is stored in the
 *  macOS Keychain (never the DB) and validated via a real Gmail SMTP login. */
export interface SetEmailSenderReq {
  /** The Gmail address mail is sent from (also the SMTP AUTH username). */
  gmail_address: string;
  /** The 16-char Gmail App Password. Never echoed back nor stored in the DB. */
  app_password: string;
}

/** Response for `PUT` and `GET /api/v1/email-sender`. Never carries the app
 *  password. `gmail_address` is absent on GET when no sender is configured. */
export interface EmailSenderResp {
  /** The configured Gmail address, or absent when no sender is set up. */
  gmail_address?: string;
  /** `true` once the app password passed a real Gmail SMTP login. */
  verified: boolean;
}

// ---------------------------------------------------------------------------
// Memory layer (TS mirror of crates/otto-memory + otto_state::memory)
// ---------------------------------------------------------------------------

export type MemoryScope = 'workspace' | 'story' | 'entity';
export type MemorySearchMode = 'hybrid' | 'semantic' | 'keyword';

export interface MemoryRef {
  kind: string;
  ref: string;
  url: string | null;
  label: string | null;
}

export interface Memory {
  id: string;
  workspace_id: string;
  collection: string;
  record_type: string;
  scope: MemoryScope;
  story_id: string | null;
  kind: string;
  title: string;
  body: string;
  entities: string[];
  tags: string[];
  source_kind: string;
  source_ref: string | null;
  refs: MemoryRef[];
  confidence: number;
  salience: number;
  content_hash: string;
  active: boolean;
  superseded_by: string | null;
  version: number;
  created_by: string;
  created_at: string;
  updated_at: string;
  last_accessed_at: string | null;
  access_count: number;
  expires_at: string | null;
  /** 'shared' (all workspace members) or 'private' (creator-only). */
  visibility: string;
}

export interface NewMemory {
  collection?: string;
  record_type?: string;
  scope: MemoryScope;
  story_id?: string | null;
  kind: string;
  title: string;
  body: string;
  entities?: string[];
  tags?: string[];
  source_kind: string;
  source_ref?: string | null;
  refs?: MemoryRef[];
  confidence?: number | null;
  salience?: number | null;
  /** 'shared' (default) or 'private'. */
  visibility?: string;
}

export interface MemoryPatch {
  title?: string | null;
  body?: string | null;
  tags?: string[] | null;
  entities?: string[] | null;
  confidence?: number | null;
  salience?: number | null;
  active?: boolean | null;
}

export interface MemoryQuery {
  text?: string;
  collection?: string;
  scope?: MemoryScope;
  story_id?: string;
  kinds?: string[];
  tags?: string[];
  entities?: string[];
  k?: number;
  mode?: MemorySearchMode;
  include_inactive?: boolean;
  recency_half_life_days?: number;
}

export interface MemoryHit {
  memory: Memory;
  score: number;
  why: string[];
}

export interface BriefSection {
  heading: string;
  body_md: string;
  refs: MemoryRef[];
}

export interface RecallBrief {
  story_id: string;
  sections: BriefSection[];
  token_estimate: number;
  used: string[];
}

export interface MemoryLink {
  src_id: string;
  dst_id: string;
  rel: string;
  weight: number;
  certainty: string | null;
}

export interface MemoryGraphNode {
  id: string;
  label: string;
  kind: string;
  collection: string;
}

export interface MemoryGraphData {
  nodes: MemoryGraphNode[];
  edges: MemoryLink[];
}

export interface MemoryImportStats {
  nodes: number;
  edges: number;
}

export interface MemoryEntityGraph {
  links: MemoryLink[];
  neighbors: Memory[];
}

export interface IngestTextReq {
  collection?: string;
  path: string;
  content: string;
}

// ---------------------------------------------------------------------------
// Message Brokers (Kafka) — mirror of crates/otto-brokers/src/types.rs
// ---------------------------------------------------------------------------

export type SecurityProtocol = 'plaintext' | 'ssl' | 'sasl_plaintext' | 'sasl_ssl';
export type SaslMechanism = 'plain' | 'scram_sha_256' | 'scram_sha_512';

/** SSH tunnel (bastion) config — shared with the DB Explorer connection form.
 * Auth is key-file/agent only (no password). Mirrors otto_ssh::SshTunnelConfig. */
export interface SshTunnelConfig {
  host: string;
  port?: number;
  user: string;
  identity_file?: string | null;
}

export interface BrokerCluster {
  id: Id;
  workspace_id: Id | null;
  name: string;
  bootstrap_servers: string;
  security_protocol: SecurityProtocol;
  sasl_mechanism: SaslMechanism | null;
  sasl_username: string | null;
  has_sasl_password: boolean;
  tls_skip_verify: boolean;
  schema_registry_url: string | null;
  schema_registry_username: string | null;
  has_sr_password: boolean;
  metrics_url: string | null;
  color: string | null;
  /** SSH tunnel to reach a private cluster (e.g. MSK) through a bastion. */
  ssh?: SshTunnelConfig | null;
  /** Section the cluster is filed under in the sidebar (null = ungrouped). */
  section_id?: Id | null;
  environment: Environment;
  read_only: boolean;
  created_by: Id;
  created_at: string;
}

/** A user-defined section (folder) grouping clusters in the sidebar. */
export interface BrokerClusterSection {
  id: Id;
  workspace_id: Id;
  parent_id: Id | null;
  name: string;
  position: number;
  created_by: Id;
  created_at: string;
}

export interface UpsertClusterReq {
  name: string;
  bootstrap_servers: string;
  security_protocol: SecurityProtocol;
  sasl_mechanism?: SaslMechanism | null;
  sasl_username?: string | null;
  sasl_password?: string | null;
  tls_skip_verify?: boolean | null;
  schema_registry_url?: string | null;
  schema_registry_username?: string | null;
  schema_registry_password?: string | null;
  metrics_url?: string | null;
  color?: string | null;
  /** SSH tunnel. Omit to keep the stored value; `null` to clear; object to set. */
  ssh?: SshTunnelConfig | null;
  /** Section assignment. Omit = keep; `null` = ungroup; id = file under section. */
  section_id?: Id | null;
  environment?: Environment | null;
  read_only?: boolean | null;
}

/** Lazily-loaded per-topic stats (message count + cleanup policy). */
export interface TopicStats {
  message_count: number;
  cleanup_policy: string | null;
  /** Approx production rate (msg/s) from the high-watermark delta between two
   *  consecutive `topics/stats` calls; null on the first sample or a count error. */
  msg_per_sec?: number | null;
}

/** Request body for POST /brokers/clusters/{id}/topics/stats (batch). */
export interface BatchStatsReq {
  names: string[];
}

export interface TestClusterResp {
  ok: boolean;
  latency_ms: number;
  message: string;
  broker_count: number;
}

export interface ClusterOverview {
  cluster_id: string | null;
  controller_id: number;
  brokers: BrokerNode[];
  topic_count: number;
  internal_topic_count: number;
  partition_count: number;
  consumer_group_count: number;
  /** Number of partitions with ISR count < replica count. */
  under_replicated_partitions?: number | null;
  /** Leadership imbalance: coefficient of variation of leader counts (0 = balanced). */
  leadership_imbalance?: number | null;
}

export interface BrokerNode {
  id: number;
  host: string;
  port: number;
  rack: string | null;
  is_controller: boolean;
  partition_leaders: number;
}

export interface TopicSummary {
  name: string;
  partitions: number;
  replication_factor: number;
  message_count: number;
  cleanup_policy: string | null;
  internal: boolean;
}

export interface TopicDetail {
  name: string;
  internal: boolean;
  partitions: PartitionInfo[];
  configs: TopicConfigEntry[];
  message_count: number;
}

export interface PartitionInfo {
  id: number;
  leader: number;
  replicas: number[];
  isr: number[];
  low: number;
  high: number;
  message_count: number;
}

export interface TopicConfigEntry {
  name: string;
  value: string | null;
  source: string;
  is_default: boolean;
  is_sensitive: boolean;
  is_read_only: boolean;
}

export interface ConfigKv {
  name: string;
  value: string;
}

export interface CreateTopicReq {
  name: string;
  partitions: number;
  replication_factor: number;
  configs?: ConfigKv[];
  confirm?: boolean;
}

export interface AlterConfigsReq {
  configs: ConfigKv[];
  confirm?: boolean;
}

export type ValueFormat = 'auto' | 'json' | 'utf8' | 'hex' | 'base64' | 'protobuf' | 'avro';

export type StartPosition =
  | { type: 'beginning' }
  | { type: 'latest' }
  | { type: 'offset'; offset: number }
  | { type: 'timestamp'; timestamp_ms: number };

export interface ConsumeReq {
  partition?: number | null;
  start?: StartPosition;
  limit?: number;
  max_wait_ms?: number | null;
  /** Applied server-side during consume (raw bytes). Limits the result to
   * messages whose key contains this substring (case-insensitive). */
  key_filter?: string | null;
  /** When true and key_filter is set, scan from the earliest offset so older
   * matching messages are found regardless of the `start` position. */
  find_from_beginning?: boolean;
  /** Applied post-decode in the service layer. */
  value_filter?: string | null;
  decode?: ValueFormat;
  /** When true, the server runs message key/value/headers through
   * `otto_core::redact` before returning. Raw payloads never leave the server
   * when this flag is set. The response `masked` field confirms it. */
  mask?: boolean;
}

export interface DecodedPayload {
  format: string;
  text: string;
  schema_id?: number;
  raw_base64?: string;
}

export interface MessageHeader {
  key: string;
  value: string;
}

export interface KafkaMessage {
  partition: number;
  offset: number;
  timestamp_ms: number | null;
  key: DecodedPayload | null;
  value: DecodedPayload | null;
  headers: MessageHeader[];
  size_bytes: number;
}

export interface PartitionRange {
  partition: number;
  low: number;
  high: number;
}

export interface ConsumeResp {
  messages: KafkaMessage[];
  partitions: PartitionRange[];
  truncated: boolean;
  /** True when message payloads were run through `otto_core::redact` server-side
   * (ConsumeReq.mask=true). The UI surfaces this as a badge. */
  masked?: boolean;
}

export interface ProduceReq {
  partition?: number | null;
  key?: string | null;
  value: string;
  headers?: MessageHeader[];
  key_base64?: boolean;
  value_base64?: boolean;
  confirm?: boolean;
}

export interface ProduceResp {
  partition: number;
  offset: number;
}

export interface GroupSummary {
  group_id: string;
  state: string;
  protocol_type: string;
  members: number;
}

export interface TopicPartition {
  topic: string;
  partition: number;
}

export interface GroupMember {
  member_id: string;
  client_id: string;
  host: string;
  assignments: TopicPartition[];
}

export interface GroupOffset {
  topic: string;
  partition: number;
  current_offset: number;
  high_watermark: number;
  lag: number;
}

export interface GroupDetail {
  group_id: string;
  state: string;
  protocol_type: string;
  protocol: string;
  members: GroupMember[];
  offsets: GroupOffset[];
  total_lag: number;
}

export type OffsetResetMode = 'earliest' | 'latest' | 'offset' | 'timestamp';

/** Request body for POST /brokers/clusters/{id}/groups/{group}/reset */
export interface GroupResetReq {
  /** Reset mode (earliest | latest | offset | timestamp). */
  mode: OffsetResetMode;
  /** Required when mode = 'offset'. */
  offset?: number;
  /** Required when mode = 'timestamp'. Epoch millis. */
  timestamp_ms?: number;
  /** Scope to a single topic (omit = all topics the group has offsets for). */
  topic?: string;
  /** Explicit confirm for guarded (prod / read-only) clusters. */
  confirm?: boolean;
}

export interface ThroughputPoint {
  ts_ms: number;
  total_messages: number;
  messages_per_sec: number;
}

export interface NamedMetric {
  name: string;
  value: number;
}

export interface BrokerResourceMetrics {
  instance: string;
  cpu_percent: number | null;
  memory_used_bytes: number | null;
  memory_total_bytes: number | null;
  extra: NamedMetric[];
}

export interface ClusterMetrics {
  throughput: ThroughputPoint[];
  messages_per_sec: number;
  total_messages: number;
  brokers: BrokerResourceMetrics[];
  prometheus_available: boolean;
  sampled_at: string;
}

export interface SchemaSubject {
  subject: string;
  version: number;
  id: number;
  schema_type: string;
  schema: string;
}

// ---------------------------------------------------------------------------
// Context-packet (B2a) — send API/DB/broker payloads to agent sessions
// ---------------------------------------------------------------------------

/** Source kind for a context packet. */
export type ContextPacketKind = 'api' | 'db' | 'broker';

/** Request body for `/context-packet/preview` and `/context-packet/send`. */
export interface ContextPacketReq {
  kind: ContextPacketKind;
  payload: unknown;
}

/** Per-kind redaction tally entry (mirrors `RedactionHit` in otto-core). */
export interface RedactionHit {
  kind: string;
  count: number;
}

/** Response from `POST /context-packet/preview`. */
export interface ContextPacketPreviewResp {
  redacted: unknown;
  redactions: RedactionHit[];
  size_bytes: number;
}

/** Response from `POST /context-packet/send`. */
export interface ContextPacketSendResp {
  ok: boolean;
  size_bytes: number;
  redactions: RedactionHit[];
}

// ---------------------------------------------------------------------------
// Cross-module search  (GET /workspaces/{id}/search?q=)
// ---------------------------------------------------------------------------

/**
 * One ranked result from the cross-module search endpoint.
 * `kind` discriminates the object type; `actions[0]` is always the primary
 * "open" navigation action.
 */
export interface SearchHit {
  /** Object type: "story" | "workflow" | "api_request" | "swarm_task" |
   *  "swarm_project" | "memory" | "repo" | "broker_cluster" */
  kind: string;
  /** Row id in the originating table. */
  id: string;
  /** Primary display title. */
  title: string;
  /** Secondary display text (method+URL, status, collection, …). */
  subtitle?: string;
  /** Contextual action labels; first entry is the primary "open" action. */
  actions: string[];
}

/** One product-analysis lens offered in the Analysis tab. Curated subset of the
 *  bundled product skills (only those that emit the Findings contract).
 *  Mirrors `otto_core::api::ProductLens`; returned by
 *  `GET /workspaces/{id}/product/lenses`. */
export interface ProductLens {
  /** Library skill name (e.g. "po-story-overview"). */
  skill: string;
  /** Human label shown in the UI checkbox row. */
  label: string;
  /** One-line description of what the lens does. */
  description: string;
  /** Whether the lens is checked by default in the Analysis tab. */
  default_on: boolean;
}

// ---------------------------------------------------------------------------
// Scheduled Tasks (mirror of otto_core::domain::ScheduledTask* — keep in lockstep)
// ---------------------------------------------------------------------------

/** A scheduled task: a recurring agent job with a cadence + delivery. */
export interface ScheduledTask {
  id: Id;
  workspace_id: Id;
  name: string;
  /** `agent_prompt` (run an agent) or `workflow` (hand off to a workflow). */
  kind: string;
  prompt: string;
  skill?: string | null;
  /** `claude` | `codex` | `agy` | `shell` | a custom provider slug. */
  provider: string;
  model: string;
  /** Working dir; `''` => a per-task scratch dir. NOT a security boundary. */
  cwd: string;
  /** `{cadence:'interval'|'daily'|'weekly'|'cron', every_min?, at?, weekday?, expr?}`. */
  schedule: Record<string, unknown>;
  /** `{type:'none'|'slack'|'telegram'|'email'|'webhook', ...}`. */
  destination: Record<string, unknown>;
  enabled: boolean;
  // v2
  /** IANA timezone the daily/weekly/cron times are interpreted in (default UTC). */
  timezone: string;
  /** For `kind:'workflow'`: the workflow this task launches. */
  workflow_id?: string | null;
  /** `none` | `worktree` (run in a fresh isolated git worktree). */
  sandbox: string;
  /** Extra agent attempts on failure (total attempts = 1 + max_retries). */
  max_retries: number;
  /** Deliver only when the report meaningfully changes from the last ok run. */
  notify_on_change: boolean;
  /** Build a proof pack (report + run metadata) for each run. */
  attach_proof: boolean;
  last_run_at?: string | null;
  last_status?: string | null;
  next_run_at?: string | null;
  created_by?: string | null;
  created_at: string;
  updated_at: string;
}

/** One execution of a scheduled task. */
export interface ScheduledTaskRun {
  id: Id;
  task_id: Id;
  workspace_id: Id;
  status: 'running' | 'ok' | 'error';
  trigger: 'schedule' | 'manual';
  started_at: string;
  finished_at?: string | null;
  summary: string;
  report_path?: string | null;
  report_rel?: string | null;
  delivered: boolean;
  delivery_error?: string | null;
  error?: string | null;
  /** The visible agent session the run drove (Open it from the run row). */
  session_id?: string | null;
  // v2
  report_hash?: string | null;
  proof_pack_id?: string | null;
  attempts: number;
  /** Delivery was suppressed because the report didn't meaningfully change. */
  skipped_delivery: boolean;
  workflow_run_id?: string | null;
  created_at: string;
}

/** A built-in template the create form can pre-fill from. */
export interface ScheduledTaskPreset {
  id: string;
  name: string;
  description: string;
  kind: string;
  prompt: string;
  schedule: Record<string, unknown>;
  suggested_destination: Record<string, unknown>;
  skill?: string | null;
}

// ---------------------------------------------------------------------------
// Run with Otto — the "one button" flow: a source item driven through a stage
// machine (resolve source → context → worktree → agent/goal-loop → proof → AI
// review → human approval → PR draft). Mirrors otto-core / otto_state::otto_runs.
// ---------------------------------------------------------------------------

/** Where a run is in its stage machine. The `*_*` mid-states are the live
 *  stages; the trailing ones are terminal. */
export type RunStatus =
  | 'queued'
  | 'resolving_source'
  | 'building_context'
  | 'provisioning'
  | 'executing'
  | 'proving'
  | 'reviewing'
  | 'awaiting_approval'
  | 'drafting_pr'
  | 'completed'
  | 'failed'
  | 'rejected'
  | 'cancelled';

/** The kind of source an a run was seeded from. */
export type SourceKind =
  | 'jira'
  | 'confluence'
  | 'github_pr'
  | 'github_issue'
  | 'channel'
  | 'product_story'
  | 'finding'
  | 'test'
  | 'scheduled_report';

/** Single agent turn vs an iterative goal loop. */
export type RunMode = 'single_agent' | 'goal_loop';

/** Where the run was launched from. */
export type RunOrigin = 'slack' | 'telegram' | 'webhook' | 'ui' | 'mcp' | 'api';

/** A Run with Otto run: a source item driven end-to-end into a reviewed,
 *  evidence-backed PR draft. */
export interface OttoRun {
  id: Id;
  workspace_id: Id;
  title: string;
  source_kind: SourceKind;
  source_ref: string;
  source_url?: string;
  goal: string;
  mode: RunMode;
  provider: string;
  repo_id?: string;
  repo_path?: string;
  base_branch?: string;
  branch?: string;
  worktree_path?: string;
  base_commit?: string;
  status: RunStatus;
  error?: string;
  origin_kind: RunOrigin;
  origin_chat?: string;
  origin_thread?: string;
  origin_user?: string;
  callback_url?: string;
  goal_loop_id?: string;
  review_id?: string;
  proof_pack_id?: string;
  proof_status?: string;
  risk_score?: number;
  findings_total: number;
  findings_blocking: number;
  pr_draft_json?: string;
  pr_url?: string;
  auto_open_pr: boolean;
  approval_decision?: string;
  approved_by?: string;
  approved_at?: string;
  result_summary?: string;
  context_summary?: string;
  created_by: string;
  created_at: string;
  updated_at: string;
}

/** One entry in a run's stage timeline (a stage start/finish/note). */
export interface RunEvent {
  id: Id;
  run_id: Id;
  workspace_id: Id;
  kind: string;
  status?: string;
  message: string;
  detail?: unknown;
  created_at: string;
}

/** Launch a run — either from a detected source (`source_kind`/`source_ref` or
 *  `url`) or free `seed_text` (→ a channel run). */
export interface LaunchRunReq {
  source_kind?: SourceKind;
  source_ref?: string;
  url?: string;
  seed_text?: string;
  mode?: RunMode;
  provider?: string;
  repo_id?: string;
  auto_open_pr?: boolean;
  title?: string;
}

/** Human approval / rejection of a run awaiting approval. */
export interface ApproveRunReq {
  decision: 'approve' | 'reject';
  note?: string;
}

/** The shape returned by `GET /runs/:id/detect` — a best-effort source guess. */
export interface RunDetectResp {
  detected?: {
    source_kind: SourceKind;
    source_ref: string;
    url: string;
  };
}
