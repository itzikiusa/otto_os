//! `/scratches` routes: the one-time copy of a user's Workbench docs (from
//! every workspace) into their Scratches folder, the file API the MCP
//! `workbench_*` tools use, and the IDE root rule that keeps a Scratches
//! folder its owner's alone.
//!
//! Real minimal `ServerCtx` (in-memory sqlite with every migration, a temp
//! data dir whose `scratches/` stands in for `~/Otto/Scratches`); requests go
//! through the real handlers via `oneshot` with the `AuthUser` extension the
//! auth middleware would inject.

use axum::body::Body;
use axum::extract::Request;
use axum::http::{header, Method, StatusCode};
use axum::Router;
use chrono::Utc;
use http_body_util::BodyExt;
use otto_core::auth::AuthUser;
use otto_core::domain::User;
use otto_server::lsp::ide_local_root_authorized;
use otto_server::ServerCtx;
use otto_state::DbPool;
use serde_json::{json, Value};
use tower::ServiceExt; // for `oneshot`

fn user(id: &str) -> User {
    User {
        id: id.into(),
        username: id.into(),
        display_name: id.into(),
        is_root: false,
        disabled: false,
        created_at: Utc::now(),
    }
}

/// A Workbench doc as the old build stored it (`created` orders the copy).
#[allow(clippy::too_many_arguments)] // a test fixture: one positional column per field
async fn seed_doc(
    pool: &DbPool,
    id: &str,
    owner: &str,
    ws: &str,
    name: &str,
    lang: &str,
    folder: &str,
    content: &str,
    trashed: bool,
) {
    let hash = otto_state::workbench::content_digest_hex(content.as_bytes());
    sqlx::query("INSERT OR IGNORE INTO workbench_blobs (hash, content, size) VALUES (?, ?, ?)")
        .bind(&hash)
        .bind(content)
        .bind(content.len() as i64)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO workbench_docs (id, workspace_id, owner_id, name, language, folder, content_hash, \
         size, created_at, updated_at, deleted_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(ws)
    .bind(owner)
    .bind(name)
    .bind(lang)
    .bind(folder)
    .bind(&hash)
    .bind(content.len() as i64)
    .bind(format!("2026-01-01T00:00:{id}Z"))
    .bind("2026-01-01T00:00:00Z")
    .bind(trashed.then_some("2026-02-01T00:00:00Z"))
    .execute(pool)
    .await
    .unwrap();
}

async fn send(
    app: &Router,
    caller: &User,
    method: Method,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let b = Request::builder().method(method).uri(uri);
    let mut req = match body {
        Some(v) => b
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(v.to_string()))
            .unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    req.extensions_mut().insert(AuthUser(caller.clone()));
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn paths(list: &Value) -> Vec<String> {
    let mut out: Vec<String> = list["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap().to_string())
        .collect();
    out.sort();
    out
}

#[tokio::test]
async fn workbench_docs_from_every_workspace_move_into_the_folder_once() {
    let tmp = tempfile::tempdir().unwrap();
    let pool = otto_server::test_support::mem_pool().await;
    seed_doc(
        &pool, "01", "alice", "ws1", "Daily", "auto", "", "SELECT 1", false,
    )
    .await;
    seed_doc(
        &pool,
        "02",
        "alice",
        "ws2",
        "notes",
        "md",
        "ops/runbooks",
        "# hi",
        false,
    )
    .await;
    seed_doc(
        &pool,
        "03",
        "alice",
        "ws2",
        "gone",
        "md",
        "",
        "# trashed",
        true,
    )
    .await;
    seed_doc(
        &pool,
        "04",
        "alice",
        "ws2",
        "Daily.sql",
        "sql",
        "",
        "SELECT 2",
        false,
    )
    .await;
    seed_doc(&pool, "05", "bob", "ws1", "bobs", "md", "", "# bob", false).await;
    let ctx = ServerCtx::for_tests(&pool, tmp.path().join("data")).await;
    let app = Router::new()
        .merge(otto_server::routes::scratches::scratches_routes())
        .with_state(ctx.clone());
    let alice = user("alice");

    let (st, first) = send(&app, &alice, Method::GET, "/scratches", None).await;
    assert_eq!(st, StatusCode::OK, "{first}");
    let root = std::path::PathBuf::from(first["root"].as_str().unwrap());
    assert_eq!(
        root,
        tmp.path()
            .join("data/scratches/alice")
            .canonicalize()
            .unwrap()
    );
    assert_eq!(
        paths(&first),
        ["Daily (2).sql", "Daily.sql", "ops/runbooks/notes.md"],
        "trashed and other users' docs stay in the DB; a name clash never overwrites"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("Daily.sql")).unwrap(),
        "SELECT 1",
        "the older doc keeps the plain name"
    );

    // Idempotent: a second pass (and a deleted copy) brings nothing back.
    std::fs::remove_file(root.join("Daily (2).sql")).unwrap();
    let (_, again) = send(&app, &alice, Method::GET, "/scratches", None).await;
    assert_eq!(paths(&again), ["Daily.sql", "ops/runbooks/notes.md"]);
    let exported: Vec<(String, Option<String>)> =
        sqlx::query_as("SELECT id, exported_path FROM workbench_docs ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(exported[0].1.as_deref(), Some("Daily.sql"));
    assert_eq!(exported[2].1, None, "a trashed doc is not exported");
    assert_eq!(exported[4].1, None, "bob's docs wait for bob");
    let docs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM workbench_docs")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(docs, 5, "nothing is deleted from the archive");

    // A doc the old build created after a rollback is picked up next time.
    seed_doc(&pool, "06", "alice", "ws1", "late", "json", "", "{}", false).await;
    let (_, late) = send(&app, &alice, Method::GET, "/scratches?query=LATE", None).await;
    assert_eq!(paths(&late), ["late.json"]);
}

#[tokio::test]
async fn files_are_read_and_written_inside_the_callers_folder_only() {
    let tmp = tempfile::tempdir().unwrap();
    let pool = otto_server::test_support::mem_pool().await;
    let ctx = ServerCtx::for_tests(&pool, tmp.path().join("data")).await;
    let app = Router::new()
        .merge(otto_server::routes::scratches::scratches_routes())
        .with_state(ctx.clone());
    let (alice, bob) = (user("alice"), user("bob"));

    let body = json!({ "path": "q/daily.sql", "content": "SELECT 1" });
    let (st, made) = send(&app, &alice, Method::PUT, "/scratches/file", Some(body)).await;
    assert_eq!(
        (st, made["created"].clone(), made["path"].clone()),
        (StatusCode::OK, json!(true), json!("q/daily.sql"))
    );
    let body = json!({ "path": "q/daily.sql", "content": "SELECT 2" });
    let (_, replaced) = send(&app, &alice, Method::PUT, "/scratches/file", Some(body)).await;
    assert_eq!(replaced["created"], false);

    let (st, got) = send(
        &app,
        &alice,
        Method::GET,
        "/scratches/file?path=q%2Fdaily.sql",
        None,
    )
    .await;
    assert_eq!(
        (st, got["content"].clone()),
        (StatusCode::OK, json!("SELECT 2"))
    );
    let (st, _) = send(
        &app,
        &bob,
        Method::GET,
        "/scratches/file?path=q%2Fdaily.sql",
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND, "bob's folder is his own");
    for bad in ["..%2Falice%2Fq%2Fdaily.sql", "%2Fetc%2Fhosts", ""] {
        let (st, _) = send(
            &app,
            &bob,
            Method::GET,
            &format!("/scratches/file?path={bad}"),
            None,
        )
        .await;
        assert_eq!(st, StatusCode::BAD_REQUEST, "{bad}");
    }
    let body = json!({ "path": "../alice/x.md", "content": "pwned" });
    let (st, _) = send(&app, &bob, Method::PUT, "/scratches/file", Some(body)).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);

    // The write is in the IDE's Local History of the Scratches project.
    let root = tmp
        .path()
        .join("data/scratches/alice")
        .canonicalize()
        .unwrap();
    let hist = ctx
        .ide
        .history
        .list_entries(&root, &[], false, None, false, Some(50), None)
        .unwrap();
    assert!(
        hist.entries.len() >= 2,
        "old + new content kept: {:?}",
        hist.entries.len()
    );
}

#[tokio::test]
async fn a_scratches_folder_opens_in_the_ide_for_its_owner_only() {
    let tmp = tempfile::tempdir().unwrap();
    let pool = otto_server::test_support::mem_pool().await;
    let data = tmp.path().join("data");
    // A workspace whose root CONTAINS every Scratches folder (like a `$HOME`
    // workspace containing `~/Otto/Scratches`), with both users as Editors.
    sqlx::query(
        "INSERT INTO users (id, username, password_hash, display_name, is_root, created_at) \
         VALUES ('alice','alice','x','alice',0,'2026-01-01T00:00:00Z'), ('bob','bob','x','bob',0,'2026-01-01T00:00:00Z')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO workspaces (id, name, root_path, settings_json, archived, created_at) VALUES ('ws', 'ws', ?, '{}', 0, '2026-01-01T00:00:00Z')")
        .bind(tmp.path().to_string_lossy().into_owned())
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO workspace_members (workspace_id, user_id, role) VALUES ('ws','alice','editor'), ('ws','bob','editor')")
        .execute(&pool)
        .await
        .unwrap();
    let ctx = ServerCtx::for_tests(&pool, &data).await;
    let alice_root = otto_server::routes::scratches::ensure_root(&ctx, &user("alice"))
        .await
        .unwrap();

    assert!(
        ide_local_root_authorized(&ctx, &user("alice"), &alice_root, true)
            .await
            .unwrap()
    );
    assert!(
        !ide_local_root_authorized(&ctx, &user("bob"), &alice_root, false)
            .await
            .unwrap(),
        "another user's Scratches folder stays closed even inside a shared workspace root"
    );
    assert!(
        !ide_local_root_authorized(&ctx, &user("bob"), &ctx.scratches_home, false)
            .await
            .unwrap()
    );
    assert!(
        ide_local_root_authorized(&ctx, &user("bob"), tmp.path(), false)
            .await
            .unwrap(),
        "the workspace itself still opens"
    );
}
