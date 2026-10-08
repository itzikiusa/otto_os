use std::sync::Arc;

use chrono::Utc;
use otto_core::auth::{BoxFuture, RoleChecker};
use otto_core::domain::{GitProviderKind, User};
use otto_core::secrets::SecretStore;
use otto_state::DbPool;
use otto_state::{GitStore, NewGitAccount, NewRepo, WorkspacesRepo};

use super::*;

/// S15-01: a non-origin remote only receives origin's account token when
/// it lives on the same host; local paths never match anything.
#[test]
fn url_host_parses_https_ssh_and_scp_forms() {
    assert_eq!(
        url_host("https://GitHub.com/o/r.git").as_deref(),
        Some("github.com")
    );
    assert_eq!(
        url_host("https://x@github.com:443/o/r").as_deref(),
        Some("github.com")
    );
    assert_eq!(
        url_host("ssh://git@gitlab.com:22/o/r").as_deref(),
        Some("gitlab.com")
    );
    assert_eq!(
        url_host("git@bitbucket.org:o/r.git").as_deref(),
        Some("bitbucket.org")
    );
    assert_eq!(url_host("/tmp/origin.git"), None);
    assert_eq!(url_host("relative/path"), None);
}

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

#[tokio::test]
async fn provider_ctx_tracks_changed_and_removed_origin() {
    let (_pool, ctx, user, ws) = fixture().await;
    let dir = init_git_repo().await;
    let git = LocalGit::new(&dir);
    git.remote_op(
        crate::ops::RemoteOp::Add,
        "origin",
        Some("https://github.com/o/old.git"),
    )
    .await
    .unwrap();
    let account = seed_account(&ctx, &user, GitProviderKind::Github, "gh").await;
    let repo = ctx
        .store
        .create_repo(NewRepo {
            workspace_id: ws,
            name: "remote-identity".into(),
            path: dir.to_string_lossy().into_owned(),
            remote_url: Some("https://github.com/o/old.git".into()),
            provider: Some(GitProviderKind::Github),
            git_account_id: Some(account.id),
        })
        .await
        .unwrap();
    // External CLI edits are not mediated by a server route.
    git.remote_op(
        crate::ops::RemoteOp::SetUrl,
        "origin",
        Some("https://github.com/o/new.git"),
    )
    .await
    .unwrap();
    let (_, remote) = provider_ctx(&ctx, &auth(&user, false), &repo)
        .await
        .unwrap();
    assert_eq!(remote.repo, "new");
    assert_eq!(
        ctx.store
            .get_repo(&repo.id)
            .await
            .unwrap()
            .remote_url
            .as_deref(),
        Some("https://github.com/o/new.git")
    );
    git.remote_op(
        crate::ops::RemoteOp::SetUrl,
        "origin",
        Some("https://gitlab.com/o/new.git"),
    )
    .await
    .unwrap();
    assert!(
        matches!(
            provider_ctx(&ctx, &auth(&user, false), &repo).await,
            Err(Error::Invalid(_))
        ),
        "old GitHub account cannot serve the new GitLab origin"
    );
    git.remote_op(crate::ops::RemoteOp::Remove, "origin", None)
        .await
        .unwrap();
    assert!(matches!(
        provider_ctx(&ctx, &auth(&user, false), &repo).await,
        Err(Error::Invalid(_))
    ));
    assert!(ctx
        .store
        .get_repo(&repo.id)
        .await
        .unwrap()
        .remote_url
        .is_none());
    let _ = tokio::fs::remove_dir_all(&dir).await;
}

#[tokio::test]
async fn remote_edit_route_persists_origin_and_force_requires_a_target() {
    use tower::ServiceExt;
    let (_pool, ctx, user, ws) = fixture().await;
    let dir = init_git_repo().await;
    let repo = ctx
        .store
        .create_repo(NewRepo {
            workspace_id: ws,
            name: "remote-edit".into(),
            path: dir.to_string_lossy().into_owned(),
            remote_url: None,
            provider: None,
            git_account_id: None,
        })
        .await
        .unwrap();
    let app = crate::ops::router()
        .layer(Extension(auth(&user, false)))
        .with_state(ctx.clone());
    let request = axum::http::Request::builder()
        .method("POST")
        .uri(format!("/repos/{}/remotes", repo.id))
        .header("content-type", "application/json")
        .body(axum::body::Body::from(
            r#"{"op":"add","name":"origin","url":"https://github.com/o/new.git"}"#,
        ))
        .unwrap();
    assert_eq!(app.oneshot(request).await.unwrap().status(), StatusCode::OK);
    let stored = ctx.store.get_repo(&repo.id).await.unwrap();
    assert_eq!(
        stored.remote_url.as_deref(),
        Some("https://github.com/o/new.git")
    );
    // No outbound call: an unbound force request must fail before Git runs.
    let error = repo_push(
        State(ctx),
        Extension(auth(&user, false)),
        Path(repo.id),
        Some(Json(PushReq {
            force_with_lease: true,
            ..Default::default()
        })),
    )
    .await
    .unwrap_err();
    assert!(matches!(&error.0, Error::Conflict(message) if message.contains("captured target")));
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

/// Process-local secret store that remembers what is stored.
#[derive(Default)]
struct MemSecrets(std::sync::Mutex<HashMap<String, String>>);
impl SecretStore for MemSecrets {
    fn put(&self, k: &str, v: &str) -> Result<()> {
        self.0.lock().unwrap().insert(k.into(), v.into());
        Ok(())
    }
    fn get(&self, k: &str) -> Result<Option<String>> {
        Ok(self.0.lock().unwrap().get(k).cloned())
    }
    fn delete(&self, k: &str) -> Result<()> {
        self.0.lock().unwrap().remove(k);
        Ok(())
    }
}

/// S2-313: a token rotation whose DB update fails keeps the OLD secret
/// (the row still names it) and drops the orphaned new one; a successful
/// rotation deletes the old secret only after the row points at the new.
#[tokio::test]
async fn token_rotation_never_strands_the_row_on_a_deleted_secret() {
    let (pool, mut ctx, user, _ws) = fixture().await;
    let secrets = Arc::new(MemSecrets::default());
    ctx.secrets = secrets.clone();
    let acct = seed_account(&ctx, &user, GitProviderKind::Github, "rot").await;
    secrets.put(&acct.token_ref, "old-token").unwrap();
    let req = || UpdateGitAccountReq {
        label: None,
        username: None,
        namespace: None,
        api_base_url: None,
        token: Some("new-token".into()),
        token_expires_at: None,
    };

    sqlx::query(
        "CREATE TRIGGER fail_acct_update BEFORE UPDATE ON git_accounts \
         BEGIN SELECT RAISE(FAIL, 'db down'); END",
    )
    .execute(&pool)
    .await
    .unwrap();
    let r = update_account(
        State(ctx.clone()),
        Extension(auth(&user, false)),
        Path(acct.id.clone()),
        Json(req()),
    )
    .await;
    assert!(r.is_err());
    let held = secrets.0.lock().unwrap().clone();
    assert_eq!(
        held.get(&acct.token_ref).map(String::as_str),
        Some("old-token")
    );
    assert_eq!(
        held.len(),
        1,
        "the new secret must not be orphaned: {held:?}"
    );

    sqlx::query("DROP TRIGGER fail_acct_update")
        .execute(&pool)
        .await
        .unwrap();
    let Json(updated) = update_account(
        State(ctx.clone()),
        Extension(auth(&user, false)),
        Path(acct.id.clone()),
        Json(req()),
    )
    .await
    .unwrap();
    assert_ne!(updated.token_ref, acct.token_ref);
    let held = secrets.0.lock().unwrap().clone();
    assert!(!held.contains_key(&acct.token_ref), "old secret removed");
    assert_eq!(
        held.get(&updated.token_ref).map(String::as_str),
        Some("new-token")
    );
}

/// S2-311: `api_base_url` must be an https forge URL — never loopback,
/// link-local / metadata, credentials, a query or a fragment; private
/// ranges (self-hosted GitLab) stay allowed, http only with the opt-out.
#[test]
fn api_base_url_refuses_internal_targets() {
    for ok in [
        "https://gitlab.example.com",
        "https://10.0.0.5/gitlab",
        "https://192.168.1.20:8443",
    ] {
        assert!(api_base_url_shape(ok, false).is_ok(), "{ok}");
    }
    for bad in [
        "http://gitlab.example.com",
        "https://127.0.0.1",
        "https://localhost:7700",
        "https://169.254.169.254/latest",
        "https://[::1]",
        "https://[::ffff:127.0.0.1]",
        "https://0.0.0.0",
        "https://user:pw@gitlab.example.com",
        "https://gitlab.example.com/?x=1",
        "https://gitlab.example.com/#frag",
        "file:///etc/passwd",
        "ftp://gitlab.example.com",
    ] {
        assert!(api_base_url_shape(bad, false).is_err(), "{bad}");
    }
    assert!(api_base_url_shape("http://gitlab.corp", true).is_ok());
    assert!(api_base_url_shape("http://127.0.0.1", true).is_err());
}
