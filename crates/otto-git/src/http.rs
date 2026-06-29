//! Axum router for contract endpoints #31–#56: git accounts, repos, local
//! operations and pull requests. Mounted by otto-server under `/api/v1`.

use std::collections::HashMap;
use std::path::{Path as FsPath, PathBuf};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Extension, Json, Router};
use otto_core::api::{
    AddRepoReq, BranchInfo, CheckoutReq, CommitInfo, CommitReq, ConflictFile, CreateGitAccountReq,
    CreatePrReq, DiffResp, MergeBranchReq, MergeCommitReq, MergeConflictStatus, MergePrReq,
    MergePreview, MergePreviewReq, MergeResult, NewPrCommentReq, PrComment, PrCommit, PrDetail,
    PrState, PrSummary, Problem,
    RefsResp, RepoStatusResp, RequestChangesReq, ResolveConflictReq, StagePathsReq, StashInfo,
    UpdateGitAccountReq, UpdatePrReq,
};
use otto_core::auth::{authorize_owner, AuthUser, RoleChecker};
use otto_core::domain::{GitAccount, GitProviderKind, Repo, WorkspaceRole};
use otto_core::event::Event;
use otto_core::secrets::SecretStore;
use otto_core::{new_id, Error, Id, Result};
use otto_state::{GitStore, NewGitAccount, NewRepo, WorkspacesRepo};
use serde::Deserialize;

use crate::local::{DiffTarget, LocalGit};
use crate::providers::{detect, make_provider, GitProvider, RemoteRef, RemoteRepoSummary};

/// Dependencies the git router needs from the host application state.
/// otto-server implements this on its `AppState`.
#[async_trait::async_trait]
pub trait GitCtx: Clone + Send + Sync + 'static {
    fn store(&self) -> &GitStore;
    /// Needed to resolve a workspace's `root_path` as the clone destination.
    fn workspaces(&self) -> &WorkspacesRepo;
    fn secrets(&self) -> &Arc<dyn SecretStore>;
    fn roles(&self) -> &Arc<dyn RoleChecker>;
    fn events(&self) -> &tokio::sync::broadcast::Sender<Event>;
    /// Optional gate run before a PR is created. The default allows everything;
    /// `otto-server` overrides it to enforce the PR's linked proof pack (a PR
    /// over an unproven pack is rejected unless `allow_unproven`).
    async fn check_pr_allowed(&self, _workspace_id: &str, _req: &CreatePrReq) -> Result<()> {
        Ok(())
    }

    /// Optional hook after a PR is successfully created. `otto-server` overrides
    /// it to (1) link the proof pack to the new PR, (2) capture its CI status as a
    /// `ci` evidence artifact, and (3) run the PR-description consistency check
    /// against the actual change, recording a `pr_check` artifact (Proof Packs v2 —
    /// automatic CI + PR-consistency integration). The full `req` is passed so the
    /// override has the PR title/description and target branch. Best-effort: the
    /// default does nothing and overrides must never fail PR creation. `ci` is the
    /// freshly-fetched aggregate for the new PR.
    async fn after_pr_created(
        &self,
        _repo: &Repo,
        _pr_number: u64,
        _req: &CreatePrReq,
        _ci: &crate::types::CiStatus,
    ) {
    }
}

/// Build the git router. Paths are relative to the `/api/v1` mount point.
pub fn router<S: GitCtx>() -> Router<S> {
    Router::new()
        // accounts (#31–33)
        .route(
            "/git/accounts",
            get(list_accounts::<S>).post(create_account::<S>),
        )
        .route(
            "/git/accounts/{id}",
            axum::routing::patch(update_account::<S>).delete(delete_account::<S>),
        )
        .route("/git/accounts/{id}/remote-repos", get(remote_repos::<S>))
        // global repo list (workspace-independent Git page)
        .route("/git/repos", get(list_all_repos::<S>))
        // repos (#34–36)
        .route(
            "/workspaces/{id}/repos",
            get(list_repos::<S>).post(add_repo::<S>),
        )
        .route("/workspaces/{id}/repos/detect", post(detect_repo::<S>))
        .route("/repos/{id}", delete(delete_repo::<S>))
        // local ops (#37–47)
        .route("/repos/{id}/status", get(repo_status::<S>))
        .route("/repos/{id}/branches", get(repo_branches::<S>))
        .route("/repos/{id}/refs", get(repo_refs::<S>))
        .route("/repos/{id}/log", get(repo_log::<S>))
        .route("/repos/{id}/stashes", get(repo_stashes::<S>))
        .route("/repos/{id}/fetch", post(repo_fetch::<S>))
        .route("/repos/{id}/diff", get(repo_diff::<S>))
        .route("/repos/{id}/stage", post(repo_stage::<S>))
        .route("/repos/{id}/unstage", post(repo_unstage::<S>))
        .route("/repos/{id}/discard", post(repo_discard::<S>))
        .route("/repos/{id}/commit", post(repo_commit::<S>))
        .route("/repos/{id}/push", post(repo_push::<S>))
        .route("/repos/{id}/pull", post(repo_pull::<S>))
        .route("/repos/{id}/checkout", post(repo_checkout::<S>))
        // graph context-menu ops (commit / branch / tag)
        .route("/repos/{id}/cherry-pick", post(repo_cherry_pick::<S>))
        .route("/repos/{id}/revert", post(repo_revert::<S>))
        .route("/repos/{id}/branch", post(repo_branch_create::<S>))
        .route("/repos/{id}/branch/rename", post(repo_branch_rename::<S>))
        .route("/repos/{id}/branch/delete", post(repo_branch_delete::<S>))
        .route("/repos/{id}/tag", post(repo_tag_create::<S>))
        .route("/repos/{id}/tag/push", post(repo_tag_push::<S>))
        .route("/repos/{id}/tag/delete", post(repo_tag_delete::<S>))
        .route("/repos/{id}/api-collections/pull", post(repo_collections_pull::<S>))
        .route("/repos/{id}/api-collections/push", post(repo_collections_push::<S>))
        .route("/repos/{id}/stash", post(repo_stash::<S>))
        // local merge + conflict resolution (#4)
        .route("/repos/{id}/merge", post(repo_merge::<S>))
        .route("/repos/{id}/merge/preview", post(repo_merge_preview::<S>))
        .route("/repos/{id}/merge/status", get(repo_merge_status::<S>))
        .route("/repos/{id}/merge/abort", post(repo_merge_abort::<S>))
        .route("/repos/{id}/merge/commit", post(repo_merge_commit::<S>))
        .route("/repos/{id}/conflict", get(repo_conflict::<S>))
        .route(
            "/repos/{id}/conflict/resolve",
            post(repo_conflict_resolve::<S>),
        )
        // PRs (#48–56)
        .route("/repos/{id}/prs", get(pr_list::<S>).post(pr_create::<S>))
        .route(
            "/repos/{id}/prs/{number}",
            get(pr_detail::<S>).patch(pr_update::<S>),
        )
        .route("/repos/{id}/prs/{number}/diff", get(pr_diff::<S>))
        .route("/repos/{id}/prs/{number}/comments", post(pr_comment::<S>))
        .route("/repos/{id}/prs/{number}/approve", post(pr_approve::<S>))
        .route("/repos/{id}/prs/{number}/merge", post(pr_merge::<S>))
        .route("/repos/{id}/prs/{number}/decline", post(pr_decline::<S>))
        .route(
            "/repos/{id}/prs/{number}/request-changes",
            post(pr_request_changes::<S>),
        )
        .route("/repos/{id}/prs/{number}/commits", get(pr_commits::<S>))
}

// ---------------------------------------------------------------------------
// Error → response
// ---------------------------------------------------------------------------

/// Local error wrapper: `otto_core::Error` → Problem JSON with the right status.
pub struct ApiError(pub Error);

impl From<Error> for ApiError {
    fn from(e: Error) -> Self {
        Self(e)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match &self.0 {
            Error::NotFound(_) => StatusCode::NOT_FOUND,
            Error::Unauthorized => StatusCode::UNAUTHORIZED,
            Error::Forbidden(_) => StatusCode::FORBIDDEN,
            Error::Conflict(_) => StatusCode::CONFLICT,
            Error::Invalid(_) => StatusCode::BAD_REQUEST,
            Error::PayloadTooLarge(_) => StatusCode::PAYLOAD_TOO_LARGE,
            Error::UnsupportedMedia(_) => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Error::Upstream(_) => StatusCode::BAD_GATEWAY,
            Error::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let body = Problem {
            code: self.0.code().to_string(),
            message: self.0.to_string(),
        };
        (status, Json(body)).into_response()
    }
}

type ApiResult<T> = std::result::Result<T, ApiError>;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Load a repo, check the caller's workspace role, return a LocalGit handle.
async fn repo_ctx<S: GitCtx>(
    s: &S,
    user: &AuthUser,
    repo_id: &Id,
    min: WorkspaceRole,
) -> Result<(Repo, LocalGit)> {
    let repo = s.store().get_repo(repo_id).await?;
    s.roles().check(&user.0, &repo.workspace_id, min).await?;
    let git = LocalGit::new(&repo.path);
    Ok((repo, git))
}

/// Resolve the push/pull token for a repo's bound account (None when no
/// account is bound — ssh remotes work through the user's agent).
fn account_token<S: GitCtx>(s: &S, account: &GitAccount) -> Result<String> {
    s.secrets()
        .get(&account.token_ref)?
        .ok_or_else(|| Error::Invalid(format!("token missing for git account {}", account.id)))
}

/// S4 guard: a repo's bound git credential may be *used* only by its owner (or
/// root). A workspace can have many members and a repo binds exactly one account,
/// so the workspace role-check alone does not stop user B from pushing / opening
/// PRs through user A's hosting token. Returns the bound account when the caller
/// is authorized; `None` when no account is bound (ssh-via-agent remotes); and
/// `Forbidden` when the caller is neither the owner nor root.
async fn authorized_repo_account<S: GitCtx>(
    s: &S,
    user: &AuthUser,
    repo: &Repo,
) -> Result<Option<GitAccount>> {
    let Some(account_id) = repo.git_account_id.as_ref() else {
        return Ok(None);
    };
    let account = s.store().get_account(account_id).await?;
    authorize_owner(&account, &user.0)?;
    Ok(Some(account))
}

/// Resolve the push/pull token for a repo's bound account, enforcing the S4
/// ownership guard. `None` when no account is bound (ssh remotes work through the
/// user's agent); `Forbidden` when the caller does not own the bound account.
async fn optional_token<S: GitCtx>(s: &S, user: &AuthUser, repo: &Repo) -> Result<Option<String>> {
    match authorized_repo_account(s, user, repo).await? {
        Some(account) => Ok(s.secrets().get(&account.token_ref)?),
        None => Ok(None),
    }
}

/// Resolve provider client + remote ref for PR routes (400 when not bound).
/// Enforces the S4 ownership guard: the caller must own the repo's bound account.
async fn provider_ctx<S: GitCtx>(
    s: &S,
    user: &AuthUser,
    repo: &Repo,
) -> Result<(Arc<dyn GitProvider>, RemoteRef)> {
    let kind = repo
        .provider
        .ok_or_else(|| Error::Invalid("repo has no git provider".into()))?;
    if repo.git_account_id.is_none() {
        return Err(Error::Invalid("repo has no git account".into()));
    }
    let account = authorized_repo_account(s, user, repo)
        .await?
        .ok_or_else(|| Error::Invalid("repo has no git account".into()))?;
    if account.provider != kind {
        return Err(Error::Invalid(
            "git account provider does not match repo provider".into(),
        ));
    }
    let remote = repo
        .remote_url
        .as_deref()
        .ok_or_else(|| Error::Invalid("repo has no remote url".into()))?;
    let (_, remote_ref) =
        detect(remote).ok_or_else(|| Error::Invalid(format!("unsupported remote: {remote}")))?;
    let token = account_token(s, &account)?;
    Ok((make_provider(&account, token), remote_ref))
}

fn notice(s: &impl GitCtx, level: &str, title: &str, body: &str) {
    let _ = s.events().send(Event::Notice {
        level: level.to_string(),
        title: title.to_string(),
        body: body.to_string(),
    });
}

/// Process-wide registry of per-repo async locks. A merge is a multi-step
/// sequence (checkout → merge → status); serialising all mutating merge/conflict
/// operations on a given repo id prevents concurrent requests from interleaving
/// and corrupting the in-progress merge state. Read-only GETs skip this.
fn repo_locks() -> &'static StdMutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>> {
    static LOCKS: OnceLock<StdMutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> =
        OnceLock::new();
    LOCKS.get_or_init(|| StdMutex::new(HashMap::new()))
}

/// Return (creating if needed) the async mutex guarding repo `id`.
fn repo_lock(id: &Id) -> Arc<tokio::sync::Mutex<()>> {
    let mut map = repo_locks().lock().expect("repo_locks poisoned");
    map.entry(id.to_string())
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone()
}

// ---------------------------------------------------------------------------
// Accounts (#31–33)
// ---------------------------------------------------------------------------

async fn list_accounts<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
) -> ApiResult<Json<Vec<GitAccount>>> {
    let accounts = s.store().list_accounts(&user.0.id).await?;
    Ok(Json(accounts))
}

async fn create_account<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Json(req): Json<CreateGitAccountReq>,
) -> ApiResult<Json<GitAccount>> {
    if req.token.trim().is_empty() {
        return Err(Error::Invalid("token must not be empty".into()).into());
    }
    if req.username.trim().is_empty() {
        return Err(Error::Invalid("username must not be empty".into()).into());
    }
    let token_ref = format!("gitacct-{}", new_id());
    s.secrets().put(&token_ref, &req.token)?;
    let created = s
        .store()
        .create_account(NewGitAccount {
            user_id: user.0.id.clone(),
            provider: req.provider,
            label: req.label,
            username: req.username,
            token_ref: token_ref.clone(),
            api_base_url: req.api_base_url,
            namespace: req.namespace,
            token_expires_at: req.token_expires_at,
        })
        .await;
    match created {
        Ok(a) => Ok(Json(a)),
        Err(e) => {
            // Don't leave an orphan secret behind.
            let _ = s.secrets().delete(&token_ref);
            Err(e.into())
        }
    }
}

async fn update_account<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<UpdateGitAccountReq>,
) -> ApiResult<Json<GitAccount>> {
    let account = s.store().get_account(&id).await?;
    if account.user_id != user.0.id && !user.0.is_root {
        return Err(Error::Forbidden("not the account owner".into()).into());
    }

    // Merge: absent field keeps current value; present field overwrites.
    let label = req.label.as_deref().unwrap_or(&account.label);
    let username = req.username.as_deref().unwrap_or(&account.username);

    // namespace / api_base_url: absent → keep current; Some("") → clear (None); Some(v) → set.
    let namespace: Option<String> = match req.namespace.as_deref() {
        None => account.namespace.clone(),
        Some("") => None,
        Some(v) => Some(v.to_string()),
    };
    let api_base_url: Option<String> = match req.api_base_url.as_deref() {
        None => account.api_base_url.clone(),
        Some("") => None,
        Some(v) => Some(v.to_string()),
    };

    // Token rotation: non-empty → store new ref, delete old; empty/absent → keep.
    let token_ref = if let Some(tok) = req.token.as_deref().filter(|t| !t.is_empty()) {
        let new_ref = format!("gitacct-{}", new_id());
        s.secrets().put(&new_ref, tok)?;
        // Best-effort cleanup of old secret; don't fail if it's already gone.
        let _ = s.secrets().delete(&account.token_ref);
        new_ref
    } else {
        account.token_ref.clone()
    };

    // token_expires_at: present → set; absent (None) → keep current.
    let token_expires_at = req.token_expires_at.or(account.token_expires_at);

    let updated = s
        .store()
        .update_account(
            &id,
            label,
            username,
            &token_ref,
            namespace.as_deref(),
            api_base_url.as_deref(),
            token_expires_at,
        )
        .await?;
    Ok(Json(updated))
}

async fn delete_account<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<StatusCode> {
    let account = s.store().get_account(&id).await?;
    if account.user_id != user.0.id && !user.0.is_root {
        return Err(Error::Forbidden("not the account owner".into()).into());
    }
    let _ = s.secrets().delete(&account.token_ref);
    s.store().delete_account(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Remote repo listing
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct RemoteReposQuery {
    q: Option<String>,
}

async fn remote_repos<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Query(q): Query<RemoteReposQuery>,
) -> ApiResult<Json<Vec<RemoteRepoSummary>>> {
    let account = s.store().get_account(&id).await?;
    if account.user_id != user.0.id && !user.0.is_root {
        return Err(Error::Forbidden("not the account owner".into()).into());
    }
    let namespace = account
        .namespace
        .as_deref()
        .filter(|n| !n.is_empty())
        .ok_or_else(|| Error::Invalid("set a namespace on this account first".into()))?;
    let token = account_token(&s, &account)?;
    let provider = make_provider(&account, token);
    let query = q.q.as_deref().filter(|s| !s.is_empty());
    Ok(Json(provider.list_repos(namespace, query).await?))
}

// ---------------------------------------------------------------------------
// Repos (#34–36)
// ---------------------------------------------------------------------------

/// `GET /git/repos` — every repo across the workspaces the caller may view,
/// ordered by name. Backs the workspace-independent Git page (top-level repo
/// tabs + landing list). Root sees all repos; a non-root user sees repos only in
/// workspaces they are a member of (any role ≥ Viewer — membership grants at
/// least Viewer). Per-repo operations still authorize against the repo's own
/// workspace, so this only widens *discovery*, not access.
async fn list_all_repos<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
) -> ApiResult<Json<Vec<Repo>>> {
    let all = s.store().list_all_repos().await?;
    if user.0.is_root {
        return Ok(Json(all));
    }
    // Membership in a workspace implies ≥ Viewer; filter to the caller's set.
    let visible: std::collections::HashSet<Id> = s
        .workspaces()
        .list_for_user(&user.0.id)
        .await?
        .into_iter()
        .map(|(ws, _role)| ws.id)
        .collect();
    Ok(Json(
        all.into_iter()
            .filter(|r| visible.contains(&r.workspace_id))
            .collect(),
    ))
}

async fn list_repos<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(ws_id): Path<Id>,
) -> ApiResult<Json<Vec<Repo>>> {
    s.roles()
        .check(&user.0, &ws_id, WorkspaceRole::Viewer)
        .await?;
    Ok(Json(s.store().list_repos(&ws_id).await?))
}

async fn add_repo<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(ws_id): Path<Id>,
    Json(req): Json<AddRepoReq>,
) -> ApiResult<Json<Repo>> {
    s.roles()
        .check(&user.0, &ws_id, WorkspaceRole::Editor)
        .await?;
    match (&req.path, &req.clone_url) {
        (Some(path), None) => Ok(Json(register_repo(&s, &user, &ws_id, path, &req).await?)),
        (None, Some(url)) => Ok(Json(
            clone_into_workspace(&s, &user, &ws_id, url, &req).await?,
        )),
        _ => Err(Error::Invalid("provide exactly one of path | clone_url".into()).into()),
    }
}

#[derive(Deserialize)]
struct DetectRepoReq {
    /// Any path inside a working tree (typically a session's cwd).
    path: String,
}

/// POST /workspaces/{id}/repos/detect — resolve the git work-tree root that
/// contains `path` and register it in the workspace (idempotent: a repo already
/// registered at that root is returned as-is). Lets the UI surface git for the
/// folder a session is running in without manual registration.
async fn detect_repo<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(ws_id): Path<Id>,
    Json(req): Json<DetectRepoReq>,
) -> ApiResult<Json<Repo>> {
    s.roles()
        .check(&user.0, &ws_id, WorkspaceRole::Editor)
        .await?;
    if req.path.trim().is_empty() {
        return Err(Error::Invalid("path must not be empty".into()).into());
    }
    let top = LocalGit::new(&req.path).toplevel().await?;
    // Already registered at this root? Return it.
    let existing = s.store().list_repos(&ws_id).await?;
    if let Some(found) = existing.into_iter().find(|r| r.path == top) {
        return Ok(Json(found));
    }
    let add = AddRepoReq {
        path: Some(top.clone()),
        clone_url: None,
        name: None,
        git_account_id: None,
        clone_dir: None,
    };
    Ok(Json(register_repo(&s, &user, &ws_id, &top, &add).await?))
}

async fn register_repo<S: GitCtx>(
    s: &S,
    user: &AuthUser,
    ws_id: &Id,
    path: &str,
    req: &AddRepoReq,
) -> Result<Repo> {
    // Resolve to the git work-tree root so registering the same repo (or any
    // subdirectory of one) is idempotent — and this also validates that `path`
    // actually IS a git repository (toplevel errors otherwise).
    let top = LocalGit::new(path)
        .toplevel()
        .await
        .map_err(|_| Error::Invalid(format!("not a git repository: {path}")))?;
    // De-dup: a repo already registered at this root in the workspace is returned
    // as-is instead of inserting a duplicate row (mirrors `detect_repo`). Without
    // this, re-adding the same local path mints a fresh id → a second, identical
    // tab in the Git page.
    if let Some(found) = s
        .store()
        .list_repos(ws_id)
        .await?
        .into_iter()
        .find(|r| r.path == top)
    {
        return Ok(found);
    }
    let p = PathBuf::from(&top);
    let git = LocalGit::new(&p);
    let remote_url = git.remote_url().await;
    let detected = remote_url.as_deref().and_then(detect);
    let provider = detected.as_ref().map(|(k, _)| *k);
    let account_id = resolve_account(s, user, req.git_account_id.as_ref(), provider).await?;
    let name = req
        .name
        .clone()
        .or_else(|| p.file_name().map(|f| f.to_string_lossy().into_owned()))
        .ok_or_else(|| Error::Invalid("cannot derive repo name from path".into()))?;
    s.store()
        .create_repo(NewRepo {
            workspace_id: ws_id.clone(),
            name,
            path: p.to_string_lossy().into_owned(),
            remote_url,
            provider,
            git_account_id: account_id,
        })
        .await
}

async fn clone_into_workspace<S: GitCtx>(
    s: &S,
    user: &AuthUser,
    ws_id: &Id,
    url: &str,
    req: &AddRepoReq,
) -> Result<Repo> {
    let ws = s.workspaces().get(ws_id).await?;
    let name = req
        .name
        .clone()
        .or_else(|| derive_repo_name(url))
        .ok_or_else(|| Error::Invalid("cannot derive repo name from clone url".into()))?;
    // Clone INTO a user-chosen parent dir when provided (`~` expanded), else the
    // workspace root. The repo lands at `<base>/<name>`.
    let base = match req.clone_dir.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(dir) => expand_home(dir),
        None => ws.root_path.clone(),
    };
    let dest = FsPath::new(&base).join(&name);
    // The chosen parent must exist for `git clone <url> <dest>` to succeed.
    let _ = tokio::fs::create_dir_all(&base).await;
    if tokio::fs::metadata(&dest).await.is_ok() {
        return Err(Error::Conflict(format!(
            "destination already exists: {}",
            dest.display()
        )));
    }

    let detected = detect(url);
    let provider = detected.as_ref().map(|(k, _)| *k);
    let account_id = resolve_account(s, user, req.git_account_id.as_ref(), provider).await?;

    // Row first — the UI sees the repo immediately; Notice events track progress.
    let repo = s
        .store()
        .create_repo(NewRepo {
            workspace_id: ws_id.clone(),
            name: name.clone(),
            path: dest.to_string_lossy().into_owned(),
            remote_url: Some(url.to_string()),
            provider,
            git_account_id: account_id.clone(),
        })
        .await?;

    let token = match &account_id {
        Some(aid) => {
            let account = s.store().get_account(aid).await?;
            s.secrets().get(&account.token_ref)?
        }
        None => None,
    };

    let task_state = s.clone();
    let task_url = url.to_string();
    let task_name = name.clone();
    tokio::spawn(async move {
        // Display URL only — strip any user:pass@ a caller may have embedded so
        // it isn't echoed into the notice/log (the real URL still drives clone).
        let display_url = crate::local::strip_url_userinfo(&task_url);
        notice(
            &task_state,
            "info",
            "Clone started",
            &format!("Cloning {task_name} from {display_url}"),
        );
        let result = crate::local::clone_repo(&task_url, &dest, token.as_deref(), |line| {
            tracing::debug!(repo = %task_name, "clone: {}", crate::local::strip_url_userinfo(&line));
        })
        .await;
        match result {
            Ok(()) => notice(
                &task_state,
                "info",
                "Clone finished",
                &format!("{task_name} is ready"),
            ),
            Err(e) => notice(
                &task_state,
                "error",
                "Clone failed",
                &format!("{task_name}: {e}"),
            ),
        }
    });

    Ok(repo)
}

/// Validate an explicit account id, or auto-match the caller's first account
/// for the detected provider.
async fn resolve_account<S: GitCtx>(
    s: &S,
    user: &AuthUser,
    explicit: Option<&Id>,
    provider: Option<GitProviderKind>,
) -> Result<Option<Id>> {
    if let Some(id) = explicit {
        let account = s.store().get_account(id).await?;
        // S4: never let a caller bind a repo to a credential they don't own —
        // otherwise any workspace member could later push through it.
        authorize_owner(&account, &user.0)?;
        return Ok(Some(account.id));
    }
    let Some(kind) = provider else {
        return Ok(None);
    };
    let accounts = s.store().list_accounts(&user.0.id).await?;
    Ok(accounts
        .into_iter()
        .find(|a| a.provider == kind)
        .map(|a| a.id))
}

fn derive_repo_name(url: &str) -> Option<String> {
    let tail = url
        .trim_end_matches('/')
        .rsplit(['/', ':'])
        .next()?
        .trim_end_matches(".git");
    if tail.is_empty() {
        None
    } else {
        Some(tail.to_string())
    }
}

/// Expand a leading `~` (or `~/…`) to the daemon user's `$HOME`. Other paths are
/// returned unchanged. Used to resolve a user-chosen clone destination.
fn expand_home(path: &str) -> String {
    if path == "~" || path.starts_with("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            let home = home.to_string_lossy();
            return if path == "~" {
                home.into_owned()
            } else {
                format!("{home}{}", &path[1..])
            };
        }
    }
    path.to_string()
}

async fn delete_repo<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<StatusCode> {
    let repo = s.store().get_repo(&id).await?;
    s.roles()
        .check(&user.0, &repo.workspace_id, WorkspaceRole::Editor)
        .await?;
    // Unregister only — never touch the files on disk.
    s.store().delete_repo(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Local ops (#37–47)
// ---------------------------------------------------------------------------

async fn repo_status<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Json<RepoStatusResp>> {
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Viewer).await?;
    Ok(Json(git.status().await?))
}

async fn repo_branches<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Json<Vec<BranchInfo>>> {
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Viewer).await?;
    Ok(Json(git.branches().await?))
}

async fn repo_refs<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Json<RefsResp>> {
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Viewer).await?;
    Ok(Json(git.refs().await?))
}

async fn repo_fetch<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Json<RepoStatusResp>> {
    let (repo, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    let token = optional_token(&s, &user, &repo).await?;
    git.fetch(token).await?;
    Ok(Json(git.status().await?))
}

#[derive(Deserialize)]
struct LogQuery {
    limit: Option<u32>,
    skip: Option<u32>,
    all: Option<bool>,
}

async fn repo_log<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Query(q): Query<LogQuery>,
) -> ApiResult<Json<Vec<CommitInfo>>> {
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Viewer).await?;
    let limit = q.limit.unwrap_or(50).min(500);
    Ok(Json(
        git.log(limit, q.skip.unwrap_or(0), q.all.unwrap_or(false))
            .await?,
    ))
}

#[derive(Deserialize)]
struct DiffQuery {
    target: Option<String>,
    /// Scope the diff to a single file (`-- <path>`). The Changes view passes
    /// this when a file is selected so it computes only that file's diff instead
    /// of the whole working tree.
    path: Option<String>,
}

async fn repo_diff<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Query(q): Query<DiffQuery>,
) -> ApiResult<Json<DiffResp>> {
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Viewer).await?;
    let target = match q.target.as_deref() {
        None => DiffTarget::Worktree,
        Some(t) => DiffTarget::parse(t)?,
    };
    Ok(Json(git.diff(target, q.path.as_deref()).await?))
}

async fn repo_stage<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<StagePathsReq>,
) -> ApiResult<Json<RepoStatusResp>> {
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    git.stage(&req.paths).await?;
    Ok(Json(git.status().await?))
}

async fn repo_unstage<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<StagePathsReq>,
) -> ApiResult<Json<RepoStatusResp>> {
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    git.unstage(&req.paths).await?;
    Ok(Json(git.status().await?))
}

async fn repo_discard<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<StagePathsReq>,
) -> ApiResult<Json<RepoStatusResp>> {
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    git.discard(&req.paths).await?;
    Ok(Json(git.status().await?))
}

async fn repo_commit<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<CommitReq>,
) -> ApiResult<Json<serde_json::Value>> {
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    let sha = git.commit(&req.message, req.amend).await?;
    Ok(Json(serde_json::json!({ "sha": sha })))
}

async fn repo_push<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Json<RepoStatusResp>> {
    let (repo, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    let token = optional_token(&s, &user, &repo).await?;
    git.push(token).await?;
    // Return the FRESH status so the UI's ahead/behind chip updates after push.
    Ok(Json(git.status().await?))
}

async fn repo_pull<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Json<RepoStatusResp>> {
    let (repo, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    let token = optional_token(&s, &user, &repo).await?;
    // Pull, then return the FRESH status so the UI's branch chip clears its
    // ahead/behind. (Previously returned `{output}`, which the UI consumed as a
    // RepoStatusResp — so the behind count never updated and pull looked like a
    // no-op even when it fast-forwarded.)
    git.pull(token).await?;
    Ok(Json(git.status().await?))
}

/// `POST /repos/{id}/api-collections/pull` — pull the repo, then read every
/// `collections/*.json` (Postman collection files) and return their contents
/// for the API client to import.
async fn repo_collections_pull<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Json<serde_json::Value>> {
    let (repo, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    let token = optional_token(&s, &user, &repo).await?;
    let _ = git.pull(token).await; // best-effort; report read result regardless
    let dir = std::path::Path::new(&repo.path).join("collections");
    let mut files: Vec<serde_json::Value> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.extension().and_then(|x| x.to_str()) == Some("json") {
                if let Ok(content) = std::fs::read_to_string(&p) {
                    files.push(serde_json::json!({
                        "name": p.file_name().and_then(|n| n.to_str()).unwrap_or(""),
                        "content": content,
                    }));
                }
            }
        }
    }
    Ok(Json(serde_json::json!({ "files": files })))
}

#[derive(serde::Deserialize)]
struct CollectionFile {
    name: String,
    content: String,
}

#[derive(serde::Deserialize)]
struct PushCollectionsReq {
    files: Vec<CollectionFile>,
    message: String,
    #[serde(default)]
    branch: Option<String>,
}

/// `POST /repos/{id}/api-collections/push` — write the given Postman collection
/// files into `collections/`, stage + commit, and push (optionally onto a new
/// branch so the user can open a PR).
async fn repo_collections_push<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<PushCollectionsReq>,
) -> ApiResult<Json<serde_json::Value>> {
    let (repo, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    if let Some(branch) = req.branch.as_deref().filter(|b| !b.is_empty()) {
        git.checkout(branch, true).await?;
    }
    let base = std::path::Path::new(&repo.path);
    std::fs::create_dir_all(base.join("collections"))
        .map_err(|e| otto_core::Error::Upstream(format!("create collections dir: {e}")))?;
    let mut staged: Vec<String> = Vec::new();
    for f in &req.files {
        let safe = f.name.replace(['/', '\\'], "_");
        let safe = if safe.ends_with(".json") { safe } else { format!("{safe}.json") };
        let rel = format!("collections/{safe}");
        std::fs::write(base.join(&rel), &f.content)
            .map_err(|e| otto_core::Error::Upstream(format!("write {rel}: {e}")))?;
        staged.push(rel);
    }
    git.stage(&staged).await?;
    let sha = git.commit(&req.message, false).await?;
    let token = optional_token(&s, &user, &repo).await?;
    let push_out = git.push(token).await?;
    Ok(Json(serde_json::json!({ "commit": sha, "push": push_out, "files": staged.len() })))
}

async fn repo_checkout<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<CheckoutReq>,
) -> ApiResult<Json<RepoStatusResp>> {
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    git.checkout(&req.branch, req.create).await?;
    Ok(Json(git.status().await?))
}

// ---------------------------------------------------------------------------
// Graph context-menu ops (commit / branch / tag). All Editor; each returns the
// FRESH RepoStatusResp (via git.status()) so the UI refreshes ahead/behind +
// graph after the op. Remote ops resolve `optional_token` first (None for ssh).
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct CherryPickReq {
    sha: String,
}

async fn repo_cherry_pick<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<CherryPickReq>,
) -> ApiResult<Json<RepoStatusResp>> {
    if req.sha.trim().is_empty() {
        return Err(Error::Invalid("sha must not be empty".into()).into());
    }
    let lock = repo_lock(&id);
    let _g = lock.lock().await;
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    git.cherry_pick(req.sha.trim()).await?;
    Ok(Json(git.status().await?))
}

#[derive(Deserialize)]
struct RevertReq {
    sha: String,
}

async fn repo_revert<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<RevertReq>,
) -> ApiResult<Json<RepoStatusResp>> {
    if req.sha.trim().is_empty() {
        return Err(Error::Invalid("sha must not be empty".into()).into());
    }
    let lock = repo_lock(&id);
    let _g = lock.lock().await;
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    git.revert(req.sha.trim()).await?;
    Ok(Json(git.status().await?))
}

#[derive(Deserialize)]
struct CreateBranchReq {
    name: String,
    #[serde(default)]
    start_point: Option<String>,
    #[serde(default)]
    checkout: Option<bool>,
}

async fn repo_branch_create<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<CreateBranchReq>,
) -> ApiResult<Json<RepoStatusResp>> {
    if req.name.trim().is_empty() {
        return Err(Error::Invalid("branch name must not be empty".into()).into());
    }
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    git.create_branch(
        req.name.trim(),
        req.start_point.as_deref().map(str::trim).filter(|s| !s.is_empty()),
        req.checkout.unwrap_or(false),
    )
    .await?;
    Ok(Json(git.status().await?))
}

#[derive(Deserialize)]
struct RenameBranchReq {
    from: String,
    to: String,
}

async fn repo_branch_rename<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<RenameBranchReq>,
) -> ApiResult<Json<RepoStatusResp>> {
    if req.from.trim().is_empty() || req.to.trim().is_empty() {
        return Err(Error::Invalid("from/to must not be empty".into()).into());
    }
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    git.rename_branch(req.from.trim(), req.to.trim()).await?;
    Ok(Json(git.status().await?))
}

#[derive(Deserialize)]
struct DeleteBranchReq {
    name: String,
    /// Also delete `origin/<name>` (and acquire the account token for the push).
    #[serde(default)]
    remote: Option<bool>,
    /// Delete the LOCAL branch. Defaults to true; the remote-ref-row "Delete
    /// origin/<name>" sends `local:false` so only the origin copy is removed.
    #[serde(default)]
    local: Option<bool>,
    /// `-D` (drop unmerged) instead of `-d`. The UI confirms first.
    #[serde(default)]
    force: Option<bool>,
}

async fn repo_branch_delete<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<DeleteBranchReq>,
) -> ApiResult<Json<RepoStatusResp>> {
    let name = req.name.trim();
    if name.is_empty() {
        return Err(Error::Invalid("branch name must not be empty".into()).into());
    }
    let want_local = req.local.unwrap_or(true);
    let want_remote = req.remote.unwrap_or(false);
    if !want_local && !want_remote {
        return Err(Error::Invalid("nothing to delete (set local and/or remote)".into()).into());
    }
    let (repo, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    if want_local {
        // Never delete the checked-out branch — git refuses, and a half-done
        // local+remote delete is worse. Reject up front with a clear message.
        if git.current_branch().await? == name {
            return Err(Error::Invalid(format!(
                "cannot delete the checked-out branch '{name}'; switch branches first"
            ))
            .into());
        }
        git.delete_branch(name, req.force.unwrap_or(true)).await?;
    }
    if want_remote {
        let token = optional_token(&s, &user, &repo).await?;
        git.delete_remote_branch(name, token).await?;
    }
    Ok(Json(git.status().await?))
}

#[derive(Deserialize)]
struct CreateTagReq {
    name: String,
    sha: String,
    /// Annotated tag message; lightweight tag when absent/empty.
    #[serde(default)]
    message: Option<String>,
    /// Push the new tag to origin after creating it.
    #[serde(default)]
    push: Option<bool>,
}

async fn repo_tag_create<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<CreateTagReq>,
) -> ApiResult<Json<RepoStatusResp>> {
    let name = req.name.trim();
    if name.is_empty() {
        return Err(Error::Invalid("tag name must not be empty".into()).into());
    }
    if req.sha.trim().is_empty() {
        return Err(Error::Invalid("tag sha must not be empty".into()).into());
    }
    let (repo, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    git.create_tag(
        name,
        req.sha.trim(),
        req.message.as_deref().map(str::trim).filter(|m| !m.is_empty()),
    )
    .await?;
    if req.push.unwrap_or(false) {
        let token = optional_token(&s, &user, &repo).await?;
        git.push_tag(name, token).await?;
    }
    Ok(Json(git.status().await?))
}

#[derive(Deserialize)]
struct PushTagReq {
    name: String,
}

async fn repo_tag_push<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<PushTagReq>,
) -> ApiResult<Json<RepoStatusResp>> {
    let name = req.name.trim();
    if name.is_empty() {
        return Err(Error::Invalid("tag name must not be empty".into()).into());
    }
    let (repo, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    let token = optional_token(&s, &user, &repo).await?;
    git.push_tag(name, token).await?;
    Ok(Json(git.status().await?))
}

#[derive(Deserialize)]
struct DeleteTagReq {
    name: String,
    /// Also delete the tag on origin (acquires the account token).
    #[serde(default)]
    remote: Option<bool>,
}

async fn repo_tag_delete<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<DeleteTagReq>,
) -> ApiResult<Json<RepoStatusResp>> {
    let name = req.name.trim();
    if name.is_empty() {
        return Err(Error::Invalid("tag name must not be empty".into()).into());
    }
    let (repo, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    git.delete_tag(name).await?;
    if req.remote.unwrap_or(false) {
        let token = optional_token(&s, &user, &repo).await?;
        git.delete_remote_tag(name, token).await?;
    }
    Ok(Json(git.status().await?))
}

/// List stashes (read-only). Viewer role — no working-tree mutation.
async fn repo_stashes<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Json<Vec<StashInfo>>> {
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Viewer).await?;
    Ok(Json(git.stash_list().await?))
}

#[derive(Deserialize)]
struct StashReq {
    op: String,
    /// Stash commit SHA for `apply`/`drop` (SHA-anchored so a renumbered stack
    /// can't hit the wrong entry). Required for those ops; ignored by
    /// `save`/`pop`, which always operate on the top of the stack.
    #[serde(default)]
    sha: Option<String>,
}

async fn repo_stash<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<StashReq>,
) -> ApiResult<Json<RepoStatusResp>> {
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    match req.op.as_str() {
        "save" => {
            git.stash_save().await?;
        }
        "pop" => {
            git.stash_pop().await?;
        }
        "apply" | "drop" => {
            let sha = req
                .sha
                .as_deref()
                .ok_or_else(|| Error::Invalid(format!("stash {} requires a sha", req.op)))?;
            if req.op == "apply" {
                git.stash_apply(sha).await?;
            } else {
                git.stash_drop(sha).await?;
            }
        }
        other => return Err(Error::Invalid(format!("bad stash op: {other}")).into()),
    }
    Ok(Json(git.status().await?))
}

// ---------------------------------------------------------------------------
// Local merge + conflict resolution (#4)
// ---------------------------------------------------------------------------

async fn repo_merge<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<MergeBranchReq>,
) -> ApiResult<Json<MergeResult>> {
    let lock = repo_lock(&id);
    let _g = lock.lock().await;
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    Ok(Json(
        git.merge_branch(&req.source, &req.target, req.strategy, req.auto_stash)
            .await?,
    ))
}

/// `POST /repos/{id}/merge/preview` — dry-run merge conflict check (no mutation).
async fn repo_merge_preview<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<MergePreviewReq>,
) -> ApiResult<Json<MergePreview>> {
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Viewer).await?;
    Ok(Json(git.merge_preview(&req.source, &req.target).await?))
}

async fn repo_merge_status<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Json<MergeConflictStatus>> {
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Viewer).await?;
    Ok(Json(git.merge_status().await?))
}

async fn repo_merge_abort<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Json<RepoStatusResp>> {
    let lock = repo_lock(&id);
    let _g = lock.lock().await;
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    Ok(Json(git.merge_abort().await?))
}

async fn repo_merge_commit<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<MergeCommitReq>,
) -> ApiResult<Json<MergeResult>> {
    let lock = repo_lock(&id);
    let _g = lock.lock().await;
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    Ok(Json(git.merge_commit(req.message).await?))
}

#[derive(Deserialize)]
struct ConflictQuery {
    path: String,
}

async fn repo_conflict<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Query(q): Query<ConflictQuery>,
) -> ApiResult<Json<ConflictFile>> {
    if q.path.trim().is_empty() {
        return Err(Error::Invalid("path must not be empty".into()).into());
    }
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Viewer).await?;
    Ok(Json(git.conflict_file(&q.path).await?))
}

async fn repo_conflict_resolve<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<ResolveConflictReq>,
) -> ApiResult<Json<RepoStatusResp>> {
    if req.path.trim().is_empty() {
        return Err(Error::Invalid("path must not be empty".into()).into());
    }
    let lock = repo_lock(&id);
    let _g = lock.lock().await;
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    git.write_resolution(&req.path, &req.content).await?;
    Ok(Json(git.status().await?))
}

// ---------------------------------------------------------------------------
// PRs (#48–56)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct PrListQuery {
    state: Option<String>,
}

async fn pr_list<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Query(q): Query<PrListQuery>,
) -> ApiResult<Json<Vec<PrSummary>>> {
    let (repo, _) = repo_ctx(&s, &user, &id, WorkspaceRole::Viewer).await?;
    let state = match q.state.as_deref() {
        None | Some("open") => PrState::Open,
        Some("merged") => PrState::Merged,
        Some("declined") => PrState::Declined,
        Some("all") => PrState::All,
        Some(other) => return Err(Error::Invalid(format!("bad pr state: {other}")).into()),
    };
    let (provider, remote) = provider_ctx(&s, &user, &repo).await?;
    Ok(Json(provider.list_prs(&remote, state).await?))
}

async fn pr_create<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<CreatePrReq>,
) -> ApiResult<Json<PrSummary>> {
    let (repo, _) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    // Proof gate: refuse to open a PR over an unproven linked proof pack.
    s.check_pr_allowed(&repo.workspace_id, &req).await?;
    let (provider, remote) = provider_ctx(&s, &user, &repo).await?;
    let summary = provider.create_pr(&remote, &req).await?;
    // Proof Packs v2: link the pack to the new PR, capture its CI status as a `ci`
    // evidence artifact, and run the PR-description consistency check (best-effort;
    // never fails the PR creation).
    let ci = provider.ci_status(&remote, summary.number).await;
    s.after_pr_created(&repo, summary.number, &req, &ci).await;
    Ok(Json(summary))
}

/// Open a PR for `repo` on behalf of `user` **in-process** — the same path as the
/// `POST /repos/{id}/pr` route (proof gate via `check_pr_allowed`, provider
/// resolution, create, then the `after_pr_created` Proof-Packs hook), without
/// going through HTTP. Exposed so the workflow engine's `git_pr` node can open a
/// PR once a review step has passed.
pub async fn create_pr_for_repo<S: GitCtx>(
    s: &S,
    user: &AuthUser,
    repo: &Repo,
    req: &CreatePrReq,
) -> Result<PrSummary> {
    s.check_pr_allowed(&repo.workspace_id, req).await?;
    let (provider, remote) = provider_ctx(s, user, repo).await?;
    let summary = provider.create_pr(&remote, req).await?;
    let ci = provider.ci_status(&remote, summary.number).await;
    s.after_pr_created(repo, summary.number, req, &ci).await;
    Ok(summary)
}

async fn pr_detail<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path((id, number)): Path<(Id, u64)>,
) -> ApiResult<Json<PrDetail>> {
    let (repo, _) = repo_ctx(&s, &user, &id, WorkspaceRole::Viewer).await?;
    let (provider, remote) = provider_ctx(&s, &user, &repo).await?;
    Ok(Json(provider.get_pr(&remote, number).await?))
}

async fn pr_update<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path((id, number)): Path<(Id, u64)>,
    Json(req): Json<UpdatePrReq>,
) -> ApiResult<StatusCode> {
    let (repo, _) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    let (provider, remote) = provider_ctx(&s, &user, &repo).await?;
    provider.update_pr(&remote, number, &req).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn pr_diff<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path((id, number)): Path<(Id, u64)>,
) -> ApiResult<Json<DiffResp>> {
    let (repo, _) = repo_ctx(&s, &user, &id, WorkspaceRole::Viewer).await?;
    let (provider, remote) = provider_ctx(&s, &user, &repo).await?;
    Ok(Json(provider.get_pr_diff(&remote, number).await?))
}

async fn pr_comment<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path((id, number)): Path<(Id, u64)>,
    Json(req): Json<NewPrCommentReq>,
) -> ApiResult<Json<PrComment>> {
    if req.body.trim().is_empty() {
        return Err(Error::Invalid("comment body must not be empty".into()).into());
    }
    let (repo, _) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    let (provider, remote) = provider_ctx(&s, &user, &repo).await?;
    Ok(Json(provider.comment(&remote, number, &req).await?))
}

async fn pr_approve<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path((id, number)): Path<(Id, u64)>,
) -> ApiResult<StatusCode> {
    let (repo, _) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    let (provider, remote) = provider_ctx(&s, &user, &repo).await?;
    provider.approve(&remote, number).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn pr_merge<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path((id, number)): Path<(Id, u64)>,
    Json(req): Json<MergePrReq>,
) -> ApiResult<StatusCode> {
    let (repo, _) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    let (provider, remote) = provider_ctx(&s, &user, &repo).await?;
    provider.merge(&remote, number, req.strategy).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn pr_decline<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path((id, number)): Path<(Id, u64)>,
) -> ApiResult<StatusCode> {
    let (repo, _) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    let (provider, remote) = provider_ctx(&s, &user, &repo).await?;
    provider.decline(&remote, number).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn pr_request_changes<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path((id, number)): Path<(Id, u64)>,
    Json(req): Json<RequestChangesReq>,
) -> ApiResult<StatusCode> {
    let (repo, _) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    let (provider, remote) = provider_ctx(&s, &user, &repo).await?;
    provider
        .request_changes(&remote, number, req.body.as_deref())
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn pr_commits<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path((id, number)): Path<(Id, u64)>,
) -> ApiResult<Json<Vec<PrCommit>>> {
    let (repo, _) = repo_ctx(&s, &user, &id, WorkspaceRole::Viewer).await?;
    let (provider, remote) = provider_ctx(&s, &user, &repo).await?;
    Ok(Json(provider.list_pr_commits(&remote, number).await?))
}

// ---------------------------------------------------------------------------
// Tests — S4 credential ownership on the git use-paths
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chrono::Utc;
    use otto_core::auth::{BoxFuture, RoleChecker};
    use otto_core::domain::{GitProviderKind, User};
    use otto_core::secrets::SecretStore;
    use otto_state::{GitStore, NewGitAccount, NewRepo, WorkspacesRepo};
    use sqlx::SqlitePool;

    use super::*;

    /// In-memory secret store that returns a fixed token for any ref.
    struct FixedSecret;
    impl SecretStore for FixedSecret {
        fn put(&self, _k: &str, _v: &str) -> Result<()> {
            Ok(())
        }
        fn get(&self, _k: &str) -> Result<Option<String>> {
            Ok(Some("token".into()))
        }
        fn delete(&self, _k: &str) -> Result<()> {
            Ok(())
        }
    }

    /// RoleChecker that always authorizes — proves the S4 guard is what blocks a
    /// non-owner, independent of the workspace role-check.
    struct AllowAll;
    impl RoleChecker for AllowAll {
        fn check<'a>(
            &'a self,
            _u: &'a User,
            _w: &'a Id,
            _m: WorkspaceRole,
        ) -> BoxFuture<'a, Result<()>> {
            Box::pin(async { Ok(()) })
        }
    }

    #[derive(Clone)]
    struct TestCtx {
        store: GitStore,
        workspaces: WorkspacesRepo,
        secrets: Arc<dyn SecretStore>,
        roles: Arc<dyn RoleChecker>,
        events: tokio::sync::broadcast::Sender<otto_core::event::Event>,
    }

    impl GitCtx for TestCtx {
        fn store(&self) -> &GitStore {
            &self.store
        }
        fn workspaces(&self) -> &WorkspacesRepo {
            &self.workspaces
        }
        fn secrets(&self) -> &Arc<dyn SecretStore> {
            &self.secrets
        }
        fn roles(&self) -> &Arc<dyn RoleChecker> {
            &self.roles
        }
        fn events(&self) -> &tokio::sync::broadcast::Sender<otto_core::event::Event> {
            &self.events
        }
    }

    async fn mem_pool() -> SqlitePool {
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!("../otto-state/migrations")
            .run(&pool)
            .await
            .unwrap();
        pool
    }

    async fn seed_user(pool: &SqlitePool, username: &str) -> Id {
        let uid = new_id();
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, display_name, is_root, created_at)
             VALUES (?, ?, ?, ?, 0, ?)",
        )
        .bind(&uid)
        .bind(username)
        .bind("hash")
        .bind(username)
        .bind(&now)
        .execute(pool)
        .await
        .unwrap();
        uid
    }

    async fn seed_workspace(pool: &SqlitePool) -> Id {
        let wid = new_id();
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO workspaces (id, name, root_path, created_at) VALUES (?, ?, ?, ?)",
        )
        .bind(&wid)
        .bind("ws")
        .bind("/tmp")
        .bind(&now)
        .execute(pool)
        .await
        .unwrap();
        wid
    }

    fn auth(id: &Id, is_root: bool) -> AuthUser {
        AuthUser(User {
            id: id.clone(),
            username: id.clone(),
            display_name: id.clone(),
            is_root,
            disabled: false,
            created_at: Utc::now(),
        })
    }

    /// A repo bound to user A's git account may have its credential *used* only by
    /// A or root; a different workspace member is forbidden even though AllowAll
    /// passes the workspace role-check. An unbound repo yields `None` (no leak).
    #[tokio::test]
    async fn repo_credential_use_is_owner_or_root_only() {
        let pool = mem_pool().await;
        let owner = seed_user(&pool, "owner").await;
        let other = seed_user(&pool, "other").await;
        let root = seed_user(&pool, "root").await;
        let ws = seed_workspace(&pool).await;

        let store = GitStore::new(pool.clone());
        let account = store
            .create_account(NewGitAccount {
                user_id: owner.clone(),
                provider: GitProviderKind::Github,
                label: "gh".into(),
                username: "octocat".into(),
                token_ref: "gitacct-1".into(),
                api_base_url: None,
                namespace: None,
                token_expires_at: None,
            })
            .await
            .unwrap();

        let bound_repo = store
            .create_repo(NewRepo {
                workspace_id: ws.clone(),
                name: "bound".into(),
                path: "/tmp/bound".into(),
                remote_url: Some("https://github.com/o/bound.git".into()),
                provider: Some(GitProviderKind::Github),
                git_account_id: Some(account.id.clone()),
            })
            .await
            .unwrap();

        let ctx = TestCtx {
            store: store.clone(),
            workspaces: WorkspacesRepo::new(pool.clone()),
            secrets: Arc::new(FixedSecret),
            roles: Arc::new(AllowAll),
            events: tokio::sync::broadcast::channel(8).0,
        };

        // Owner ✅
        assert!(
            authorized_repo_account(&ctx, &auth(&owner, false), &bound_repo)
                .await
                .unwrap()
                .is_some()
        );
        // Root ✅
        assert!(
            authorized_repo_account(&ctx, &auth(&root, true), &bound_repo)
                .await
                .unwrap()
                .is_some()
        );
        // Non-owner ⛔ — Forbidden, even with AllowAll roles.
        let err = authorized_repo_account(&ctx, &auth(&other, false), &bound_repo)
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Forbidden(_)), "got {err:?}");

        // The token resolver enforces the same: owner gets a token, other is
        // forbidden (not silently `None`).
        assert!(optional_token(&ctx, &auth(&owner, false), &bound_repo)
            .await
            .unwrap()
            .is_some());
        assert!(matches!(
            optional_token(&ctx, &auth(&other, false), &bound_repo)
                .await
                .unwrap_err(),
            Error::Forbidden(_)
        ));

        // An unbound repo carries no credential → None for anyone (no leak path).
        let unbound = store
            .create_repo(NewRepo {
                workspace_id: ws.clone(),
                name: "unbound".into(),
                path: "/tmp/unbound".into(),
                remote_url: None,
                provider: None,
                git_account_id: None,
            })
            .await
            .unwrap();
        assert!(authorized_repo_account(&ctx, &auth(&other, false), &unbound)
            .await
            .unwrap()
            .is_none());
        assert!(optional_token(&ctx, &auth(&other, false), &unbound)
            .await
            .unwrap()
            .is_none());
    }

    /// `git init` a throwaway repo under the temp dir and return its path.
    async fn init_git_repo() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("otto-git-test-{}", new_id()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let ok = std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(&dir)
            .status()
            .expect("spawn git init")
            .success();
        assert!(ok, "git init failed");
        dir
    }

    fn reg_req(path: &str) -> AddRepoReq {
        AddRepoReq {
            path: Some(path.to_string()),
            clone_url: None,
            name: None,
            git_account_id: None,
            clone_dir: None,
        }
    }

    /// Re-registering the same local path (or any subdirectory of the repo) must
    /// return the EXISTING repo rather than minting a duplicate row — otherwise
    /// the Git page opens a second, identical tab for the same repository.
    #[tokio::test]
    async fn register_repo_is_idempotent_by_path() {
        let pool = mem_pool().await;
        let user = seed_user(&pool, "u").await;
        let ws = seed_workspace(&pool).await;
        let store = GitStore::new(pool.clone());
        let ctx = TestCtx {
            store: store.clone(),
            workspaces: WorkspacesRepo::new(pool.clone()),
            secrets: Arc::new(FixedSecret),
            roles: Arc::new(AllowAll),
            events: tokio::sync::broadcast::channel(8).0,
        };

        let dir = init_git_repo().await;
        let path = dir.to_string_lossy().into_owned();

        let first = register_repo(&ctx, &auth(&user, false), &ws, &path, &reg_req(&path))
            .await
            .unwrap();
        let second = register_repo(&ctx, &auth(&user, false), &ws, &path, &reg_req(&path))
            .await
            .unwrap();
        assert_eq!(
            first.id, second.id,
            "re-registering the same path must return the same repo"
        );

        // A subdirectory resolves to the same work-tree root → same repo, no dup.
        let sub = dir.join("nested");
        tokio::fs::create_dir_all(&sub).await.unwrap();
        let subpath = sub.to_string_lossy().into_owned();
        let third = register_repo(&ctx, &auth(&user, false), &ws, &subpath, &reg_req(&subpath))
            .await
            .unwrap();
        assert_eq!(
            first.id, third.id,
            "registering a subdirectory must dedup to the repo root"
        );

        // Exactly one row persisted.
        let repos = store.list_repos(&ws).await.unwrap();
        assert_eq!(repos.len(), 1, "expected one repo row, got {}", repos.len());

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
}
