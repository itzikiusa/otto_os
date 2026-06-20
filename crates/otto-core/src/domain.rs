//! Core domain entities, mirrored 1:1 by the SQLite schema and the UI types.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Id;

/// Global (instance-level) role of a user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GlobalRole {
    Root,
    Member,
}

/// Role of a user inside one workspace. Ordering: Viewer < Editor < Admin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceRole {
    Viewer,
    Editor,
    Admin,
}

impl WorkspaceRole {
    /// Parse from the lowercase string stored in SQLite.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "viewer" => Some(Self::Viewer),
            "editor" => Some(Self::Editor),
            "admin" => Some(Self::Admin),
            _ => None,
        }
    }

    /// Lowercase string form stored in SQLite.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Viewer => "viewer",
            Self::Editor => "editor",
            Self::Admin => "admin",
        }
    }
}

/// A user account. Passwords live in `users.password_hash`, never in this struct.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: Id,
    pub username: String,
    pub display_name: String,
    pub is_root: bool,
    pub disabled: bool,
    pub created_at: DateTime<Utc>,
}

/// A workspace: a project directory plus its sessions, connections and repos.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    pub id: Id,
    pub name: String,
    pub root_path: String,
    pub settings: Value,
    pub archived: bool,
    pub created_at: DateTime<Utc>,
}

/// What kind of process a session hosts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionKind {
    /// An agent CLI (claude, codex) or plain shell.
    Agent,
    /// A terminal opened from a connection profile (ssh, db client, custom).
    Connection,
}

impl SessionKind {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "agent" => Some(Self::Agent),
            "connection" => Some(Self::Connection),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::Connection => "connection",
        }
    }
}

/// Live status of a session, derived from PTY activity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    /// Child alive, no recent output classification yet.
    Running,
    /// Output flowed within the activity window — the agent is doing work.
    Working,
    /// No output for the idle window.
    Idle,
    /// Child exited.
    Exited,
    /// Daemon restarted and this session can be reconnected on demand.
    Reconnectable,
}

impl SessionStatus {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "running" => Some(Self::Running),
            "working" => Some(Self::Working),
            "idle" => Some(Self::Idle),
            "exited" => Some(Self::Exited),
            "reconnectable" => Some(Self::Reconnectable),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Working => "working",
            Self::Idle => "idle",
            Self::Exited => "exited",
            Self::Reconnectable => "reconnectable",
        }
    }
}

/// A terminal session living in the daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: Id,
    pub workspace_id: Id,
    pub kind: SessionKind,
    /// Provider name for agent sessions ("claude", "codex", "shell") or the
    /// connection kind for connection sessions ("ssh", "mysql", ...).
    pub provider: String,
    pub title: String,
    pub status: SessionStatus,
    pub cwd: String,
    /// Provider-side session id (e.g. claude session uuid) used for resume.
    pub provider_session_id: Option<String>,
    pub connection_id: Option<Id>,
    pub created_by: Id,
    pub created_at: DateTime<Utc>,
    pub last_active_at: DateTime<Utc>,
    /// Archived sessions keep their row + history but have no live PTY; they
    /// are hidden from the active list and shown in an "Archived" section.
    pub archived: bool,
    pub meta: Value,
}

/// Supported connection kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionKind {
    Ssh,
    Mysql,
    Redis,
    Mongodb,
    Clickhouse,
    Custom,
}

impl ConnectionKind {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "ssh" => Some(Self::Ssh),
            "mysql" => Some(Self::Mysql),
            "redis" => Some(Self::Redis),
            "mongodb" => Some(Self::Mongodb),
            "clickhouse" => Some(Self::Clickhouse),
            "custom" => Some(Self::Custom),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ssh => "ssh",
            Self::Mysql => "mysql",
            Self::Redis => "redis",
            Self::Mongodb => "mongodb",
            Self::Clickhouse => "clickhouse",
            Self::Custom => "custom",
        }
    }
}

/// Deployment environment a connection points at. Drives the write-gate and the
/// UI danger styling: `Prod` connections refuse writes/DDL without an explicit
/// confirm flag, exactly like a `read_only` connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Environment {
    /// Local / development (default; no guardrail).
    #[default]
    Dev,
    /// Pre-production / staging (no guardrail by default).
    Staging,
    /// Production — writes are gated behind an explicit confirmation.
    Prod,
}

impl Environment {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "dev" => Some(Self::Dev),
            "staging" => Some(Self::Staging),
            "prod" => Some(Self::Prod),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Dev => "dev",
            Self::Staging => "staging",
            Self::Prod => "prod",
        }
    }

    /// True when this environment is treated as dangerous (production).
    pub fn is_production(&self) -> bool {
        matches!(self, Self::Prod)
    }
}

/// A saved connection profile. Secrets are NEVER stored here — only a
/// Keychain reference in `secret_ref`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Connection {
    pub id: Id,
    /// None = global profile (root-managed), visible to all workspaces.
    pub workspace_id: Option<Id>,
    pub name: String,
    pub kind: ConnectionKind,
    /// Non-secret parameters: host, port, user, db, identity_file, jump,
    /// conn_string_public, command_template — per kind.
    pub params: Value,
    /// Keychain item name holding the password/secret, if any.
    pub secret_ref: Option<String>,
    /// Optional command written to the PTY right after connect.
    pub first_command: Option<String>,
    /// Section this profile belongs to (workspace-scoped); None = ungrouped.
    pub section_id: Option<Id>,
    /// Deployment environment (defaults to `Dev`). `Prod` is treated as
    /// dangerous: writes/DDL require explicit confirmation.
    #[serde(default)]
    pub environment: Environment,
    /// When true, the connection refuses writes/DDL without confirmation —
    /// independent of `environment` (so a non-prod profile can still be locked).
    #[serde(default)]
    pub read_only: bool,
    pub created_by: Id,
    pub created_at: DateTime<Utc>,
    /// Last time this connection was opened (via DB Explorer or "Open as
    /// terminal"). Absent when never opened.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_opened_at: Option<DateTime<Utc>>,
    /// When true the connection is surfaced in a "Pinned" group above "Recent"
    /// regardless of recency. Defaults to false.
    #[serde(default)]
    pub pinned: bool,
}

impl Connection {
    /// True when this profile is guarded: production OR explicitly read-only.
    /// Guarded connections reject write/DDL statements unless the caller passes
    /// an explicit confirm flag.
    pub fn is_write_guarded(&self) -> bool {
        self.read_only || self.environment.is_production()
    }
}

/// A user-defined grouping of connection profiles within a workspace.
/// Sections nest into a tree via `parent_id` (None = top-level).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionSection {
    pub id: Id,
    pub workspace_id: Id,
    /// Parent section, or None for a top-level section.
    pub parent_id: Option<Id>,
    pub name: String,
    pub position: i64,
    /// Which tree this section belongs to: "connections" (the Connections page)
    /// or "db" (the Database Explorer). Keeps the two hierarchies independent.
    pub scope: String,
    pub created_by: Id,
    pub created_at: DateTime<Utc>,
}

/// A user-configured MCP (Model Context Protocol) server for a workspace.
/// Enabled servers are merged into the workspace's `.mcp.json` (alongside Otto's
/// own managed entries) when an agent session spawns there. Never auto-enabled.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServer {
    pub id: Id,
    pub workspace_id: Id,
    /// Key under `.mcp.json`'s `mcpServers` map; unique within the workspace.
    pub name: String,
    pub command: String,
    /// Command arguments, in order.
    pub args: Vec<String>,
    /// Extra environment passed to the server process. Stored in plaintext for
    /// now (like `.mcp.json` on disk) — sensitive values belong in the user's
    /// own MCP config until Keychain secret-refs land.
    pub env: std::collections::BTreeMap<String, String>,
    /// Off by default: a server is only written to `.mcp.json` once enabled.
    pub enabled: bool,
    pub created_by: Id,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Git hosting providers supported for PR workflows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitProviderKind {
    Github,
    Bitbucket,
    Gitlab,
}

impl GitProviderKind {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "github" => Some(Self::Github),
            "bitbucket" => Some(Self::Bitbucket),
            "gitlab" => Some(Self::Gitlab),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Github => "github",
            Self::Bitbucket => "bitbucket",
            Self::Gitlab => "gitlab",
        }
    }
}

/// A stored git hosting account. Token lives in Keychain under `token_ref`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitAccount {
    pub id: Id,
    pub user_id: Id,
    pub provider: GitProviderKind,
    pub label: String,
    pub username: String,
    pub token_ref: String,
    /// Custom API base for self-hosted instances (GitLab), else None.
    pub api_base_url: Option<String>,
    /// Namespace to scope remote-repo listing: Bitbucket workspace, GitHub org,
    /// GitLab group. None means the user has not configured it yet.
    pub namespace: Option<String>,
    /// When the token expires, if known. Auto-detected where the provider
    /// exposes it (GitHub/GitLab); otherwise user-entered (Bitbucket). Drives
    /// expiry notifications. None = unknown / no expiry.
    pub token_expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// Issue tracking providers supported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueProviderKind {
    Jira,
}

impl IssueProviderKind {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "jira" => Some(Self::Jira),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Jira => "jira",
        }
    }
}

/// A stored issue-tracking account. Token lives in Keychain under `token_ref`.
/// The DB columns are `username` (email) and `api_base_url` (base_url) to
/// mirror the git_accounts table layout.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueAccount {
    pub id: Id,
    pub user_id: Id,
    pub provider: IssueProviderKind,
    pub label: String,
    /// The user's Jira account email (serialised as `email`; stored as `username`).
    pub email: String,
    /// Keychain item name holding the API token.
    #[serde(skip_serializing)]
    pub token_ref: String,
    /// The Jira instance base URL (serialised as `base_url`; stored as `api_base_url`).
    pub base_url: String,
    /// When the token expires, if known (user-entered for Jira). Drives expiry
    /// notifications. None = unknown / no expiry.
    pub token_expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// A persisted notification surfaced in the notification center. Created by the
/// credential monitor, session events, and other sources; streamed live via
/// `Event::Notification` and listed at `GET /api/v1/notifications`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notice {
    pub id: Id,
    pub created_at: DateTime<Utc>,
    pub read: bool,
    pub kind: NoticeKind,
    pub severity: NoticeSeverity,
    pub title: String,
    pub body: String,
    /// Stable key for de-duping recurring notices (e.g.
    /// `"git_account:<id>:expiry"`, `"session:<id>:exited"`). None = always new.
    pub source_key: Option<String>,
    /// Optional action the UI can offer (open URL, focus session, re-auth).
    pub action: Option<NoticeAction>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeKind {
    Credential,
    Session,
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeSeverity {
    Info,
    Warn,
    Error,
}

/// What the UI does when the user clicks a notice's action button.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NoticeAction {
    /// Open a URL in the system browser.
    OpenUrl { url: String },
    /// Focus an existing session.
    OpenSession { session_id: Id },
    /// Prompt the user to re-authenticate. `target` is e.g. "claude", "codex",
    /// `git:<account_id>`, or `issue:<account_id>`.
    Reauth { target: String },
}

/// One append-only entry in the security audit log, listed (root only) at
/// `GET /api/v1/audit-log` and surfaced in the Trust & Safety Center. Written
/// best-effort by `ServerCtx::audit` at sensitive sites (login, token
/// mint/revoke, settings/network-listener changes, confirmed DB writes). Never
/// updated or deleted — treat the stream as forward-only.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub id: Id,
    pub ts: DateTime<Utc>,
    /// Acting user; None for an unauthenticated actor (e.g. a failed login for
    /// an unknown username) or a daemon-internal caller.
    pub user_id: Option<Id>,
    /// Stable snake_case verb, e.g. `"login.success"`, `"token.mint"`.
    pub action: String,
    /// Optional subject of the action (a username, token id, connection name…).
    pub target: Option<String>,
    /// Optional action-specific context.
    pub detail: Option<Value>,
    /// Optional client IP (the real socket peer; forwarding headers untrusted).
    pub ip: Option<String>,
}

// ---------------------------------------------------------------------------
// Agent activity: per-session live trail + normalized task tracker
// ---------------------------------------------------------------------------

/// Who produced a trail entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrailSource {
    /// A human action in Otto (e.g. a note, or an injected command).
    User,
    /// The agent CLI (a tool it ran, a skill it loaded, its reply).
    Agent,
    /// Otto itself (lifecycle: session spawned, resumed, archived).
    Otto,
}

impl TrailSource {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "user" => Some(Self::User),
            "agent" => Some(Self::Agent),
            "otto" => Some(Self::Otto),
            _ => None,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Agent => "agent",
            Self::Otto => "otto",
        }
    }
}

/// Coarse category of a trail entry — drives the UI icon and grouping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrailKind {
    /// Session lifecycle (started, resumed, finished, archived).
    Session,
    /// A user prompt submitted to the agent.
    Prompt,
    /// A skill was loaded/invoked.
    Skill,
    /// A shell command was run.
    Command,
    /// A generic tool call (MCP tool, Task sub-agent, …).
    Tool,
    /// A file was read/written/edited.
    File,
    /// A web fetch/search.
    Web,
    /// A change to the task tracker.
    Task,
    /// A free-form note (typically authored by a human).
    Note,
    Other,
}

impl TrailKind {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "session" => Some(Self::Session),
            "prompt" => Some(Self::Prompt),
            "skill" => Some(Self::Skill),
            "command" => Some(Self::Command),
            "tool" => Some(Self::Tool),
            "file" => Some(Self::File),
            "web" => Some(Self::Web),
            "task" => Some(Self::Task),
            "note" => Some(Self::Note),
            "other" => Some(Self::Other),
            _ => None,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::Prompt => "prompt",
            Self::Skill => "skill",
            Self::Command => "command",
            Self::Tool => "tool",
            Self::File => "file",
            Self::Web => "web",
            Self::Task => "task",
            Self::Note => "note",
            Self::Other => "other",
        }
    }
}

/// Severity of a trail entry — drives row coloring and notification raising.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrailLevel {
    Info,
    Warn,
    Error,
}

impl TrailLevel {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "info" => Some(Self::Info),
            "warn" => Some(Self::Warn),
            "error" => Some(Self::Error),
            _ => None,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

/// One entry in a session's live activity trail.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrailEvent {
    pub id: Id,
    pub session_id: Id,
    pub workspace_id: Id,
    pub ts: DateTime<Utc>,
    pub source: TrailSource,
    pub kind: TrailKind,
    pub level: TrailLevel,
    /// One-line human summary ("$ cargo build", "Loaded skill: brainstorming").
    pub summary: String,
    /// Optional structured payload (raw tool input, etc.). `null` when absent.
    pub detail: Option<Value>,
}

/// Status of a tracked agent task — the union over provider-native states
/// (Claude TodoWrite `pending|in_progress|completed`, plus blocked/cancelled).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    InProgress,
    Completed,
    Blocked,
    Cancelled,
}

impl TaskStatus {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(Self::Pending),
            "in_progress" => Some(Self::InProgress),
            "completed" | "done" => Some(Self::Completed),
            "blocked" => Some(Self::Blocked),
            "cancelled" | "canceled" => Some(Self::Cancelled),
            _ => None,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
            Self::Blocked => "blocked",
            Self::Cancelled => "cancelled",
        }
    }
}

/// A compact per-session roll-up of the task tracker + trail, for the
/// multi-agent overview (sidebar chips). Built by `ActivityRepo::workspace_summary`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionActivitySummary {
    pub session_id: Id,
    pub total: i64,
    pub done: i64,
    /// Title of the first in-progress task, if any (what the agent is doing now).
    pub in_progress: Option<String>,
    /// Timestamp of the most recent trail entry, if any.
    pub last_ts: Option<DateTime<Utc>>,
}

/// One task in a session's normalized task tracker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTask {
    pub id: Id,
    pub session_id: Id,
    pub workspace_id: Id,
    /// Provider-native id when the source supplies a stable one (else `None`).
    pub ext_id: Option<String>,
    pub title: String,
    pub status: TaskStatus,
    pub position: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A Jira project (key + display name) returned by the project listing endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueProject {
    pub key: String,
    pub name: String,
}

/// A brief issue summary returned by search.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueSummary {
    pub key: String,
    pub summary: String,
    pub status: String,
    pub issue_type: String,
    pub url: String,
}

/// Full issue detail returned by the single-issue fetch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueDetail {
    pub key: String,
    pub summary: String,
    pub status: String,
    pub issue_type: String,
    pub url: String,
    pub description: String,
    pub assignee: String,
}

/// Supported messaging channel kinds for workspace integrations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    Slack,
    Telegram,
}

impl Channel {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "slack" => Some(Self::Slack),
            "telegram" => Some(Self::Telegram),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Slack => "slack",
            Self::Telegram => "telegram",
        }
    }
}

/// A workspace integration with a messaging channel (Slack / Telegram).
/// Tokens are never exposed; `has_bot_token`/`has_app_token` indicate presence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Integration {
    pub workspace_id: Id,
    pub channel: Channel,
    pub enabled: bool,
    pub allowed_users: String,
    pub agent_reply: bool,
    pub reply_instructions: String,
    pub channel_id: String,
    /// Preferred agent CLI for this channel's sessions. Empty = use the
    /// workspace/global default agent (then "claude" as the last resort).
    pub preferred_cli: String,
    /// True when a bot token is stored in the keychain.
    pub has_bot_token: bool,
    /// True when a Slack app-level token is stored in the keychain.
    pub has_app_token: bool,
    pub updated_at: DateTime<Utc>,
}

/// A git repository registered inside a workspace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Repo {
    pub id: Id,
    pub workspace_id: Id,
    pub name: String,
    pub path: String,
    pub remote_url: Option<String>,
    pub provider: Option<GitProviderKind>,
    pub git_account_id: Option<Id>,
    pub created_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// PR review agents
// ---------------------------------------------------------------------------

/// Status of a PR agent review run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewStatus {
    Running,
    Done,
    Error,
}

impl ReviewStatus {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "running" => Some(Self::Running),
            "done" => Some(Self::Done),
            "error" => Some(Self::Error),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Done => "done",
            Self::Error => "error",
        }
    }
}

/// Severity of a draft review comment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommentSeverity {
    Info,
    Warn,
    Bug,
}

impl CommentSeverity {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "info" => Some(Self::Info),
            "warn" => Some(Self::Warn),
            "bug" => Some(Self::Bug),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Bug => "bug",
        }
    }
}

/// State of a draft review comment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommentState {
    Draft,
    Approved,
    Declined,
}

impl CommentState {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "draft" => Some(Self::Draft),
            "approved" => Some(Self::Approved),
            "declined" => Some(Self::Declined),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Approved => "approved",
            Self::Declined => "declined",
        }
    }
}

/// A single comment produced by the review agents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewComment {
    pub id: Id,
    pub review_id: Id,
    /// File path the comment refers to (None for general comments).
    pub path: Option<String>,
    /// Line number within the file (None for general comments).
    pub line: Option<u32>,
    pub severity: CommentSeverity,
    pub body: String,
    pub state: CommentState,
    /// True when the comment has been posted to the PR via the provider API.
    pub posted: bool,
    pub created_at: DateTime<Utc>,
}

/// Configuration for one review agent lens.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReviewAgentCfg {
    pub name: String,
    /// "claude" | "codex" | "agy" — backward-compat single provider field.
    /// Effective providers = `providers` when non-empty, else `[provider]`.
    pub provider: String,
    /// List of CLIs to run this agent on (expands to one run per entry).
    /// When empty the effective provider list is `[provider]`.
    #[serde(default)]
    pub providers: Vec<String>,
    /// Model hint: "haiku" | "sonnet" | "opus" | "" (empty = provider default).
    pub model: String,
    /// Lens prompt — what this agent looks for.
    pub prompt: String,
}

/// One finding produced by a single review agent (before summarization), for
/// the expandable per-agent view in the UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewFinding {
    pub path: Option<String>,
    pub line: Option<u32>,
    /// "info" | "warn" | "bug".
    pub severity: String,
    pub body: String,
}

/// Live state of one review agent during a run (stored as agents_json).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewAgentState {
    pub name: String,
    pub provider: String,
    pub model: String,
    /// "pending" | "running" | "waiting" | "done" | "error".
    /// "waiting" means the agent appears blocked on input (e.g. a prompt the
    /// guard couldn't auto-accept) — the user should Open it and respond.
    pub status: String,
    /// Short preview — first ~80 chars of output, "N findings", or error msg.
    pub note: String,
    pub comment_count: u32,
    /// The live session this agent runs in (openable in the UI). None until
    /// spawned, or for the headless summarizer.
    #[serde(default)]
    pub session_id: Option<String>,
    /// This agent's own findings (before summarization). Empty until it
    /// completes; powers the per-agent expandable list.
    #[serde(default)]
    pub findings: Vec<ReviewFinding>,
}

/// A PR agent review run, together with all its draft comments.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Review {
    pub id: Id,
    pub repo_id: Id,
    pub pr_number: u64,
    pub status: ReviewStatus,
    /// Error message when status == "error".
    pub error: Option<String>,
    pub comments: Vec<ReviewComment>,
    /// Live state of each configured agent (populated during the run).
    #[serde(default)]
    pub agents: Vec<ReviewAgentState>,
    pub created_at: DateTime<Utc>,
    /// Overall verdict: "approved" | "changes_requested" | "needs_review".
    /// Computed from comment severity/state after the summarizer completes.
    #[serde(default)]
    pub verdict: Option<String>,
    /// Count of bug-severity comments still in draft state (merge blocker count).
    #[serde(default)]
    pub blocker_count: Option<u32>,
    /// Short markdown summary of findings (generated by the summarizer).
    #[serde(default)]
    pub summary_md: Option<String>,
}

// ---------------------------------------------------------------------------
// Skills Evaluator (test-and-improve a skill across scored iterations)
// ---------------------------------------------------------------------------

/// Overall lifecycle status of a skill-evaluation run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillEvalStatus {
    Running,
    Done,
    Error,
    Cancelled,
}

impl SkillEvalStatus {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "running" => Some(Self::Running),
            "done" => Some(Self::Done),
            "error" => Some(Self::Error),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Done => "done",
            Self::Error => "error",
            Self::Cancelled => "cancelled",
        }
    }
}

/// A single problem a validation agent found, with the concrete fix it suggests.
/// This is the unit the UI renders ("what was wrong" + "how to fix it").
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalFinding {
    /// "info" | "warn" | "fail". `fail` findings fail the validation.
    pub severity: String,
    /// What is wrong or missing.
    pub issue: String,
    /// The concrete suggested fix for this issue.
    #[serde(default)]
    pub suggestion: String,
    /// Optional location/context (e.g. a file path or symbol name).
    #[serde(default)]
    pub location: Option<String>,
}

/// Live state of one validation agent (one validation × one provider) within an
/// iteration. Stored inside the iteration's `agents_json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalValidationState {
    /// The validation this agent runs (e.g. "logs", "docs", "naming").
    pub validation: String,
    /// Display name, e.g. "logs · claude" when one validation fans across CLIs.
    pub name: String,
    pub provider: String,
    pub model: String,
    /// "pending" | "running" | "waiting" | "done" | "error".
    pub status: String,
    /// Short note: a preview, "N issues", or an error message.
    pub note: String,
    /// Whether the validation passed (no `fail`-severity findings).
    #[serde(default)]
    pub passed: bool,
    /// 0–100 score this validation gives the produced code/skill.
    #[serde(default)]
    pub score: f64,
    /// Live session this agent runs in (openable in the UI).
    #[serde(default)]
    pub session_id: Option<String>,
    /// The issues this validation found, each with a suggested fix.
    #[serde(default)]
    pub findings: Vec<EvalFinding>,
}

/// One iteration (round) of a skill evaluation: the skill copy used, the
/// implementation it produced, the validations' findings, a score, and the
/// improvement the improver applied to seed the next iteration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalIteration {
    pub id: Id,
    pub eval_id: Id,
    /// 1-based iteration number.
    pub iter: u32,
    /// Iteration this one's skill was derived from (None for the first).
    #[serde(default)]
    pub base_iter: Option<u32>,
    /// Name of the temporary skill copy used this iteration
    /// (e.g. `golang-feature-implementation-run-ab12-iter2`).
    pub skill_name: String,
    /// The skill content used (and tested) this iteration.
    pub skill_before: String,
    /// The improved skill content produced for the NEXT iteration (None when
    /// this is the last iteration or no improvement was made).
    #[serde(default)]
    pub skill_after: Option<String>,
    /// Provider that ran the implementation.
    pub impl_provider: String,
    /// Live implementation session (openable in the UI).
    #[serde(default)]
    pub impl_session_id: Option<String>,
    /// Short summary the implementation agent reported.
    #[serde(default)]
    pub impl_summary: String,
    /// Filesystem path of the git worktree this iteration ran in.
    #[serde(default)]
    pub worktree_path: Option<String>,
    /// "pending" | "implementing" | "validating" | "improving" | "done" | "error".
    pub status: String,
    #[serde(default)]
    pub note: String,
    /// 0–100 aggregate score for this iteration (mean of its validations).
    #[serde(default)]
    pub score: f64,
    /// Per-validation live state + findings.
    #[serde(default)]
    pub agents: Vec<EvalValidationState>,
    /// What the improver changed and why (seeds the next iteration).
    #[serde(default)]
    pub improvement_summary: String,
    /// Unified-ish diff between this iteration's skill and the improved one.
    #[serde(default)]
    pub skill_diff: String,
    pub created_at: DateTime<Utc>,
}

/// A complete skill-evaluation run: a skill tested against a task and a set of
/// validations across one or more scored, self-improving iterations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillEval {
    pub id: Id,
    pub workspace_id: Id,
    /// The original skill's display name.
    pub source_skill: String,
    /// The task the implementation agent was asked to perform.
    pub task: String,
    /// The single CLI that performed the implementation.
    pub impl_cli: String,
    /// Number of iterations requested.
    pub target_iterations: u32,
    pub status: SkillEvalStatus,
    #[serde(default)]
    pub error: Option<String>,
    /// Final human-readable summary of the run.
    #[serde(default)]
    pub summary: String,
    /// Iteration that scored highest.
    #[serde(default)]
    pub best_iteration: Option<u32>,
    #[serde(default)]
    pub best_score: Option<f64>,
    /// All iterations, oldest first.
    #[serde(default)]
    pub iterations: Vec<EvalIteration>,
    /// The original `StartSkillEvalReq` JSON (task, validations, improver, …) so
    /// a single validation can be re-run and the run can be relaunched.
    #[serde(default)]
    pub config: Value,
    pub created_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Agent self-improvement (scheduled workspace self-reflection)
// ---------------------------------------------------------------------------

/// What kicked off an improvement run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImprovementTrigger {
    Scheduled,
    Manual,
    /// The in-loop evolver: fired after a watched interaction concludes.
    Live,
}
impl ImprovementTrigger {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "scheduled" => Some(Self::Scheduled),
            "manual" => Some(Self::Manual),
            "live" => Some(Self::Live),
            _ => None,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Scheduled => "scheduled",
            Self::Manual => "manual",
            Self::Live => "live",
        }
    }
}

/// Lifecycle of an improvement run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImprovementRunStatus {
    Running,
    Done,
    Skipped,
    Failed,
}
impl ImprovementRunStatus {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "running" => Some(Self::Running),
            "done" => Some(Self::Done),
            "skipped" => Some(Self::Skipped),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Done => "done",
            Self::Skipped => "skipped",
            Self::Failed => "failed",
        }
    }
}

/// What an edit targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImprovementTarget {
    Skill,
    Memory,
}
impl ImprovementTarget {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "skill" => Some(Self::Skill),
            "memory" => Some(Self::Memory),
            _ => None,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Skill => "skill",
            Self::Memory => "memory",
        }
    }
}

/// Nature of the change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImprovementEditKind {
    Add,
    Modify,
    Remove,
}
impl ImprovementEditKind {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "add" => Some(Self::Add),
            "modify" => Some(Self::Modify),
            "remove" => Some(Self::Remove),
            _ => None,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::Modify => "modify",
            Self::Remove => "remove",
        }
    }
}

/// Risk classification of an edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImprovementRisk {
    Low,
    Structural,
}
impl ImprovementRisk {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "low" => Some(Self::Low),
            "structural" => Some(Self::Structural),
            _ => None,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Structural => "structural",
        }
    }
}

/// Status of a single edit in the version log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImprovementEditStatus {
    Pending,
    Applied,
    Rejected,
    RolledBack,
    Conflict,
}
impl ImprovementEditStatus {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(Self::Pending),
            "applied" => Some(Self::Applied),
            "rejected" => Some(Self::Rejected),
            "rolled_back" => Some(Self::RolledBack),
            "conflict" => Some(Self::Conflict),
            _ => None,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Applied => "applied",
            Self::Rejected => "rejected",
            Self::RolledBack => "rolled_back",
            Self::Conflict => "conflict",
        }
    }
}

/// Per-workspace autonomy policy for applying edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Autonomy {
    /// Low-risk auto-applies; structural queues.
    #[default]
    Tiered,
    /// Everything queues for approval.
    Propose,
    /// Everything auto-applies.
    Auto,
}
impl Autonomy {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "tiered" => Some(Self::Tiered),
            "propose" => Some(Self::Propose),
            "auto" => Some(Self::Auto),
            _ => None,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Tiered => "tiered",
            Self::Propose => "propose",
            Self::Auto => "auto",
        }
    }
}

/// A self-reflection run record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImprovementRun {
    pub id: Id,
    pub workspace_id: Id,
    pub trigger: ImprovementTrigger,
    pub status: ImprovementRunStatus,
    pub summary: String,
    pub sessions_reviewed: i64,
    pub applied: i64,
    pub pending: i64,
    pub error: Option<String>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

/// A single edit in the version log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImprovementEdit {
    pub id: Id,
    pub run_id: Id,
    pub workspace_id: Id,
    pub target: ImprovementTarget,
    pub target_ref: String,
    pub target_path: String,
    pub kind: ImprovementEditKind,
    pub risk: ImprovementRisk,
    pub status: ImprovementEditStatus,
    pub rationale: String,
    pub evidence: Vec<String>,
    /// File content before this edit. `None` when the file did not exist (add).
    pub before_content: Option<String>,
    /// Full file content this edit writes.
    pub after_content: String,
    pub applied_at: Option<DateTime<Utc>>,
    pub actor: Option<String>,
    pub created_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// API client ("Postman" section) — workspace-scoped
// ---------------------------------------------------------------------------

/// A collection (or nested folder via `parent_id`) of saved API requests.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiCollection {
    pub id: Id,
    pub workspace_id: Id,
    pub name: String,
    /// Parent collection for folder nesting; None = top-level.
    pub parent_id: Option<Id>,
    pub position: i64,
    pub created_at: DateTime<Utc>,
}

/// A saved HTTP request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiRequest {
    pub id: Id,
    pub workspace_id: Id,
    pub collection_id: Option<Id>,
    pub name: String,
    pub method: String,
    pub url: String,
    /// `[{ "key", "value", "enabled" }]`
    pub headers: Value,
    /// `[{ "key", "value", "enabled" }]`
    pub query: Value,
    /// `none | json | raw | form | graphql`
    pub body_mode: String,
    pub body: String,
    /// `{ "type": "none|bearer|basic|api_key", ... }`
    pub auth: Value,
    pub position: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A named set of `{{variable}}` values applied at execute time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiEnvironment {
    pub id: Id,
    pub workspace_id: Id,
    pub name: String,
    /// `{ "<key>": "<value>" }`
    pub variables: Value,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
}

/// One past request execution + its response snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiHistoryEntry {
    pub id: Id,
    pub workspace_id: Id,
    pub method: String,
    pub url: String,
    pub status: Option<i64>,
    pub duration_ms: Option<i64>,
    /// Snapshot of the executed request fields.
    pub request: Value,
    /// `{ status, status_text, headers, body, size_bytes, content_type }`
    pub response: Value,
    pub executed_at: DateTime<Utc>,
}

/// A saved automation: an ordered sequence of saved-request executions with
/// optional per-step assertions and variable extraction (chained across steps).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiAutomation {
    pub id: Id,
    pub workspace_id: Id,
    pub name: String,
    /// `[{ "request_id", "assertions": [{ "kind": "status|json_path|duration_ms",
    ///   "path"?, "op": "eq|ne|contains|lt|gt", "value" }],
    ///   "extract": [{ "path", "var" }] }]`
    pub steps: Value,
    pub created_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// RBAC: per-user, per-feature access ladder
// ---------------------------------------------------------------------------

/// The level of access a user holds for a given feature.
/// Variant order is significant: `None < View < Edit < Admin` (derived `Ord`).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    None,
    View,
    Edit,
    Admin,
}

impl Capability {
    /// Parse from the lowercase string stored in SQLite / JSON.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "none" => Some(Self::None),
            "view" => Some(Self::View),
            "edit" => Some(Self::Edit),
            "admin" => Some(Self::Admin),
            _ => None,
        }
    }

    /// Lowercase string form stored in SQLite / JSON.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::View => "view",
            Self::Edit => "edit",
            Self::Admin => "admin",
        }
    }
}

/// The 18 independently-gatable product features.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Feature {
    Agents,
    Connections,
    Database,
    Git,
    Issues,
    Product,
    Swarm,
    ApiClient,
    Workflows,
    Channels,
    SkillEval,
    Skills,
    Insights,
    Usage,
    SelfImprovement,
    Context,
    Settings,
    Users,
}

impl Feature {
    /// Parse from the snake_case string stored in SQLite / JSON.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "agents" => Some(Self::Agents),
            "connections" => Some(Self::Connections),
            "database" => Some(Self::Database),
            "git" => Some(Self::Git),
            "issues" => Some(Self::Issues),
            "product" => Some(Self::Product),
            "swarm" => Some(Self::Swarm),
            "api_client" => Some(Self::ApiClient),
            "workflows" => Some(Self::Workflows),
            "channels" => Some(Self::Channels),
            "skill_eval" => Some(Self::SkillEval),
            "skills" => Some(Self::Skills),
            "insights" => Some(Self::Insights),
            "usage" => Some(Self::Usage),
            "self_improvement" => Some(Self::SelfImprovement),
            "context" => Some(Self::Context),
            "settings" => Some(Self::Settings),
            "users" => Some(Self::Users),
            _ => None,
        }
    }

    /// Snake_case string form stored in SQLite / JSON.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Agents => "agents",
            Self::Connections => "connections",
            Self::Database => "database",
            Self::Git => "git",
            Self::Issues => "issues",
            Self::Product => "product",
            Self::Swarm => "swarm",
            Self::ApiClient => "api_client",
            Self::Workflows => "workflows",
            Self::Channels => "channels",
            Self::SkillEval => "skill_eval",
            Self::Skills => "skills",
            Self::Insights => "insights",
            Self::Usage => "usage",
            Self::SelfImprovement => "self_improvement",
            Self::Context => "context",
            Self::Settings => "settings",
            Self::Users => "users",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Capability, Feature};

    #[test]
    fn capability_orders_and_roundtrips() {
        assert!(Capability::View < Capability::Edit && Capability::Edit < Capability::Admin);
        assert_eq!(Capability::parse("edit"), Some(Capability::Edit));
        assert_eq!(Feature::parse("database"), Some(Feature::Database));
        assert_eq!(Feature::Database.as_str(), "database");
    }
}
