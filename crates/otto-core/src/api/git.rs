//! Git: accounts, repos, local ops; Git: local merge + conflict resolution; Git: pull requests.
//!
//! Split out of `api.rs` by area (re-exported from there, so every path is
//! still `otto_core::api::*`). Mirrored by `ui/src/lib/api/types/git.ts / git-merge.ts`.
use super::*;

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
    /// Tri-state user-entered token expiry: absent → keep current, `null` →
    /// clear it, a timestamp → set it. (A plain `Option` read `null` as "keep",
    /// so the "Token expired" chip could never be cleared.)
    #[serde(
        default,
        deserialize_with = "de_double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub token_expires_at: Option<Option<DateTime<Utc>>>,
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
    /// Optional parent directory to clone INTO (the repo lands at
    /// `<clone_dir>/<name>`). A leading `~` is expanded. Defaults to the
    /// workspace `root_path` when absent. Only meaningful with `clone_url`.
    #[serde(default)]
    pub clone_dir: Option<String>,
}

/// `PATCH /api/v1/repos/{id}` — (re)bind a registered repo to a hosting
/// account. Registration resolves the account once, so a repo added before its
/// account existed needs this to reach a provider at all.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateRepoReq {
    /// Account to bind; `null` unbinds (ssh-agent remotes need no account).
    #[serde(default)]
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
    /// Multi-step git operation currently in progress in this worktree:
    /// "merge" | "rebase" | "cherry_pick" | "revert" — detected from the git
    /// dir's state files (MERGE_HEAD, rebase-merge/, CHERRY_PICK_HEAD, …).
    /// `None` when no operation is underway. Conflicted files can still exist
    /// with `None` (e.g. a conflicting `stash pop` leaves no state file).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub op_in_progress: Option<String>,
    /// Total untracked paths git reported. Present only when the untracked
    /// rows in `changes` were capped (`untracked_truncated`), so a client can
    /// say "N more untracked" — a non-ignored build dir must not ship 200k rows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub untracked_total: Option<u32>,
    /// True when `changes` holds only the first `UNTRACKED_ROW_CAP` untracked
    /// rows (tracked/staged/conflicted rows are never capped).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub untracked_truncated: bool,
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
    /// Commits relative to this local branch's upstream (zero without a live
    /// upstream, and for remote refs). Computed without checking out the branch.
    #[serde(default)]
    pub ahead: u32,
    #[serde(default)]
    pub behind: u32,
    /// True when this branch's tip is already contained in the repo's cleanup
    /// base branch (`git merge-base --is-ancestor`, surfaced as a bulk
    /// `git branch --merged <base>`). A hint that the branch is safe to delete;
    /// the base branch itself is never flagged. Defaults false for older
    /// clients / responses that predate the field.
    #[serde(default)]
    pub merged_into_base: bool,
    /// Every integration branch (the cleanup base plus `develop` / `main` /
    /// `master` when present, local else `origin/`) whose history already holds
    /// this tip — so a git-flow repo shows a release "in develop" even while
    /// the base (`main`) doesn't hold it yet. Integration branches and their
    /// remote twins are never listed. Empty for older responses.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub merged_into: Vec<String>,
    /// Full sha this branch points at. Lets a client locate the branch in the
    /// graph WITHOUT the tip having to be in the loaded page of history — the
    /// tip of a long-dormant branch is exactly the one that isn't. Empty for
    /// responses that predate the field.
    #[serde(default)]
    pub sha: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefTag {
    pub name: String,
    /// Full sha of the COMMIT the tag resolves to — an annotated tag is
    /// dereferenced (`%(*objectname)`), so this is never the tag object's own
    /// sha and always matches a commit in `git log`. Empty for responses that
    /// predate the field.
    #[serde(default)]
    pub sha: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefsResp {
    pub local: Vec<RefBranch>,
    pub remote: Vec<RefBranch>,
    pub tags: Vec<RefTag>,
    /// The base branch that `merged_into_base` was computed against (the per-repo
    /// `cleanup_base_branch` override, else the detected default branch). `None`
    /// when the repo has no resolvable base (e.g. no branches yet) — then no
    /// branch is flagged merged.
    #[serde(default)]
    pub base_branch: Option<String>,
}

/// GET/PUT `/repos/{id}/cleanup-base` — the per-repo base branch used for the
/// "safe to delete (merged)" branch indicators. `base_branch` is the stored
/// override (`None` = follow the detected default); `resolved` is what it
/// currently resolves to (override if valid, else the detected default).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanupBaseResp {
    pub base_branch: Option<String>,
    pub resolved: Option<String>,
}

/// PUT `/repos/{id}/cleanup-base` body. A `None`/empty `base_branch` clears the
/// override so the repo follows its detected default branch again.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetCleanupBaseReq {
    #[serde(default)]
    pub base_branch: Option<String>,
}

/// One entry from `git stash list`. Read-only; surfaced in the graph's STASHES
/// sidebar section and rendered as a dashed node in the commit graph when the
/// stash commit happens to be present in the (`--all`) log. `parents` are
/// `[base, index, (untracked)]` — the helper commits the frontend de-emphasises.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StashInfo {
    /// N in `stash@{N}` (0 = most recent).
    pub index: u32,
    /// Reflog selector, e.g. "stash@{0}".
    #[serde(rename = "ref")]
    pub r#ref: String,
    /// Full SHA of the stash (WIP) commit.
    pub sha: String,
    /// Parent SHAs: `[base, index, (untracked)]`.
    #[serde(default)]
    pub parents: Vec<String>,
    /// Author date, ISO 8601 (the frontend formats it).
    pub date: String,
    /// Reflog subject, e.g. "On main: my WIP work".
    pub message: String,
    /// Branch the stash was created on, parsed from the message (if present).
    pub branch: Option<String>,
}

/// One entry from `git worktree list --porcelain`. Surfaced in the graph's
/// WORKTREES sidebar section so agent-created worktrees (swarm, Run-with-Otto,
/// goal loops, manual `git worktree add`) are visible and reapable from the UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeInfo {
    /// Absolute path of the worktree.
    pub path: String,
    /// HEAD commit SHA ("" for a bare entry).
    #[serde(default)]
    pub head: String,
    /// Checked-out branch (short name); None when detached.
    pub branch: Option<String>,
    /// True for the main worktree (the repo itself) — never removable.
    pub is_main: bool,
    /// True when locked (`git worktree lock`).
    #[serde(default)]
    pub locked: bool,
    /// Lock reason, when one was recorded.
    pub lock_reason: Option<String>,
    /// True when git considers the entry stale (its directory is gone) —
    /// removable via `worktree prune` rather than `worktree remove`.
    #[serde(default)]
    pub prunable: bool,
    /// True when the worktree has uncommitted changes (best-effort; false when
    /// the check fails or the entry is prunable).
    #[serde(default)]
    pub dirty: bool,
    /// Whether the bounded status probe succeeded; unknown is never force-safe.
    #[serde(default)]
    pub dirty_known: bool,
}

/// One entry from `git submodule status`, enriched with `.gitmodules` config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmoduleInfo {
    /// Path relative to the repo root (also the display name).
    pub path: String,
    /// Recorded/checked-out commit SHA (gitlink).
    pub sha: String,
    /// "ok" | "uninitialized" (-) | "modified" (+ checked-out ≠ recorded) |
    /// "conflict" (U merge conflicts).
    pub state: String,
    /// `git describe` output for the checked-out commit, when git printed one.
    pub describe: Option<String>,
    /// Remote URL from `.gitmodules` (if configured).
    pub url: Option<String>,
    /// Tracking branch from `.gitmodules` (if configured).
    pub branch: Option<String>,
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

/// How a file changed — git's `A/D/M/R/C/T` status letters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileChangeStatus {
    Added,
    Deleted,
    Modified,
    Renamed,
    Copied,
    /// File type changed (regular file ↔ symlink ↔ submodule).
    Typechange,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDiff {
    /// SHA-256 of the byte-exact file diff; required by hunk mutations.
    #[serde(default)]
    pub fingerprint: String,
    pub path: String,
    pub old_path: Option<String>,
    pub is_binary: bool,
    pub hunks: Vec<Hunk>,
    /// True when the file diff hit the server's per-file cap (rendered text >
    /// 200 KB or > 5,000 lines): `hunks` is empty, counts are kept.
    #[serde(default)]
    pub too_large: Option<bool>,
    /// True when `hunks` was left out (summary mode, per-file cap or the
    /// whole-response budget) — fetch them with `?path=`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hunks_omitted: Option<bool>,
    /// Number of added lines (populated by the diff parser; `None` for binary).
    #[serde(default)]
    pub added: Option<u32>,
    /// Number of deleted lines (populated by the diff parser; `None` for binary).
    #[serde(default)]
    pub deleted: Option<u32>,
    /// A/D/M/R/C/T. `None` only where the source can't tell (provider diffs
    /// without headers).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<FileChangeStatus>,
    /// Detected language hint (e.g. "rust", "typescript").
    #[serde(default)]
    pub language: Option<String>,
}

/// `GET /repos/{id}/diff?target=worktree|staged|commit:<sha>|range:<a>..<b>|range:<a>...<b>`
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DiffResp {
    pub files: Vec<FileDiff>,
    /// The whole-response budget was hit; later files carry `hunks_omitted`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
    /// Σ `files[].added` / `files[].deleted` (binary files count 0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_added: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_deleted: Option<u64>,
    /// `true` when git skipped rename detection (more files than the
    /// `-l1000` limit): some renames are reported as a delete + an add.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub renames_incomplete: Option<bool>,
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
    /// `true` → `-S`, `false` → `--no-gpg-sign`, absent → repo config
    /// (`commit.gpgsign`). Lets the composer's Sign toggle override the repo
    /// default per commit without writing config.
    #[serde(default)]
    pub sign: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckoutReq {
    pub branch: String,
    #[serde(default)]
    pub create: bool,
    /// Wrap the switch in stash -u → checkout → pop when the tree is dirty.
    /// Nothing is ever pulled or merged — that is `POST /pull`.
    #[serde(default)]
    pub auto_stash: bool,
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

pub(super) fn default_local_merge_strategy() -> LocalMergeStrategy {
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
    /// When true and the working tree is dirty, auto-stash before the merge and
    /// pop the stash afterwards (stash → merge → pop). When false, a dirty tree
    /// is refused. Default false.
    #[serde(default)]
    pub auto_stash: bool,
}

/// `POST /repos/{id}/merge/preview` — dry-run a merge of `source` into `target`
/// using `git merge-tree` (writes only to the object DB; the working tree and
/// index are NOT touched). Lets the UI warn about conflicts BEFORE starting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergePreviewReq {
    pub source: String,
    pub target: String,
}

/// Result of a merge dry-run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergePreview {
    /// True if merging would produce conflicts.
    pub conflicts: bool,
    /// Files that would conflict (best-effort; from `merge-tree --name-only`).
    #[serde(default)]
    pub conflicted_files: Vec<String>,
    /// True if the merge would be a no-op (source already in target).
    #[serde(default)]
    pub up_to_date: bool,
}

/// Outcome of a local merge or merge-completion. Conflicts are a NORMAL 200
/// result (`status == "conflicts"`), not an error, so the resolver can read the
/// conflicted-file list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeResult {
    /// "merged" | "conflicts" | "up_to_date" | "paused"
    pub status: String,
    /// New HEAD sha when merged; None for conflicts / up_to_date.
    pub commit: Option<String>,
    #[serde(default)]
    pub conflicted_files: Vec<String>,
    /// Fresh repo status after the operation.
    pub repo_status: RepoStatusResp,
    /// Optional human-readable note (e.g. auto-stash outcome) for the UI to
    /// surface as a toast. None for the common case.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// `GET /repos/{id}/merge/status`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeConflictStatus {
    /// True when ANY resolvable operation is underway or conflicted files
    /// exist — the signal for "the resolver has work to do". (Historically
    /// merge-only; now also rebase/cherry-pick/revert and a conflicted
    /// stash-pop, so those stopped being invisible dead ends.)
    pub merging: bool,
    /// Which operation: "merge" | "rebase" | "cherry_pick" | "revert", or
    /// `None` for conflicts with no state file (stash pop / squash).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub op: Option<String>,
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
    #[serde(default)]
    pub ours_present: bool,
    #[serde(default)]
    pub theirs_present: bool,
    #[serde(default)]
    pub worktree_present: bool,
    #[serde(default)]
    pub trailing_newline: bool,
    pub path: String,
    pub is_binary: bool,
    pub segments: Vec<ConflictSegment>,
}

/// `POST /repos/{id}/conflict/resolve` — resolve one conflicted file and stage
/// it: either write `content` (the fully-rebuilt file, markers removed), or
/// take a whole side with `side: "ours" | "theirs"` (`git checkout --ours/
/// --theirs` + `add`; `content` is ignored then).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolveConflictReq {
    pub path: String,
    #[serde(default)]
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub side: Option<String>,
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
    /// True if this is a draft PR.
    #[serde(default)]
    pub draft: Option<bool>,
    /// Simplified CI/check status: "passing" | "failing" | "pending" | "unknown".
    #[serde(default)]
    pub ci_status: Option<String>,
    /// Labels on the PR (empty when provider doesn't expose them).
    #[serde(default)]
    pub labels: Vec<String>,
    /// Warnings from PR creation (e.g. a reviewer request that failed after the
    /// PR was opened). Only ever populated on the create response; empty (and
    /// omitted from JSON) everywhere else.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reviewer_warnings: Vec<String>,
    /// The source branch's head commit as the provider reports it (Bitbucket
    /// abbreviates it). Opaque: clients pass it back as the PR diff's `rev=`
    /// so a push re-keys the diff memo instead of serving the old diff.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_sha: Option<String>,
}

/// One page of `GET /repos/{id}/prs`. `has_more` is the provider's own
/// next-page signal (never `items.len() == per_page`, which lies on a full last
/// page); `page`/`per_page` echo the CLAMPED request so a client can build the
/// next call without re-deriving the bounds.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrListResp {
    pub items: Vec<PrSummary>,
    pub has_more: bool,
    pub page: u32,
    pub per_page: u32,
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
    /// Thread resolution status (meaningful on thread heads only; replies
    /// inherit the head's state and default to `false`).
    #[serde(default)]
    pub resolved: bool,
    /// Provider id to pass to the resolve/unresolve endpoint. Bitbucket: the
    /// top-level comment id; GitLab: the discussion id; GitHub: the GraphQL
    /// review-thread node id (NOT the REST comment id). None on replies and
    /// on non-resolvable comments.
    #[serde(default)]
    pub thread_id: Option<String>,
    /// Which side of the diff `line` counts on — `old` for a comment on a
    /// deleted line, `new` (or absent) otherwise. Lets the UI render the
    /// comment under exactly one row instead of every row whose old OR new
    /// number happens to match.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub side: Option<PrCommentSide>,
    /// The forge no longer maps the comment onto the current diff (GitHub
    /// `line: null`, GitLab/Bitbucket outdated positions). `line` then holds
    /// the original line; the UI lists it under "File comments" with an
    /// "outdated" chip rather than pinning it to an unrelated row.
    #[serde(default)]
    pub outdated: bool,
}

/// Side of a unified diff an inline PR comment anchors to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PrCommentSide {
    /// The base (left) side — a deleted line; `line` is the OLD line number.
    Old,
    /// The head (right) side — an added or unchanged line; `line` is the NEW
    /// line number.
    #[default]
    New,
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

/// A minimal hosted **issue** (not a PR). Used by Run with Otto's GitHub-issue
/// source adapter; only GitHub implements `GitProvider::get_issue` today.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueLite {
    pub number: u64,
    pub title: String,
    pub body: String,
    pub url: String,
    /// `open` | `closed`.
    pub state: String,
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

/// One entry from `GET /repos/{id}/collaborators?q=` — a person the bound
/// forge account can request as a reviewer. `name` is the provider-native
/// handle submitted back in `CreatePrReq::reviewers`; `display_name` is for
/// the typeahead UI only.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Collaborator {
    pub name: String,
    pub display_name: String,
}

/// `POST /git/accounts/test` — verify a not-yet-saved account form. The token
/// travels in the body exactly once and is never persisted or logged. The
/// stored-token variant (`POST /git/accounts/{id}/test`) takes no body.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TestGitAccountReq {
    #[serde(default)]
    pub provider: Option<crate::domain::GitProviderKind>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub api_base_url: Option<String>,
}

/// Result of a git-account connection test. Auth/provider failures come back
/// as `ok: false` + `error` (HTTP 200) so the form can render them inline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitAccountTestResp {
    pub ok: bool,
    /// Login the provider authenticated us as (when `ok`).
    #[serde(default)]
    pub login: Option<String>,
    /// Token scopes echoed from response headers where the provider exposes
    /// them (`X-OAuth-Scopes` on GitHub/Bitbucket); empty otherwise.
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default)]
    pub error: Option<String>,
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
    /// Open the PR as a draft. GitHub: native `draft` flag; GitLab: `Draft: `
    /// title prefix (the API-blessed convention); Bitbucket Cloud: `draft` on
    /// the create payload. Absent/None = today's ready-for-review behavior.
    #[serde(default)]
    pub draft: Option<bool>,
    /// Reviewers to request at creation (provider-native usernames). GitHub
    /// requests them in a best-effort second call after create; GitLab resolves
    /// usernames to `reviewer_ids`; Bitbucket resolves workspace members to
    /// `{uuid}` entries. Names that can't be requested/resolved surface in
    /// `PrSummary::reviewer_warnings` — they never fail the PR creation.
    #[serde(default)]
    pub reviewers: Option<Vec<String>>,
}

/// `POST /repos/{id}/pr/draft` — ask an agent to draft a PR title + description
/// from a branch's diff against `base`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftPrReq {
    pub base: String,
    /// The PR's source branch. When set the draft describes
    /// `merge-base(base, head)..head` — that branch's committed work — and
    /// `DraftPrResp::source_branch` echoes it. Absent/empty ⇒ the checked-out
    /// branch (plus its working tree), as before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftPrResp {
    pub title: String,
    pub description: String,
    pub source_branch: String,
    pub target_branch: String,
    /// The Otto session that did the drafting. Drafting runs as a REAL session
    /// (visible in Agents from the moment it spawns, so it can be opened in a
    /// pane and watched while the modal spins), and this is how the caller links
    /// to it afterwards. `None` only for callers that draft without a session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<Id>,
}

/// `POST /repos/{id}/draft-commit-message` — ask an agent to draft a commit
/// message from the staged diff (falls back to the full working diff when
/// nothing is staged). No request body is required.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DraftCommitMessageReq {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftCommitMessageResp {
    pub message: String,
    /// Whether the message was drafted from the staged diff (`true`) or fell
    /// back to the full working diff because nothing was staged (`false`).
    pub from_staged: bool,
    /// The Otto session that drafted this (a visible `commit-draft` lean turn
    /// on the drafting model — same path as `DraftPrResp.session_id`).
    #[serde(default)]
    pub session_id: Option<Id>,
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
    /// Diff side `line` counts on (default `new`). A comment on a deleted
    /// line sends `old` with `line` = its OLD line number.
    #[serde(default)]
    pub side: Option<PrCommentSide>,
    /// The OLD line number of the anchored row when it has one (deleted and
    /// unchanged rows). GitLab requires both `old_line` and `new_line` for a
    /// comment on an unchanged context line.
    #[serde(default)]
    pub old_line: Option<u32>,
    /// Head commit the reviewer was looking at. Anchors the inline comment
    /// to that diff and saves the daemon a PR round-trip for the head sha
    /// (GitHub); ignored by forges that don't need it.
    #[serde(default)]
    pub commit_id: Option<String>,
}

/// Body for `POST /repos/{id}/prs/{number}/comments/{cid}/resolve` — `{cid}`
/// is the thread's `PrComment.thread_id`. `resolved: false` reopens.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ResolvePrThreadReq {
    pub resolved: bool,
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
    /// Ask the provider to delete the PR's source branch as part of the merge
    /// (GitHub: a follow-up ref delete; GitLab: `should_remove_source_branch`;
    /// Bitbucket: `close_source_branch`). Additive — absent means "keep it".
    #[serde(default)]
    pub delete_source_branch: bool,
    /// The PR head the caller reviewed (`PrSummary::head_sha` at dialog load).
    /// When set, a PR whose head has moved since is refused with 409 "PR
    /// changed — re-check" (GitHub/GitLab: forwarded as the forge's `sha` pin;
    /// Bitbucket: compared by prefix before merging). Absent ⇒ unpinned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_head_sha: Option<String>,
}

pub(super) fn default_merge_strategy() -> MergeStrategy {
    MergeStrategy::Merge
}
