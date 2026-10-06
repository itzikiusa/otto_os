//! HTTP-level integration tests for the Workbench routes
//! (`/workspaces/{ws}/workbench/...`): revision coalescing + checkpoint seal,
//! revisions / diff / restore, per-owner isolation, the trash → permanent
//! delete lifecycle, and image assets.
//!
//! Same "real minimal ServerCtx" harness as `canvas_refs_api.rs` (in-memory
//! sqlite with every migration, stub secrets/spawner); requests go through the
//! real handlers via `tower::ServiceExt::oneshot` with the `AuthUser`
//! extension injected as the production auth middleware does.

use axum::body::Body;
use axum::extract::Request;
use axum::http::{header, Method, StatusCode};
use axum::Router;
use chrono::Utc;
use http_body_util::BodyExt;
use otto_core::auth::AuthUser;
use otto_core::domain::User;
use otto_server::ServerCtx;
use otto_state::DbPool;
use serde_json::{json, Value};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use tower::ServiceExt; // for `oneshot`

// ---------------------------------------------------------------------------
// Database pool + fixtures
// ---------------------------------------------------------------------------

async fn mem_pool() -> DbPool {
    let opts = SqliteConnectOptions::new()
        .in_memory(true)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .expect("connect in-memory sqlite");
    sqlx::migrate!("../otto-state/migrations")
        .run(&pool)
        .await
        .expect("run migrations");
    pool.into()
}

fn user(id: &str, is_root: bool) -> User {
    User {
        id: id.into(),
        username: id.into(),
        display_name: id.into(),
        is_root,
        disabled: false,
        created_at: Utc::now(),
    }
}

async fn seed_user(pool: &DbPool, id: &str, is_root: bool) {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO users (id, username, password_hash, display_name, is_root, created_at)
         VALUES (?, ?, 'x', ?, ?, ?)",
    )
    .bind(id)
    .bind(id)
    .bind(id)
    .bind(is_root as i64)
    .bind(&now)
    .execute(pool)
    .await
    .expect("seed user");
}

async fn seed_workspace(pool: &DbPool, ws_id: &str) {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO workspaces (id, name, root_path, settings_json, archived, created_at)
         VALUES (?, 'ws', '/tmp', '{}', 0, ?)",
    )
    .bind(ws_id)
    .bind(&now)
    .execute(pool)
    .await
    .expect("seed workspace");
}

async fn set_member(pool: &DbPool, ws_id: &str, user_id: &str, role: &str) {
    sqlx::query("INSERT INTO workspace_members (workspace_id, user_id, role) VALUES (?, ?, ?)")
        .bind(ws_id)
        .bind(user_id)
        .bind(role)
        .execute(pool)
        .await
        .expect("set member");
}

// ---------------------------------------------------------------------------
// Minimal ServerCtx construction (mirrors activity_isolation.rs::test_ctx)
// ---------------------------------------------------------------------------

async fn test_ctx(pool: &DbPool) -> ServerCtx {
    ServerCtx::for_tests(pool, "/tmp/otto-test-workbench-api").await
}

/// Minimal router exposing only the workbench endpoints at their production paths.
fn workbench_router(ctx: ServerCtx) -> Router {
    Router::new()
        .merge(otto_server::routes::workbench::workbench_routes())
        .with_state(ctx)
}

struct Resp {
    status: StatusCode,
    content_type: Option<String>,
    body: Vec<u8>,
}

impl Resp {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or_else(|e| {
            panic!(
                "non-JSON body ({e}) for {}: {}",
                self.status,
                String::from_utf8_lossy(&self.body)
            )
        })
    }
}

async fn send(
    app: &Router,
    caller: &User,
    method: Method,
    uri: &str,
    content_type: Option<&str>,
    body: Vec<u8>,
) -> Resp {
    let mut b = Request::builder().method(method).uri(uri);
    if let Some(ct) = content_type {
        b = b.header("content-type", ct);
    }
    let mut req = b.body(Body::from(body)).unwrap();
    req.extensions_mut().insert(AuthUser(caller.clone()));
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let content_type = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let body = resp
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec();
    Resp {
        status,
        content_type,
        body,
    }
}

async fn get(app: &Router, u: &User, uri: &str) -> Resp {
    send(app, u, Method::GET, uri, None, Vec::new()).await
}

async fn post(app: &Router, u: &User, uri: &str, body: Value) -> Resp {
    let bytes = serde_json::to_vec(&body).unwrap();
    send(app, u, Method::POST, uri, Some("application/json"), bytes).await
}

async fn patch(app: &Router, u: &User, uri: &str, body: Value) -> Resp {
    let bytes = serde_json::to_vec(&body).unwrap();
    send(app, u, Method::PATCH, uri, Some("application/json"), bytes).await
}

async fn delete(app: &Router, u: &User, uri: &str) -> Resp {
    send(app, u, Method::DELETE, uri, None, Vec::new()).await
}

/// One workspace `ws1` with `alice` and `bob` as editors; returns the app.
async fn setup() -> Router {
    let pool = mem_pool().await;
    seed_user(&pool, "alice", false).await;
    seed_user(&pool, "bob", false).await;
    seed_workspace(&pool, "ws1").await;
    set_member(&pool, "ws1", "alice", "editor").await;
    set_member(&pool, "ws1", "bob", "editor").await;
    workbench_router(test_ctx(&pool).await)
}

const DOCS: &str = "/workspaces/ws1/workbench/docs";

async fn create_doc(app: &Router, u: &User, name: &str, content: &str) -> String {
    let r = post(
        app,
        u,
        DOCS,
        json!({ "name": name, "language": "txt", "content": content }),
    )
    .await;
    assert_eq!(
        r.status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&r.body)
    );
    let doc = r.json();
    assert_eq!(doc["content"], content);
    assert_eq!(doc["rev"], 1);
    doc["id"].as_str().unwrap().to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Autosaves coalesce into one `auto` revision, an identical-content
/// checkpoint seals it, the next save opens a new one; revisions / revision
/// detail / diff / restore round-trip over HTTP.
#[tokio::test]
async fn revisions_coalesce_seal_diff_and_restore() {
    let app = setup().await;
    let alice = user("alice", false);
    let id = create_doc(&app, &alice, "notes.txt", "a\n").await;
    let doc_uri = format!("{DOCS}/{id}");

    let r = get(&app, &alice, &doc_uri).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.json()["content"], "a\n");

    // Two quick autosaves fold into ONE `auto` revision (seq 2, saves = 2).
    for c in ["b\n", "c\n"] {
        let r = patch(&app, &alice, &doc_uri, json!({ "content": c })).await;
        assert_eq!(r.status, StatusCode::OK);
        assert_eq!(r.json()["rev"], 2);
    }
    let revs = get(&app, &alice, &format!("{doc_uri}/revisions"))
        .await
        .json();
    assert_eq!(revs.as_array().unwrap().len(), 2);
    assert_eq!(revs[0]["seq"], 2);
    assert_eq!(revs[0]["kind"], "auto");
    assert_eq!(revs[0]["saves"], 2);

    // ⌘S on unchanged content seals the burst instead of adding a revision.
    let r = patch(
        &app,
        &alice,
        &doc_uri,
        json!({ "content": "c\n", "checkpoint": true }),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);
    let revs = get(&app, &alice, &format!("{doc_uri}/revisions"))
        .await
        .json();
    assert_eq!(revs.as_array().unwrap().len(), 2);
    assert_eq!(revs[0]["kind"], "checkpoint");

    // The next autosave starts a new revision.
    let r = patch(&app, &alice, &doc_uri, json!({ "content": "d\n" })).await;
    assert_eq!(r.json()["rev"], 3);
    let revs = get(&app, &alice, &format!("{doc_uri}/revisions"))
        .await
        .json();
    let seqs: Vec<i64> = revs
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["seq"].as_i64().unwrap())
        .collect();
    assert_eq!(seqs, vec![3, 2, 1], "newest first");
    assert_eq!(revs[2]["kind"], "create");

    // Revision detail carries that revision's content.
    let r = get(&app, &alice, &format!("{doc_uri}/revisions/2")).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.json()["content"], "c\n");
    let r = get(&app, &alice, &format!("{doc_uri}/revisions/99")).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);

    // Diff rev 1 → current: `a` removed, `d` added.
    let r = get(&app, &alice, &format!("{doc_uri}/diff?from=1")).await;
    assert_eq!(r.status, StatusCode::OK);
    let diff = r.json();
    assert_eq!(diff["added"], 1);
    assert_eq!(diff["removed"], 1);
    let lines = diff["lines"].as_array().unwrap();
    assert!(lines.iter().any(|l| l["op"] == "del" && l["text"] == "a"));
    assert!(lines.iter().any(|l| l["op"] == "add" && l["text"] == "d"));
    // Between two stored revisions.
    let r = get(&app, &alice, &format!("{doc_uri}/diff?from=2&to=3")).await;
    assert_eq!(r.json()["to"], 3);

    // Restoring rev 1 APPENDS a `restore` revision; nothing is overwritten.
    let r = post(
        &app,
        &alice,
        &format!("{doc_uri}/revisions/1/restore"),
        json!({}),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);
    let doc = r.json();
    assert_eq!(doc["content"], "a\n");
    assert_eq!(doc["rev"], 4);
    let revs = get(&app, &alice, &format!("{doc_uri}/revisions"))
        .await
        .json();
    assert_eq!(revs.as_array().unwrap().len(), 4);
    assert_eq!(revs[0]["kind"], "restore");
    assert_eq!(revs[0]["restored_from"], 1);
    assert_eq!(get(&app, &alice, &doc_uri).await.json()["content"], "a\n");
}

/// Docs are per user: another editor of the same workspace sees nothing.
#[tokio::test]
async fn docs_are_private_to_their_owner() {
    let app = setup().await;
    let alice = user("alice", false);
    let bob = user("bob", false);
    let id = create_doc(&app, &alice, "mine.sql", "SELECT 1").await;
    let doc_uri = format!("{DOCS}/{id}");

    let r = get(&app, &bob, DOCS).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.json().as_array().unwrap().is_empty());
    assert_eq!(
        get(&app, &bob, &doc_uri).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get(&app, &bob, &format!("{doc_uri}/revisions"))
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        patch(&app, &bob, &doc_uri, json!({ "content": "x" }))
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        delete(&app, &bob, &doc_uri).await.status,
        StatusCode::NOT_FOUND
    );
    // Alice's doc is untouched.
    assert_eq!(
        get(&app, &alice, &doc_uri).await.json()["content"],
        "SELECT 1"
    );
    assert_eq!(
        get(&app, &alice, DOCS)
            .await
            .json()
            .as_array()
            .unwrap()
            .len(),
        1
    );

    // A non-member is refused outright.
    let mallory = user("mallory", false);
    let r = get(&app, &mallory, DOCS).await;
    assert!(r.status.is_client_error(), "non-member got {}", r.status);
}

/// Trash is a soft delete; only a TRASHED doc can be purged, and the purge
/// erases its history.
#[tokio::test]
async fn trash_restore_and_permanent_delete() {
    let app = setup().await;
    let alice = user("alice", false);
    let id = create_doc(&app, &alice, "tmp.md", "# hi").await;
    let doc_uri = format!("{DOCS}/{id}");

    // Purging a live doc is refused.
    let r = delete(&app, &alice, &format!("{doc_uri}?permanent=true")).await;
    assert_eq!(r.status, StatusCode::CONFLICT);

    // Soft delete → 200 + the doc with deleted_at.
    let r = delete(&app, &alice, &doc_uri).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.json()["deleted_at"].is_string());
    assert!(get(&app, &alice, DOCS)
        .await
        .json()
        .as_array()
        .unwrap()
        .is_empty());
    let trash = get(&app, &alice, &format!("{DOCS}?trash=true"))
        .await
        .json();
    assert_eq!(trash.as_array().unwrap().len(), 1);
    assert_eq!(trash[0]["id"], id.as_str());
    // History is still readable while trashed; editing is not allowed.
    assert_eq!(
        get(&app, &alice, &format!("{doc_uri}/revisions"))
            .await
            .status,
        StatusCode::OK
    );
    let r = patch(&app, &alice, &doc_uri, json!({ "content": "edit" })).await;
    assert_eq!(r.status, StatusCode::CONFLICT);

    // Restore → back in the list with its content.
    let r = post(&app, &alice, &format!("{doc_uri}/restore"), json!({})).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.json()["deleted_at"].is_null());
    assert_eq!(
        get(&app, &alice, DOCS)
            .await
            .json()
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(get(&app, &alice, &doc_uri).await.json()["content"], "# hi");

    // Trash again, then delete forever: 204, and the doc + history are gone.
    assert_eq!(delete(&app, &alice, &doc_uri).await.status, StatusCode::OK);
    let r = delete(&app, &alice, &format!("{doc_uri}?permanent=true")).await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    assert_eq!(
        get(&app, &alice, &doc_uri).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get(&app, &alice, &format!("{doc_uri}/revisions"))
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert!(get(&app, &alice, &format!("{DOCS}?trash=true"))
        .await
        .json()
        .as_array()
        .unwrap()
        .is_empty());
}

/// Images upload as raw bytes (magic-byte sniffed) and come back verbatim.
#[tokio::test]
async fn image_assets_roundtrip_and_reject_non_images() {
    let app = setup().await;
    let alice = user("alice", false);
    let assets = "/workspaces/ws1/workbench/assets";
    let png: Vec<u8> = [b"\x89PNG\r\n\x1a\n".as_slice(), b"\0\0\0\rIHDRfake"].concat();

    let r = send(
        &app,
        &alice,
        Method::POST,
        assets,
        Some("image/png"),
        png.clone(),
    )
    .await;
    assert_eq!(
        r.status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&r.body)
    );
    let asset = r.json();
    assert_eq!(asset["mime"], "image/png");
    assert_eq!(asset["size"], png.len());
    let aid = asset["id"].as_str().unwrap().to_string();

    let r = get(&app, &alice, &format!("{assets}/{aid}")).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.content_type.as_deref(), Some("image/png"));
    assert_eq!(r.body, png);

    // Another user can't fetch it.
    let bob = user("bob", false);
    assert_eq!(
        get(&app, &bob, &format!("{assets}/{aid}")).await.status,
        StatusCode::NOT_FOUND
    );

    // Text bytes declared as PNG are refused.
    let r = send(
        &app,
        &alice,
        Method::POST,
        assets,
        Some("image/png"),
        b"definitely not an image".to_vec(),
    )
    .await;
    assert!(r.status.is_client_error(), "got {}", r.status);
}

/// Real route + isolated SQLite fixture: all revisions share the existing tiny
/// content blob, so lifetime metadata growth is tested without large bodies.
async fn many_revision_fixture(count: i64) -> (Router, DbPool, User, String) {
    let pool = mem_pool().await;
    seed_user(&pool, "alice", false).await;
    seed_workspace(&pool, "ws1").await;
    set_member(&pool, "ws1", "alice", "editor").await;
    let app = workbench_router(test_ctx(&pool).await);
    let alice = user("alice", false);
    let id = create_doc(&app, &alice, "lifetime.txt", "original content").await;
    sqlx::query(
        "WITH RECURSIVE seq(n) AS (SELECT 2 UNION ALL SELECT n+1 FROM seq WHERE n < ?) \
         INSERT INTO workbench_revisions (doc_id,seq,kind,content_hash,size,created_at,updated_at,saves) \
         SELECT r.doc_id,seq.n,'checkpoint',r.content_hash,r.size,r.created_at,r.updated_at,1 \
         FROM seq CROSS JOIN workbench_revisions r WHERE r.doc_id=? AND r.seq=1",
    ).bind(count).bind(&id).execute(&pool).await.unwrap();
    sqlx::query("UPDATE workbench_docs SET rev=? WHERE id=?")
        .bind(count)
        .bind(&id)
        .execute(&pool)
        .await
        .unwrap();
    (app, pool, alice, id)
}

#[tokio::test]
#[ignore = "50k lifetime-metadata scale gate; run explicitly"]
async fn revision_history_default_and_maximum_page_are_bounded_at_50k_revisions() {
    let (app, pool, alice, id) = many_revision_fixture(50_000).await;
    let uri = format!("{DOCS}/{id}/revisions");
    let page = get(&app, &alice, &uri).await;
    assert_eq!(page.status, StatusCode::OK);
    let rows = page.json();
    assert_eq!(
        rows.as_array().unwrap().len(),
        100,
        "initial history must not serialize its whole lifetime"
    );
    assert_eq!(rows[0]["seq"], 50_000);
    assert_eq!(rows[99]["seq"], 49_901);
    assert!(
        page.body.len() < 100 * 512,
        "bounded metadata, never revision bodies"
    );
    let oversized = get(&app, &alice, &format!("{uri}?limit=999999")).await;
    assert_eq!(oversized.status, StatusCode::OK);
    assert_eq!(oversized.json().as_array().unwrap().len(), 200);
    let retained: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM workbench_revisions WHERE doc_id=?")
            .bind(&id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(retained, 50_000, "paging is not retention/deletion");
}

#[tokio::test]
async fn revision_history_exclusive_cursor_survives_new_save_and_reaches_oldest_restore() {
    let (app, pool, alice, id) = many_revision_fixture(205).await;
    let doc_uri = format!("{DOCS}/{id}");
    let uri = format!("{doc_uri}/revisions");
    let first = get(&app, &alice, &format!("{uri}?limit=7&before_seq=206")).await;
    assert_eq!(first.status, StatusCode::OK);
    let first = first.json();
    assert_eq!(first.as_array().unwrap().len(), 7);
    assert_eq!(first[0]["seq"], 205);
    assert_eq!(first[6]["seq"], 199);
    let changed = patch(
        &app,
        &alice,
        &doc_uri,
        json!({"content":"new head", "checkpoint":true}),
    )
    .await;
    assert_eq!(changed.status, StatusCode::OK);
    assert_eq!(changed.json()["rev"], 206);
    let next = get(&app, &alice, &format!("{uri}?limit=7&before_seq=199"))
        .await
        .json();
    assert_eq!(next.as_array().unwrap().len(), 7);
    assert_eq!(next[0]["seq"], 198);
    assert_eq!(next[6]["seq"], 192);
    let oldest = get(&app, &alice, &format!("{uri}?limit=7&before_seq=2"))
        .await
        .json();
    assert_eq!(oldest.as_array().unwrap().len(), 1);
    assert_eq!(oldest[0]["seq"], 1);
    let exhausted = get(&app, &alice, &format!("{uri}?limit=7&before_seq=1"))
        .await
        .json();
    assert!(exhausted.as_array().unwrap().is_empty());
    let detail = get(&app, &alice, &format!("{uri}/1")).await.json();
    assert_eq!(detail["content"], "original content");
    let diff = get(&app, &alice, &format!("{doc_uri}/diff?from=1&to=206")).await;
    assert_eq!(
        diff.status,
        StatusCode::OK,
        "oldest revisions remain valid comparison targets"
    );
    let restored = post(&app, &alice, &format!("{uri}/1/restore"), json!({})).await;
    assert_eq!(restored.status, StatusCode::OK);
    assert_eq!(restored.json()["content"], "original content");
    assert_eq!(restored.json()["rev"], 207);
    let retained: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM workbench_revisions WHERE doc_id=?")
            .bind(&id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(retained, 207);
}
