//! Session-scoped working-folder files (PC-1, access-control-4-03) through the
//! REAL daemon router on a loopback port:
//! `GET /sessions/{id}/files/{browse,read}` and the `/ws/lsp` root rule.
//!
//! What it pins:
//! - a non-root owner lists and reads their session's cwd, which lies in an
//!   agent work area of Otto's data dir (`<data>/personal/<agent>`);
//! - another non-admin member of the same workspace is refused (403);
//! - `../../otto.db` and an absolute path are 400;
//! - `/ws/lsp` rooted at the owner's persona dir is not refused for access,
//!   and the same root is 403 for another member.

use chrono::Utc;
use otto_core::domain::{Capability, Feature, SessionKind};
use otto_rbac::AuthRepo;
use otto_state::{DbPool, GrantsRepo, NewSession, SessionsRepo};
use serde_json::Value;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

struct Daemon {
    base: String,
    http: reqwest::Client,
    /// Session owner (ws Editor).
    carol: String,
    /// Another ws Editor.
    dave: String,
    session: String,
    persona: std::path::PathBuf,
    _tmp: tempfile::TempDir,
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
    for (u, r) in [("root", true), ("carol", false), ("dave", false)] {
        seed_user(&pool, u, r).await;
    }
    // The workspace root must NOT contain the temp data dir: on Linux the
    // tempdir sits under `/tmp`, so a `/tmp` root would let any ws member's
    // LSP pass as "inside a workspace" (the global data-dir deny never sees
    // this test's data dir) — macOS hid it with `/var/folders`.
    let ws_root = tmp.path().join("ws-root");
    std::fs::create_dir_all(&ws_root).unwrap();
    sqlx::query(
        "INSERT INTO workspaces (id, name, root_path, settings_json, archived, created_at)
         VALUES ('ws1', 'ws1', ?, '{}', 0, ?)",
    )
    .bind(ws_root.to_string_lossy().into_owned())
    .bind(Utc::now().to_rfc3339())
    .execute(&pool)
    .await
    .expect("seed workspace");
    for (u, role) in [("root", "admin"), ("carol", "editor"), ("dave", "editor")] {
        sqlx::query(
            "INSERT INTO workspace_members (workspace_id, user_id, role) VALUES ('ws1', ?, ?)",
        )
        .bind(u)
        .bind(role)
        .execute(&pool)
        .await
        .expect("member");
    }
    let grants = GrantsRepo::new(pool.clone());
    for u in ["carol", "dave"] {
        grants
            .set_grants(u, &[(Feature::Agents, Capability::Edit)])
            .await
            .unwrap();
    }

    // Carol's personal-agent chat: its cwd is the persona dir in the data dir.
    let persona = tmp.path().join("personal").join("agent-1");
    std::fs::create_dir_all(persona.join("memory")).unwrap();
    std::fs::write(persona.join("memory/notes.md"), "- likes tea\n").unwrap();
    std::fs::write(persona.join("AGENTS.md"), "# Scout\n").unwrap();
    let session = SessionsRepo::new(pool.clone())
        .create(NewSession {
            workspace_id: "ws1".into(),
            kind: SessionKind::Agent,
            provider: "claude".into(),
            title: "Scout chat".into(),
            cwd: persona.to_string_lossy().into_owned(),
            provider_session_id: None,
            connection_id: None,
            created_by: "carol".into(),
            meta: serde_json::json!({}),
        })
        .await
        .unwrap()
        .id;

    let auth = AuthRepo::new(pool.clone());
    let mut tokens = Vec::new();
    for u in ["carol", "dave"] {
        tokens.push(
            auth.issue_api_token(&u.to_string(), Some("ui"))
                .await
                .unwrap()
                .0,
        );
    }
    let [carol, dave]: [String; 2] = tokens.try_into().unwrap();

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
        carol,
        dave,
        session,
        persona,
        _tmp: tmp,
    }
}

impl Daemon {
    async fn get(&self, token: &str, op: &str, path: &str) -> (u16, Value) {
        let resp = self
            .http
            .get(format!(
                "{}/api/v1/sessions/{}/files/{op}",
                self.base, self.session
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

    /// The pre-upgrade status of `/ws/lsp` rooted at `root`.
    async fn lsp_status(&self, token: &str, root: &std::path::Path) -> u16 {
        self.http
            .get(format!("{}/ws/lsp", self.base))
            // An unknown language: the access check runs first, so a refused
            // root is 403 and an allowed one is 400 — no server is spawned.
            .query(&[
                ("lang", "otto-test-none"),
                ("root", &root.to_string_lossy()),
            ])
            .header("Connection", "Upgrade")
            .header("Upgrade", "websocket")
            .header("Sec-WebSocket-Version", "13")
            .header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==")
            .header("Sec-WebSocket-Protocol", format!("otto-bearer, {token}"))
            .send()
            .await
            .unwrap()
            .status()
            .as_u16()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_owner_lists_and_reads_their_sessions_working_folder() {
    let d = boot().await;
    let (st, body) = d.get(&d.carol, "browse", "").await;
    assert_eq!(st, 200, "{body}");
    assert_eq!(body["path"], "");
    let paths: Vec<&str> = body["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, vec!["memory", "AGENTS.md"], "{body}");

    let (st, body) = d.get(&d.carol, "read", "memory/notes.md").await;
    assert_eq!(st, 200, "{body}");
    assert_eq!(body["content"], "- likes tea\n");
    assert_eq!(body["path"], "memory/notes.md");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn another_member_is_refused() {
    let d = boot().await;
    for op in ["browse", "read"] {
        let (st, body) = d.get(&d.dave, op, "AGENTS.md").await;
        assert_eq!(st, 403, "{op}: {body}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn escapes_are_400() {
    let d = boot().await;
    for bad in ["../../otto.db", "../otto.db", "/etc/passwd"] {
        let (st, body) = d.get(&d.carol, "read", bad).await;
        assert_eq!(st, 400, "{bad}: {body}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lsp_opens_on_the_owners_persona_dir_only() {
    let d = boot().await;
    // The owner passes the access check and reaches the language lookup.
    assert_eq!(d.lsp_status(&d.carol, &d.persona).await, 400);
    // Another member rooted at Carol's persona dir is refused.
    assert_eq!(d.lsp_status(&d.dave, &d.persona).await, 403);
}
