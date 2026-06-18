//! API request/response DTOs for `/api/v1`.
//!
//! These types are mirrored by `ui/src/lib/api/types.ts`. Endpoint shapes are
//! documented in `docs/contracts/api.md`; the WS protocol in `docs/contracts/ws.md`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::domain::{
    Connection, ConnectionKind, GitProviderKind, IssueProviderKind, ReviewAgentCfg, Session,
    SessionKind, User, Workspace, WorkspaceRole,
};
use crate::Id;

// ---------------------------------------------------------------------------
// Meta / onboarding / auth
// ---------------------------------------------------------------------------

/// `GET /api/v1/meta`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetaResp {
    pub version: String,
    pub api_version: u32,
    pub needs_onboarding: bool,
    pub network_listener: bool,
    /// Detected external tools: name -> found on PATH.
    pub tools: Vec<ToolStatus>,
    /// Available agent providers (from the provider registry).
    pub providers: Vec<String>,
    /// The configured default agent (a provider name), if one is set.
    /// New sessions and channel replies fall back to this when no explicit
    /// provider is chosen.
    pub default_provider: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolStatus {
    pub name: String,
    pub found: bool,
    pub version: Option<String>,
}

/// `POST /api/v1/onboarding/root`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnboardRootReq {
    pub password: String,
    pub display_name: Option<String>,
}

/// `POST /api/v1/auth/login`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginReq {
    pub username: String,
    pub password: String,
}

/// Response for login and onboarding (root is auto-logged-in).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginResp {
    pub token: String,
    pub user: User,
}

// ---------------------------------------------------------------------------
// Users / workspaces
// ---------------------------------------------------------------------------

/// `POST /api/v1/users` (root only)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateUserReq {
    pub username: String,
    pub password: String,
    pub display_name: Option<String>,
}

/// `PATCH /api/v1/users/{id}` (root only)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateUserReq {
    pub display_name: Option<String>,
    pub password: Option<String>,
    pub disabled: Option<bool>,
}

/// `POST /api/v1/workspaces`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateWorkspaceReq {
    pub name: String,
    pub root_path: String,
}

/// `PATCH /api/v1/workspaces/{id}`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateWorkspaceReq {
    pub name: Option<String>,
    pub root_path: Option<String>,
    pub settings: Option<Value>,
    pub archived: Option<bool>,
}

/// One row of `GET /api/v1/workspaces/{id}/members`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberEntry {
    pub user_id: Id,
    pub username: String,
    pub display_name: String,
    pub role: WorkspaceRole,
}

/// `PUT /api/v1/workspaces/{id}/members` — full replacement list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetMembersReq {
    pub members: Vec<SetMemberEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetMemberEntry {
    pub user_id: Id,
    pub role: WorkspaceRole,
}

/// Workspace plus the calling user's role in it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceWithRole {
    #[serde(flatten)]
    pub workspace: Workspace,
    pub my_role: WorkspaceRole,
}

// ---------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------

/// `POST /api/v1/workspaces/{id}/sessions`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSessionReq {
    pub kind: SessionKind,
    /// Agent provider ("claude" | "codex" | "shell") for kind=agent;
    /// ignored for kind=connection (derived from the connection).
    pub provider: Option<String>,
    pub title: Option<String>,
    pub cwd: Option<String>,
    pub connection_id: Option<Id>,
    pub meta: Option<Value>,
}

/// `PATCH /api/v1/sessions/{id}`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateSessionReq {
    pub title: Option<String>,
    #[serde(default)]
    pub meta: Option<Value>,
}

/// Session list/detail responses use `otto_core::domain::Session` directly.
pub type SessionResp = Session;

// ---------------------------------------------------------------------------
// Orchestrator
// ---------------------------------------------------------------------------

/// `POST /api/v1/workspaces/{id}/orchestrate`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestrateReq {
    pub text: String,
    #[serde(default)]
    pub optimize: bool,
    #[serde(default)]
    pub ai_fallback: bool,
    /// Session that currently has focus in the UI (fallback target).
    pub focused_session_id: Option<Id>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestrateResp {
    pub plan: ActionPlan,
    /// The optimized prompt, when `optimize` was requested.
    pub optimized_text: Option<String>,
}

/// `POST /api/v1/workspaces/{id}/orchestrate/execute`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutePlanReq {
    pub plan: ActionPlan,
}

pub type ActionPlan = Vec<Action>;

/// A single orchestrator action, produced by parsing plain English.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Action {
    /// Spawn `count` agent sessions of `provider`.
    SpawnSessions { provider: String, count: u8 },
    /// Send `text` to every running agent session in the workspace.
    Broadcast { text: String },
    /// Open a saved connection as a new session.
    OpenConnection { connection_id: Id },
    /// Send `text` to one specific session.
    RunCommand { session_id: Id, text: String },
}

// ---------------------------------------------------------------------------
// Broadcast (dedicated, AI-free relay to multiple sessions)
// ---------------------------------------------------------------------------

/// `POST /api/v1/workspaces/{id}/broadcast` — relay `text` verbatim to live
/// agent sessions. This is deliberately separate from the orchestrator: no
/// parsing, no AI, no fallback — it always broadcasts the literal text.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BroadcastReq {
    /// The message to send. Submitted as if typed + Enter.
    pub text: String,
    /// Sessions to target. `None`/absent (or empty) → every live agent session
    /// in the workspace. When `Some`, only the listed sessions that are live
    /// agents receive it.
    #[serde(default)]
    pub session_ids: Option<Vec<Id>>,
}

/// Result of a broadcast: the sessions that actually received the message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BroadcastResp {
    pub session_ids: Vec<Id>,
}

// ---------------------------------------------------------------------------
// Connections
// ---------------------------------------------------------------------------

/// `POST /api/v1/workspaces/{id}/connections` and PATCH variant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpsertConnectionReq {
    pub name: String,
    pub kind: ConnectionKind,
    pub params: Value,
    /// Write-only secret; stored in Keychain, never echoed back.
    pub secret: Option<String>,
    pub first_command: Option<String>,
    /// Section to place this profile in; None/absent = ungrouped.
    #[serde(default)]
    pub section_id: Option<Id>,
}

/// `POST /api/v1/workspaces/{id}/connection-sections` and `PATCH /connection-sections/{id}`.
/// On create, `parent_id` nests the section (absent/None = top-level); rename ignores it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpsertSectionReq {
    pub name: String,
    #[serde(default)]
    pub parent_id: Option<Id>,
    /// Tree this section lives in on create: "connections" (default) or "db".
    /// Ignored on rename. See [`crate::domain::ConnectionSection::scope`].
    #[serde(default)]
    pub scope: Option<String>,
}

/// Query string for `GET /workspaces/{id}/connection-sections?scope=…`.
/// Absent → "connections" (the Connections page tree).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SectionScopeQuery {
    #[serde(default)]
    pub scope: Option<String>,
}

/// `POST /api/v1/workspaces/{id}/connection-sections/reorder`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReorderSectionsReq {
    pub ids: Vec<Id>,
}

/// `POST /api/v1/connection-sections/{id}/move` — reparent (None = top-level).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoveSectionReq {
    #[serde(default)]
    pub parent_id: Option<Id>,
}

/// `POST /api/v1/connections/{id}/test`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestConnectionResp {
    pub ok: bool,
    pub latency_ms: Option<u64>,
    pub message: String,
    /// True when the built command unavoidably exposes the secret in argv
    /// (clickhouse-client) — UI shows a warning banner.
    pub warn_argv: bool,
}

/// Connection responses use the domain type (secret_ref is opaque).
pub type ConnectionResp = Connection;

// ---------------------------------------------------------------------------
// Git: accounts, repos, local ops
// ---------------------------------------------------------------------------

/// `POST /api/v1/git/accounts`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateGitAccountReq {
    pub provider: GitProviderKind,
    pub label: String,
    pub username: String,
    /// Write-only; stored in Keychain.
    pub token: String,
    pub api_base_url: Option<String>,
    /// Namespace for remote-repo browsing: Bitbucket workspace, GitHub org,
    /// GitLab group. Optional.
    pub namespace: Option<String>,
    /// Optional user-entered token expiry (for providers that don't expose it,
    /// e.g. Bitbucket). Drives expiry notifications.
    #[serde(default)]
    pub token_expires_at: Option<DateTime<Utc>>,
}

/// `POST /api/v1/issue/accounts`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateIssueAccountReq {
    pub provider: IssueProviderKind,
    pub label: String,
    pub email: String,
    pub base_url: String,
    /// Write-only; stored in Keychain.
    pub token: String,
    /// Optional user-entered token expiry. Drives expiry notifications.
    #[serde(default)]
    pub token_expires_at: Option<DateTime<Utc>>,
}

/// `PATCH /api/v1/git/accounts/{id}`
/// Any field present is updated; absent fields keep their current value.
/// `namespace` / `api_base_url`: empty string clears to NULL, non-empty sets, absent keeps.
/// `token`: non-empty rotates the Keychain secret; empty/absent keeps the existing secret.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateGitAccountReq {
    pub label: Option<String>,
    pub username: Option<String>,
    /// Empty string → clear to NULL; non-empty → set; absent (None) → keep current.
    pub namespace: Option<String>,
    /// Empty string → clear to NULL; non-empty → set; absent (None) → keep current.
    pub api_base_url: Option<String>,
    /// Non-empty → rotate Keychain secret; empty/absent → keep existing.
    pub token: Option<String>,
    /// Set the user-entered token expiry; absent (None) → keep current.
    #[serde(default)]
    pub token_expires_at: Option<DateTime<Utc>>,
}

/// `PATCH /api/v1/issue/accounts/{id}`
/// Any field present is updated; absent fields keep their current value.
/// `token`: non-empty rotates the Keychain secret; empty/absent keeps the existing secret.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateIssueAccountReq {
    pub label: Option<String>,
    pub email: Option<String>,
    pub base_url: Option<String>,
    /// Non-empty → rotate Keychain secret; empty/absent → keep existing.
    pub token: Option<String>,
    /// Set the user-entered token expiry; absent (None) → keep current.
    #[serde(default)]
    pub token_expires_at: Option<DateTime<Utc>>,
}

// ---------------------------------------------------------------------------
// Notifications
// ---------------------------------------------------------------------------

/// `GET /api/v1/notifications/settings` and `PUT` body (all fields required on PUT).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationSettings {
    /// Notify this many days before a credential expires.
    pub expiry_threshold_days: u32,
    /// Raise native OS notifications for warn/error notices.
    pub native_enabled: bool,
    /// Emit notices for session-progress events (finished / awaiting-input / exited).
    pub session_events: bool,
}

impl Default for NotificationSettings {
    fn default() -> Self {
        Self {
            expiry_threshold_days: 3,
            native_enabled: true,
            session_events: true,
        }
    }
}

/// `POST /api/v1/workspaces/{id}/repos` — register existing path or clone.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddRepoReq {
    /// Existing local path to register; mutually exclusive with `clone_url`.
    pub path: Option<String>,
    /// Remote URL to clone into the workspace directory.
    pub clone_url: Option<String>,
    pub name: Option<String>,
    pub git_account_id: Option<Id>,
}

/// One changed file in `GET /repos/{id}/status`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileChange {
    pub path: String,
    pub orig_path: Option<String>,
    /// "modified" | "added" | "deleted" | "renamed" | "untracked" | "conflicted"
    pub kind: String,
    pub staged: bool,
    pub unstaged: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoStatusResp {
    pub branch: String,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub changes: Vec<FileChange>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchInfo {
    pub name: String,
    pub is_current: bool,
    pub upstream: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitInfo {
    pub sha: String,
    pub short_sha: String,
    pub author: String,
    pub date: DateTime<Utc>,
    pub subject: String,
    #[serde(default)]
    pub parents: Vec<String>,
    #[serde(default)]
    pub refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefBranch {
    pub name: String,
    pub is_current: bool,
    pub upstream: Option<String>,
    pub remote: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefTag {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefsResp {
    pub local: Vec<RefBranch>,
    pub remote: Vec<RefBranch>,
    pub tags: Vec<RefTag>,
}

/// One line origin in a diff hunk: "context" | "add" | "del".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineOrigin {
    Context,
    Add,
    Del,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffLine {
    pub origin: LineOrigin,
    pub content: String,
    pub old_line: Option<u32>,
    pub new_line: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hunk {
    pub header: String,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDiff {
    pub path: String,
    pub old_path: Option<String>,
    pub is_binary: bool,
    pub hunks: Vec<Hunk>,
}

/// `GET /repos/{id}/diff?target=worktree|staged|commit:<sha>|range:<a>..<b>`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffResp {
    pub files: Vec<FileDiff>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StagePathsReq {
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitReq {
    pub message: String,
    #[serde(default)]
    pub amend: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckoutReq {
    pub branch: String,
    #[serde(default)]
    pub create: bool,
}

// ---------------------------------------------------------------------------
// Git: local merge + conflict resolution (#4)
// ---------------------------------------------------------------------------

/// Strategy for a local branch merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalMergeStrategy {
    /// `git merge --no-ff` — always create a merge commit.
    MergeCommit,
    /// `git merge` — fast-forward when possible, otherwise a merge commit.
    Ff,
    /// `git merge --ff-only` — fail (no write) when not fast-forwardable.
    FfOnly,
    /// `git merge --squash` — stage the merge as a single commit (no merge parent).
    Squash,
}

fn default_local_merge_strategy() -> LocalMergeStrategy {
    LocalMergeStrategy::MergeCommit
}

/// `POST /repos/{id}/merge` — merge `source` into `target` (target is checked
/// out first). Never auto-resolves conflicts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeBranchReq {
    pub source: String,
    pub target: String,
    #[serde(default = "default_local_merge_strategy")]
    pub strategy: LocalMergeStrategy,
}

/// Outcome of a local merge or merge-completion. Conflicts are a NORMAL 200
/// result (`status == "conflicts"`), not an error, so the resolver can read the
/// conflicted-file list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeResult {
    /// "merged" | "conflicts" | "up_to_date"
    pub status: String,
    /// New HEAD sha when merged; None for conflicts / up_to_date.
    pub commit: Option<String>,
    #[serde(default)]
    pub conflicted_files: Vec<String>,
    /// Fresh repo status after the operation.
    pub repo_status: RepoStatusResp,
}

/// `GET /repos/{id}/merge/status`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeConflictStatus {
    /// True when a merge is in progress (`MERGE_HEAD` present).
    pub merging: bool,
    /// Best-effort source branch/ref being merged, when known.
    pub source: Option<String>,
    #[serde(default)]
    pub conflicted_files: Vec<String>,
}

/// One segment of a conflicted file: either shared context or a conflict region
/// with both sides (and the merge base when diff3 data is available).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConflictSegment {
    Context {
        lines: Vec<String>,
    },
    Conflict {
        ours: Vec<String>,
        theirs: Vec<String>,
        #[serde(default)]
        base: Vec<String>,
    },
}

/// `GET /repos/{id}/conflict?path=<p>` — a conflicted file split into segments
/// so the client can render each conflict and deterministically rebuild the file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictFile {
    pub path: String,
    pub is_binary: bool,
    pub segments: Vec<ConflictSegment>,
}

/// `POST /repos/{id}/conflict/resolve` — write the fully-resolved file content
/// (markers removed) and stage it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolveConflictReq {
    pub path: String,
    pub content: String,
}

/// `POST /repos/{id}/merge/commit` — finish an in-progress merge.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeCommitReq {
    /// Commit message; when omitted, the prepared MERGE_MSG / `--no-edit` is used.
    #[serde(default)]
    pub message: Option<String>,
}

// ---------------------------------------------------------------------------
// Git: pull requests
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrState {
    Open,
    Merged,
    Declined,
    All,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrSummary {
    /// Provider-native id used in follow-up calls (number for GH/GL, id for BB).
    pub number: u64,
    pub title: String,
    pub author: String,
    pub state: PrState,
    pub source_branch: String,
    pub target_branch: String,
    pub updated_at: DateTime<Utc>,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrComment {
    pub id: String,
    pub author: String,
    pub body: String,
    /// File path + line for inline comments; None for general comments.
    pub path: Option<String>,
    pub line: Option<u32>,
    pub created_at: DateTime<Utc>,
    pub replies: Vec<PrComment>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrDetail {
    #[serde(flatten)]
    pub summary: PrSummary,
    pub description_md: String,
    pub comments: Vec<PrComment>,
    /// Display names of approvers (kept for back-compat; see `reviewers`).
    pub approved_by: Vec<String>,
    /// Structured reviewers with approval state and best-effort avatar/timestamp.
    #[serde(default)]
    pub reviewers: Vec<PrReviewer>,
    pub mergeable: Option<bool>,
}

/// A PR reviewer/participant with their approval state. `avatar_url` and
/// `reviewed_at` are best-effort (provider-dependent; None when unavailable).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrReviewer {
    pub name: String,
    pub approved: bool,
    #[serde(default)]
    pub avatar_url: Option<String>,
    #[serde(default)]
    pub reviewed_at: Option<DateTime<Utc>>,
}

/// One commit on a pull/merge request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrCommit {
    pub sha: String,
    pub short_sha: String,
    pub author: String,
    pub date: DateTime<Utc>,
    pub subject: String,
}

/// `POST /repos/{id}/prs/{number}/request-changes`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestChangesReq {
    #[serde(default)]
    pub body: Option<String>,
}

/// `GET /prs/...` diff reuses `DiffResp`.

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatePrReq {
    pub title: String,
    pub description: String,
    pub source_branch: String,
    pub target_branch: String,
}

/// `POST /repos/{id}/pr/draft` — ask an agent to draft a PR title + description
/// from the current branch's diff against `base`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftPrReq {
    pub base: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftPrResp {
    pub title: String,
    pub description: String,
    pub source_branch: String,
    pub target_branch: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdatePrReq {
    pub title: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewPrCommentReq {
    pub body: String,
    pub path: Option<String>,
    pub line: Option<u32>,
    /// Reply to an existing comment id, if any.
    pub in_reply_to: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeStrategy {
    Merge,
    Squash,
    Rebase,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergePrReq {
    #[serde(default = "default_merge_strategy")]
    pub strategy: MergeStrategy,
}

fn default_merge_strategy() -> MergeStrategy {
    MergeStrategy::Merge
}

// ---------------------------------------------------------------------------
// Channels / workspace integrations
// ---------------------------------------------------------------------------

/// `PUT /api/v1/workspaces/{id}/integrations/{channel}`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpsertIntegrationReq {
    pub enabled: bool,
    /// Write-only; when Some and non-empty the token is (re-)stored in keychain.
    pub bot_token: Option<String>,
    /// Write-only; Slack app-level token (slack only).
    pub app_token: Option<String>,
    pub allowed_users: String,
    pub agent_reply: bool,
    pub reply_instructions: String,
    pub channel_id: String,
    /// Preferred agent CLI for this channel. Empty = use the default agent.
    #[serde(default)]
    pub preferred_cli: String,
}

// ---------------------------------------------------------------------------
// Provider updates
// ---------------------------------------------------------------------------

/// `POST /api/v1/workspaces/{id}/providers/update`
///
/// When `provider` is `Some`, only that provider's update command is run.
/// When `None`, all providers with a configured update command are updated.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpdateProvidersReq {
    /// Optionally restrict to a single named provider.
    pub provider: Option<String>,
}

// ---------------------------------------------------------------------------
// PR Review config
// ---------------------------------------------------------------------------

/// Persisted configuration for the PR review pipeline.
/// Stored in the `settings` table under the key `pr_review`.
/// `GET /api/v1/settings/pr-review` and `PUT /api/v1/settings/pr-review`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewConfig {
    pub agents: Vec<ReviewAgentCfg>,
    pub summarizer: ReviewAgentCfg,
    /// User-defined reusable reviewer presets the UI offers alongside the
    /// built-in ones. Persisted with the config; not used by the runner.
    #[serde(default)]
    pub custom_presets: Vec<ReviewAgentCfg>,
}

// ---------------------------------------------------------------------------
// PR Review start request
// ---------------------------------------------------------------------------

/// `POST /api/v1/repos/{id}/prs/{number}/review` — optional request body.
/// All fields default to `None` so an empty (or absent) body still parses.
#[derive(Debug, Default, Deserialize)]
pub struct StartReviewReq {
    #[serde(default)]
    pub issue_account_id: Option<String>,
    #[serde(default)]
    pub issue_key: Option<String>,
    /// Free-text guidance from the user (e.g. "what to focus on"). Passed to the
    /// review agents alongside the diff. Empty/absent behaves as before.
    #[serde(default)]
    pub context: Option<String>,
}

// ---------------------------------------------------------------------------
// Local review
// ---------------------------------------------------------------------------

/// `POST /api/v1/repos/{id}/local-review` — start a review of the local
/// working tree against a base branch/ref.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalReviewReq {
    /// The base git ref to diff against (e.g. `origin/develop`, `main`).
    pub base: String,
}

/// `POST /api/v1/reviews/{id}/handoff` — hand review findings to a new agent
/// session so the agent can fix them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandoffReq {
    /// The provider to spawn (e.g. "claude", "codex", "agy").
    pub provider: String,
    /// Optional list of comment ids to include. When `None`, all non-declined
    /// comments are included. When `Some`, only the listed comments are sent.
    #[serde(default)]
    pub comment_ids: Option<Vec<String>>,
}

/// Where a handover brief is delivered: a freshly spawned agent, or an existing
/// running agent in the same workspace.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HandoverTarget {
    /// Spawn a new agent of `provider` ("claude" | "codex" | "agy" | …).
    NewAgent { provider: String },
    /// Inject into an existing agent session already in this workspace.
    ExistingSession { session_id: Id },
}

/// `POST /api/v1/sessions/{id}/handover` — pass the source agent's working
/// context (summarized best-effort, optionally with git state) into the target
/// agent, so it can continue the work.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandoverReq {
    /// Where to deliver the brief.
    pub target: HandoverTarget,
    /// Free-text note describing what the receiving agent should focus on. The
    /// generated brief is weighted toward this.
    #[serde(default)]
    pub focus: Option<String>,
    /// Title for the new session (NewAgent only). Defaults to "Handover from <source>".
    #[serde(default)]
    pub title: Option<String>,
    /// A pre-generated/edited brief. When present, the server skips
    /// summarization and injects this verbatim (the "review before sending" flow).
    #[serde(default)]
    pub brief: Option<String>,
    /// Include the repo's git state (branch, changed files, recent commits) in
    /// the generated brief. Ignored when `brief` is supplied. Defaults to true.
    #[serde(default)]
    pub include_git: Option<bool>,
    /// Summarize with a fast model (haiku) instead of the default. Ignored when
    /// `brief` is supplied. Defaults to false.
    #[serde(default)]
    pub fast: Option<bool>,
    /// Archive the source session once the handover is sent. Defaults to false.
    #[serde(default)]
    pub archive_source: Option<bool>,
}

/// `POST /api/v1/sessions/{id}/handover/brief` — generate the handover brief
/// (synchronously) so the user can review/edit it before sending.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandoverBriefReq {
    #[serde(default)]
    pub focus: Option<String>,
    #[serde(default)]
    pub include_git: Option<bool>,
    #[serde(default)]
    pub fast: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandoverBriefResp {
    /// The generated brief (markdown). Empty when there was no context at all.
    pub brief: String,
    /// True when summarization was unavailable and `brief` is raw context.
    pub fallback: bool,
    /// True when some source context (transcript/scrollback/git) was found.
    pub had_context: bool,
}

// ---------------------------------------------------------------------------
// Session input
// ---------------------------------------------------------------------------

/// `POST /api/v1/sessions/{id}/input` — inject text into a session's PTY.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendInputReq {
    /// Text to write into the PTY.
    pub text: String,
    /// When `None` or `true`, append `"\n"` so the agent immediately executes
    /// the text.  When `false`, send the text verbatim (no newline) so the
    /// user can inspect / edit before pressing Enter.
    #[serde(default)]
    pub submit: Option<bool>,
}

// ---------------------------------------------------------------------------
// Problem response
// ---------------------------------------------------------------------------

/// Error body returned by every endpoint on failure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Problem {
    pub code: String,
    pub message: String,
}

// ---------------------------------------------------------------------------
// Agent self-improvement
// ---------------------------------------------------------------------------

/// Per-workspace self-reflection config (stored under
/// `Workspace.settings.self_improvement`). `last_run_at`/`next_run_at` are
/// scheduler-managed; the rest are user-editable.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelfImprovementConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_cadence_minutes")]
    pub cadence_minutes: u32,
    #[serde(default = "default_lookback_hours")]
    pub lookback_hours: u32,
    #[serde(default)]
    pub skill_allowlist: Vec<String>,
    #[serde(default)]
    pub autonomy: crate::domain::Autonomy,
    /// Agent CLIs to run the analysis on. Each runs independently with its own
    /// default model, so you get a separate set of suggestions per provider.
    /// Defaults to `["claude"]`.
    #[serde(default = "default_providers")]
    pub providers: Vec<String>,
    /// When true, the in-loop evolver watches this workspace's live agent
    /// sessions and improves the skills they use right after each interaction.
    #[serde(default)]
    pub live_evolve: bool,
    #[serde(default)]
    pub last_run_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub next_run_at: Option<DateTime<Utc>>,
}

fn default_cadence_minutes() -> u32 {
    60
}
fn default_lookback_hours() -> u32 {
    24
}
fn default_providers() -> Vec<String> {
    vec!["claude".to_string()]
}

impl Default for SelfImprovementConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            cadence_minutes: default_cadence_minutes(),
            lookback_hours: default_lookback_hours(),
            skill_allowlist: Vec::new(),
            autonomy: crate::domain::Autonomy::default(),
            providers: default_providers(),
            live_evolve: false,
            last_run_at: None,
            next_run_at: None,
        }
    }
}

/// `PUT /workspaces/{id}/self-improvement` — user-editable fields only.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateSelfImprovementReq {
    pub enabled: bool,
    pub cadence_minutes: u32,
    pub lookback_hours: u32,
    pub skill_allowlist: Vec<String>,
    pub autonomy: crate::domain::Autonomy,
    #[serde(default = "default_providers")]
    pub providers: Vec<String>,
    #[serde(default)]
    pub live_evolve: bool,
}

/// `POST /workspaces/{id}/self-improvement/run`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunNowResp {
    pub run_id: Id,
}

// ---------------------------------------------------------------------------
// Context provisioning (skills + souls + context library, per-workspace config)
// ---------------------------------------------------------------------------

/// A skill entry in the Otto library.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibrarySkill {
    pub name: String,
    pub description: String,
    pub body: String,
}

/// A soul (persona) entry in the Otto library.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibrarySoul {
    pub name: String,
    pub body: String,
}

/// A reusable context snippet in the Otto library.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibraryContext {
    pub name: String,
    pub body: String,
}

/// `PUT /library/skills/{name}` (and souls/context — only `body` is sent;
/// `name` comes from the path).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpsertLibraryEntryReq {
    pub body: String,
}

/// `PUT /library/default-soul`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalSoulReq {
    pub name: String,
}

/// `GET /library/default-soul`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalSoulResp {
    pub name: Option<String>,
}

/// Per-workspace context config, stored under `Workspace.settings.context`.
/// `skills = None` ⇒ all library skills active; `soul = None` ⇒ global default.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceContextConfig {
    #[serde(default)]
    pub skills: Option<Vec<String>>,
    #[serde(default)]
    pub soul: Option<String>,
    #[serde(default)]
    pub extra_context_md: String,
    #[serde(default = "default_include_memory")]
    pub include_memory: bool,
}

fn default_include_memory() -> bool {
    true
}

impl Default for WorkspaceContextConfig {
    fn default() -> Self {
        Self {
            skills: None,
            soul: None,
            extra_context_md: String::new(),
            include_memory: true,
        }
    }
}

/// `PUT /workspaces/{id}/context`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateWorkspaceContextReq {
    #[serde(default)]
    pub skills: Option<Vec<String>>,
    #[serde(default)]
    pub soul: Option<String>,
    #[serde(default)]
    pub extra_context_md: String,
    #[serde(default = "default_include_memory")]
    pub include_memory: bool,
}

/// One provider's result from a materialize action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaterializeProviderResult {
    pub provider: String,
    pub files_written: Vec<String>,
    pub skipped: bool,
}

/// `POST /workspaces/{id}/context/materialize`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaterializeResp {
    pub provider_results: Vec<MaterializeProviderResult>,
}

// ---------------------------------------------------------------------------
// API client ("Postman" section). Collection/request/environment routes are
// workspace-scoped (`/workspaces/{wid}/api-client/...`), so `workspace_id`
// comes from the path, not these bodies.
// ---------------------------------------------------------------------------

fn default_body_mode() -> String {
    "none".to_string()
}

/// `POST/PATCH /workspaces/{wid}/api-client/collections[/{id}]`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpsertApiCollectionReq {
    pub name: String,
    #[serde(default)]
    pub parent_id: Option<Id>,
}

/// `POST/PATCH /workspaces/{wid}/api-client/requests[/{id}]`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpsertApiRequestReq {
    #[serde(default)]
    pub collection_id: Option<Id>,
    pub name: String,
    pub method: String,
    pub url: String,
    #[serde(default)]
    pub headers: Value,
    #[serde(default)]
    pub query: Value,
    #[serde(default = "default_body_mode")]
    pub body_mode: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub auth: Value,
}

/// `POST/PATCH /workspaces/{wid}/api-client/environments[/{id}]`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpsertApiEnvironmentReq {
    pub name: String,
    #[serde(default)]
    pub variables: Value,
}

/// `POST /workspaces/{wid}/api-client/execute` — run a request through the
/// daemon. `{{var}}` placeholders are substituted from `environment_id` (or the
/// workspace's active environment when absent). The run is recorded in history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecuteApiReq {
    pub method: String,
    pub url: String,
    #[serde(default)]
    pub headers: Value,
    #[serde(default)]
    pub query: Value,
    #[serde(default = "default_body_mode")]
    pub body_mode: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub auth: Value,
    #[serde(default)]
    pub environment_id: Option<Id>,
    /// Per-request execution settings (timeout, redirects, TLS verification).
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub follow_redirects: Option<bool>,
    #[serde(default)]
    pub verify_ssl: Option<bool>,
    #[serde(default)]
    pub vars: Option<Value>,
}

/// Response of `POST .../api-client/execute`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiResponse {
    pub status: u16,
    pub status_text: String,
    /// `[{ "key", "value" }]`
    pub headers: Value,
    /// UTF-8 (lossy) body for display, truncated to a display cap when large.
    pub body: String,
    /// Full response bytes, base64-encoded — used for binary preview (images)
    /// and "save to disk". Empty when the response is `too_large`.
    #[serde(default)]
    pub body_base64: String,
    /// `body` was cut to the display cap (full bytes still in `body_base64`).
    #[serde(default)]
    pub truncated: bool,
    /// Body exceeded the inline cap: neither `body` nor `body_base64` is set.
    #[serde(default)]
    pub too_large: bool,
    pub duration_ms: i64,
    pub size_bytes: i64,
    pub content_type: Option<String>,
    /// Per-phase trace of the request lifecycle (resolved request, TTFB,
    /// download, redirects, completion) for the response "Trace" tab.
    #[serde(default)]
    pub trace: Vec<TraceStep>,
}

/// One step in a request's execution trace (see [`ApiResponse::trace`]).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceStep {
    pub label: String,
    pub detail: String,
    /// Duration of this phase in milliseconds, when measured.
    #[serde(default)]
    pub ms: Option<i64>,
    /// One of: info | timing | redirect | success | error (UI styling hint).
    #[serde(default)]
    pub level: String,
}

/// `POST /api-client/import-curl` — parse a curl command into request fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportCurlReq {
    pub curl: String,
}

/// Parsed request fields from a curl command (drop into a request form).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedCurl {
    pub method: String,
    pub url: String,
    pub headers: Value,
    pub query: Value,
    pub body_mode: String,
    pub body: String,
    pub auth: Value,
}

/// `POST/PATCH /workspaces/{wid}/api-client/automations[/{id}]`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpsertApiAutomationReq {
    pub name: String,
    #[serde(default)]
    pub steps: Value,
}

/// One step's result in an automation run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiRunStepResult {
    pub request_id: Id,
    pub name: String,
    pub status: Option<u16>,
    pub duration_ms: i64,
    /// True when the request succeeded AND every assertion passed.
    pub ok: bool,
    /// `[{ "desc", "passed" }]`
    pub assertions: Value,
    pub error: Option<String>,
}

/// `POST /workspaces/{wid}/api-client/automations/{id}/run` → run report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiRunResult {
    pub automation_id: Id,
    pub steps: Vec<ApiRunStepResult>,
    /// True when every step was `ok`.
    pub passed: bool,
}

// ---------------------------------------------------------------------------
// Skills Evaluator
// ---------------------------------------------------------------------------

/// One validation the evaluator runs against the produced implementation. Each
/// validation fans out to one agent per entry in `providers` (so a single
/// validation can be cross-checked by several CLIs).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SkillEvalValidationCfg {
    /// Short identifier, e.g. "logs", "docs", "naming".
    pub name: String,
    /// What this validation checks and how to judge it. Passed to the agent.
    pub criteria: String,
    /// CLIs to run this validation on (one agent each). Empty falls back to the
    /// run's implementation CLI.
    #[serde(default)]
    pub providers: Vec<String>,
    /// Model hint ("haiku" | "sonnet" | "opus" | ""). Empty = provider default.
    #[serde(default)]
    pub model: String,
}

/// Config for the agent that edits/improves the skill between iterations.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SkillEvalImproverCfg {
    pub provider: String,
    #[serde(default)]
    pub model: String,
}

/// Persisted defaults for the Skills Evaluator (settings key `skill_eval`).
/// `GET /api/v1/settings/skill-eval` and `PUT /api/v1/settings/skill-eval`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillEvalConfig {
    /// Default validations offered when starting a new evaluation.
    pub validations: Vec<SkillEvalValidationCfg>,
    /// Default improver agent.
    pub improver: SkillEvalImproverCfg,
    /// Default number of iterations.
    #[serde(default = "default_iterations")]
    pub iterations: u32,
    /// Default validation passes (averaged) — see `StartSkillEvalReq`.
    #[serde(default = "default_validator_passes")]
    pub validator_passes: u32,
}

fn default_iterations() -> u32 {
    2
}

/// Where the skill under test comes from.
/// - `kind = "library"`: `reference` is the Otto library skill name.
/// - `kind = "path"`: `reference` is an absolute path to a skill folder, a
///   `SKILL.md`/`.md` file, or a `.zip`/`.gz`/`.tgz` archive containing one.
/// - `kind = "provider"`: `reference` is a skill name under
///   `~/.<provider>/skills/<name>/SKILL.md` (provider in `provider`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillSourceReq {
    pub kind: String,
    pub reference: String,
    #[serde(default)]
    pub provider: Option<String>,
}

/// `POST /api/v1/workspaces/{id}/skill-evaluations` — start an evaluation run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartSkillEvalReq {
    pub source: SkillSourceReq,
    /// The task to implement using the skill (e.g. "add endpoint X").
    pub task: String,
    /// The single CLI that implements the task.
    pub impl_cli: String,
    /// Validations to run after each implementation.
    pub validations: Vec<SkillEvalValidationCfg>,
    /// Total iterations (rounds). >= 1. Round 1 is the baseline; each later
    /// round improves the skill and re-runs.
    pub iterations: u32,
    /// Agent that edits the skill between iterations (defaults to the impl CLI).
    #[serde(default)]
    pub improver: Option<SkillEvalImproverCfg>,
    /// Git ref to create each iteration's worktree from (defaults to HEAD).
    #[serde(default)]
    pub base_ref: Option<String>,
    /// How many times to run each validation and average — higher reduces the
    /// noise from nondeterministic graders. 1–3, defaults to 1.
    #[serde(default = "default_validator_passes")]
    pub validator_passes: u32,
}

fn default_validator_passes() -> u32 {
    1
}

/// `POST /api/v1/skill-evaluations/{id}/promote` — save an iteration's skill
/// back into the Otto library under `name`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromoteSkillReq {
    pub iteration_id: Id,
    /// "tested" = the skill that iteration ran with; "improved" = the edited
    /// version it produced for the next round.
    pub source: String,
    /// Target library skill name (safe segment).
    pub name: String,
}

/// `GET /api/v1/skill-evaluations/{id}/iterations/{iter_id}/diff` — the code the
/// implementation agent produced in that iteration's worktree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImplDiffResp {
    pub diff: String,
    pub truncated: bool,
}

/// A discoverable skill source the UI can offer in the start form.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillSourceInfo {
    /// "library" | "provider".
    pub kind: String,
    /// Skill name.
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Set for `kind = "provider"` (claude/codex/agy).
    #[serde(default)]
    pub provider: Option<String>,
}

/// `GET /api/v1/workspaces/{id}/skill-sources` — skills the user can pick from.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillSourcesResp {
    pub sources: Vec<SkillSourceInfo>,
}

// ---------------------------------------------------------------------------
// Agent activity (live trail + task tracker)
// ---------------------------------------------------------------------------

/// `POST /workspaces/{wid}/sessions/{sid}/trail` — append one trail entry.
/// `source`/`kind` are lowercase strings (default `user`/`note`).
#[derive(Debug, Clone, Deserialize)]
pub struct AppendTrailReq {
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    /// `info | warn | error` (default `info`).
    #[serde(default)]
    pub level: Option<String>,
    pub summary: String,
    #[serde(default)]
    pub detail: Option<Value>,
}

/// One task in a [`PutTasksReq`].
#[derive(Debug, Clone, Deserialize)]
pub struct TaskInput {
    #[serde(default)]
    pub ext_id: Option<String>,
    pub title: String,
    /// `pending | in_progress | completed | blocked | cancelled`.
    pub status: String,
}

/// `PUT /workspaces/{wid}/sessions/{sid}/tasks` — replace the whole task list
/// (the task tracker is provider-synced; each push is the source of truth).
#[derive(Debug, Clone, Deserialize)]
pub struct PutTasksReq {
    pub tasks: Vec<TaskInput>,
}
