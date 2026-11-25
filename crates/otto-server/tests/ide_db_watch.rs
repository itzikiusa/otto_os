//! Integration suite for IDE Live Database Watch & Component-Test Watcher
//! (`POST /api/v1/ide/db-watch/detect` and the watch routes `…/watches`).
//!
//! Verifies:
//! 1. Multi-language code scanner detects SQL tables, MongoDB collections,
//!    Redis key prefixes, and ClickHouse tables within authorized workspace roots.
//! 2. Workspace non-members cannot scan arbitrary directories (`403 Forbidden`).
//! 3. Saved connections enforce `Database:View` + workspace membership +
//!    `connections.owner_private`.
//! 4. Ephemeral local targets require human `root_authority` (`403 Forbidden`
//!    for non-root users).
//! 5. A watch runs only fixed reads (no custom statement reaches it).
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

    let root_str = ws1_root.to_string_lossy().to_string();
    let watches = format!("{base}/api/v1/ide/db-watch/watches");
    let start = |target: Value| json!({ "root": root_str, "target": target, "items": [{ "object": "wallet:bal:" }] });
    let saved_target = json!({ "kind": "saved", "connection_id": saved_redis.id });
    let eph_target = json!({ "kind": "ephemeral", "ephemeral_id": "miniredis:test" });

    // 3. No Database:View: the saved connection is not visible (`404`); a
    // non-root user gets no ephemeral watch (`403`).
    let (s, _) = call(
        reqwest::Method::POST,
        watches.clone(),
        &editor_no_db_tok,
        Some(start(saved_target.clone())),
    )
    .await;
    assert_eq!(s, 404);
    let (s, _) = call(
        reqwest::Method::POST,
        watches.clone(),
        &editor_db_tok,
        Some(start(eph_target.clone())),
    )
    .await;
    assert_eq!(s, 403);

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
    // Discovery also lists any database the host happens to be running, so
    // only the seeded target is asserted, never the total.
    let ephemeral = root_detect["ephemeral_targets"].as_array().unwrap();
    assert!(
        ephemeral
            .iter()
            .any(|t| t["ephemeral_id"] == "miniredis:test"),
        "seeded ephemeral target missing: {} targets",
        ephemeral.len()
    );

    // 6. Baseline, mutate Redis, checkpoint: added / modified / removed.
    let (s, w) = call(
        reqwest::Method::POST,
        watches.clone(),
        &root_tok,
        Some(start(eph_target)),
    )
    .await;
    assert_eq!(s, 200, "{w}");
    let id = w["id"].as_str().unwrap().to_string();
    let (_, before) = call(
        reqwest::Method::GET,
        format!("{watches}/{id}?rows=1"),
        &root_tok,
        None,
    )
    .await;
    assert_eq!(
        before["items"][0]["current"]["rows"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    {
        let mut guard = redis_data.lock().unwrap();
        guard.insert("wallet:bal:1".into(), "250.00".into()); // modified
        guard.remove("wallet:bal:2"); // removed
        guard.insert("wallet:bal:3".into(), "75.00".into()); // added
    }
    let (_, after) = call(
        reqwest::Method::POST,
        format!("{watches}/{id}/snapshot"),
        &root_tok,
        Some(json!({})),
    )
    .await;
    let item = &after["items"][0];
    assert_eq!(item["added_count"], 1, "{item}");
    assert_eq!(item["modified_count"], 1);
    assert_eq!(item["removed_count"], 1);
    assert_eq!(item["unchanged_count"], 0);

    // 7a. A new connection fails closed (`404`) for non-root even with
    // `Database:View` until a rule grants `discover` + `db_browse`.
    let (s, _) = call(
        reqwest::Method::POST,
        watches.clone(),
        &editor_db_tok,
        Some(start(saved_target.clone())),
    )
    .await;
    assert_eq!(s, 404);

    // 7b. Grant `discover` + `db_browse` to `editor_db`: the watch is armed,
    // but reading VALUES hits the same native privilege ceiling as a DB
    // Explorer query on an enforced connection (C1-016; before, the watch's
    // Redis path skipped `verify_native` and returned the 2 values).
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
    let (s, w) = call(
        reqwest::Method::POST,
        watches.clone(),
        &editor_db_tok,
        Some(start(saved_target)),
    )
    .await;
    assert_eq!(s, 200, "{w}");
    let (_, r) = call(
        reqwest::Method::GET,
        format!("{watches}/{}?rows=1", w["id"].as_str().unwrap()),
        &editor_db_tok,
        None,
    )
    .await;
    let err = r["items"][0]["error"].as_str().unwrap_or_default();
    assert!(err.contains("native_scope_required"), "{r}");
    assert!(r["items"][0]["current"].is_null(), "no values read: {r}");
}

// ── Watch engine (`/ide/db-watch/watches`) ────────────────────────────────

struct WatchEnv {
    base: String,
    pool: DbPool,
    root_tok: String,
    editor_tok: String,
    events: tokio::sync::broadcast::Receiver<otto_core::event::Event>,
    root_dir: tempfile::TempDir,
    ctx_ide: Arc<otto_ide::IdeEngine>,
    db: Arc<otto_dbviewer::DbViewerService>,
}

async fn watch_env(redis_port: u16) -> WatchEnv {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("proj")).unwrap();
    let pool = otto_server::test_support::mem_pool().await;
    let root = seed_user(&pool, "root", true).await;
    let editor = seed_user(&pool, "editor", false).await;
    seed_workspace(&pool, "ws1", &tmp.path().join("proj")).await;
    add_member(&pool, "ws1", &editor.id, "editor").await;
    GrantsRepo::new(pool.clone())
        .set_grants(
            &editor.id,
            &[
                (Feature::Agents, Capability::Edit),
                (Feature::Database, Capability::View),
            ],
        )
        .await
        .unwrap();
    let auth = AuthRepo::new(pool.clone());
    let root_tok = auth
        .issue_api_token(&root.id, Some("root"))
        .await
        .unwrap()
        .0;
    let editor_tok = auth
        .issue_api_token(&editor.id, Some("ed"))
        .await
        .unwrap()
        .0;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let mut ctx = ServerCtx::for_tests(&pool, tmp.path().join("data")).await;
    ctx.base_url = base.clone();
    ctx.ide.db_watch.register_loopback_target(
        "miniredis:test",
        "miniredis",
        IdeDbEngine::Redis,
        redis_port,
        None,
        None,
        Some("0".into()),
    );
    let events = ctx.events.subscribe();
    let ctx_ide = Arc::clone(&ctx.ide);
    let db = Arc::clone(&ctx.db_explorer);
    let (api_extras, root_extras) = otto_server::modules::module_routers(&ctx);
    let app = otto_server::build_router(ctx, api_extras, root_extras);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    WatchEnv {
        base,
        pool,
        root_tok,
        editor_tok,
        events,
        root_dir: tmp,
        ctx_ide,
        db,
    }
}

async fn call(
    method: reqwest::Method,
    url: String,
    tok: &str,
    body: Option<Value>,
) -> (u16, Value) {
    let mut req = reqwest::Client::new().request(method, url).bearer_auth(tok);
    if let Some(b) = body {
        req = req.json(&b);
    }
    let res = req.send().await.unwrap();
    let status = res.status().as_u16();
    (status, res.json().await.unwrap_or(Value::Null))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_watch_checkpoints_diffs_masks_for_agents_and_belongs_to_its_owner() {
    let data = Arc::new(Mutex::new(BTreeMap::from([
        ("wallet:bal:1".to_string(), "100.00".to_string()),
        ("auth:token:1".to_string(), "hunter2".to_string()),
    ])));
    let port = spawn_mini_resp_server(Arc::clone(&data)).await;
    let mut env = watch_env(port).await;
    let root = env
        .root_dir
        .path()
        .join("proj")
        .to_string_lossy()
        .to_string();
    let url = |p: &str| format!("{}/api/v1/ide/db-watch{p}", env.base);
    let start = json!({
        "root": root,
        "target": {"kind": "ephemeral", "ephemeral_id": "miniredis:test"},
        "items": [{"object": "wallet:"}, {"object": "auth:"}],
    });

    // The stateless route that ran a custom statement is gone.
    let (s, _) = call(
        reqwest::Method::POST,
        url("/snapshot"),
        &env.root_tok,
        Some(json!({
            "root": root, "target_kind": "ephemeral", "ephemeral_id": "miniredis:test",
            "engine": "redis", "object_name": "wallet:", "custom_query": "KEYS *"
        })),
    )
    .await;
    assert!(s == 404 || s == 405, "{s}");

    // Ephemeral targets are root-only.
    let (s, _) = call(
        reqwest::Method::POST,
        url("/watches"),
        &env.editor_tok,
        Some(start.clone()),
    )
    .await;
    assert_eq!(s, 403);

    let (s, w) = call(
        reqwest::Method::POST,
        url("/watches"),
        &env.root_tok,
        Some(start),
    )
    .await;
    assert_eq!(s, 200, "{w}");
    let id = w["id"].as_str().unwrap().to_string();
    assert_eq!(w["checkpoint_count"], 1);
    assert_eq!(w["latest"]["reason"], "start");
    assert_eq!(w["target"]["local"], true);
    assert_eq!(w["items"][0]["primary_key"], json!(["key"]));
    let ev = env.events.recv().await.unwrap();
    assert!(
        matches!(
            ev,
            otto_core::event::Event::IdeDbWatchCheckpoint { seq: 0, .. }
        ),
        "{ev:?}"
    );

    data.lock()
        .unwrap()
        .insert("wallet:bal:1".into(), "175.50".into());
    data.lock()
        .unwrap()
        .insert("auth:token:1".into(), "hunter3".into());
    let (s, r) = call(
        reqwest::Method::POST,
        url(&format!("/watches/{id}/snapshot")),
        &env.root_tok,
        Some(json!({})),
    )
    .await;
    assert_eq!(s, 200, "{r}");
    assert_eq!(
        (r["from_seq"].clone(), r["to_seq"].clone()),
        (json!(0), json!(1))
    );
    let wallet = &r["items"][0];
    assert_eq!(wallet["modified_count"], 1, "{wallet}");
    let change = &wallet["diffs"][0]["changes"][0];
    assert_eq!(
        (change["before"].clone(), change["after"].clone()),
        (json!("100.00"), json!("175.50"))
    );
    assert_eq!(
        r["items"][1]["diffs"][0]["changes"][0]["after"], "hunter3",
        "a person reads raw values"
    );

    // What an agent reads is masked; the change still shows.
    let (s, a) = call(
        reqwest::Method::GET,
        url(&format!("/watches/{id}?for_agent=1")),
        &env.root_tok,
        None,
    )
    .await;
    assert_eq!(s, 200);
    let token = &a["items"][1]["diffs"][0];
    assert_eq!(token["masked"], true, "{token}");
    assert_eq!(token["changes"][0]["masked"], true);
    assert!(!a.to_string().contains("hunter"), "{a}");

    // An agent session's own credential never reaches a watch directly (the
    // agent-debug tools go through the person's UI bridge, with for_agent).
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO sessions (id, workspace_id, kind, provider, title, status, cwd, created_by, created_at, last_active_at)
         VALUES ('dbw-agent', 'ws1', 'agent', 'claude', 't', 'idle', '/tmp', 'root', ?, ?)",
    )
    .bind(&now)
    .bind(&now)
    .execute(&env.pool)
    .await
    .unwrap();
    let agent_tok = AuthRepo::new(env.pool.clone())
        .issue_session_api_token(&"root".to_string(), &"dbw-agent".to_string())
        .await
        .unwrap()
        .0;
    let (s, a) = call(
        reqwest::Method::GET,
        url(&format!("/watches/{id}")),
        &agent_tok,
        None,
    )
    .await;
    assert_eq!(s, 403, "{a}");
    assert!(!a.to_string().contains("hunter"), "{a}");
    let (s, _) = call(
        reqwest::Method::POST,
        url("/watches"),
        &agent_tok,
        Some(json!({
            "root": root,
            "target": {"kind": "ephemeral", "ephemeral_id": "miniredis:test"},
            "items": [{"object": "wallet:"}],
        })),
    )
    .await;
    assert_ne!(
        s, 200,
        "an agent credential cannot arm a watch on an ephemeral target"
    );

    // Owner only.
    let (s, _) = call(
        reqwest::Method::GET,
        url(&format!("/watches/{id}")),
        &env.editor_tok,
        None,
    )
    .await;
    assert_eq!(s, 404);
    let (s, l) = call(
        reqwest::Method::GET,
        url(&format!("/watches?root={root}")),
        &env.editor_tok,
        None,
    )
    .await;
    assert_eq!((s, l["watches"].as_array().unwrap().len()), (200, 0));
    let (_, l) = call(
        reqwest::Method::GET,
        url(&format!("/watches?root={root}")),
        &env.root_tok,
        None,
    )
    .await;
    assert_eq!(l["watches"].as_array().unwrap().len(), 1);

    // Objects of the target.
    let (s, o) = call(
        reqwest::Method::POST,
        url("/objects"),
        &env.root_tok,
        Some(json!({
            "root": root, "target": {"kind": "ephemeral", "ephemeral_id": "miniredis:test"}
        })),
    )
    .await;
    assert_eq!(s, 200, "{o}");

    let (s, _) = call(
        reqwest::Method::DELETE,
        url(&format!("/watches/{id}")),
        &env.root_tok,
        None,
    )
    .await;
    assert_eq!(s, 200);
    let (s, _) = call(
        reqwest::Method::GET,
        url(&format!("/watches/{id}")),
        &env.root_tok,
        None,
    )
    .await;
    assert_eq!(s, 404);
    drop(env.ctx_ide);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_production_target_needs_a_person_confirm_and_never_a_remembered_one() {
    let data = Arc::new(Mutex::new(BTreeMap::from([(
        "k:1".to_string(),
        "v".to_string(),
    )])));
    let port = spawn_mini_resp_server(data).await;
    let env = watch_env(port).await;
    let root_user = UsersRepo::new(env.pool.clone())
        .get(&"root".to_string())
        .await
        .unwrap();
    let prod = ConnectionsRepo::new(env.pool.clone())
        .create(NewConnection {
            workspace_id: Some("ws1".into()),
            name: "Prod Redis".into(),
            kind: ConnectionKind::Redis,
            params: json!({"host": "127.0.0.1", "port": port}),
            secret_ref: None,
            first_command: None,
            section_id: None,
            environment: Environment::Prod,
            read_only: false,
            created_by: root_user.id.clone(),
        })
        .await
        .unwrap();
    let root = env
        .root_dir
        .path()
        .join("proj")
        .to_string_lossy()
        .to_string();
    let url = format!("{}/api/v1/ide/db-watch/watches", env.base);
    let mut body = json!({
        "root": root,
        "target": {"kind": "saved", "connection_id": prod.id},
        "items": [{"object": "k:"}],
    });
    let (s, e) = call(
        reqwest::Method::POST,
        url.clone(),
        &env.root_tok,
        Some(body.clone()),
    )
    .await;
    assert_eq!(s, 403, "{e}");
    assert!(e.to_string().contains("production"), "{e}");
    body["confirm_production"] = json!(true);
    let (s, w) = call(
        reqwest::Method::POST,
        url.clone(),
        &env.root_tok,
        Some(body),
    )
    .await;
    assert_eq!(s, 200, "{w}");
    assert_eq!(w["target"]["environment"], "prod");
    assert_eq!(w["target"]["local"], true);
    // Changing it again needs the confirm again.
    let id = w["id"].as_str().unwrap();
    let (s, _) = call(
        reqwest::Method::PATCH,
        format!("{url}/{id}"),
        &env.root_tok,
        Some(json!({"items": [{"object": "k:"}, {"object": "x:"}]})),
    )
    .await;
    assert_eq!(s, 403);
}

/// Real engines (OTTO_DBW_E2E=1, containers from the dbw-engine report):
/// MariaDB 23306 (user nopw, db pr_bo_1), Postgres 25432, Mongo 27717,
/// ClickHouse 28123, Redis 26379.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn real_engines_list_describe_and_survive_rapid_stepping() {
    if std::env::var("OTTO_DBW_E2E").is_err() {
        return;
    }
    let env = watch_env(1).await;
    let root_user = UsersRepo::new(env.pool.clone())
        .get(&"root".to_string())
        .await
        .unwrap();
    let root = env
        .root_dir
        .path()
        .join("proj")
        .to_string_lossy()
        .to_string();
    let url = |p: &str| format!("{}/api/v1/ide/db-watch{p}", env.base);
    let reg = &env.ctx_ide.db_watch;
    reg.register_loopback_target(
        "e:pg",
        "pg",
        IdeDbEngine::Postgres,
        25432,
        Some("otto".into()),
        Some("ottopw".into()),
        Some("shopdb".into()),
    );
    reg.register_loopback_target(
        "e:mongo",
        "mongo",
        IdeDbEngine::Mongodb,
        27717,
        Some("otto".into()),
        Some("ottopw".into()),
        Some("shopdb".into()),
    );
    reg.register_loopback_target(
        "e:ch",
        "ch",
        IdeDbEngine::Clickhouse,
        28123,
        Some("otto".into()),
        Some("ottopw".into()),
        Some("shopdb".into()),
    );
    reg.register_loopback_target(
        "e:redis",
        "redis",
        IdeDbEngine::Redis,
        26379,
        None,
        None,
        Some("0".into()),
    );
    reg.register_loopback_target(
        "e:maria",
        "maria",
        IdeDbEngine::Mysql,
        23306,
        Some("nopw".into()),
        None,
        Some("pr_bo_1".into()),
    );

    // Spike 2: list + describe on every engine.
    for (eph, expect, pk) in [
        ("e:maria", "session_route", json!(["game_id", "account_id"])),
        ("e:pg", "users", json!(["id"])),
        ("e:mongo", "orders", json!(["_id"])),
        ("e:ch", "events", json!(["id"])),
        ("e:redis", "cart:", json!(["key"])),
    ] {
        let target = json!({"kind": "ephemeral", "ephemeral_id": eph});
        let (s, o) = call(
            reqwest::Method::POST,
            url("/objects"),
            &env.root_tok,
            Some(json!({"root": root, "target": target})),
        )
        .await;
        assert_eq!(s, 200, "{eph}: {o}");
        let names: Vec<&str> = o["objects"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|x| x["name"].as_str())
            .collect();
        assert!(names.contains(&expect), "{eph}: {names:?}");
        let eng_root = format!("{root}/{}", eph.replace(':', "_"));
        std::fs::create_dir_all(&eng_root).unwrap();
        let (s, w) = call(
            reqwest::Method::POST,
            url("/watches"),
            &env.root_tok,
            Some(json!({
                "root": eng_root, "target": target, "items": [{"object": expect}]
            })),
        )
        .await;
        assert_eq!(s, 200, "{eph}: {w}");
        assert_eq!(w["items"][0]["primary_key"], pk, "{eph}: {w}");
        assert!(w["items"][0]["error"].is_null(), "{eph}: {w}");
        let (_, r) = call(
            reqwest::Method::GET,
            url(&format!("/watches/{}?rows=1", w["id"].as_str().unwrap())),
            &env.root_tok,
            None,
        )
        .await;
        eprintln!(
            "{eph}: estimate {} · {} · {}",
            w["items"][0]["row_estimate"],
            r["items"][0]["window_note"],
            r["items"][0]["executed_query"]
        );
        assert!(r["items"][0]["error"].is_null(), "{eph}: {r}");
    }

    // A saved MariaDB connection (shared pool): an INSERT on a 400-row table
    // shows; 20 stops in 2 s with a 500 ms item; no history rows.
    let saved = ConnectionsRepo::new(env.pool.clone())
        .create(NewConnection {
            workspace_id: None,
            name: "maria".into(),
            kind: ConnectionKind::Mysql,
            params: json!({"host": "127.0.0.1", "port": 23306, "user": "nopw", "db": "pr_bo_1"}),
            secret_ref: None,
            first_command: None,
            section_id: None,
            environment: Environment::Dev,
            read_only: false,
            created_by: root_user.id.clone(),
        })
        .await
        .unwrap();
    let (s, w) = call(reqwest::Method::POST, url("/watches"), &env.root_tok, Some(json!({
        "root": root,
        "target": {"kind": "saved", "connection_id": saved.id},
        "items": [{"object": "session_info"}, {"object": "session_route"}, {"object": "slow_v"}],
    }))).await;
    assert_eq!(s, 200, "{w}");
    assert_eq!(w["target"]["local"], true);
    let id = w["id"].as_str().unwrap().to_string();
    let conns_before = maria_connections();
    let mysql = |sql: &str| {
        let out = std::process::Command::new("docker")
            .args([
                "exec",
                "nx-dbw-maria",
                "mariadb",
                "-uotto",
                "-pottopw",
                "pr_bo_1",
                "-e",
                sql,
            ])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    mysql("INSERT INTO session_info (account_id, status) VALUES (9001, 'new')");
    let started = std::time::Instant::now();
    let mut tasks = Vec::new();
    for i in 0..20 {
        let (u, tok) = (
            url(&format!("/watches/{id}/snapshot")),
            env.root_tok.clone(),
        );
        tasks.push(tokio::spawn(async move {
            let (s, r) = call(
                reqwest::Method::POST,
                u,
                &tok,
                Some(json!({"label": format!("stop {i}")})),
            )
            .await;
            (s, r)
        }));
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let mut seqs = Vec::new();
    for t in tasks {
        let (s, r) = t.await.unwrap();
        assert_eq!(s, 200, "{r}");
        for it in r["items"].as_array().unwrap() {
            assert!(it["error"].is_null(), "no pool timeouts: {it}");
        }
        seqs.push(r["to_seq"].as_u64().unwrap_or(0));
    }
    let took = started.elapsed();
    let conns_after = maria_connections();
    let (_, r) = call(
        reqwest::Method::GET,
        url(&format!("/watches/{id}?compare=start")),
        &env.root_tok,
        None,
    )
    .await;
    let info = &r["items"][0];
    assert_eq!(
        info["added_count"], 1,
        "the INSERT on a 400-row table shows: {info}"
    );
    assert_eq!(
        info["removed_count"], 0,
        "the row pushed out of the window is not a delete: {info}"
    );
    let checkpoints = r["watch"]["checkpoint_count"].as_u64().unwrap();
    eprintln!(
        "rapid: 20 requests in {took:?} -> {checkpoints} checkpoints; MariaDB connections +{}; window: {}",
        conns_after - conns_before,
        info["window_note"]
    );
    assert!(checkpoints < 21, "coalesced");
    let history = otto_state::DbExplorerRepo::new(env.pool.clone())
        .list_history(&saved.id, 100)
        .await
        .unwrap();
    assert!(
        history.is_empty(),
        "no DB Explorer history rows: {}",
        history.len()
    );

    // Spike 1 baseline: the old path (`run_read_only`, a client deadline)
    // retires the pooled MySQL session after every read.
    let before = maria_connections();
    for _ in 0..10 {
        let q = otto_dbviewer::types::QueryRequest {
            statement: "SELECT * FROM session_route LIMIT 150".into(),
            timeout_ms: Some(10_000),
            ..Default::default()
        };
        env.db
            .run_read_only(&saved.id, &root_user.id, &q)
            .await
            .unwrap();
    }
    eprintln!(
        "old path: 10 reads -> MariaDB connections +{}",
        maria_connections() - before
    );
}

fn maria_connections() -> i64 {
    let out = std::process::Command::new("docker")
        .args([
            "exec",
            "nx-dbw-maria",
            "mariadb",
            "-uroot",
            "-prootpw",
            "-N",
            "-e",
            "SHOW GLOBAL STATUS LIKE 'Connections'",
        ])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap()
}

/// Real Redis (OTTO_DBW_E2E=1, redis:8.8 on 26379): a glob item reads the
/// keys it matches, with types, previews and TTLs from pipelined reads.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn real_redis_glob_items_and_pipelined_values() {
    if std::env::var("OTTO_DBW_E2E").is_err() {
        return;
    }
    let env = watch_env(1).await;
    env.ctx_ide.db_watch.register_loopback_target(
        "e:redis",
        "redis",
        IdeDbEngine::Redis,
        26379,
        None,
        None,
        Some("0".into()),
    );
    let root = env
        .root_dir
        .path()
        .join("proj")
        .to_string_lossy()
        .to_string();
    let url = |p: &str| format!("{}/api/v1/ide/db-watch{p}", env.base);
    let (s, w) = call(
        reqwest::Method::POST,
        url("/watches"),
        &env.root_tok,
        Some(json!({
            "root": root,
            "target": {"kind": "ephemeral", "ephemeral_id": "e:redis"},
            "items": [{"object": "glob:*:route"}, {"object": "glob:"}],
        })),
    )
    .await;
    assert_eq!(s, 200, "{w}");
    let (_, r) = call(
        reqwest::Method::GET,
        url(&format!("/watches/{}?rows=1", w["id"].as_str().unwrap())),
        &env.root_tok,
        None,
    )
    .await;
    let mut keys: Vec<String> = r["items"][0]["current"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row[0].as_str().unwrap().to_string())
        .collect();
    keys.sort();
    assert_eq!(keys, vec!["glob:1:route", "glob:2:route"], "{r}");
    let rows = r["items"][1]["current"]["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 4, "{r}");
    let by_key = |k: &str| rows.iter().find(|row| row[0] == k).unwrap().clone();
    assert_eq!(
        by_key("glob:1:route"),
        json!(["glob:1:route", "string", "a", -1])
    );
    let h = by_key("glob:h");
    assert_eq!(
        (h[1].clone(), h[2].clone()),
        (json!("hash"), json!({"f": "v"})),
        "{h}"
    );
    assert!(h[3].as_i64().unwrap() > 0, "TTL read: {h}");
    assert_eq!(by_key("glob:l")[2], json!(["x", "y"]));
}
