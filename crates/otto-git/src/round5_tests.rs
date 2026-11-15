use std::path::Path as FsPath;
use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use axum::Extension;
use chrono::Utc;
use otto_core::api::RepoStatusResp;
use otto_core::auth::{AuthUser, BoxFuture, RoleChecker};
use otto_core::domain::{Repo, User, WorkspaceRole};
use otto_core::secrets::SecretStore;
use otto_core::{new_id, Id, Result};
use otto_state::{DbPool, GitStore, WorkspacesRepo};
use serde_json::json;
use tower::ServiceExt;

use super::RepoFileResp;
use crate::http::{router, GitCtx};

struct DummySecrets;
impl SecretStore for DummySecrets {
    fn put(&self, _k: &str, _v: &str) -> Result<()> {
        Ok(())
    }
    fn get(&self, _k: &str) -> Result<Option<String>> {
        Ok(None)
    }
    fn delete(&self, _k: &str) -> Result<()> {
        Ok(())
    }
}

struct AllowAllRoles;
impl RoleChecker for AllowAllRoles {
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
struct Round5Ctx {
    store: GitStore,
    workspaces: WorkspacesRepo,
    secrets: Arc<dyn SecretStore>,
    roles: Arc<dyn RoleChecker>,
    events: tokio::sync::broadcast::Sender<otto_core::event::Event>,
}

impl GitCtx for Round5Ctx {
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

async fn setup_ctx() -> (Round5Ctx, AuthUser, Id) {
    let opts = sqlx::sqlite::SqliteConnectOptions::new()
        .in_memory(true)
        .foreign_keys(true);
    let raw_pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .unwrap();
    sqlx::migrate!("../otto-state/migrations")
        .run(&raw_pool)
        .await
        .unwrap();
    let pool: DbPool = raw_pool.into();
    let uid = new_id();
    let wid = new_id();
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO users (id, username, password_hash, display_name, is_root, created_at)
         VALUES (?, 'u1', 'h', 'u1', 0, ?)",
    )
    .bind(&uid)
    .bind(&now)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO workspaces (id, name, root_path, created_at) VALUES (?, 'ws', '/tmp', ?)",
    )
    .bind(&wid)
    .bind(&now)
    .execute(&pool)
    .await
    .unwrap();

    let user = AuthUser(User {
        id: uid.clone(),
        username: "u1".into(),
        display_name: "u1".into(),
        is_root: false,
        disabled: false,
        created_at: Utc::now(),
    });
    let ctx = Round5Ctx {
        store: GitStore::new(pool.clone()),
        workspaces: WorkspacesRepo::new(pool),
        secrets: Arc::new(DummySecrets),
        roles: Arc::new(AllowAllRoles),
        events: tokio::sync::broadcast::channel(16).0,
    };
    (ctx, user, wid)
}

fn git_sh(dir: &FsPath, args: &[&str]) {
    let out = crate::hardened_std_command()
        .current_dir(dir)
        .args(args)
        .output()
        .expect("spawn git");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn init_repo_at(dir: &FsPath) {
    git_sh(dir, &["init", "-q", "-b", "main"]);
    git_sh(dir, &["config", "user.email", "test@otto.local"]);
    git_sh(dir, &["config", "user.name", "Otto Test"]);
    git_sh(dir, &["config", "commit.gpgsign", "false"]);
    git_sh(dir, &["config", "core.autocrlf", "false"]);
}

#[tokio::test]
async fn detect_and_register_repo_reject_broad_roots_and_accept_nested_subrepo() {
    let (ctx, user, ws) = setup_ctx().await;
    let app = router::<Round5Ctx>()
        .with_state(ctx.clone())
        .layer(Extension(user));

    for broad in ["/tmp", "~", "/"] {
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/workspaces/{ws}/repos/detect"))
                    .header("content-type", "application/json")
                    .body(Body::from(json!({ "path": broad }).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::BAD_REQUEST,
            "detect_repo must reject broad root {broad}"
        );

        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/workspaces/{ws}/repos"))
                    .header("content-type", "application/json")
                    .body(Body::from(json!({ "path": broad }).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::BAD_REQUEST,
            "register_repo must reject broad root {broad}"
        );
    }

    // Nested repo inside a parent directory that also has `.git` resolves to the nested subrepo.
    let tmp = tempfile::tempdir().unwrap();
    let canon = tmp.path().canonicalize().unwrap();
    let outer = canon.join("outer");
    let subrepo = outer.join("subrepo");
    let sub_src = subrepo.join("src");
    std::fs::create_dir_all(&sub_src).unwrap();
    init_repo_at(&outer);
    init_repo_at(&subrepo);

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/workspaces/{ws}/repos/detect"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({ "path": sub_src.to_string_lossy() }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
    let detected: Repo = serde_json::from_slice(&body).unwrap();
    assert_eq!(detected.path, subrepo.to_string_lossy());
}

#[tokio::test]
async fn status_fresh_query_bypasses_memo_immediately() {
    let (ctx, user, ws) = setup_ctx().await;
    let tmp = tempfile::tempdir().unwrap();
    let repo_dir = tmp.path().canonicalize().unwrap();
    init_repo_at(&repo_dir);
    std::fs::write(repo_dir.join("tracked.txt"), "v1\n").unwrap();
    git_sh(&repo_dir, &["add", "tracked.txt"]);
    git_sh(&repo_dir, &["commit", "-q", "-m", "init"]);

    let app = router::<Round5Ctx>().with_state(ctx).layer(Extension(user));

    let reg = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/workspaces/{ws}/repos"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({ "path": repo_dir.to_string_lossy() }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(reg.status(), StatusCode::OK);
    let repo: Repo =
        serde_json::from_slice(&to_bytes(reg.into_body(), 64 * 1024).await.unwrap()).unwrap();

    // First status call warms the 1s status_cache memo.
    let r1 = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/repos/{}/status", repo.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r1.status(), StatusCode::OK);
    let s1: RepoStatusResp =
        serde_json::from_slice(&to_bytes(r1.into_body(), 64 * 1024).await.unwrap()).unwrap();
    assert!(s1.changes.is_empty());

    // Mutate tracked + untracked files immediately within the 1s memo window.
    std::fs::write(repo_dir.join("tracked.txt"), "v2\n").unwrap();
    std::fs::write(repo_dir.join("new_untracked.txt"), "hello\n").unwrap();

    // `?fresh=1` bypasses the memo immediately.
    let r_fresh = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/repos/{}/status?fresh=1", repo.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r_fresh.status(), StatusCode::OK);
    assert_eq!(
        r_fresh
            .headers()
            .get("x-otto-status-cache")
            .and_then(|v| v.to_str().ok()),
        Some("miss")
    );
    let sf: RepoStatusResp =
        serde_json::from_slice(&to_bytes(r_fresh.into_body(), 64 * 1024).await.unwrap()).unwrap();
    assert_eq!(sf.changes.len(), 2);

    // `?fresh=true` also parses and bypasses memo.
    std::fs::write(repo_dir.join("another.txt"), "3\n").unwrap();
    let r_fresh_bool = app
        .oneshot(
            Request::builder()
                .uri(format!("/repos/{}/status?fresh=true", repo.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r_fresh_bool.status(), StatusCode::OK);
    let sf2: RepoStatusResp =
        serde_json::from_slice(&to_bytes(r_fresh_bool.into_body(), 64 * 1024).await.unwrap())
            .unwrap();
    assert_eq!(sf2.changes.len(), 3);
}

#[tokio::test]
async fn repo_file_reads_exact_head_blob_crlf_rename_binary_untracked_and_unborn() {
    let (ctx, user, ws) = setup_ctx().await;
    let tmp = tempfile::tempdir().unwrap();
    let repo_dir = tmp.path().canonicalize().unwrap();
    init_repo_at(&repo_dir);

    let app = router::<Round5Ctx>().with_state(ctx).layer(Extension(user));

    let reg = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/workspaces/{ws}/repos"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({ "path": repo_dir.to_string_lossy() }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let repo: Repo =
        serde_json::from_slice(&to_bytes(reg.into_body(), 64 * 1024).await.unwrap()).unwrap();

    // Unborn HEAD returns 200 with exists: false.
    std::fs::write(repo_dir.join("draft.txt"), "uncommitted").unwrap();
    let unborn_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/repos/{}/file?path=draft.txt", repo.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unborn_resp.status(), StatusCode::OK);
    let unborn_body: RepoFileResp =
        serde_json::from_slice(&to_bytes(unborn_resp.into_body(), 64 * 1024).await.unwrap())
            .unwrap();
    assert!(!unborn_body.exists);
    assert_eq!(unborn_body.content, None);

    // Commit CRLF without trailing newline, staged file, renamed file, and binary file.
    std::fs::remove_file(repo_dir.join("draft.txt")).unwrap();
    std::fs::write(repo_dir.join("crlf.txt"), b"alpha\r\nbeta").unwrap();
    std::fs::write(repo_dir.join("staged.txt"), b"clean staged\n").unwrap();
    std::fs::write(repo_dir.join("before_rename.txt"), b"orig\r\ncontent").unwrap();
    std::fs::write(repo_dir.join("blob.bin"), b"hdr\0payload").unwrap();
    git_sh(
        &repo_dir,
        &[
            "add",
            "crlf.txt",
            "staged.txt",
            "before_rename.txt",
            "blob.bin",
        ],
    );
    git_sh(&repo_dir, &["commit", "-q", "-m", "initial"]);

    // Modify working tree, stage an edit, stage a rename, and create an untracked file.
    std::fs::write(repo_dir.join("crlf.txt"), b"modified\n").unwrap();
    std::fs::write(repo_dir.join("staged.txt"), b"dirty index\n").unwrap();
    git_sh(&repo_dir, &["add", "staged.txt"]);
    git_sh(&repo_dir, &["mv", "before_rename.txt", "after_rename.txt"]);
    std::fs::write(repo_dir.join("after_rename.txt"), b"orig\r\ncontent_edited").unwrap();
    std::fs::write(repo_dir.join("untracked.txt"), b"brand new\n").unwrap();

    let get_file = |q: &str| {
        let app = app.clone();
        let rid = repo.id.clone();
        let uri = format!("/repos/{rid}/file?{q}");
        async move {
            let r = app
                .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            let status = r.status();
            let bytes = to_bytes(r.into_body(), 64 * 1024).await.unwrap();
            (status, bytes)
        }
    };

    let (st, b) = get_file("path=crlf.txt").await;
    assert_eq!(st, StatusCode::OK);
    let crlf: RepoFileResp = serde_json::from_slice(&b).unwrap();
    assert!(crlf.exists);
    assert_eq!(crlf.content.as_deref(), Some("alpha\r\nbeta"));

    let (_, b) = get_file("path=staged.txt").await;
    let staged: RepoFileResp = serde_json::from_slice(&b).unwrap();
    assert_eq!(staged.content.as_deref(), Some("clean staged\n"));

    let (_, b) = get_file("path=after_rename.txt").await;
    let renamed: RepoFileResp = serde_json::from_slice(&b).unwrap();
    assert!(renamed.exists);
    assert_eq!(renamed.content.as_deref(), Some("orig\r\ncontent"));

    let (_, b) = get_file("path=blob.bin").await;
    let bin: RepoFileResp = serde_json::from_slice(&b).unwrap();
    assert!(bin.exists && bin.binary && bin.content.is_none());

    let (_, b) = get_file("path=untracked.txt").await;
    let untracked: RepoFileResp = serde_json::from_slice(&b).unwrap();
    assert!(!untracked.exists && untracked.content.is_none());

    // Reject path traversal, leading `-`, and NUL.
    for bad in [
        "path=../etc/passwd",
        "path=-option.txt",
        "path=a%00b.txt",
        "rev=-flag&path=crlf.txt",
    ] {
        let (st, _) = get_file(bad).await;
        assert_eq!(st, StatusCode::BAD_REQUEST, "expected 400 for {bad}");
    }
}
