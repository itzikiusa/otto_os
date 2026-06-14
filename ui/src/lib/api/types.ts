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

export interface Connection {
  id: Id;
  workspace_id: Id | null;
  name: string;
  kind: ConnectionKind;
  params: Record<string, unknown>;
  secret_ref: string | null;
  first_command: string | null;
  section_id: Id | null;
  created_by: Id;
  created_at: string;
}

export interface ConnectionSection {
  id: Id;
  workspace_id: Id;
  parent_id: Id | null;
  name: string;
  position: number;
  created_by: Id;
  created_at: string;
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
// Events (WS /ws/events)
// ---------------------------------------------------------------------------

export type OttoEvent =
  | { type: 'session_status'; session_id: Id; workspace_id: Id; status: SessionStatus }
  | { type: 'session_created'; session: Session }
  | { type: 'session_removed'; session_id: Id; workspace_id: Id }
  | { type: 'notice'; level: 'info' | 'warn' | 'error'; title: string; body: string }
  | { type: 'notification'; notice: Notice };

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

// ---------------------------------------------------------------------------
// Orchestrator
// ---------------------------------------------------------------------------

export type Action =
  | { action: 'spawn_sessions'; provider: string; count: number }
  | { action: 'broadcast'; text: string }
  | { action: 'open_connection'; connection_id: Id }
  | { action: 'run_command'; session_id: Id; text: string };

export type ActionPlan = Action[];

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
  model: string;
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
  | { type: 'api_key'; key: string; value: string; in: 'header' | 'query' };

export type ApiBodyMode = 'none' | 'json' | 'raw' | 'form' | 'graphql';

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
}

export interface ApiResponse {
  status: number;
  status_text: string;
  headers: ApiKeyVal[];
  body: string;
  duration_ms: number;
  size_bytes: number;
  content_type: string | null;
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
