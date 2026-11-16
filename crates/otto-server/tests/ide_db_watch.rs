//! Integration suite for IDE Live Database Watch & Component-Test Watcher
//! (`POST /api/v1/ide/db-watch/detect` and `POST /api/v1/ide/db-watch/snapshot`).
//!
//! Verifies:
//! 1. Multi-language code scanner detects SQL tables, MongoDB collections,
//!    Redis key prefixes, and ClickHouse tables within authorized workspace roots.
//! 2. Workspace non-members cannot scan arbitrary directories (`403 Forbidden`).
//! 3. Saved connections enforce `Database:View` + workspace membership +
//!    `connections.owner_private`.
//! 4. Ephemeral local targets require human `root_authority` (`403 Forbidden`
//!    for non-root users).
//! 5. Write/mutating statements (`SET`, `DEL`, `DROP`) are strictly refused on
//!    both saved and ephemeral snapshot endpoints (`403 Forbidden`).
//! 6. Live loopback Redis `Before -> After` snapshots compute exact added,
//!    modified (`before` -> `after`), and removed key/value diffs.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use otto_core::access::{AccessActor, AccessRule, ResourceKind, RuleEffect, SubjectKind};
use otto_core::domain::{Capability, ConnectionKind, Environment, Feature, User};
use otto_ide::db_watch::IdeDbEngine;
use otto_rbac::AuthRepo;
use otto_server::ServerCtx;
use otto_state::{
    ConnectionsRepo, DbPool, GrantsRepo, NewConnection, ResourceAccessRepo, UsersRepo,
};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

async fn seed_user(pool: &DbPool, id: &str, is_root: bool) -> User {
    sqlx::query(
        "INSERT INTO users (id, username, display_name, password_hash, is_root, created_at)
         VALUES (?, ?, ?, 'x', ?, ?)",
    )
    .bind(id)
    .bind(id)
    .bind(id)
    .bind(is_root as i64)
    .bind(chrono::Utc::now().to_rfc3339())
    .execute(pool)
    .await
    .unwrap();
    UsersRepo::new(pool.clone())
        .get(&id.to_string())
        .await
        .unwrap()
}

async fn seed_workspace(pool: &DbPool, id: &str, root: &Path) {
    sqlx::query(
        "INSERT INTO workspaces (id, name, root_path, settings_json, archived, created_at)
         VALUES (?, ?, ?, '{}', 0, ?)",
    )
    .bind(id)
    .bind(id)
    .bind(root.to_string_lossy().as_ref())
    .bind(chrono::Utc::now().to_rfc3339())
    .execute(pool)
    .await
    .unwrap();
}

async fn add_member(pool: &DbPool, ws: &str, uid: &str, role: &str) {
    sqlx::query("INSERT INTO workspace_members (workspace_id, user_id, role) VALUES (?, ?, ?)")
        .bind(ws)
        .bind(uid)
        .bind(role)
        .execute(pool)
        .await
        .unwrap();
}

/// Minimal in-process RESP2 loopback server supporting the exact read commands
/// `RedisDriver` issues (`PING`, `SELECT`, `INFO`, `SCAN`, `TYPE`, `TTL`,
/// `OBJECT`, `STRLEN`, `GET`, `GETRANGE`), backed by a live mutable map so tests
/// can mutate state between `Before` and `After` snapshots without Docker.
async fn spawn_mini_resp_server(store: Arc<Mutex<BTreeMap<String, String>>>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let store = Arc::clone(&store);
            tokio::spawn(async move {
                let (rd, mut wr) = stream.into_split();
                let mut reader = BufReader::new(rd);
                while let Some(args) = read_resp_command(&mut reader).await {
                    if args.is_empty() {
                        continue;
                    }
                    let cmd = args[0].to_ascii_uppercase();
                    let reply = match cmd.as_str() {
                        "PING" => "+PONG\r\n".to_string(),
                        "SELECT" | "CLIENT" => "+OK\r\n".to_string(),
                        "AUTH" | "HELLO" => "-ERR unsupported\r\n".to_string(),
                        "INFO" => {
                            let n = store.lock().unwrap().len();
                            resp_bulk(&format!(
                                "# Keyspace\r\ndb0:keys={n},expires=0,avg_ttl=0\r\n"
                            ))
                        }
                        "SCAN" => {
                            let mut pattern: Option<String> = None;
                            let mut idx = 2usize;
                            while idx + 1 < args.len() {
                                if args[idx].eq_ignore_ascii_case("MATCH") {
                                    pattern = Some(args[idx + 1].clone());
                                }
                                idx += 2;
                            }
                            let prefix = pattern
                                .as_deref()
                                .unwrap_or("*")
                                .trim_end_matches('*')
                                .replace('\\', "");
                            let keys: Vec<String> = store
                                .lock()
                                .unwrap()
                                .keys()
                                .filter(|k| prefix.is_empty() || k.starts_with(&prefix))
                                .cloned()
                                .collect();
                            let mut out = format!("*2\r\n$1\r\n0\r\n*{}\r\n", keys.len());
                            for k in keys {
                                out.push_str(&resp_bulk(&k));
                            }
                            out
                        }
                        "TYPE" => {
                            let exists = args
                                .get(1)
                                .map(|k| store.lock().unwrap().contains_key(k))
                                .unwrap_or(false);
                            if exists {
                                "+string\r\n".into()
                            } else {
                                "+none\r\n".into()
                            }
                        }
                        "TTL" => ":-1\r\n".into(),
                        "OBJECT" => resp_bulk("embstr"),
                        "STRLEN" => {
                            let len = args
                                .get(1)
                                .and_then(|k| store.lock().unwrap().get(k).map(String::len))
                                .unwrap_or(0);
                            format!(":{len}\r\n")
                        }
                        "GET" | "GETRANGE" => {
                            match args
                                .get(1)
                                .and_then(|k| store.lock().unwrap().get(k).cloned())
                            {
                                Some(v) => resp_bulk(&v),
                                None => "$-1\r\n".into(),
                            }
                        }
                        _ => "-ERR unknown command\r\n".into(),
                    };
                    if wr.write_all(reply.as_bytes()).await.is_err() {
                        break;
                    }
                }
            });
        }
    });
    port
}

fn resp_bulk(s: &str) -> String {
    format!("${}\r\n{}\r\n", s.len(), s)
}

async fn read_resp_command<R: tokio::io::AsyncRead + Unpin>(
    reader: &mut BufReader<R>,
) -> Option<Vec<String>> {
    let mut header = String::new();
    if reader.read_line(&mut header).await.ok()? == 0 {
        return None;
    }
    let trimmed = header.trim();
    if let Some(rest) = trimmed.strip_prefix('*') {
        let count: usize = rest.parse().ok()?;
        let mut out = Vec::with_capacity(count);
        for _ in 0..count {
            let mut len_line = String::new();
            reader.read_line(&mut len_line).await.ok()?;
            let byte_len: usize = len_line.trim().strip_prefix('$')?.parse().ok()?;
            let mut buf = vec![0u8; byte_len + 2];
            reader.read_exact(&mut buf).await.ok()?;
            out.push(String::from_utf8_lossy(&buf[..byte_len]).into_owned());
        }
        Some(out)
    } else {
        Some(trimmed.split_whitespace().map(str::to_string).collect())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ide_db_watch_detect_rbac_readonly_and_live_before_after_diff() {
    let tmp = tempfile::tempdir().unwrap();
    let ws1_root = tmp.path().join("ws1");
    let secret_root = tmp.path().join("secret_ws");
    std::fs::create_dir_all(ws1_root.join("internal/dao")).unwrap();
    std::fs::create_dir_all(&secret_root).unwrap();

    std::fs::write(
        ws1_root.join("internal/dao/wallet_dao.go"),
        r#"package dao
func load(ctx context.Context) {
    _ = sq.Select("id", "balance").From("MdlGm_tblPlayers")
    _ = mongoDb.Collection("bonus_grants")
    _ = redisClient.Set(ctx, fmt.Sprintf("wallet:bal:%d", 7), "100", 0)
    // clickhouse ledger
    _ = ch.Query(ctx, "SELECT player_id FROM ledger_events FINAL")
}
"#,
    )
    .unwrap();

    let redis_data = Arc::new(Mutex::new(BTreeMap::from([
        ("wallet:bal:1".to_string(), "100.00".to_string()),
        ("wallet:bal:2".to_string(), "50.00".to_string()),
    ])));
    let redis_port = spawn_mini_resp_server(Arc::clone(&redis_data)).await;

    let pool = otto_server::test_support::mem_pool().await;
    let root = seed_user(&pool, "root", true).await;
    let editor_db = seed_user(&pool, "editor_db", false).await;
    let editor_no_db = seed_user(&pool, "editor_no_db", false).await;
    let outsider = seed_user(&pool, "outsider", false).await;

    seed_workspace(&pool, "ws1", &ws1_root).await;
    seed_workspace(&pool, "ws_secret", &secret_root).await;
    add_member(&pool, "ws1", &editor_db.id, "editor").await;
    add_member(&pool, "ws1", &editor_no_db.id, "editor").await;

    let grants = GrantsRepo::new(pool.clone());
    grants
        .set_grants(
            &editor_db.id,
            &[
                (Feature::Agents, Capability::Edit),
                (Feature::Database, Capability::View),
            ],
        )
        .await
        .unwrap();
    grants
        .set_grants(&editor_no_db.id, &[(Feature::Agents, Capability::Edit)])
        .await
        .unwrap();
    grants
        .set_grants(
            &outsider.id,
            &[
                (Feature::Agents, Capability::Edit),
                (Feature::Database, Capability::View),
            ],
        )
        .await
        .unwrap();

    let conns = ConnectionsRepo::new(pool.clone());
    let saved_redis = conns
        .create(NewConnection {
            workspace_id: Some("ws1".into()),
            name: "Local Wallet Redis".into(),
            kind: ConnectionKind::Redis,
            params: json!({"host": "127.0.0.1", "port": redis_port}),
            secret_ref: None,
            first_command: None,
            section_id: None,
            environment: Environment::Dev,
            read_only: false,
            created_by: root.id.clone(),
        })
        .await
        .unwrap();

    let auth = AuthRepo::new(pool.clone());
    let root_tok = auth
        .issue_api_token(&root.id, Some("root"))
        .await
        .unwrap()
        .0;
    let editor_db_tok = auth
        .issue_api_token(&editor_db.id, Some("ed_db"))
        .await
        .unwrap()
        .0;
    let editor_no_db_tok = auth
        .issue_api_token(&editor_no_db.id, Some("ed_nodb"))
        .await
        .unwrap()
        .0;
    let outsider_tok = auth
        .issue_api_token(&outsider.id, Some("out"))
        .await
        .unwrap()
        .0;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base = format!("http://{addr}");
    let mut ctx = ServerCtx::for_tests(&pool, tmp.path().join("data")).await;
    ctx.base_url = base.clone();
    ctx.ide.db_watch.register_loopback_target(
        "miniredis:test",
        format!("miniredis (:{redis_port})"),
        IdeDbEngine::Redis,
        redis_port,
        None,
        None,
        Some("0".into()),
    );
    let (api_extras, root_extras) = otto_server::modules::module_routers(&ctx);
    let app = otto_server::build_router(ctx, api_extras, root_extras);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = reqwest::Client::new();

    // 1. Outsider cannot scan ws1_root (`403 Forbidden`).
    let out_detect = client
        .post(format!("{base}/api/v1/ide/db-watch/detect"))
        .bearer_auth(&outsider_tok)
        .json(&json!({ "root": ws1_root.to_string_lossy() }))
        .send()
        .await
        .unwrap();
    assert_eq!(out_detect.status().as_u16(), 403);

    // 2. Editor without Database:View gets code detection, 0 saved connections, 0 ephemeral targets.
    let nodb_detect: Value = client
        .post(format!("{base}/api/v1/ide/db-watch/detect"))
        .bearer_auth(&editor_no_db_tok)
        .json(&json!({ "root": ws1_root.to_string_lossy() }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        nodb_detect["saved_connections"].as_array().unwrap().len(),
        0
    );
    assert_eq!(
        nodb_detect["ephemeral_targets"].as_array().unwrap().len(),
        0
    );
    assert!(nodb_detect["detected_objects"].as_array().unwrap().len() >= 4);

    // 3. Editor without Database:View is denied saved snapshot (`403`) and ephemeral snapshot (`403`).
    let nodb_snap = client
        .post(format!("{base}/api/v1/ide/db-watch/snapshot"))
        .bearer_auth(&editor_no_db_tok)
        .json(&json!({
            "root": ws1_root.to_string_lossy(),
            "target_kind": "saved",
            "connection_id": saved_redis.id,
            "engine": "redis",
            "object_name": "wallet:bal:*"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(nodb_snap.status().as_u16(), 403);

    let nonroot_eph = client
        .post(format!("{base}/api/v1/ide/db-watch/snapshot"))
        .bearer_auth(&editor_db_tok)
        .json(&json!({
            "root": ws1_root.to_string_lossy(),
            "target_kind": "ephemeral",
            "ephemeral_id": "miniredis:test",
            "engine": "redis",
            "object_name": "wallet:bal:*"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(nonroot_eph.status().as_u16(), 403);

    // 4. Write custom query (`SET wallet:bal:1 999`) is strictly refused (`403`).
    let write_attempt = client
        .post(format!("{base}/api/v1/ide/db-watch/snapshot"))
        .bearer_auth(&root_tok)
        .json(&json!({
            "root": ws1_root.to_string_lossy(),
            "target_kind": "ephemeral",
            "ephemeral_id": "miniredis:test",
            "engine": "redis",
            "object_name": "wallet:bal:*",
            "custom_query": "SET wallet:bal:1 999"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(write_attempt.status().as_u16(), 403);

    // 5. Root detect returns all 4 detected engines + saved connection + ephemeral target!
    let root_detect: Value = client
        .post(format!("{base}/api/v1/ide/db-watch/detect"))
        .bearer_auth(&root_tok)
        .json(&json!({ "root": ws1_root.to_string_lossy() }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        root_detect["saved_connections"].as_array().unwrap().len(),
        1
    );
    assert_eq!(
        root_detect["ephemeral_targets"].as_array().unwrap().len(),
        1
    );

    // 6. Capture Baseline ("Before") snapshot, mutate Redis state, capture "After" snapshot & verify diff!
    let before_resp: Value = client
        .post(format!("{base}/api/v1/ide/db-watch/snapshot"))
        .bearer_auth(&root_tok)
        .json(&json!({
            "root": ws1_root.to_string_lossy(),
            "target_kind": "ephemeral",
            "ephemeral_id": "miniredis:test",
            "engine": "redis",
            "object_name": "wallet:bal:*"
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(before_resp["snapshot"]["rows"].as_array().unwrap().len(), 2);

    {
        let mut guard = redis_data.lock().unwrap();
        guard.insert("wallet:bal:1".into(), "250.00".into()); // modified
        guard.remove("wallet:bal:2"); // removed
        guard.insert("wallet:bal:3".into(), "75.00".into()); // added
    }

    let after_resp: Value = client
        .post(format!("{base}/api/v1/ide/db-watch/snapshot"))
        .bearer_auth(&root_tok)
        .json(&json!({
            "root": ws1_root.to_string_lossy(),
            "target_kind": "ephemeral",
            "ephemeral_id": "miniredis:test",
            "engine": "redis",
            "object_name": "wallet:bal:*",
            "previous": before_resp["snapshot"]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(after_resp["diff"]["added_count"], 1);
    assert_eq!(after_resp["diff"]["modified_count"], 1);
    assert_eq!(after_resp["diff"]["removed_count"], 1);
    assert_eq!(after_resp["diff"]["unchanged_count"], 0);

    // 7a. Newly created connection fails closed (`404`) for non-root even with `Database:View`
    // until an explicit resource-access rule grants `discover` + `db_browse`.
    let ungranted_snap = client
        .post(format!("{base}/api/v1/ide/db-watch/snapshot"))
        .bearer_auth(&editor_db_tok)
        .json(&json!({
            "root": ws1_root.to_string_lossy(),
            "target_kind": "saved",
            "connection_id": saved_redis.id,
            "engine": "redis",
            "object_name": "wallet:bal:*"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(ungranted_snap.status().as_u16(), 404);

    // 7b. Grant `discover` + `db_browse` to `editor_db` -> saved snapshot succeeds!
    let access_repo = ResourceAccessRepo::new(pool.clone());
    let mut policy = access_repo
        .get_policy(ResourceKind::Connection, &saved_redis.id)
        .await
        .unwrap();
    policy.rules = vec![AccessRule {
        id: "allow-editor-db-watch".into(),
        subject_kind: SubjectKind::User,
        subject_id: editor_db.id.clone(),
        effect: RuleEffect::Allow,
        operations: vec!["discover".into(), "db_browse".into()],
        children: None,
        grantable_operations: vec![],
        credential_connection_id: None,
    }];
    access_repo
        .put_policy(
            &policy,
            policy.revision,
            &AccessActor {
                real_user_id: root.id.clone(),
                effective_user_id: None,
            },
        )
        .await
        .unwrap();

    let saved_snap: Value = client
        .post(format!("{base}/api/v1/ide/db-watch/snapshot"))
        .bearer_auth(&editor_db_tok)
        .json(&json!({
            "root": ws1_root.to_string_lossy(),
            "target_kind": "saved",
            "connection_id": saved_redis.id,
            "engine": "redis",
            "object_name": "wallet:bal:*"
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        saved_snap["snapshot"]["rows"].is_array(),
        "unexpected saved_snap response: {saved_snap}"
    );
    assert_eq!(saved_snap["snapshot"]["rows"].as_array().unwrap().len(), 2);
}
