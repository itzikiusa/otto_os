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
  | { type: 'swarm_status'; workspace_id: Id; swarm_id: Id; status: string };

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

/** The 18 protected features (snake_case, mirrors Rust Feature enum). */
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
  | 'users';

/** Capability ladder (None < View < Edit < Admin). */
export type Capability = 'none' | 'view' | 'edit' | 'admin';

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
}

/** Outcome of a local merge / merge-completion. Conflicts are a normal result. */
export interface MergeResult {
  status: 'merged' | 'conflicts' | 'up_to_date';
  commit: string | null;
  conflicted_files: string[];
  repo_status: RepoStatusResp;
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
// PR Review (AI agents)
// ---------------------------------------------------------------------------

export type ReviewStatus = 'running' | 'done' | 'error';
export type ReviewCommentState = 'draft' | 'approved' | 'declined';
export type ReviewAgentStatus = 'pending' | 'running' | 'waiting' | 'done' | 'error';

export interface ReviewFinding {
  path: string | null;
  line: number | null;
  severity: string; // 'info' | 'warn' | 'bug'
  body: string;
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

export type Channel = 'slack' | 'telegram';

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
}

export interface UpdateWorkspaceContextReq {
  skills: string[] | null;
  soul: string | null;
  extra_context_md: string;
  include_memory: boolean;
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
  created_at: string;
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
}

export interface PromoteSkillReq {
  iteration_id: Id;
  /** 'tested' = the skill that iteration ran with; 'improved' = its edited version. */
  source: 'tested' | 'improved';
  name: string;
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

export interface WorkflowNode {
  id: string;
  kind: string;
  name: string;
  x: number;
  y: number;
  params: unknown;
}

export interface WorkflowEdge {
  id: string;
  source: string;
  target: string;
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
  created_by: Id;
  created_at: string;
  updated_at: string;
}

export type RunStatus = 'pending' | 'running' | 'success' | 'error' | 'canceled';
export type NodeStatus = 'pending' | 'running' | 'success' | 'error' | 'skipped';

export interface NodeRunState {
  node_id: string;
  status: NodeStatus;
  output?: unknown;
  error?: string | null;
  logs: string[];
  duration_ms?: number | null;
}

export interface WorkflowRun {
  id: Id;
  workflow_id: Id;
  workspace_id: Id;
  status: RunStatus;
  input: unknown;
  nodes: NodeRunState[];
  error?: string | null;
  started_at: string;
  finished_at?: string | null;
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
}
