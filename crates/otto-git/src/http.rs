//! Axum router for contract endpoints #31–#56: git accounts, repos, local
//! operations and pull requests. Mounted by otto-server under `/api/v1`.

use std::collections::HashMap;
use std::path::{Path as FsPath, PathBuf};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::{Extension, Json, Router};
use otto_core::api::{
    AddRepoReq, BranchInfo, CheckoutReq, CleanupBaseResp, Collaborator, CommitReq, ConflictFile,
    CreateGitAccountReq, CreatePrReq, DiffResp, GitAccountTestResp, MergeBranchReq, MergeCommitReq,
    MergeConflictStatus, MergePrReq, MergePreview, MergePreviewReq, MergeResult, NewPrCommentReq,
    PrComment, PrCommit, PrDetail, PrState, PrSummary, Problem, RefsResp, RepoStatusResp,
    RequestChangesReq, ResolveConflictReq, ResolvePrThreadReq, SetCleanupBaseReq, StagePathsReq,
    StashInfo, SubmoduleInfo, TestGitAccountReq, UpdateGitAccountReq, UpdatePrReq, UpdateRepoReq,
    WorktreeInfo,
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

    /// The per-repo `cleanup_base_branch` override used by the "safe to delete
    /// (merged)" branch indicators. The default returns `None` (follow the
    /// detected default branch); `otto-server` overrides it to read the stored
    /// setting.
    async fn cleanup_base_branch(&self, _repo_id: &Id) -> Option<String> {
        None
    }

    /// Persist (or clear, on `None`/empty) the per-repo `cleanup_base_branch`
    /// override. The default is a no-op; `otto-server` overrides it.
    async fn set_cleanup_base_branch(&self, _repo_id: &Id, _base: Option<String>) -> Result<()> {
        Ok(())
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
            patch(update_account::<S>).delete(delete_account::<S>),
        )
        .route("/git/accounts/{id}/remote-repos", get(remote_repos::<S>))
        // connection test: stored-token (id) + draft-form variants
        .route("/git/accounts/{id}/test", post(test_account::<S>))
        .route("/git/accounts/test", post(test_account_draft::<S>))
        // global repo list (workspace-independent Git page)
        .route("/git/repos", get(list_all_repos::<S>))
        // repos (#34–36)
        .route(
            "/workspaces/{id}/repos",
            get(list_repos::<S>).post(add_repo::<S>),
        )
        .route("/workspaces/{id}/repos/detect", post(detect_repo::<S>))
        .route(
            "/repos/{id}",
            patch(update_repo::<S>).delete(delete_repo::<S>),
        )
        // local ops (#37–47)
        .route("/repos/{id}/status", get(repo_status::<S>))
        .route("/repos/{id}/branches", get(repo_branches::<S>))
        .route("/repos/{id}/refs", get(repo_refs::<S>))
        .route(
            "/repos/{id}/cleanup-base",
            get(repo_cleanup_base::<S>).put(repo_set_cleanup_base::<S>),
        )
        .route("/repos/{id}/log", get(repo_log::<S>))
        .route("/repos/{id}/stashes", get(repo_stashes::<S>))
        .route("/repos/{id}/worktrees", get(repo_worktrees::<S>))
        .route(
            "/repos/{id}/worktrees/remove",
            post(repo_worktree_remove::<S>),
        )
        .route(
            "/repos/{id}/worktrees/prune",
            post(repo_worktree_prune::<S>),
        )
        .route("/repos/{id}/submodules", get(repo_submodules::<S>))
        .route(
            "/repos/{id}/submodules/update",
            post(repo_submodule_update::<S>),
        )
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
        .route(
            "/repos/{id}/api-collections/pull",
            post(repo_collections_pull::<S>),
        )
        .route(
            "/repos/{id}/api-collections/push",
            post(repo_collections_push::<S>),
        )
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
        .route("/repos/{id}/collaborators", get(repo_collaborators::<S>))
        .route("/repos/{id}/prs", get(pr_list::<S>).post(pr_create::<S>))
        .route(
            "/repos/{id}/prs/{number}",
            get(pr_detail::<S>).patch(pr_update::<S>),
        )
        .route("/repos/{id}/prs/{number}/diff", get(pr_diff::<S>))
        .route("/repos/{id}/prs/{number}/comments", post(pr_comment::<S>))
        .route(
            "/repos/{id}/prs/{number}/comments/{cid}/resolve",
            post(pr_resolve_thread::<S>),
        )
        .route("/repos/{id}/prs/{number}/approve", post(pr_approve::<S>))
        .route("/repos/{id}/prs/{number}/merge", post(pr_merge::<S>))
        .route("/repos/{id}/prs/{number}/decline", post(pr_decline::<S>))
        .route(
            "/repos/{id}/prs/{number}/request-changes",
            post(pr_request_changes::<S>),
        )
        .route("/repos/{id}/prs/{number}/commits", get(pr_commits::<S>))
        .merge(crate::patch::router::<S>())
        .merge(crate::pr_checks::router::<S>())
        .merge(crate::history::router::<S>())
        .merge(crate::ops::router::<S>())
        .merge(crate::recovery::router::<S>())
        .layer(axum::middleware::from_fn(invalidate_status_after_write))
}

/// Every non-GET under `/repos/{id}/…` (stage, commit, checkout, stash, PR
/// merge, …) invalidates that repo's status memo once it has run, so the
/// status read right after a mutation never comes from before it — without
/// waiting for the watcher's event.
async fn invalidate_status_after_write(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let write = !matches!(
        *req.method(),
        axum::http::Method::GET | axum::http::Method::HEAD | axum::http::Method::OPTIONS
    );
    let id = write.then(|| repo_id_of(req.uri().path())).flatten();
    let resp = next.run(req).await;
    if let Some(id) = id {
        crate::status_cache::bump(&id);
    }
    resp
}

/// The `{id}` of a `…/repos/{id}/…` request path.
fn repo_id_of(path: &str) -> Option<String> {
    let mut segs = path.split('/');
    segs.by_ref().find(|s| *s == "repos")?;
    segs.next().filter(|s| !s.is_empty()).map(str::to_string)
}

// ---------------------------------------------------------------------------
// Error → response
// ---------------------------------------------------------------------------

/// Local error wrapper: `otto_core::Error` → Problem JSON with the right status.
#[derive(Debug)]
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

pub(crate) type ApiResult<T> = std::result::Result<T, ApiError>;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Populate `Repo::forge` live from the remote URL (otto-state leaves it None
/// — it has no detection logic). "unrecognized" drives the UI's honest
/// unsupported-forge empty state; None = repo has no remote at all.
fn fill_forge(repo: &mut Repo) {
    repo.forge = repo
        .remote_url
        .as_deref()
        .map(|u| crate::providers::detect::forge(u).to_string());
}

/// Load a repo, check the caller's workspace role, return a LocalGit handle.
pub(crate) async fn repo_ctx<S: GitCtx>(
    s: &S,
    user: &AuthUser,
    repo_id: &Id,
    min: WorkspaceRole,
) -> Result<(Repo, LocalGit)> {
    let repo = s.store().get_repo(repo_id).await?;
    s.roles().check(&user.0, &repo.workspace_id, min).await?;
    // A git route call is a person's deliberate act: their hooks run, an
    // in-work-tree `core.hooksPath` (husky) included. (An agent's own token
    // calling these routes is the open outward-route decision, S11-05.)
    let git = LocalGit::new(&repo.path).person_initiated();
    Ok((repo, git))
}

/// Read one secret OFF the runtime. `SecretStore::get` is a SYNCHRONOUS
/// Keychain FFI call (an XPC round-trip to `securityd`, plus whatever an EDR
/// agent adds); on a loaded box it parks a tokio worker for tens of
/// milliseconds, and it sits on request paths as hot as the Git page's 10 s
/// auto-fetch. The trait stays sync — we wrap at the call sites.
async fn secret(store: &Arc<dyn SecretStore>, key: &str) -> Result<Option<String>> {
    let store = store.clone();
    let key = key.to_string();
    tokio::task::spawn_blocking(move || store.get(&key))
        .await
        .map_err(|e| Error::Internal(format!("keychain read: {e}")))?
}

/// Resolve the push/pull token for a repo's bound account (None when no
/// account is bound — ssh remotes work through the user's agent).
async fn account_token<S: GitCtx>(s: &S, account: &GitAccount) -> Result<String> {
    secret(s.secrets(), &account.token_ref)
        .await?
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
pub(crate) async fn optional_token<S: GitCtx>(
    s: &S,
    user: &AuthUser,
    repo: &Repo,
) -> Result<Option<String>> {
    match authorized_repo_account(s, user, repo).await? {
        Some(account) => secret(s.secrets(), &account.token_ref).await,
        None => Ok(None),
    }
}

/// Resolve provider client + remote ref for PR routes (400 when not bound).
/// Enforces the S4 ownership guard: the caller must own the repo's bound account.
pub(crate) async fn provider_ctx<S: GitCtx>(
    s: &S,
    user: &AuthUser,
    repo: &Repo,
) -> Result<(Arc<dyn GitProvider>, RemoteRef)> {
    let (provider, remote, _) = provider_ctx_with_account(s, user, repo).await?;
    Ok((provider, remote))
}

/// [`provider_ctx`] plus the id of the git account whose credential the
/// provider carries — for memo keys that must not serve one credential's
/// fetch to another.
async fn provider_ctx_with_account<S: GitCtx>(
    s: &S,
    user: &AuthUser,
    repo: &Repo,
) -> Result<(Arc<dyn GitProvider>, RemoteRef, Id)> {
    // The remote is read ONCE at registration, and `LocalGit::remote_url` folds
    // any failure into `None` — so a repo registered before its `origin` existed
    // (or during a bulk import where the `git` spawn failed) is permanently
    // recorded as remote-less and dead-ends every provider call. Re-read from
    // disk before giving up; nothing else ever revisits that snapshot.
    let refreshed;
    let repo = match repo.provider {
        Some(_) => repo,
        None => {
            refreshed = refresh_remote(s, repo.clone()).await?;
            &refreshed
        }
    };
    let kind = repo
        .provider
        .ok_or_else(|| Error::Invalid("repo has no git provider".into()))?;
    let account = match authorized_repo_account(s, user, repo).await? {
        Some(account) => account,
        None => adopt_account(s, user, repo, kind).await?,
    };
    if account.provider != kind {
        return Err(Error::Invalid(
            "git account provider does not match repo provider".into(),
        ));
    }
    let remote = repo
        .remote_url
        .as_deref()
        .ok_or_else(|| Error::Invalid("repo has no remote url".into()))?;
    // Userinfo stripped: a remote URL can embed `user:token@`, and this text
    // reaches the UI and the logs.
    let (_, remote_ref) = detect(remote).ok_or_else(|| {
        Error::Invalid(format!(
            "unsupported remote: {}",
            crate::local::strip_url_userinfo(remote)
        ))
    })?;
    // Before the token is loaded: a GitHub Enterprise remote must never send
    // it to api.github.com.
    crate::providers::check_remote_reachable(kind, remote, account.api_base_url.as_deref())?;
    let token = account_token(s, &account).await?;
    Ok((make_provider(&account, token), remote_ref, account.id))
}

/// Bind an unbound repo to the caller's account for `kind` and persist it.
///
/// Registration already picks "the caller's first account for the detected
/// provider" — but it only runs once, so a repo registered BEFORE its account
/// existed stays unbound forever and every provider call dies with "repo has no
/// git account". Applying the same rule lazily heals those repos; it is only
/// ever the caller's OWN credential, so it opens no path S4 didn't already
/// allow. Ambiguity (two accounts on one provider) is never guessed — the user
/// picks via `PATCH /repos/{id}`.
async fn adopt_account<S: GitCtx>(
    s: &S,
    user: &AuthUser,
    repo: &Repo,
    kind: GitProviderKind,
) -> Result<GitAccount> {
    let mut mine = s
        .store()
        .list_accounts(&user.0.id)
        .await?
        .into_iter()
        .filter(|a| a.provider == kind);
    let (Some(account), None) = (mine.next(), mine.next()) else {
        return Err(Error::Invalid(format!(
            "repo has no git account: link a {} account to this repo (Git → repo card → Account)",
            kind.as_str()
        )));
    };
    // Binding is a SHARED repo setting (#36b: Editor + owner) — once bound,
    // every push/fetch needs the binder's token. A Viewer merely opening the
    // PR tab must not decide that for the workspace: use the account for this
    // request only.
    if s.roles()
        .check(&user.0, &repo.workspace_id, WorkspaceRole::Editor)
        .await
        .is_err()
    {
        return Ok(account);
    }
    s.store()
        .set_repo_account(&repo.id, Some(&account.id))
        .await?;
    tracing::info!(
        repo = %repo.name,
        account = %account.label,
        "linked repo to the only {} account on file",
        kind.as_str()
    );
    Ok(account)
}

pub(crate) fn notice(s: &impl GitCtx, level: &str, title: &str, body: &str) {
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
pub(crate) fn repo_lock(id: &Id) -> Arc<tokio::sync::Mutex<()>> {
    let mut map = repo_locks().lock().expect("repo_locks poisoned");
    map.entry(id.to_string())
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone()
}

/// Per-repo locks for the NETWORK legs (push, tag push, fetch, pull), kept
/// apart from [`repo_lock`] so a push — up to the 180 s remote budget — no
/// longer queues stage/commit/discard behind it. A push writes only
/// `refs/remotes/*` (plus `branch.*` config for `--set-upstream`, under git's
/// own `config.lock`); it never touches the index or the worktree, which is
/// what `repo_lock` protects. Two remote operations still serialise (push vs
/// fetch both rewrite the tracking refs).
///
/// Lock order is ALWAYS `repo_lock` → `remote_lock` (pull holds both: it
/// writes the worktree and the tracking refs); nothing takes them the other
/// way round, so the pair cannot deadlock.
fn remote_locks() -> &'static StdMutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>> {
    static LOCKS: OnceLock<StdMutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> =
        OnceLock::new();
    LOCKS.get_or_init(|| StdMutex::new(HashMap::new()))
}

/// Return (creating if needed) the network-leg mutex of repo `id`.
pub(crate) fn remote_lock(id: &Id) -> Arc<tokio::sync::Mutex<()>> {
    let mut map = remote_locks().lock().expect("remote_locks poisoned");
    map.entry(id.to_string())
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone()
}

/// When each repo's last SUCCESSFUL fetch finished — the single-flight
/// marker [`fetch_coalesced`] compares a caller's arrival against.
fn fetch_done() -> &'static StdMutex<HashMap<String, std::time::Instant>> {
    static DONE: OnceLock<StdMutex<HashMap<String, std::time::Instant>>> = OnceLock::new();
    DONE.get_or_init(|| StdMutex::new(HashMap::new()))
}

/// Single-flight `git fetch` per repo. Callers serialise on the repo's
/// remote lock; one that arrived while another fetch of the same repo was
/// running reuses that fetch's refs instead of re-running it, so N windows
/// (or the auto-fetch sweep racing a manual click) cost ONE network round,
/// not N back-to-back ones (telemetry: `POST /repos/{id}/fetch` ~6/min at
/// p95 3 s). Only a success is shared — after a failure the next waiter runs
/// its own fetch (it may carry a different, valid token). Returns the held
/// lock guard and whether this caller actually ran the fetch.
pub(crate) async fn fetch_coalesced<'a, F, Fut>(
    id: &Id,
    lock: &'a tokio::sync::Mutex<()>,
    fetch: F,
) -> Result<(tokio::sync::MutexGuard<'a, ()>, bool)>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<()>>,
{
    let arrived = std::time::Instant::now();
    let guard = lock.lock().await;
    let shared = fetch_done()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(id.as_str())
        .is_some_and(|done| *done > arrived);
    if shared {
        return Ok((guard, false));
    }
    fetch().await?;
    fetch_done()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(id.to_string(), std::time::Instant::now());
    Ok((guard, true))
}

/// Forget a deleted repo's locks (both maps otherwise only ever grow). An
/// in-flight holder keeps its own `Arc`, so dropping the entry is safe.
fn forget_repo_locks(id: &Id) {
    repo_locks()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .remove(id.as_str());
    remote_locks()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .remove(id.as_str());
    fetch_done()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .remove(id.as_str());
}

/// The fresh status a mutating route answers with, read AFTER its lock is
/// released. `status()` is lock-free (`GIT_OPTIONAL_LOCKS=0`: it never takes
/// `index.lock`), so a stage queued behind this route starts while the walk
/// runs instead of waiting for it; the read may already include that next
/// write, which only makes it fresher.
pub(crate) async fn status_after_release(
    git: &LocalGit,
    guard: tokio::sync::MutexGuard<'_, ()>,
) -> ApiResult<Json<RepoStatusResp>> {
    drop(guard);
    Ok(Json(git.status().await?))
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
// Connection test (Test button on the git-account form)
// ---------------------------------------------------------------------------

/// Map a `verify_token` outcome to the wire response. Auth/provider failures
/// come back `ok: false` + error text under HTTP 200 so the form renders them
/// inline instead of tripping generic error toasts.
fn test_resp(r: Result<crate::providers::TokenCheck>) -> GitAccountTestResp {
    match r {
        Ok(t) => GitAccountTestResp {
            ok: true,
            login: Some(t.login),
            scopes: t.scopes,
            error: None,
        },
        Err(e) => GitAccountTestResp {
            ok: false,
            login: None,
            scopes: Vec::new(),
            error: Some(e.to_string()),
        },
    }
}

/// `POST /git/accounts/{id}/test` — exercise the **stored** token with the
/// provider's cheapest authenticated call. Owner-or-root only (S4: a member
/// must not probe another user's token). The token itself never leaves the
/// daemon.
async fn test_account<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Json<GitAccountTestResp>> {
    let account = s.store().get_account(&id).await?;
    authorize_owner(&account, &user.0)?;
    let token = account_token(&s, &account).await?;
    let provider = make_provider(&account, token);
    Ok(Json(test_resp(provider.verify_token().await)))
}

/// `POST /git/accounts/test` — verify a not-yet-saved account form. The draft
/// token travels in the body exactly once and is neither persisted nor logged.
async fn test_account_draft<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Json(req): Json<TestGitAccountReq>,
) -> ApiResult<Json<GitAccountTestResp>> {
    let _ = (&s, &user); // auth is the route policy (Git Edit); no stored state involved.
    let provider_kind = req
        .provider
        .ok_or_else(|| Error::Invalid("provider is required".into()))?;
    let token = req
        .token
        .as_deref()
        .filter(|t| !t.trim().is_empty())
        .ok_or_else(|| Error::Invalid("token is required".into()))?;
    let account = GitAccount {
        id: String::new(),
        user_id: user.0.id.clone(),
        provider: provider_kind,
        label: String::new(),
        username: req.username.clone().unwrap_or_default(),
        token_ref: String::new(),
        api_base_url: req.api_base_url.clone().filter(|u| !u.trim().is_empty()),
        namespace: None,
        token_expires_at: None,
        created_at: chrono::Utc::now(),
    };
    let provider = make_provider(&account, token.to_string());
    Ok(Json(test_resp(provider.verify_token().await)))
}

// ---------------------------------------------------------------------------
// Collaborators (create-PR reviewer typeahead)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct CollaboratorsQuery {
    q: Option<String>,
}

/// Per-repo cache slot: fetch time + the unfiltered collaborator list.
type CollaboratorsCache = StdMutex<HashMap<String, (std::time::Instant, Vec<Collaborator>)>>;

/// Process-wide 30 s cache of the UNFILTERED collaborator list per repo id —
/// the typeahead fires per keystroke and the provider list is stable; `q`
/// filtering happens after the cache.
fn collaborators_cache() -> &'static CollaboratorsCache {
    static CACHE: OnceLock<CollaboratorsCache> = OnceLock::new();
    CACHE.get_or_init(|| StdMutex::new(HashMap::new()))
}

const COLLABORATORS_TTL: std::time::Duration = std::time::Duration::from_secs(30);

fn collaborators_key(repo: &Id, account: &Id) -> String {
    format!("{repo}\u{1f}{account}")
}

/// `GET /repos/{id}/collaborators?q=` — provider-backed reviewer typeahead.
/// ws viewer + S4 (the call uses the bound account's token via provider_ctx).
/// Provider errors surface as-is; the UI degrades to a free-text input.
async fn repo_collaborators<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Query(q): Query<CollaboratorsQuery>,
) -> ApiResult<Json<Vec<Collaborator>>> {
    let (repo, _) = repo_ctx(&s, &user, &id, WorkspaceRole::Viewer).await?;
    let query = q.q.unwrap_or_default();
    // S4 on EVERY call, cache hit or not, and the slot is keyed by the account
    // whose token fetched it: a hit used to hand a non-owner the list the
    // owner's credential had fetched, without any ownership check.
    let bound = authorized_repo_account(&s, &user, &repo).await?;
    let cached = bound.as_ref().and_then(|a| {
        collaborators_cache()
            .lock()
            .expect("collaborators cache poisoned")
            .get(&collaborators_key(&id, &a.id))
            .filter(|(at, _)| at.elapsed() < COLLABORATORS_TTL)
            .map(|(_, list)| list.clone())
    });
    let all = match cached {
        Some(list) => list,
        None => {
            let (provider, remote, account_id) =
                provider_ctx_with_account(&s, &user, &repo).await?;
            // Fetch unfiltered so one provider call serves every keystroke.
            let list = provider.list_collaborators(&remote, "").await?;
            collaborators_cache()
                .lock()
                .expect("collaborators cache poisoned")
                .insert(
                    collaborators_key(&id, &account_id),
                    (std::time::Instant::now(), list.clone()),
                );
            list
        }
    };
    let needle = query.to_ascii_lowercase();
    Ok(Json(
        all.into_iter()
            .filter(|c| {
                needle.is_empty()
                    || c.name.to_ascii_lowercase().contains(&needle)
                    || c.display_name.to_ascii_lowercase().contains(&needle)
            })
            .collect(),
    ))
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
    let token = account_token(&s, &account).await?;
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
/// Drop repos whose path no longer exists on disk (failed/aborted clones,
/// deleted checkouts) so they don't linger in pickers. Rows are kept — a repo
/// on an unmounted volume reappears when the path does.
async fn retain_on_disk(repos: &mut Vec<Repo>) {
    let mut keep = Vec::with_capacity(repos.len());
    for r in repos.drain(..) {
        if tokio::fs::metadata(&r.path).await.is_ok() {
            keep.push(r);
        }
    }
    *repos = keep;
}

async fn list_all_repos<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
) -> ApiResult<Json<Vec<Repo>>> {
    let mut all = s.store().list_all_repos().await?;
    retain_on_disk(&mut all).await;
    all.iter_mut().for_each(fill_forge);
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
    let mut repos = s.store().list_repos(&ws_id).await?;
    retain_on_disk(&mut repos).await;
    repos.iter_mut().for_each(fill_forge);
    Ok(Json(repos))
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
    let mut repo = match (&req.path, &req.clone_url) {
        (Some(path), None) => register_repo(&s, &user, &ws_id, path, &req).await?,
        (None, Some(url)) => clone_into_workspace(&s, &user, &ws_id, url, &req).await?,
        _ => return Err(Error::Invalid("provide exactly one of path | clone_url".into()).into()),
    };
    fill_forge(&mut repo);
    Ok(Json(repo))
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
    if let Some(mut found) = existing.into_iter().find(|r| r.path == top) {
        fill_forge(&mut found);
        return Ok(Json(found));
    }
    let add = AddRepoReq {
        path: Some(top.clone()),
        clone_url: None,
        name: None,
        git_account_id: None,
        clone_dir: None,
    };
    let mut repo = register_repo(&s, &user, &ws_id, &top, &add).await?;
    fill_forge(&mut repo);
    Ok(Json(repo))
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
    // Before anything else: a clone URL is caller input that reaches `git
    // clone` as a positional argument.
    crate::local::validate_remote_url(url)?;
    let ws = s.workspaces().get(ws_id).await?;
    let name = req
        .name
        .clone()
        .or_else(|| derive_repo_name(url))
        .ok_or_else(|| Error::Invalid("cannot derive repo name from clone url".into()))?;
    // Clone INTO a user-chosen parent dir when provided (`~` expanded), else the
    // workspace root. The repo lands at `<base>/<name>`.
    let base = match req
        .clone_dir
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
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

    let dest_str = dest.to_string_lossy().into_owned();
    // Row first — the UI sees the repo immediately; Notice events track progress.
    // Reuse a leftover row for the same path (a prior failed clone) instead of
    // minting a duplicate — same rule as `register_repo` for local adds.
    let existing = s
        .store()
        .list_repos(ws_id)
        .await?
        .into_iter()
        .find(|r| r.path == dest_str);
    let repo = match existing {
        Some(r) => r,
        None => {
            s.store()
                .create_repo(NewRepo {
                    workspace_id: ws_id.clone(),
                    name: name.clone(),
                    path: dest_str,
                    remote_url: Some(url.to_string()),
                    provider,
                    git_account_id: account_id.clone(),
                })
                .await?
        }
    };

    let token = match &account_id {
        Some(aid) => {
            let account = s.store().get_account(aid).await?;
            secret(s.secrets(), &account.token_ref).await?
        }
        None => None,
    };

    let task_state = s.clone();
    let task_url = url.to_string();
    let task_name = name.clone();
    let task_repo_id = repo.id.clone();
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
            Err(e) => {
                notice(
                    &task_state,
                    "error",
                    "Clone failed",
                    &format!("{task_name}: {e}"),
                );
                // The eager row now points at nothing (git removes the dest it
                // created on failure) — unregister it so failed attempts don't
                // pile up as dead entries in the repo list. Files untouched;
                // if the dest somehow exists, keep the row.
                if tokio::fs::metadata(&dest).await.is_err() {
                    let _ = task_state.store().delete_repo(&task_repo_id).await;
                }
            }
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

/// `PATCH /repos/{id}` — (re)bind the repo's hosting account. `git_account_id`
/// is authoritative: an id binds, `null` unbinds. Registration is the only other
/// place an account is resolved, so without this a repo added BEFORE its account
/// existed (or one whose `origin` was added later) can never reach a provider.
async fn update_repo<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<UpdateRepoReq>,
) -> ApiResult<Json<Repo>> {
    let repo = s.store().get_repo(&id).await?;
    s.roles()
        .check(&user.0, &repo.workspace_id, WorkspaceRole::Editor)
        .await?;
    // Re-read `origin` first: a repo can only be bound to a provider account
    // once we know WHICH provider it points at, and the stored remote is a
    // snapshot from registration time.
    let repo = refresh_remote(&s, repo).await?;
    let Some(account_id) = req.git_account_id.as_ref() else {
        return Ok(Json(with_forge(
            s.store().set_repo_account(&id, None).await?,
        )));
    };
    let account = s.store().get_account(account_id).await?;
    // S4: binding a repo to someone else's credential would let this caller
    // push through it later — the same guard registration applies.
    authorize_owner(&account, &user.0)?;
    match repo.provider {
        Some(kind) if kind != account.provider => {
            return Err(Error::Invalid(format!(
                "repo remote is {} but the account is {}",
                kind.as_str(),
                account.provider.as_str()
            ))
            .into())
        }
        // No detectable remote (or an unsupported host): binding an account
        // would be a lie — provider calls resolve the remote, not the account.
        None => {
            return Err(Error::Invalid(
                "repo has no supported remote — add an origin on a supported host first".into(),
            )
            .into())
        }
        Some(_) => {}
    }
    Ok(Json(with_forge(
        s.store().set_repo_account(&id, Some(&account.id)).await?,
    )))
}

/// Re-detect the repo's `origin` from disk and persist it when it differs from
/// the stored snapshot. Returns the (possibly updated) repo; a working tree that
/// has gone missing is left untouched rather than having its remote wiped.
async fn refresh_remote<S: GitCtx>(s: &S, repo: Repo) -> Result<Repo> {
    if tokio::fs::metadata(&repo.path).await.is_err() {
        return Ok(repo);
    }
    let remote_url = LocalGit::new(&repo.path).remote_url().await;
    let provider = remote_url.as_deref().and_then(detect).map(|(k, _)| k);
    if remote_url == repo.remote_url && provider == repo.provider {
        return Ok(repo);
    }
    s.store()
        .set_repo_remote(&repo.id, remote_url.as_deref(), provider)
        .await
}

fn with_forge(mut repo: Repo) -> Repo {
    fill_forge(&mut repo);
    repo
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
    crate::watch::global(s.events()).forget(&id);
    forget_repo_locks(&id);
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Local ops (#37–47)
// ---------------------------------------------------------------------------

async fn repo_status<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Response> {
    let (repo, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Viewer).await?;
    watch_repo(&s, &repo, &git).await;
    // Shared across windows: one walk per change, a 1 s memo between (see
    // `status_cache`). Tagged `x-otto-status-cache: hit|miss`.
    let (body, cache) = crate::status_cache::status_body(&id, &git).await?;
    Ok(json_tagged(body, "x-otto-status-cache", cache))
}

/// Keep the repo's working-tree watcher armed (see `watch.rs`): the client
/// asking for status/fetch is what marks a repo as "open". Arming runs off the
/// request path, so it never delays the response.
async fn watch_repo<S: GitCtx>(s: &S, repo: &Repo, git: &LocalGit) {
    let registry = crate::watch::global(s.events());
    let git_dir = git.git_dir().await;
    let (id, ws, root) = (
        repo.id.clone(),
        repo.workspace_id.clone(),
        PathBuf::from(&repo.path),
    );
    tokio::task::spawn_blocking(move || registry.touch(&id, &ws, &root, git_dir.as_deref()));
    // The session's first status also seeds a missing commit-graph (once,
    // guarded): a never-fetched repo otherwise never gets one, and the
    // graph's `log --all --date-order` walks all history without it.
    git.seed_commit_graph();
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
    // Flag merged branches against the per-repo cleanup base (else the detected
    // default). Reading the setting is cheap and best-effort inside GitCtx.
    let base = s.cleanup_base_branch(&id).await;
    Ok(Json(git.refs_with_base(base.as_deref()).await?))
}

/// GET the repo's cleanup base branch: the stored override (or `None`) plus what
/// it currently resolves to (override if valid, else detected default branch).
async fn repo_cleanup_base<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Json<CleanupBaseResp>> {
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Viewer).await?;
    let base_branch = s.cleanup_base_branch(&id).await;
    let resolved = git.resolve_cleanup_base(base_branch.as_deref()).await;
    Ok(Json(CleanupBaseResp {
        base_branch,
        resolved,
    }))
}

/// PUT the repo's cleanup base branch override. An empty/`None` value clears it
/// so the repo follows its detected default branch again. Never deletes or moves
/// any branch — this only picks which base the "merged" indicators compute against.
async fn repo_set_cleanup_base<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<SetCleanupBaseReq>,
) -> ApiResult<Json<CleanupBaseResp>> {
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    let base = req
        .base_branch
        .map(|b| b.trim().to_string())
        .filter(|b| !b.is_empty());
    s.set_cleanup_base_branch(&id, base.clone()).await?;
    let resolved = git.resolve_cleanup_base(base.as_deref()).await;
    Ok(Json(CleanupBaseResp {
        base_branch: base,
        resolved,
    }))
}

async fn repo_fetch<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Json<RepoStatusResp>> {
    let (repo, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    watch_repo(&s, &repo, &git).await;
    let token = optional_token(&s, &user, &repo).await?;
    // Fetch and push both rewrite the tracking refs: one at a time per repo,
    // so neither dies on "cannot lock ref". The worktree lock is not needed.
    // Concurrent callers share one fetch (see `fetch_coalesced`).
    let lock = remote_lock(&id);
    let (_g, _ran) =
        fetch_coalesced(&id, &lock, || async { git.fetch(token).await.map(drop) }).await?;
    status_after_release(&git, _g).await
}

#[derive(Deserialize)]
struct LogQuery {
    limit: Option<u32>,
    skip: Option<u32>,
    all: Option<bool>,
    /// Scope history to one path (file history); `follow` walks it across
    /// renames (git requires exactly one pathspec for that → 400 without one).
    path: Option<String>,
    follow: Option<bool>,
    /// Server-side search (`--grep` / `--author`): literal, case-insensitive.
    grep: Option<String>,
    author: Option<String>,
    /// Page TO this commit sha in one spawn (see `LogOpts::until`).
    until: Option<String>,
}

async fn repo_log<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Query(q): Query<LogQuery>,
) -> ApiResult<Response> {
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Viewer).await?;
    // No server-side ceiling: `limit=0` (or an explicit large limit) returns the
    // full reachable history. The graph pages through history with skip/limit and
    // must be able to walk back to the ROOT commit — a silent .min(500) here made
    // older commits unreachable no matter what the client asked for.
    let limit = q.limit.unwrap_or(50);
    let blank = |s: &Option<String>| {
        s.as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
    };
    let opts = crate::history::LogOpts {
        limit,
        skip: q.skip.unwrap_or(0),
        all: q.all.unwrap_or(false),
        path: blank(&q.path),
        follow: q.follow.unwrap_or(false),
        grep: blank(&q.grep),
        author: blank(&q.author),
        until: blank(&q.until),
    };
    let commits = git.log_with(&opts).await?;
    let est = commits.len() * 256;
    Ok(json_off_runtime(commits, est).await?)
}

#[derive(Deserialize)]
struct DiffQuery {
    target: Option<String>,
    /// Scope the diff to a single file (`-- <path>`). The Changes view passes
    /// this when a file is selected so it computes only that file's diff instead
    /// of the whole working tree.
    path: Option<String>,
    /// A rename's origin, sent with `path` so both pathspecs reach git and
    /// `-M` still pairs the two sides.
    old_path: Option<String>,
    /// File list + counts only (`--raw --numstat`, never the patch).
    summary: Option<bool>,
    /// With `path`: lift the per-file cap to the hard ceiling.
    full: Option<bool>,
}

impl DiffQuery {
    fn opts(&self) -> crate::local::DiffOpts {
        use crate::parse::DiffCaps;
        let blank = |s: &Option<String>| s.clone().filter(|v| !v.is_empty());
        let (path, old_path) = (blank(&self.path), blank(&self.old_path));
        let summary = self.summary.unwrap_or(false);
        let caps = if summary {
            None
        } else if self.full.unwrap_or(false) && path.is_some() {
            Some(DiffCaps::FULL_FILE)
        } else {
            Some(DiffCaps::DEFAULT)
        };
        crate::local::DiffOpts {
            path,
            old_path,
            summary,
            caps,
        }
    }
}

/// `application/json` response from already-serialized bytes, tagged with
/// whether the diff memo answered it (`x-otto-diff-cache: hit|miss|off`).
fn json_body(body: axum::body::Bytes, cache: &'static str) -> Response {
    use axum::http::header::{HeaderName, HeaderValue, CONTENT_TYPE};
    (
        [
            (CONTENT_TYPE, HeaderValue::from_static("application/json")),
            (
                HeaderName::from_static("x-otto-diff-cache"),
                HeaderValue::from_static(cache),
            ),
        ],
        body,
    )
        .into_response()
}

/// `application/json` from serialized bytes plus one diagnostic header.
fn json_tagged(body: axum::body::Bytes, name: &'static str, value: &'static str) -> Response {
    use axum::http::header::{HeaderName, HeaderValue, CONTENT_TYPE};
    (
        [
            (CONTENT_TYPE, HeaderValue::from_static("application/json")),
            (
                HeaderName::from_static(name),
                HeaderValue::from_static(value),
            ),
        ],
        body,
    )
        .into_response()
}

/// Serialize any response body off the async workers once its estimated
/// size (`est` bytes) is big enough to matter — a 10k-commit graph page or a
/// big blame is several MB of JSON that axum's `Json` would encode inline.
pub(crate) async fn json_off_runtime<T: serde::Serialize + Send + 'static>(
    value: T,
    est: usize,
) -> Result<Response> {
    use axum::http::header::{HeaderValue, CONTENT_TYPE};
    let body = crate::local::off_runtime(est, move || serde_json::to_vec(&value))
        .await?
        .map_err(|e| Error::Internal(format!("json: {e}")))?;
    Ok((
        [(CONTENT_TYPE, HeaderValue::from_static("application/json"))],
        axum::body::Bytes::from(body),
    )
        .into_response())
}

/// Serialize a diff off the async workers once it is big enough to matter.
async fn diff_json(resp: DiffResp) -> Result<axum::body::Bytes> {
    let lines: usize = resp
        .files
        .iter()
        .flat_map(|f| f.hunks.iter())
        .map(|h| h.lines.len())
        .sum();
    crate::local::off_runtime(lines * 64 + resp.files.len() * 128, move || {
        serde_json::to_vec(&resp)
    })
    .await?
    .map(axum::body::Bytes::from)
    .map_err(|e| Error::Internal(format!("diff json: {e}")))
}

/// The content-addressed form of an immutable `target` plus its memo key, or
/// `None` for the worktree/index (always recomputed). Refs are resolved to
/// full commit ids first (one `rev-parse` each; a full hex id costs nothing),
/// so a branch that moves never serves a stale entry. A rev that doesn't
/// resolve falls through to the uncached path and its usual git error.
async fn immutable_diff_key(
    git: &LocalGit,
    target: &DiffTarget,
    opts: &crate::local::DiffOpts,
) -> Option<(DiffTarget, String)> {
    let (kind, resolved) = match target {
        DiffTarget::Commit(rev) => (
            "commit",
            DiffTarget::Commit(git.resolve_commit(rev).await.ok()?),
        ),
        DiffTarget::Range(a, b) => (
            "range2",
            DiffTarget::Range(
                git.resolve_commit(a).await.ok()?,
                git.resolve_commit(b).await.ok()?,
            ),
        ),
        DiffTarget::MergeBase(a, b) => (
            "range3",
            DiffTarget::MergeBase(
                git.resolve_commit(a).await.ok()?,
                git.resolve_commit(b).await.ok()?,
            ),
        ),
        DiffTarget::Worktree | DiffTarget::Working | DiffTarget::Staged => return None,
    };
    let oids = match &resolved {
        DiffTarget::Commit(c) => c.clone(),
        DiffTarget::Range(a, b) | DiffTarget::MergeBase(a, b) => format!("{a}:{b}"),
        _ => return None,
    };
    let mode = match (opts.summary, opts.caps) {
        (true, _) => "summary".to_string(),
        (false, Some(c)) => format!("capped:{}:{}", c.file_lines, c.total_lines),
        (false, None) => "full".to_string(),
    };
    let key = format!(
        "{}\0{kind}\0{oids}\0{}\0{}\0{mode}",
        git.path().display(),
        opts.path.as_deref().unwrap_or(""),
        opts.old_path.as_deref().unwrap_or(""),
    );
    Some((resolved, key))
}

async fn repo_diff<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Query(q): Query<DiffQuery>,
) -> ApiResult<Response> {
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Viewer).await?;
    let target = match q.target.as_deref() {
        None => DiffTarget::Worktree,
        Some(t) => DiffTarget::parse(t)?,
    };
    let opts = q.opts();
    if let Some((resolved, key)) = immutable_diff_key(&git, &target, &opts).await {
        if let Some(body) = crate::diff_cache::get_body(&key) {
            return Ok(json_body(body, "hit"));
        }
        // One computation per key however many identical requests race; it is
        // dropped (git killed) only when every one of them has gone away.
        let repo_path = git.path().to_path_buf();
        let body = crate::diff_cache::diff_flights()
            .run(&key, || {
                let key = key.clone();
                async move {
                    let git = LocalGit::new(repo_path);
                    let body = diff_json(git.diff_with(&resolved, &opts).await?).await?;
                    crate::diff_cache::put_body(key, body.clone());
                    Ok(body)
                }
            })
            .await?;
        return Ok(json_body(body, "miss"));
    }
    let resp = git.diff_with(&target, &opts).await?;
    Ok(json_body(diff_json(resp).await?, "off"))
}

async fn repo_stage<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<StagePathsReq>,
) -> ApiResult<Json<RepoStatusResp>> {
    let lock = repo_lock(&id);
    let _g = lock.lock().await;
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    git.stage(&req.paths).await?;
    status_after_release(&git, _g).await
}

async fn repo_unstage<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<StagePathsReq>,
) -> ApiResult<Json<RepoStatusResp>> {
    let lock = repo_lock(&id);
    let _g = lock.lock().await;
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    git.unstage(&req.paths).await?;
    status_after_release(&git, _g).await
}

/// `POST /repos/{id}/discard` body: [`StagePathsReq`] plus `keep_staged`,
/// which discards only the unstaged side (the Unstaged list's Discard).
#[derive(Debug, serde::Deserialize)]
struct DiscardReq {
    paths: Vec<String>,
    #[serde(default)]
    keep_staged: bool,
}

async fn repo_discard<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<DiscardReq>,
) -> ApiResult<Json<RepoStatusResp>> {
    let lock = repo_lock(&id);
    let _g = lock.lock().await;
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    git.discard_with(&req.paths, req.keep_staged).await?;
    status_after_release(&git, _g).await
}

async fn repo_commit<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<CommitReq>,
) -> ApiResult<Json<serde_json::Value>> {
    let lock = repo_lock(&id);
    let _g = lock.lock().await;
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    let sha = git.commit_signed(&req.message, req.amend, req.sign).await?;
    Ok(Json(serde_json::json!({ "sha": sha })))
}

/// Optional push body: `branch` pushes THAT branch explicitly (Create-PR
/// pushes the selected source branch, not whatever happens to be checked out).
/// Absent/empty body keeps the current-branch behavior.
#[derive(Debug, Default, serde::Deserialize)]
struct PushReq {
    #[serde(default)]
    branch: Option<String>,
    /// Overwrite the remote branch with `--force-with-lease --force-if-includes`
    /// (never a bare `--force`). Only ever sent after the user confirmed it in
    /// the UI; absent/false is a normal push that a diverged remote rejects
    /// with a 409.
    #[serde(default)]
    force_with_lease: bool,
}

async fn repo_push<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    body: Option<Json<PushReq>>,
) -> ApiResult<Json<RepoStatusResp>> {
    // The network-leg lock, NOT `repo_lock`: a slow push must not queue
    // stage/commit behind it (see `remote_lock`).
    let lock = remote_lock(&id);
    let _g = lock.lock().await;
    let (repo, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    let token = optional_token(&s, &user, &repo).await?;
    let branch = body.as_ref().and_then(|b| b.branch.clone());
    let branch = branch.as_deref().map(str::trim).filter(|b| !b.is_empty());
    let force = body.as_ref().is_some_and(|b| b.force_with_lease);
    if force {
        // Rewriting shared history is rare and consequential — leave a trace.
        tracing::info!(repo = %id, user = %user.0.username, branch = ?branch, "force push (with lease)");
    }
    git.push_with(token, branch, force).await?;
    // Return the FRESH status so the UI's ahead/behind chip updates after push.
    status_after_release(&git, _g).await
}

/// Optional pull body: `auto_stash` wraps the pull in stash → pull → pop when
/// the tree is dirty (the retry the UI offers after a 409 "commit or stash
/// first" refusal). `mode` overrides how the pull reconciles for this call
/// only; absent, the repo's own git config decides (`pull.rebase` / `pull.ff`)
/// instead of a silent `--no-rebase`.
#[derive(Debug, Default, serde::Deserialize)]
struct PullReq {
    #[serde(default)]
    auto_stash: bool,
    #[serde(default)]
    mode: Option<crate::ops::PullMode>,
}

async fn repo_pull<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    body: Option<Json<PullReq>>,
) -> ApiResult<Json<serde_json::Value>> {
    // Pull writes the worktree AND the tracking refs: both locks, in the
    // fixed `repo_lock` → `remote_lock` order.
    let lock = repo_lock(&id);
    let _g = lock.lock().await;
    let rlock = remote_lock(&id);
    let _rg = rlock.lock().await;
    let (repo, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    let token = optional_token(&s, &user, &repo).await?;
    // Pull, then return the FRESH status so the UI's branch chip clears its
    // ahead/behind. (Previously returned `{output}`, which the UI consumed as a
    // RepoStatusResp — so the behind count never updated and pull looked like a
    // no-op even when it fast-forwarded.)
    //
    // A pull whose merge CONFLICTS is not a failure: the fetch landed and a
    // merge is now in progress. Return 200 with the conflicted status (the
    // unmerged paths are in `changes` as kind="conflicted") so the UI can route
    // the user into the conflict resolver instead of showing "Pull failed" and
    // leaving the incoming files looking like mystery WIP changes.
    let body = body.map(|Json(b)| b).unwrap_or_default();
    let mode = match body.mode {
        Some(m) => m,
        None => git.pull_mode_default().await,
    };
    let note = if body.auto_stash {
        let (_, note) = git.pull_autostash_mode(token, mode).await?;
        note
    } else {
        git.pull_outcome_mode(token, mode).await?;
        None
    };
    drop(_rg);
    drop(_g);
    Ok(Json(serde_json::json!({
        "status": git.status().await?,
        "note": note,
    })))
}

/// `POST /repos/{id}/api-collections/pull` — pull the repo, then read every
/// `collections/*.json` (Postman collection files) and return their contents
/// for the API client to import.
async fn repo_collections_pull<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Json<serde_json::Value>> {
    let lock = repo_lock(&id);
    let _g = lock.lock().await;
    let rlock = remote_lock(&id);
    let _rg = rlock.lock().await;
    let (repo, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    let token = optional_token(&s, &user, &repo).await?;
    let _ = git.pull(token).await; // best-effort; report read result regardless
    let dir = std::path::Path::new(&repo.path).join("collections");
    let mut files: Vec<serde_json::Value> = Vec::new();
    if let Ok(mut entries) = tokio::fs::read_dir(&dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let p = entry.path();
            if p.extension().and_then(|x| x.to_str()) == Some("json") {
                if let Ok(content) = tokio::fs::read_to_string(&p).await {
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
    let lock = repo_lock(&id);
    let _g = lock.lock().await;
    let rlock = remote_lock(&id);
    let _rg = rlock.lock().await;
    let (repo, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    if let Some(branch) = req.branch.as_deref().filter(|b| !b.is_empty()) {
        git.checkout(branch, true).await?;
    }
    let base = std::path::Path::new(&repo.path);
    // Local disk IO failing is OUR failure (500), not an upstream outage (502)
    // — a full disk must not raise the "git provider is down" banner.
    tokio::fs::create_dir_all(base.join("collections"))
        .await
        .map_err(|e| otto_core::Error::Internal(format!("create collections dir: {e}")))?;
    let mut staged: Vec<String> = Vec::new();
    for f in &req.files {
        let safe = f.name.replace(['/', '\\'], "_");
        let safe = if safe.ends_with(".json") {
            safe
        } else {
            format!("{safe}.json")
        };
        let rel = format!("collections/{safe}");
        tokio::fs::write(base.join(&rel), &f.content)
            .await
            .map_err(|e| otto_core::Error::Internal(format!("write {rel}: {e}")))?;
        staged.push(rel);
    }
    git.stage(&staged).await?;
    // ONLY the collection files: a plain `commit` took the whole index, so
    // the user's own staged work was committed and pushed as "collections".
    let sha = git.commit_only(&req.message, &staged).await?;
    let token = optional_token(&s, &user, &repo).await?;
    let push_out = git.push(token).await?;
    Ok(Json(
        serde_json::json!({ "commit": sha, "push": push_out, "files": staged.len() }),
    ))
}

/// `POST /repos/{id}/checkout` — switch branches. NEVER pulls, fetches or
/// merges: `auto_stash:true` only wraps the switch in stash -u → checkout →
/// pop so a dirty tree isn't a dead end. A conflicting pop comes back as a
/// normal 200 whose status carries `kind:"conflicted"` rows.
async fn repo_checkout<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<CheckoutReq>,
) -> ApiResult<Json<RepoStatusResp>> {
    let branch = req.branch.trim();
    if branch.is_empty() {
        return Err(Error::Invalid("branch must not be empty".into()).into());
    }
    let lock = repo_lock(&id);
    let _g = lock.lock().await;
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    if req.auto_stash {
        git.checkout_autostash(branch, req.create).await?;
    } else {
        git.checkout(branch, req.create).await?;
    }
    status_after_release(&git, _g).await
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
    status_after_release(&git, _g).await
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
    status_after_release(&git, _g).await
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
    // Same per-repo lock as stage/commit/merge: a concurrent checkout or
    // merge must not interleave with this ref/worktree mutation.
    let lock = repo_lock(&id);
    let _g = lock.lock().await;
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    git.create_branch(
        req.name.trim(),
        req.start_point
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty()),
        req.checkout.unwrap_or(false),
    )
    .await?;
    status_after_release(&git, _g).await
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
    let lock = repo_lock(&id);
    let _g = lock.lock().await;
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    git.rename_branch(req.from.trim(), req.to.trim()).await?;
    status_after_release(&git, _g).await
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
    /// `-D` (drop unmerged) instead of `-d`. The UI escalates to this only
    /// after its own "not fully merged" confirm — the default is the SAFE
    /// `-d`, which refuses to drop unmerged work.
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
    let lock = repo_lock(&id);
    let _g = lock.lock().await;
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
        git.delete_branch(name, req.force.unwrap_or(false)).await?;
    }
    if want_remote {
        let token = optional_token(&s, &user, &repo).await?;
        git.delete_remote_branch(name, token).await?;
    }
    status_after_release(&git, _g).await
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
    let lock = repo_lock(&id);
    let _g = lock.lock().await;
    let (repo, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    git.create_tag(
        name,
        req.sha.trim(),
        req.message
            .as_deref()
            .map(str::trim)
            .filter(|m| !m.is_empty()),
    )
    .await?;
    if req.push.unwrap_or(false) {
        let token = optional_token(&s, &user, &repo).await?;
        git.push_tag(name, token).await?;
    }
    status_after_release(&git, _g).await
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
    let lock = remote_lock(&id);
    let _g = lock.lock().await;
    git.push_tag(name, token).await?;
    status_after_release(&git, _g).await
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
    let lock = repo_lock(&id);
    let _g = lock.lock().await;
    let (repo, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    git.delete_tag(name).await?;
    if req.remote.unwrap_or(false) {
        let token = optional_token(&s, &user, &repo).await?;
        git.delete_remote_tag(name, token).await?;
    }
    status_after_release(&git, _g).await
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

async fn repo_worktrees<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Json<Vec<WorktreeInfo>>> {
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Viewer).await?;
    Ok(Json(git.worktree_list().await?))
}

#[derive(Deserialize)]
struct WorktreeRemoveReq {
    /// Absolute worktree path, exactly as returned by GET …/worktrees.
    path: String,
    /// Force-remove a dirty/locked worktree (git refuses otherwise).
    #[serde(default)]
    force: Option<bool>,
}

fn checked_worktree_force(wt: &WorktreeInfo, requested: bool) -> otto_core::Result<bool> {
    if requested && !wt.dirty_known {
        return Err(Error::Conflict(
            "Worktree status is unknown; retry the status check before forcing removal".into(),
        ));
    }
    Ok(requested)
}

#[cfg(test)]
#[test]
fn worktree_unknown_locked_status_never_authorizes_force() {
    let mut rows =
        crate::parse::parse_worktree_list("worktree /fixture\nHEAD abc\nlocked fixture\n");
    let row = &mut rows[0];
    assert!(row.locked);
    assert!(matches!(
        checked_worktree_force(row, true),
        Err(Error::Conflict(_))
    ));
    assert!(!checked_worktree_force(row, false).unwrap());
    row.dirty_known = true;
    row.dirty = true;
    assert!(checked_worktree_force(row, true).unwrap());
    row.dirty_known = false;
    assert!(
        checked_worktree_force(row, true).is_err(),
        "fresh unknown overrides an older known UI snapshot"
    );
}

async fn repo_worktree_remove<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<WorktreeRemoveReq>,
) -> ApiResult<Json<Vec<WorktreeInfo>>> {
    let path = req.path.trim();
    if path.is_empty() {
        return Err(Error::Invalid("worktree path must not be empty".into()).into());
    }
    let lock = repo_lock(&id);
    let _g = lock.lock().await;
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    // Only paths this repo actually lists are removable — the path is caller
    // input, and `git worktree remove` on an arbitrary directory must never
    // be reachable. The main worktree is the repo itself; git refuses it too,
    // but rejecting up front gives a clear message instead of git's.
    let wts = git.worktree_list().await?;
    let Some(wt) = wts.iter().find(|w| w.path == path) else {
        return Err(Error::Invalid(format!("'{path}' is not a worktree of this repo")).into());
    };
    if wt.is_main {
        return Err(Error::Invalid("cannot remove the main worktree".into()).into());
    }
    if wt.prunable {
        // The directory is already gone — remove the stale registration.
        git.worktree_prune().await?;
    } else {
        let force = checked_worktree_force(wt, req.force.unwrap_or(false))?;
        git.worktree_remove_checked(path, force).await?;
    }
    Ok(Json(git.worktree_list().await?))
}

async fn repo_worktree_prune<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Json<Vec<WorktreeInfo>>> {
    let lock = repo_lock(&id);
    let _g = lock.lock().await;
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    git.worktree_prune().await?;
    Ok(Json(git.worktree_list().await?))
}

async fn repo_submodules<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Json<Vec<SubmoduleInfo>>> {
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Viewer).await?;
    Ok(Json(git.submodule_list().await?))
}

#[derive(Deserialize)]
struct SubmoduleUpdateReq {
    /// Update just this submodule (repo-relative path); all when absent.
    #[serde(default)]
    path: Option<String>,
}

async fn repo_submodule_update<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<SubmoduleUpdateReq>,
) -> ApiResult<Json<Vec<SubmoduleInfo>>> {
    let lock = repo_lock(&id);
    let _g = lock.lock().await;
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    if let Some(p) = req.path.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
        // Same containment rule as worktree remove: only known submodule paths.
        let known = git.submodule_list().await?;
        if !known.iter().any(|s| s.path == p) {
            return Err(Error::Invalid(format!("'{p}' is not a submodule of this repo")).into());
        }
        git.submodule_update(Some(p)).await?;
    } else {
        git.submodule_update(None).await?;
    }
    Ok(Json(git.submodule_list().await?))
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
    let lock = repo_lock(&id);
    let _g = lock.lock().await;
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
    status_after_release(&git, _g).await
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
    match req.side.as_deref() {
        Some(side) => git.resolve_take_side(&req.path, side).await?,
        None => git.write_resolution(&req.path, &req.content).await?,
    }
    status_after_release(&git, _g).await
}

// ---------------------------------------------------------------------------
// PRs (#48–56)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct PrListQuery {
    state: Option<String>,
    page: Option<u32>,
    per_page: Option<u32>,
}

async fn pr_list<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Query(q): Query<PrListQuery>,
) -> ApiResult<Json<otto_core::api::PrListResp>> {
    let (repo, _) = repo_ctx(&s, &user, &id, WorkspaceRole::Viewer).await?;
    let state = match q.state.as_deref() {
        None | Some("open") => PrState::Open,
        Some("merged") => PrState::Merged,
        Some("declined") => PrState::Declined,
        Some("all") => PrState::All,
        Some(other) => return Err(Error::Invalid(format!("bad pr state: {other}")).into()),
    };
    // Clamp rather than reject: paging bounds are a UI detail, and a silly
    // `per_page=100000` must never become a URL we hand to a forge.
    let page = q.page.unwrap_or(1).clamp(1, 10_000);
    let per_page = q.per_page.unwrap_or(50).clamp(1, 100);
    let (provider, remote) = provider_ctx(&s, &user, &repo).await?;
    let p = provider.list_prs(&remote, state, page, per_page).await?;
    Ok(Json(otto_core::api::PrListResp {
        items: p.items,
        has_more: p.has_more,
        page,
        per_page,
    }))
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

#[derive(Deserialize)]
struct PrDiffQuery {
    /// File list + counts only.
    summary: Option<bool>,
    /// One file (its current path; a deleted file's old path).
    path: Option<String>,
    /// A rename's origin, matched alongside `path`.
    old_path: Option<String>,
    /// With `path`: lift the per-file cap to the hard ceiling.
    full: Option<bool>,
    /// Opaque revision token (the UI passes `PrSummary.head_sha`): part of the
    /// memo key, so a push is a new key rather than a stale hit.
    rev: Option<String>,
}

async fn pr_diff<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path((id, number)): Path<(Id, u64)>,
    Query(q): Query<PrDiffQuery>,
) -> ApiResult<Response> {
    use crate::diff_cache::{approx_size, pr_diffs, pr_flights};
    let (repo, _) = repo_ctx(&s, &user, &id, WorkspaceRole::Viewer).await?;
    // Access is decided per request (role + the caller's provider credential)
    // BEFORE the shared memo is consulted.
    let (provider, remote, account) = provider_ctx_with_account(&s, &user, &repo).await?;
    let rev = q.rev.as_deref().unwrap_or("");
    let key = format!(
        "{}\0{}/{}\0{number}\0{account}\0{rev}",
        repo.id, remote.owner, remote.repo
    );
    let cached = pr_diffs()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(&key);
    // Summary-first: unless the whole diff is memoized already, a summary or
    // one-file request goes to the provider's NARROW API (Bitbucket's
    // `diffstat` / `diff?path=`), memoized under its own key — the whole PR
    // diff is only downloaded by a caller that actually wants all of it, or
    // for a provider with no such API.
    let narrow_key = match (&cached, q.path.as_deref().filter(|p| !p.is_empty())) {
        (Some(_), _) => None,
        (None, Some(p)) => Some(format!(
            "{key}\0file\0{p}\0{}",
            q.old_path.as_deref().unwrap_or("")
        )),
        (None, None) if q.summary == Some(true) => Some(format!("{key}\0summary")),
        (None, None) => None,
    };
    let narrow = match narrow_key {
        None => None,
        Some(nk) => {
            let hit = pr_diffs()
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .get(&nk);
            match hit {
                Some(d) => Some((d, "hit")),
                None => {
                    let (provider, remote) = (provider.clone(), remote.clone());
                    let (path, old_path) = (q.path.clone(), q.old_path.clone());
                    crate::diff_cache::pr_narrow_flights()
                        .run(&nk, || {
                            let nk = nk.clone();
                            async move {
                                let got = match path.as_deref().filter(|p| !p.is_empty()) {
                                    Some(p) => {
                                        provider
                                            .get_pr_file_diff(
                                                &remote,
                                                number,
                                                p,
                                                old_path.as_deref(),
                                            )
                                            .await?
                                    }
                                    None => provider.get_pr_diff_summary(&remote, number).await?,
                                };
                                Ok(got.map(|mut d| {
                                    for f in &mut d.files {
                                        crate::parse::fill_counts(f);
                                    }
                                    crate::parse::fill_totals(&mut d);
                                    let d = Arc::new(d);
                                    pr_diffs().lock().unwrap_or_else(|p| p.into_inner()).put(
                                        nk,
                                        d.clone(),
                                        approx_size(&d),
                                    );
                                    d
                                }))
                            }
                        })
                        .await?
                        .map(|d| (d, "miss"))
                }
            }
        }
    };
    let (diff, cache) = match (narrow, cached) {
        (Some(n), _) => n,
        (None, Some(d)) => (d, "hit"),
        (None, None) => {
            let d = pr_flights()
                .run(&key, || {
                    let key = key.clone();
                    async move {
                        let mut d = provider.get_pr_diff(&remote, number).await?;
                        for f in &mut d.files {
                            crate::parse::fill_counts(f);
                        }
                        crate::parse::fill_totals(&mut d);
                        let d = Arc::new(d);
                        pr_diffs().lock().unwrap_or_else(|p| p.into_inner()).put(
                            key,
                            d.clone(),
                            approx_size(&d),
                        );
                        Ok(d)
                    }
                })
                .await?;
            (d, "miss")
        }
    };
    let opts = DiffQuery {
        target: None,
        path: q.path,
        old_path: q.old_path,
        summary: q.summary,
        full: q.full,
    }
    .opts();
    let size = if opts.path.is_some() || opts.summary {
        0
    } else {
        approx_size(&diff)
    };
    let body = crate::local::off_runtime(size, move || {
        let keep = |f: &otto_core::api::FileDiff| match (&opts.path, &opts.old_path) {
            (None, _) => true,
            (Some(p), old) => {
                f.path == *p
                    || f.old_path.as_deref() == Some(p.as_str())
                    || (old.is_some() && f.old_path.as_deref() == old.as_deref())
            }
        };
        let view = crate::parse::capped_view(&diff, keep, opts.caps.as_ref(), opts.summary);
        serde_json::to_vec(&view)
    })
    .await?
    .map_err(|e| Error::Internal(format!("diff json: {e}")))?;
    Ok(json_body(body.into(), cache))
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

/// Resolve (`{"resolved":true}`) or reopen (`false`) a review thread. `{cid}`
/// is the provider thread id surfaced as `PrComment.thread_id`.
async fn pr_resolve_thread<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path((id, number, cid)): Path<(Id, u64, String)>,
    Json(req): Json<ResolvePrThreadReq>,
) -> ApiResult<StatusCode> {
    let (repo, _) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    let (provider, remote) = provider_ctx(&s, &user, &repo).await?;
    provider
        .resolve_pr_thread(&remote, number, &cid, req.resolved)
        .await?;
    Ok(StatusCode::NO_CONTENT)
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
    provider
        .merge(&remote, number, req.strategy, req.delete_source_branch)
        .await?;
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
    use otto_state::DbPool;
    use otto_state::{GitStore, NewGitAccount, NewRepo, WorkspacesRepo};

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

    async fn mem_pool() -> DbPool {
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
        pool.into()
    }

    async fn seed_user(pool: &DbPool, username: &str) -> Id {
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

    async fn seed_workspace(pool: &DbPool) -> Id {
        let wid = new_id();
        let now = Utc::now().to_rfc3339();
        sqlx::query("INSERT INTO workspaces (id, name, root_path, created_at) VALUES (?, ?, ?, ?)")
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

    #[tokio::test]
    async fn worktree_unknown_force_route_preserves_checkout_and_registration() {
        let (_pool, ctx, user, ws) = fixture().await;
        let dir = tempfile::tempdir().unwrap();
        let canonical = dir.path().canonicalize().unwrap();
        let root = canonical.join("repo");
        let linked = canonical.join("linked");
        std::fs::create_dir(&root).unwrap();
        let git = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .current_dir(&root)
                .args(args)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            String::from_utf8(out.stdout).unwrap()
        };
        git(&["init", "-q"]);
        git(&[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.test",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--allow-empty",
            "-qm",
            "fixture",
        ]);
        git(&[
            "worktree",
            "add",
            "-q",
            "-b",
            "linked",
            linked.to_str().unwrap(),
        ]);
        // Keep a registered, non-prunable checkout whose status command fails.
        let gitdir = std::fs::read_to_string(linked.join(".git")).unwrap();
        let gitdir = std::path::Path::new(gitdir.trim().strip_prefix("gitdir: ").unwrap());
        std::fs::write(gitdir.join("index"), b"invalid fixture index").unwrap();
        std::fs::write(linked.join("keep.txt"), b"must survive").unwrap();
        let repo = ctx
            .store
            .create_repo(NewRepo {
                workspace_id: ws,
                name: "fixture".into(),
                path: root.to_string_lossy().into_owned(),
                remote_url: None,
                provider: None,
                git_account_id: None,
            })
            .await
            .unwrap();
        let error = repo_worktree_remove(
            State(ctx),
            Extension(auth(&user, false)),
            Path(repo.id),
            Json(WorktreeRemoveReq {
                path: linked.to_string_lossy().into_owned(),
                force: Some(true),
            }),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(&error.0, Error::Conflict(message) if message.contains("unknown")),
            "{:?}",
            error.0
        );
        assert_eq!(error.into_response().status(), StatusCode::CONFLICT);
        assert_eq!(
            std::fs::read(linked.join("keep.txt")).unwrap(),
            b"must survive"
        );
        assert!(git(&["worktree", "list", "--porcelain"]).contains(linked.to_str().unwrap()));
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
        assert!(
            authorized_repo_account(&ctx, &auth(&other, false), &unbound)
                .await
                .unwrap()
                .is_none()
        );
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

    /// Build a ctx over a fresh in-memory pool, plus a user and a workspace.
    async fn fixture() -> (DbPool, TestCtx, Id, Id) {
        let pool = mem_pool().await;
        let user = seed_user(&pool, "u").await;
        let ws = seed_workspace(&pool).await;
        let ctx = TestCtx {
            store: GitStore::new(pool.clone()),
            workspaces: WorkspacesRepo::new(pool.clone()),
            secrets: Arc::new(FixedSecret),
            roles: Arc::new(AllowAll),
            events: tokio::sync::broadcast::channel(8).0,
        };
        (pool, ctx, user, ws)
    }

    async fn seed_account(
        ctx: &TestCtx,
        user: &Id,
        provider: GitProviderKind,
        label: &str,
    ) -> GitAccount {
        ctx.store
            .create_account(NewGitAccount {
                user_id: user.clone(),
                provider,
                label: label.into(),
                username: "who".into(),
                token_ref: format!("gitacct-{label}"),
                api_base_url: None,
                namespace: None,
                token_expires_at: None,
            })
            .await
            .unwrap()
    }

    async fn seed_repo(ctx: &TestCtx, ws: &Id, name: &str, remote: Option<&str>) -> Repo {
        ctx.store
            .create_repo(NewRepo {
                workspace_id: ws.clone(),
                name: name.into(),
                path: format!("/tmp/{name}"),
                remote_url: remote.map(str::to_string),
                provider: remote.and_then(detect).map(|(k, _)| k),
                git_account_id: None,
            })
            .await
            .unwrap()
    }

    /// `provider_ctx`'s Ok side holds a `dyn GitProvider` (not Debug), so error
    /// cases can't use `unwrap_err`.
    async fn expect_provider_err(ctx: &TestCtx, user: &Id, repo: &Repo) -> Error {
        match provider_ctx(ctx, &auth(user, false), repo).await {
            Ok(_) => panic!("expected provider_ctx to fail for an unbound repo"),
            Err(e) => e,
        }
    }

    /// The regression this whole path exists for: a repo registered BEFORE its
    /// hosting account existed is unbound forever, and every provider call dies
    /// with "repo has no git account". With exactly one candidate account the
    /// repo adopts it on first use — and the binding is PERSISTED, so the other
    /// call sites that read `git_account_id` directly (push tokens, run engine)
    /// are healed too, not just this request.
    #[tokio::test]
    async fn unbound_repo_adopts_the_only_matching_account() {
        let (_pool, ctx, user, ws) = fixture().await;
        let repo = seed_repo(&ctx, &ws, "late", Some("https://github.com/o/late.git")).await;
        let account = seed_account(&ctx, &user, GitProviderKind::Github, "gh").await;

        provider_ctx(&ctx, &auth(&user, false), &repo)
            .await
            .unwrap();

        let stored = ctx.store.get_repo(&repo.id).await.unwrap();
        assert_eq!(
            stored.git_account_id.as_ref(),
            Some(&account.id),
            "the adopted account must be written back to the repo row"
        );
    }

    /// Grants Viewer and nothing above it.
    struct ViewerOnly;
    impl RoleChecker for ViewerOnly {
        fn check<'a>(
            &'a self,
            _u: &'a User,
            _w: &'a Id,
            m: WorkspaceRole,
        ) -> BoxFuture<'a, Result<()>> {
            Box::pin(async move {
                match m {
                    WorkspaceRole::Viewer => Ok(()),
                    _ => Err(Error::Forbidden("viewer".into())),
                }
            })
        }
    }

    /// A Viewer opening the PR tab may USE their own only account for that
    /// request, but must not bind the shared repo to it (binding is Editor-
    /// only, #36b — and once bound every push needs the binder's token).
    #[tokio::test]
    async fn viewer_adoption_is_per_request_and_never_persisted() {
        let (_pool, mut ctx, user, ws) = fixture().await;
        ctx.roles = Arc::new(ViewerOnly);
        let repo = seed_repo(&ctx, &ws, "seen", Some("https://github.com/o/seen.git")).await;
        seed_account(&ctx, &user, GitProviderKind::Github, "gh").await;

        provider_ctx(&ctx, &auth(&user, false), &repo)
            .await
            .unwrap();

        let stored = ctx.store.get_repo(&repo.id).await.unwrap();
        assert_eq!(
            stored.git_account_id, None,
            "a viewer must not bind the repo"
        );
    }

    /// The reviewer-typeahead cache must not let a non-owner read the list the
    /// owner's credential fetched: S4 runs before the cache, every call.
    #[tokio::test]
    async fn collaborators_cache_hit_still_enforces_s4() {
        let (pool, ctx, owner, ws) = fixture().await;
        let other = seed_user(&pool, "other").await;
        let account = seed_account(&ctx, &owner, GitProviderKind::Github, "gh").await;
        let repo = seed_repo(&ctx, &ws, "collab", Some("https://github.com/o/collab.git")).await;
        ctx.store
            .set_repo_account(&repo.id, Some(&account.id))
            .await
            .unwrap();
        collaborators_cache().lock().unwrap().insert(
            collaborators_key(&repo.id, &account.id),
            (std::time::Instant::now(), Vec::new()),
        );

        // Owner: served from the cache (no provider call in a unit test).
        let ok = repo_collaborators(
            State(ctx.clone()),
            Extension(auth(&owner, false)),
            Path(repo.id.clone()),
            Query(CollaboratorsQuery { q: None }),
        )
        .await;
        assert!(ok.is_ok());
        // Non-owner: refused even though the slot is warm.
        let denied = repo_collaborators(
            State(ctx.clone()),
            Extension(auth(&other, false)),
            Path(repo.id.clone()),
            Query(CollaboratorsQuery { q: None }),
        )
        .await;
        assert!(denied.is_err(), "a cache hit must not bypass S4");
    }

    /// Two accounts on one provider is a real choice, not a coin flip: the call
    /// fails with an actionable message and the repo stays unbound.
    #[tokio::test]
    async fn ambiguous_provider_accounts_are_never_guessed() {
        let (_pool, ctx, user, ws) = fixture().await;
        let repo = seed_repo(&ctx, &ws, "two", Some("https://github.com/o/two.git")).await;
        seed_account(&ctx, &user, GitProviderKind::Github, "work").await;
        seed_account(&ctx, &user, GitProviderKind::Github, "personal").await;

        let err = expect_provider_err(&ctx, &user, &repo).await;
        assert!(
            matches!(&err, Error::Invalid(m) if m.contains("link a github account")),
            "expected an actionable link hint, got {err:?}"
        );
        assert!(ctx
            .store
            .get_repo(&repo.id)
            .await
            .unwrap()
            .git_account_id
            .is_none());
    }

    /// Another user's account is not a candidate — adoption only ever reaches
    /// for the caller's OWN credential (S4).
    #[tokio::test]
    async fn adoption_ignores_other_users_accounts() {
        let (pool, ctx, user, ws) = fixture().await;
        let stranger = seed_user(&pool, "stranger").await;
        let repo = seed_repo(&ctx, &ws, "theirs", Some("https://github.com/o/theirs.git")).await;
        seed_account(&ctx, &stranger, GitProviderKind::Github, "not-mine").await;

        let err = expect_provider_err(&ctx, &user, &repo).await;
        assert!(matches!(err, Error::Invalid(_)), "got {err:?}");
        assert!(ctx
            .store
            .get_repo(&repo.id)
            .await
            .unwrap()
            .git_account_id
            .is_none());
    }

    /// `PATCH /repos/{id}` binds explicitly (the multi-account escape hatch) and
    /// unbinds on `null`.
    #[tokio::test]
    async fn patch_repo_binds_and_unbinds_the_account() {
        let (_pool, ctx, user, ws) = fixture().await;
        let repo = seed_repo(&ctx, &ws, "pick", Some("https://github.com/o/pick.git")).await;
        seed_account(&ctx, &user, GitProviderKind::Github, "work").await;
        let chosen = seed_account(&ctx, &user, GitProviderKind::Github, "personal").await;

        let bound = update_repo(
            State(ctx.clone()),
            Extension(auth(&user, false)),
            Path(repo.id.clone()),
            Json(UpdateRepoReq {
                git_account_id: Some(chosen.id.clone()),
            }),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(bound.git_account_id.as_ref(), Some(&chosen.id));

        let cleared = update_repo(
            State(ctx.clone()),
            Extension(auth(&user, false)),
            Path(repo.id.clone()),
            Json(UpdateRepoReq::default()),
        )
        .await
        .unwrap()
        .0;
        assert!(cleared.git_account_id.is_none(), "null must unbind");
    }

    /// A bitbucket token can't drive a github remote — binding one is rejected
    /// rather than stored to fail later at request time.
    #[tokio::test]
    async fn patch_repo_rejects_provider_mismatch() {
        let (_pool, ctx, user, ws) = fixture().await;
        let repo = seed_repo(&ctx, &ws, "gh", Some("https://github.com/o/gh.git")).await;
        let bb = seed_account(&ctx, &user, GitProviderKind::Bitbucket, "bb").await;

        let err = update_repo(
            State(ctx.clone()),
            Extension(auth(&user, false)),
            Path(repo.id.clone()),
            Json(UpdateRepoReq {
                git_account_id: Some(bb.id),
            }),
        )
        .await
        .unwrap_err()
        .0;
        assert!(
            matches!(&err, Error::Invalid(m) if m.contains("github") && m.contains("bitbucket")),
            "got {err:?}"
        );
    }

    /// Binding someone else's credential to a repo would let the caller push
    /// through it later — Forbidden, even with AllowAll workspace roles (S4).
    #[tokio::test]
    async fn patch_repo_refuses_a_foreign_account() {
        let (pool, ctx, user, ws) = fixture().await;
        let stranger = seed_user(&pool, "stranger").await;
        let repo = seed_repo(&ctx, &ws, "gh2", Some("https://github.com/o/gh2.git")).await;
        let theirs = seed_account(&ctx, &stranger, GitProviderKind::Github, "theirs").await;

        let err = update_repo(
            State(ctx.clone()),
            Extension(auth(&user, false)),
            Path(repo.id.clone()),
            Json(UpdateRepoReq {
                git_account_id: Some(theirs.id),
            }),
        )
        .await
        .unwrap_err()
        .0;
        assert!(matches!(err, Error::Forbidden(_)), "got {err:?}");
    }

    /// A bulk import whose `git remote get-url` spawn failed records the repo as
    /// remote-less (the helper folds errors into `None`), which used to dead-end
    /// every provider call with "repo has no git provider" — and with no
    /// provider the UI won't even offer an account picker. Provider calls
    /// re-read the remote from disk, so the row heals itself on first use.
    #[tokio::test]
    async fn provider_ctx_recovers_a_remote_that_was_never_recorded() {
        let (_pool, ctx, user, ws) = fixture().await;
        let dir = init_git_repo().await;
        let ok = std::process::Command::new("git")
            .args(["remote", "add", "origin", "https://github.com/o/real.git"])
            .current_dir(&dir)
            .status()
            .expect("spawn git remote add")
            .success();
        assert!(ok, "git remote add failed");

        // The row as a failed bulk import left it: real checkout, no remote.
        let repo = ctx
            .store
            .create_repo(NewRepo {
                workspace_id: ws.clone(),
                name: "bulk".into(),
                path: dir.to_string_lossy().into_owned(),
                remote_url: None,
                provider: None,
                git_account_id: None,
            })
            .await
            .unwrap();
        seed_account(&ctx, &user, GitProviderKind::Github, "gh").await;

        provider_ctx(&ctx, &auth(&user, false), &repo)
            .await
            .unwrap();

        let stored = ctx.store.get_repo(&repo.id).await.unwrap();
        assert_eq!(stored.provider, Some(GitProviderKind::Github));
        assert_eq!(
            stored.remote_url.as_deref(),
            Some("https://github.com/o/real.git")
        );
        assert!(
            stored.git_account_id.is_some(),
            "recovering the remote must also let the account adopt"
        );

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    /// A repo registered before its `origin` existed has no provider to bind
    /// against. PATCH re-reads the remote from disk first, so adding the remote
    /// later is enough — no unregister/re-add dance.
    #[tokio::test]
    async fn patch_repo_picks_up_a_remote_added_after_registration() {
        let (_pool, ctx, user, ws) = fixture().await;
        let dir = init_git_repo().await;
        let path = dir.to_string_lossy().into_owned();
        let repo = register_repo(&ctx, &auth(&user, false), &ws, &path, &reg_req(&path))
            .await
            .unwrap();
        assert!(repo.provider.is_none(), "fixture starts with no remote");
        let account = seed_account(&ctx, &user, GitProviderKind::Github, "gh").await;

        // Binding is refused while the repo has nothing to talk to…
        let err = update_repo(
            State(ctx.clone()),
            Extension(auth(&user, false)),
            Path(repo.id.clone()),
            Json(UpdateRepoReq {
                git_account_id: Some(account.id.clone()),
            }),
        )
        .await
        .unwrap_err()
        .0;
        assert!(
            matches!(&err, Error::Invalid(m) if m.contains("no supported remote")),
            "got {err:?}"
        );

        // …and works once `origin` exists, without re-registering the repo.
        let ok = std::process::Command::new("git")
            .args(["remote", "add", "origin", "https://github.com/o/later.git"])
            .current_dir(&dir)
            .status()
            .expect("spawn git remote add")
            .success();
        assert!(ok, "git remote add failed");

        let bound = update_repo(
            State(ctx.clone()),
            Extension(auth(&user, false)),
            Path(repo.id.clone()),
            Json(UpdateRepoReq {
                git_account_id: Some(account.id.clone()),
            }),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(bound.provider, Some(GitProviderKind::Github));
        assert_eq!(
            bound.remote_url.as_deref(),
            Some("https://github.com/o/later.git")
        );
        assert_eq!(bound.git_account_id.as_ref(), Some(&account.id));

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    /// R0: the switch endpoint NEVER pulls — `auto_stash` only clears a dirty
    /// tree around it. A repo with a second branch and an uncommitted change
    /// lands on that branch with the change restored, and no stash left over.
    #[tokio::test]
    async fn checkout_handler_honours_auto_stash() {
        let (_pool, ctx, user, ws) = fixture().await;
        let dir = init_git_repo().await;
        let sh = |args: &[&str]| {
            let ok = std::process::Command::new("git")
                .args(args)
                .current_dir(&dir)
                .status()
                .expect("spawn git")
                .success();
            assert!(ok, "git {args:?} failed");
        };
        sh(&["config", "user.email", "otto@test.local"]);
        sh(&["config", "user.name", "Otto Test"]);
        sh(&["config", "commit.gpgsign", "false"]);
        std::fs::write(dir.join("a.txt"), "shared\n").unwrap();
        std::fs::write(dir.join("b.txt"), "base\n").unwrap();
        sh(&["add", "-A"]);
        sh(&["commit", "-q", "-m", "init"]);
        sh(&["checkout", "-q", "-b", "develop"]);
        std::fs::write(dir.join("b.txt"), "develop\n").unwrap();
        sh(&["commit", "-q", "-am", "develop edits b"]);
        sh(&["checkout", "-q", "-"]);
        // Uncommitted work the user must not lose across the switch.
        std::fs::write(dir.join("a.txt"), "work in progress\n").unwrap();

        let repo = ctx
            .store
            .create_repo(NewRepo {
                workspace_id: ws.clone(),
                name: "switch".into(),
                path: dir.to_string_lossy().into_owned(),
                remote_url: None,
                provider: None,
                git_account_id: None,
            })
            .await
            .unwrap();

        let st = repo_checkout(
            State(ctx.clone()),
            Extension(auth(&user, false)),
            Path(repo.id.clone()),
            Json(CheckoutReq {
                branch: "develop".into(),
                create: false,
                auto_stash: true,
            }),
        )
        .await
        .unwrap()
        .0;

        assert_eq!(st.branch, "develop");
        assert_eq!(
            std::fs::read_to_string(dir.join("a.txt")).unwrap(),
            "work in progress\n",
            "the stashed change is restored on the new branch"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("b.txt")).unwrap(),
            "develop\n",
            "the switch actually happened"
        );
        assert!(
            LocalGit::new(&dir).stash_list().await.unwrap().is_empty(),
            "a clean pop leaves no stash entry"
        );
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    /// R1: a push holds only the network-leg lock. A stage issued while a
    /// push is stuck on a slow remote (a `pre-receive` hook that sleeps)
    /// returns at once instead of queueing for the whole round-trip; the
    /// push still lands and answers with the post-push status.
    #[tokio::test]
    async fn stage_is_not_blocked_by_a_slow_push() {
        let (_pool, ctx, user, ws) = fixture().await;
        let dir = init_git_repo().await;
        let remote = dir.with_extension("remote.git");
        let sh = |cwd: &std::path::Path, args: &[&str]| {
            let ok = std::process::Command::new("git")
                .args(args)
                .current_dir(cwd)
                .status()
                .expect("spawn git")
                .success();
            assert!(ok, "git {args:?} failed");
        };
        sh(&dir, &["config", "user.email", "otto@test.local"]);
        sh(&dir, &["config", "user.name", "Otto Test"]);
        sh(&dir, &["config", "commit.gpgsign", "false"]);
        sh(&dir, &["checkout", "-q", "-b", "main"]);
        std::fs::write(dir.join("a.txt"), "a\n").unwrap();
        sh(&dir, &["add", "-A"]);
        sh(&dir, &["commit", "-q", "-m", "init"]);
        sh(
            dir.parent().unwrap(),
            &["init", "-q", "--bare", remote.to_str().unwrap()],
        );
        sh(&dir, &["remote", "add", "origin", remote.to_str().unwrap()]);
        sh(&dir, &["push", "-q", "-u", "origin", "main"]);
        // From now on every push parks in the remote for ~3 s, after
        // leaving a marker so the test knows the push is in flight.
        let marker = dir.with_extension("pushing");
        let hook = remote.join("hooks/pre-receive");
        std::fs::write(
            &hook,
            format!("#!/bin/sh\ntouch '{}'\nsleep 3\n", marker.display()),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(dir.join("a.txt"), "b\n").unwrap();
        sh(&dir, &["commit", "-q", "-am", "second"]);
        std::fs::write(dir.join("c.txt"), "new\n").unwrap();

        let repo = ctx
            .store
            .create_repo(NewRepo {
                workspace_id: ws.clone(),
                name: "slow-push".into(),
                path: dir.to_string_lossy().into_owned(),
                remote_url: None,
                provider: None,
                git_account_id: None,
            })
            .await
            .unwrap();

        let push = tokio::spawn(repo_push(
            State(ctx.clone()),
            Extension(auth(&user, false)),
            Path(repo.id.clone()),
            None,
        ));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !marker.exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "push never reached the remote"
            );
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }

        let t0 = std::time::Instant::now();
        let st = repo_stage(
            State(ctx.clone()),
            Extension(auth(&user, false)),
            Path(repo.id.clone()),
            Json(StagePathsReq {
                paths: vec!["c.txt".into()],
            }),
        )
        .await
        .unwrap()
        .0;
        let took = t0.elapsed();
        assert!(
            !push.is_finished(),
            "the push should still be parked in the hook"
        );
        assert!(
            took < std::time::Duration::from_secs(1),
            "stage waited {took:?} behind the push"
        );
        assert!(st.changes.iter().any(|c| c.path == "c.txt" && c.staged));

        let after = push.await.unwrap().unwrap().0;
        assert_eq!(after.ahead, 0, "the push landed");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        let _ = tokio::fs::remove_dir_all(&remote).await;
        let _ = std::fs::remove_file(&marker);
    }

    /// `/diff` end to end through the handler: summary, the cache (hit on
    /// the second identical call, keyed by the RESOLVED commit so a branch
    /// name and its sha share an entry), the worktree never memoized, and a
    /// per-file rename request.
    #[tokio::test]
    async fn diff_route_summary_per_file_and_memo() {
        let (_pool, ctx, user, ws) = fixture().await;
        let dir = init_git_repo().await;
        let sh = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .args(args)
                .current_dir(&dir)
                .output()
                .expect("spawn git");
            assert!(out.status.success(), "git {args:?} failed");
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        sh(&["config", "user.email", "otto@test.local"]);
        sh(&["config", "user.name", "Otto Test"]);
        sh(&["config", "commit.gpgsign", "false"]);
        let body: String = (0..30).map(|i| format!("line {i}\n")).collect();
        std::fs::write(dir.join("old.txt"), &body).unwrap();
        sh(&["add", "-A"]);
        sh(&["commit", "-q", "-m", "init"]);
        std::fs::remove_file(dir.join("old.txt")).unwrap();
        std::fs::write(dir.join("new.txt"), format!("{body}tail\n")).unwrap();
        std::fs::write(dir.join("other.txt"), "x\n").unwrap();
        sh(&["add", "-A"]);
        sh(&["commit", "-q", "-m", "move"]);
        let head = sh(&["rev-parse", "HEAD"]);
        let repo = ctx
            .store
            .create_repo(NewRepo {
                workspace_id: ws.clone(),
                name: "diffs".into(),
                path: dir.to_string_lossy().into_owned(),
                remote_url: None,
                provider: None,
                git_account_id: None,
            })
            .await
            .unwrap();
        let call = |q: DiffQuery| {
            let (ctx, user, id) = (ctx.clone(), user.clone(), repo.id.clone());
            async move {
                let resp = repo_diff(
                    State(ctx),
                    Extension(auth(&user, false)),
                    Path(id),
                    Query(q),
                )
                .await
                .unwrap();
                let cache = resp.headers()["x-otto-diff-cache"]
                    .to_str()
                    .unwrap()
                    .to_string();
                let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
                    .await
                    .unwrap();
                let d: DiffResp = serde_json::from_slice(&bytes).unwrap();
                (cache, d)
            }
        };
        let q = |target: &str, summary: bool, path: Option<&str>, old: Option<&str>| DiffQuery {
            target: Some(target.to_string()),
            path: path.map(str::to_string),
            old_path: old.map(str::to_string),
            summary: Some(summary),
            full: None,
        };

        let (c1, s1) = call(q("commit:HEAD", true, None, None)).await;
        assert_eq!(c1, "miss");
        assert_eq!(s1.files.len(), 2);
        assert!(s1
            .files
            .iter()
            .all(|f| f.hunks.is_empty() && f.hunks_omitted == Some(true)));
        let ren = s1.files.iter().find(|f| f.path == "new.txt").unwrap();
        assert_eq!(ren.old_path.as_deref(), Some("old.txt"));
        assert_eq!(ren.status, Some(otto_core::api::FileChangeStatus::Renamed));
        assert_eq!(s1.total_added, Some(2));
        // Same commit by its full id → the same memo entry.
        let (c2, s2) = call(q(&format!("commit:{head}"), true, None, None)).await;
        assert_eq!(c2, "hit");
        assert_eq!(s2.files.len(), s1.files.len());

        // Per-file with old_path: the rename pairs, hunks present.
        let (_, one) = call(q("commit:HEAD", false, Some("new.txt"), Some("old.txt"))).await;
        assert_eq!(one.files.len(), 1);
        assert_eq!(one.files[0].old_path.as_deref(), Some("old.txt"));
        assert_eq!(one.files[0].hunks.len(), 1);
        assert_eq!(one.files[0].hunks_omitted, None);

        // The worktree is never memoized.
        let (c3, _) = call(q("worktree", true, None, None)).await;
        assert_eq!(c3, "off");
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    /// The route that pulled during a switch is GONE, not merely unused by the
    /// UI: asserted against the real router so a stray client gets a 404 rather
    /// than an unadvertised merge.
    #[tokio::test]
    async fn checkout_update_route_is_gone() {
        use tower::ServiceExt;
        let (_pool, ctx, _user, _ws) = fixture().await;
        let app = router::<TestCtx>().with_state(ctx);
        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/repos/any/checkout-update")
            .header("content-type", "application/json")
            .body(axum::body::Body::from("{}"))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    /// Fetches queued behind a running one reuse its result; a later fetch
    /// (or one after a failure) runs again.
    #[tokio::test]
    async fn concurrent_fetches_of_one_repo_share_a_single_run() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let id: Id = "repo-fetch-single-flight".into();
        let lock = Arc::new(tokio::sync::Mutex::new(()));
        let runs = Arc::new(AtomicUsize::new(0));
        let spawn = |delay: u64| {
            let (id, lock, runs) = (id.clone(), lock.clone(), runs.clone());
            tokio::spawn(async move {
                let (_g, ran) = fetch_coalesced(&id, &lock, || async {
                    runs.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                    Ok(())
                })
                .await
                .unwrap();
                ran
            })
        };
        let first = spawn(80);
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        let joined: Vec<_> = (0..3).map(|_| spawn(0)).collect();
        assert!(first.await.unwrap());
        for j in joined {
            assert!(!j.await.unwrap(), "a waiter must reuse the running fetch");
        }
        assert_eq!(runs.load(Ordering::SeqCst), 1);
        // Arriving after it finished: a fresh fetch.
        assert!(spawn(0).await.unwrap());
        assert_eq!(runs.load(Ordering::SeqCst), 2);
        // A failure is never shared with the next caller.
        let err = fetch_coalesced(&id, &lock, || async {
            tokio::time::sleep(std::time::Duration::from_millis(40)).await;
            Err(Error::Internal("offline".into()))
        });
        let (r, after) = tokio::join!(err, async {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            spawn(0).await.unwrap()
        });
        assert!(r.is_err());
        assert!(after);
        forget_repo_locks(&id);
    }
}
