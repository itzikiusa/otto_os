//! Run-scoped context files (PC-1, access-control-3-04) through the REAL daemon
//! router on a loopback port: `GET /workflow-runs/{id}/context/{browse,read}`.
//!
//! What it pins:
//! - a non-root workspace **Viewer** reads their run's `final-output.md` and
//!   lists its context dir (through `/fs/*` only root could: the context dir
//!   lives inside Otto's data dir, which `/fs/*` refuses non-root callers);
//! - a member of ANOTHER workspace is refused (403/404), an unknown run is 404;
//! - `../x`, an absolute path and a symlink escaping the context dir are 400.

use chrono::Utc;
use otto_core::domain::{Capability, Feature};
use otto_rbac::AuthRepo;
use otto_state::{DbPool, GrantsRepo};
use serde_json::Value;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

struct Daemon {
    base: String,
    http: reqwest::Client,
    /// Viewer of ws1 (the run's workspace).
    viewer: String,
    /// Member of ws2 only.
    outsider: String,
    root: String,
    run: String,
    tmp: tempfile::TempDir,
}

async fn seed_user(pool: &DbPool, id: &str, is_root: bool) {
    sqlx::query(
        "INSERT INTO users (id, username, password_hash, display_name, is_root, created_at)
         VALUES (?, ?, 'x', ?, ?, ?)",
    )
    .bind(id)
    .bind(id)
    .bind(id)
    .bind(is_root as i64)
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await
    .expect("seed user");
}

async fn seed_workspace(pool: &DbPool, ws: &str, members: &[(&str, &str)]) {
    sqlx::query(
        "INSERT INTO workspaces (id, name, root_path, settings_json, archived, created_at)
         VALUES (?, ?, '/tmp', '{}', 0, ?)",
    )
    .bind(ws)
    .bind(ws)
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await
    .expect("seed workspace");
    for (user, role) in members {
        sqlx::query("INSERT INTO workspace_members (workspace_id, user_id, role) VALUES (?, ?, ?)")
            .bind(ws)
            .bind(user)
            .bind(role)
            .execute(pool)
            .await
            .expect("member");
    }
}

async fn boot() -> Daemon {
    let tmp = tempfile::tempdir().unwrap();
    let opts = SqliteConnectOptions::new()
        .filename(tmp.path().join("otto.db"))
        .create_if_missing(true)
        .foreign_keys(true);
    let raw = SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(opts)
        .await
        .expect("connect sqlite");
    sqlx::migrate!("../otto-state/migrations")
        .run(&raw)
        .await
        .expect("run migrations");
    let pool: DbPool = raw.into();
    seed_user(&pool, "root", true).await;
    seed_user(&pool, "vera", false).await;
    seed_user(&pool, "olaf", false).await;
    seed_workspace(&pool, "ws1", &[("root", "admin"), ("vera", "viewer")]).await;
    seed_workspace(&pool, "ws2", &[("root", "admin"), ("olaf", "admin")]).await;
    let grants = GrantsRepo::new(pool.clone());
    for user in ["vera", "olaf"] {
        grants
            .set_grants(user, &[(Feature::Workflows, Capability::View)])
            .await
            .unwrap();
    }
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO workflows (id, workspace_id, name, created_by, created_at, updated_at)
         VALUES ('wf1', 'ws1', 'Ship', 'root', ?, ?)",
    )
    .bind(&now)
    .bind(&now)
    .execute(&pool)
    .await
    .expect("seed workflow");
    sqlx::query(
        "INSERT INTO workflow_runs (id, workflow_id, workspace_id, status, started_at)
         VALUES ('run1', 'wf1', 'ws1', 'success', ?)",
    )
    .bind(&now)
    .execute(&pool)
    .await
    .expect("seed run");

    // The run's context dir inside the daemon's data dir, plus a secret just
    // outside it that a symlink tries to reach.
    let ctx_dir = tmp.path().join("workflow-context").join("run1");
    std::fs::create_dir_all(ctx_dir.join("nodes")).unwrap();
    std::fs::write(ctx_dir.join("final-output.md"), "# Deliverable\n").unwrap();
    std::fs::write(ctx_dir.join("nodes/plan.md"), "plan").unwrap();
    std::fs::write(tmp.path().join("workflow-context/secret.txt"), "s3cret").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        tmp.path().join("workflow-context/secret.txt"),
        ctx_dir.join("escape.txt"),
    )
    .unwrap();

    let auth = AuthRepo::new(pool.clone());
    let mut tokens = Vec::new();
    for user in ["vera", "olaf", "root"] {
        tokens.push(
            auth.issue_api_token(&user.to_string(), Some("ui"))
                .await
                .unwrap()
                .0,
        );
    }
    let [viewer, outsider, root]: [String; 3] = tokens.try_into().unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let mut ctx = otto_server::ServerCtx::for_tests(&pool, tmp.path()).await;
    ctx.base_url = base.clone();
    let (api_extras, root_extras) = otto_server::modules::module_routers(&ctx);
    let app = otto_server::build_router(ctx, api_extras, root_extras);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Daemon {
        base,
        http: reqwest::Client::new(),
        viewer,
        outsider,
        root,
        run: "run1".into(),
        tmp,
    }
}

impl Daemon {
    async fn get(&self, token: &str, run: &str, op: &str, path: &str) -> (u16, Value) {
        let resp = self
            .http
            .get(format!(
                "{}/api/v1/workflow-runs/{run}/context/{op}",
                self.base
            ))
            .query(&[("path", path)])
            .bearer_auth(token)
            .send()
            .await
            .unwrap();
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        (status, serde_json::from_str(&text).unwrap_or(Value::Null))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_non_root_viewer_reads_and_lists_their_runs_context_files() {
    let d = boot().await;
    let (st, body) = d.get(&d.viewer, &d.run, "read", "final-output.md").await;
    assert_eq!(st, 200, "{body}");
    assert_eq!(body["content"], "# Deliverable\n");
    assert_eq!(body["path"], "final-output.md");
    assert_eq!(body["language"], "markdown");
    assert_eq!(body["truncated"], false);

    let (st, body) = d.get(&d.viewer, &d.run, "browse", "").await;
    assert_eq!(st, 200, "{body}");
    assert_eq!(body["path"], "");
    assert_eq!(body["parent"], Value::Null);
    let paths: Vec<&str> = body["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["path"].as_str().unwrap())
        .collect();
    // Relative paths, dirs first; the escaping symlink is not listed.
    assert_eq!(paths, vec!["nodes", "final-output.md"], "{body}");

    let (st, body) = d.get(&d.viewer, &d.run, "browse", "nodes").await;
    assert_eq!(st, 200, "{body}");
    assert_eq!(body["parent"], "");
    assert_eq!(body["entries"][0]["path"], "nodes/plan.md");

    // Root passes too; a missing file is 404.
    let (st, _) = d.get(&d.root, &d.run, "read", "final-output.md").await;
    assert_eq!(st, 200);
    let (st, _) = d.get(&d.viewer, &d.run, "read", "nope.md").await;
    assert_eq!(st, 404);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn another_workspace_and_unknown_runs_are_refused() {
    let d = boot().await;
    let (st, body) = d.get(&d.outsider, &d.run, "read", "final-output.md").await;
    assert!(st == 403 || st == 404, "{st}: {body}");
    assert!(
        !body.to_string().contains("Deliverable"),
        "outsider must not see content: {body}"
    );
    let (st, _) = d.get(&d.outsider, &d.run, "browse", "").await;
    assert!(st == 403 || st == 404, "{st}");
    let (st, _) = d
        .get(&d.viewer, "no-such-run", "read", "final-output.md")
        .await;
    assert_eq!(st, 404);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn escaping_paths_are_400() {
    let d = boot().await;
    for bad in ["../secret.txt", "../../otto.db", "nodes/../../secret.txt"] {
        let (st, body) = d.get(&d.viewer, &d.run, "read", bad).await;
        assert_eq!(st, 400, "{bad}: {body}");
    }
    let abs = d.tmp.path().join("workflow-context/secret.txt");
    let (st, body) = d
        .get(&d.viewer, &d.run, "read", &abs.to_string_lossy())
        .await;
    assert_eq!(st, 400, "absolute: {body}");
    let (st, body) = d.get(&d.viewer, &d.run, "browse", "..").await;
    assert_eq!(st, 400, "browse ..: {body}");
    #[cfg(unix)]
    {
        let (st, body) = d.get(&d.viewer, &d.run, "read", "escape.txt").await;
        assert_eq!(st, 400, "symlink escape: {body}");
        assert!(!body.to_string().contains("s3cret"), "{body}");
    }
}
