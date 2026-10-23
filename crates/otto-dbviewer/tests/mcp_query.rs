//! Integration tests for the read-only MCP query path
//! ([`DbViewerService::run_read_only`]) — the trust boundary for agent-supplied
//! SQL. A write/DDL is refused **before** the driver is touched, on ANY
//! connection (independent of the connection's write-guard), while a read is
//! allowed through to execution.
//!
//! Uses an in-memory SQLite metadata DB and a connection that points at a dead
//! local port, so a "read" attempt fails at connect — which proves it passed the
//! read-only gate — without needing a real external database.

use std::sync::Arc;

use chrono::Utc;
use otto_core::auth::{with_request_credential, RequestCredential};
use otto_core::domain::{ConnectionKind, Environment};
use otto_core::secrets::SecretStore;
use otto_core::{Error, Id, Result};
use otto_dbviewer::service::{MCP_READ_ONLY_PREFIX, READ_ONLY_PREFIX, WRITE_BLOCKED_PREFIX};
use otto_dbviewer::types::QueryRequest;
use otto_dbviewer::DbViewerService;
use otto_state::{ConnectionsRepo, DbExplorerRepo, DbPool, NewConnection};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

struct NullSecrets;
impl SecretStore for NullSecrets {
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

async fn mem_pool() -> DbPool {
    let opts = SqliteConnectOptions::new()
        .in_memory(true)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .expect("in-memory pool");
    sqlx::migrate!("../otto-state/migrations")
        .run(&pool)
        .await
        .expect("migrations");
    pool.into()
}

async fn seed_user(pool: &DbPool) -> Id {
    let id = otto_core::new_id();
    let now = Utc::now().format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string();
    sqlx::query(
        "INSERT INTO users (id, username, password_hash, display_name, is_root, disabled, created_at)
         VALUES (?, ?, ?, ?, 0, 0, ?)",
    )
    .bind(&id)
    .bind("u")
    .bind("h")
    .bind("u")
    .bind(&now)
    .execute(pool)
    .await
    .expect("seed user");
    id
}

async fn seed_ws(pool: &DbPool) -> Id {
    let id = otto_core::new_id();
    let now = Utc::now().format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string();
    sqlx::query("INSERT INTO workspaces (id, name, root_path, created_at) VALUES (?, ?, ?, ?)")
        .bind(&id)
        .bind("ws")
        .bind("/tmp")
        .bind(&now)
        .execute(pool)
        .await
        .expect("seed workspace");
    id
}

async fn seed_conn(pool: &DbPool, ws: Option<Id>, user: &Id, kind: ConnectionKind) -> Id {
    seed_guarded(pool, ws, user, kind, Environment::Dev, false).await
}

/// [`seed_conn`] with the connection's write-guard settings: its environment
/// and its `read_only` flag.
async fn seed_guarded(
    pool: &DbPool,
    ws: Option<Id>,
    user: &Id,
    kind: ConnectionKind,
    environment: Environment,
    read_only: bool,
) -> Id {
    let conn = ConnectionsRepo::new(pool.clone())
        .create(NewConnection {
            workspace_id: ws,
            name: "t".into(),
            kind,
            // A dead local port: a read passes the gate, then fails at connect.
            params: serde_json::json!({ "host": "127.0.0.1", "port": 1, "user": "x", "db": "x" }),
            secret_ref: None,
            first_command: None,
            section_id: None,
            environment,
            read_only,
            created_by: user.clone(),
        })
        .await
        .expect("seed connection");
    let repo = otto_state::resource_access::ResourceAccessRepo::new(pool.clone());
    let mut policy = repo
        .get_policy(otto_core::access::ResourceKind::Connection, &conn.id)
        .await
        .unwrap();
    let revision = policy.revision;
    policy.mode = otto_core::access::AccessMode::Legacy;
    policy.rules.clear();
    repo.put_policy(
        &policy,
        revision,
        &otto_core::access::AccessActor {
            real_user_id: user.clone(),
            effective_user_id: None,
        },
    )
    .await
    .unwrap();
    conn.id
}

fn service(pool: &DbPool) -> DbViewerService {
    DbViewerService::new(
        ConnectionsRepo::new(pool.clone()),
        Arc::new(NullSecrets),
        DbExplorerRepo::new(pool.clone()),
    )
}

fn req(stmt: &str) -> QueryRequest {
    QueryRequest {
        statement: stmt.into(),
        ..Default::default()
    }
}

#[tokio::test]
async fn write_is_blocked_before_connecting() {
    let pool = mem_pool().await;
    let user = seed_user(&pool).await;
    let ws = seed_ws(&pool).await;
    // A non-write-guarded (dev, read_only=false) connection — the write-guard does
    // NOT protect it; only run_read_only's unconditional gate does.
    let conn = seed_conn(&pool, Some(ws), &user, ConnectionKind::Mysql).await;
    let svc = service(&pool);
    for stmt in [
        "DROP TABLE t",
        "DELETE FROM t",
        "UPDATE t SET a = 1",
        "INSERT INTO t VALUES (1)",
        "SELECT 1; DROP TABLE t", // injection via a trailing statement
    ] {
        let err = svc
            .run_read_only(&conn, &user, &req(stmt))
            .await
            .expect_err("a write must be refused");
        assert!(
            matches!(err, Error::Forbidden(_)),
            "expected Forbidden for {stmt:?}, got {err:?}"
        );
    }
}

#[tokio::test]
async fn read_passes_the_gate_and_reaches_execution() {
    let pool = mem_pool().await;
    let user = seed_user(&pool).await;
    let ws = seed_ws(&pool).await;
    let conn = seed_conn(&pool, Some(ws), &user, ConnectionKind::Mysql).await;
    let svc = service(&pool);
    // A SELECT is allowed through the read-only gate; it then attempts to connect
    // (dead port). The crucial property: it is NOT refused as a write. Bounded to
    // 3s — a still-connecting timeout equally proves the gate let it through.
    let qreq = req("SELECT 1");
    let fut = svc.run_read_only(&conn, &user, &qreq);
    match tokio::time::timeout(std::time::Duration::from_secs(3), fut).await {
        Ok(Ok(_)) => {} // unexpectedly reachable — still proves it passed the gate
        Ok(Err(e)) => assert!(
            !matches!(e, Error::Forbidden(_)),
            "a SELECT must not be refused as a write; got {e:?}"
        ),
        Err(_timeout) => {} // still trying to connect ⇒ it passed the read-only gate
    }
}

#[tokio::test]
async fn non_queryable_kind_is_invalid() {
    let pool = mem_pool().await;
    let user = seed_user(&pool).await;
    let ws = seed_ws(&pool).await;
    let conn = seed_conn(&pool, Some(ws), &user, ConnectionKind::Ssh).await;
    let svc = service(&pool);
    let err = svc
        .run_read_only(&conn, &user, &req("SELECT 1"))
        .await
        .expect_err("ssh is not a queryable database");
    assert!(matches!(err, Error::Invalid(_)), "got {err:?}");
}

// --- `QueryRequest.read_only` over the normal `run` path (agent-driven UI) ----

fn ro(stmt: &str) -> QueryRequest {
    QueryRequest {
        statement: stmt.into(),
        read_only: true,
        ..Default::default()
    }
}

/// Bounded run: a dead-port connect either errors or is still retrying after 3s;
/// both mean the statement got PAST the gate. `None` = still connecting.
async fn run_bounded(
    svc: &DbViewerService,
    conn: &Id,
    user: &Id,
    q: &QueryRequest,
) -> Option<Result<otto_dbviewer::types::QueryResult>> {
    tokio::time::timeout(std::time::Duration::from_secs(3), svc.run(conn, user, q))
        .await
        .ok()
}

#[tokio::test]
async fn read_only_run_refuses_a_write_on_an_unguarded_connection() {
    let pool = mem_pool().await;
    let user = seed_user(&pool).await;
    let ws = seed_ws(&pool).await;
    // Dev + read_only=false: the write-guard would let every one of these through.
    let conn = seed_conn(&pool, Some(ws), &user, ConnectionKind::Mysql).await;
    let svc = service(&pool);
    for stmt in [
        "DROP TABLE t",
        "DELETE FROM t",
        "UPDATE t SET a = 1",
        "INSERT INTO t VALUES (1)",
        "SELECT 1; DROP TABLE t",
    ] {
        match svc.run(&conn, &user, &ro(stmt)).await {
            Err(Error::Forbidden(m)) => assert!(
                m.starts_with(READ_ONLY_PREFIX) && !m.starts_with(MCP_READ_ONLY_PREFIX),
                "{stmt:?}: wrong marker: {m}"
            ),
            // Only the outcome's shape — never a row/connection value — goes
            // into the panic message.
            Ok(_) => panic!("{stmt:?}: expected a read_only Forbidden, got Ok"),
            Err(_) => panic!("{stmt:?}: expected a read_only Forbidden, got another error kind"),
        }
    }
}

#[tokio::test]
async fn read_only_wins_over_confirm_write() {
    let pool = mem_pool().await;
    let user = seed_user(&pool).await;
    let ws = seed_ws(&pool).await;
    let conn = seed_conn(&pool, Some(ws), &user, ConnectionKind::Mysql).await;
    let svc = service(&pool);
    let q = QueryRequest {
        confirm_write: true,
        ..ro("DELETE FROM t")
    };
    let err = svc
        .run(&conn, &user, &q)
        .await
        .expect_err("read_only must refuse");
    assert!(
        matches!(err, Error::Forbidden(ref m) if m.starts_with(READ_ONLY_PREFIX)),
        "{err:?}"
    );
}

#[tokio::test]
async fn the_same_write_without_read_only_is_not_refused() {
    // The control: the flag is what refuses — a plain run of the same write on
    // the same unguarded connection passes every gate and fails only at connect.
    let pool = mem_pool().await;
    let user = seed_user(&pool).await;
    let ws = seed_ws(&pool).await;
    let conn = seed_conn(&pool, Some(ws), &user, ConnectionKind::Mysql).await;
    let svc = service(&pool);
    if let Some(Err(e)) = run_bounded(&svc, &conn, &user, &req("DELETE FROM t")).await {
        assert!(
            !matches!(e, Error::Forbidden(_)),
            "plain run refused: {e:?}"
        );
    }
}

#[tokio::test]
async fn read_only_read_passes_the_gate_and_reaches_execution() {
    let pool = mem_pool().await;
    let user = seed_user(&pool).await;
    let ws = seed_ws(&pool).await;
    let conn = seed_conn(&pool, Some(ws), &user, ConnectionKind::Mysql).await;
    let svc = service(&pool);
    if let Some(Err(e)) = run_bounded(&svc, &conn, &user, &ro("SELECT 1")).await {
        assert!(
            !matches!(e, Error::Forbidden(_)),
            "a read_only SELECT must not be refused; got {e:?}"
        );
    }
}

#[tokio::test]
async fn read_only_on_a_non_queryable_kind_is_invalid() {
    let pool = mem_pool().await;
    let user = seed_user(&pool).await;
    let ws = seed_ws(&pool).await;
    let conn = seed_conn(&pool, Some(ws), &user, ConnectionKind::Ssh).await;
    let svc = service(&pool);
    let err = svc
        .run(&conn, &user, &ro("SELECT 1"))
        .await
        .expect_err("ssh is not a queryable database");
    assert!(matches!(err, Error::Invalid(_)), "got {err:?}");
}

#[test]
fn read_only_defaults_off_and_deserializes() {
    let off: QueryRequest = serde_json::from_str(r#"{"statement":"SELECT 1"}"#).unwrap();
    assert!(!off.read_only, "an old client that omits it runs normally");
    let on: QueryRequest =
        serde_json::from_str(r#"{"statement":"SELECT 1","read_only":true}"#).unwrap();
    assert!(on.read_only);
}

// --- an agent's write over the Explorer path (`otto.query_db`) -------------
//
// The governed `otto.query_db` tool self-calls `/db/query` (`run`) with an
// agent session's credential and never sends `confirm_write`. These pin that
// the connection's write-guard is what decides, and that its refusal tells
// the agent which setting lifts it.

/// An agent session's credential, as the governed self-call presents it.
const AGENT: RequestCredential = RequestCredential {
    agent: true,
    root_withheld: true,
};

async fn agent_run(
    svc: &DbViewerService,
    conn: &Id,
    user: &Id,
    q: &QueryRequest,
) -> Option<Result<otto_dbviewer::types::QueryResult>> {
    with_request_credential(AGENT, run_bounded(svc, conn, user, q)).await
}

#[tokio::test]
async fn an_agent_write_on_a_writable_connection_passes_every_gate() {
    let pool = mem_pool().await;
    let user = seed_user(&pool).await;
    let ws = seed_ws(&pool).await;
    let conn = seed_conn(&pool, Some(ws), &user, ConnectionKind::Mysql).await;
    let svc = service(&pool);
    for stmt in [
        "DELETE FROM t",
        "INSERT INTO t VALUES (1)",
        "CREATE TABLE t2 (a INT)",
    ] {
        if let Some(Err(e)) = agent_run(&svc, &conn, &user, &req(stmt)).await {
            assert!(
                !matches!(e, Error::Forbidden(_) | Error::Conflict(_)),
                "{stmt:?}: a writable connection refused the agent's write: {e:?}"
            );
        }
    }
}

#[tokio::test]
async fn an_agent_write_by_a_non_root_owner_on_an_enforced_connection_runs() {
    // New connections start access-enforced and private to their creator: the
    // owner's own writable connection must still take the agent's write.
    let pool = mem_pool().await;
    let user = seed_user(&pool).await;
    // The UI's own prerequisite for running a write: Database Edit.
    otto_state::GrantsRepo::new(pool.clone())
        .set_grants(
            &user,
            &[(
                otto_core::domain::Feature::Database,
                otto_core::domain::Capability::Edit,
            )],
        )
        .await
        .unwrap();
    let ws = seed_ws(&pool).await;
    sqlx::query(
        "INSERT INTO workspace_members (workspace_id, user_id, role) VALUES (?, ?, 'admin')",
    )
    .bind(&ws)
    .bind(&user)
    .execute(&pool)
    .await
    .expect("membership");
    let conn = ConnectionsRepo::new(pool.clone())
        .create(NewConnection {
            workspace_id: Some(ws),
            name: "mine".into(),
            kind: ConnectionKind::Mysql,
            params: serde_json::json!({ "host": "127.0.0.1", "port": 1, "user": "x", "db": "x" }),
            secret_ref: None,
            first_command: None,
            section_id: None,
            environment: Environment::Dev,
            read_only: false,
            created_by: user.clone(),
        })
        .await
        .expect("seed connection")
        .id;
    // The creator policy the Connections service seeds for a Database-Edit
    // owner (`otto_connections::service`, on create).
    let ops: Vec<String> = [
        "discover",
        "configure",
        "manage_access",
        "db_browse",
        "db_query",
        "db_export",
        "db_data",
        "db_schema",
        "change_submit",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    let access = otto_state::resource_access::ResourceAccessRepo::new(pool.clone());
    access
        .initialize_owner_policy(
            otto_core::access::ResourceKind::Connection,
            &conn,
            &user,
            &ops,
            &ops,
            &otto_core::access::AccessActor {
                real_user_id: user.clone(),
                effective_user_id: None,
            },
        )
        .await
        .unwrap();
    let policy = access
        .get_policy(otto_core::access::ResourceKind::Connection, &conn)
        .await
        .unwrap();
    assert_eq!(policy.mode, otto_core::access::AccessMode::Enforced);
    let svc = service(&pool);
    let q = QueryRequest {
        node: Some("x".into()),
        ..req("DELETE FROM t")
    };
    if let Some(Err(e)) = agent_run(&svc, &conn, &user, &q).await {
        assert!(
            !matches!(
                e,
                Error::Forbidden(_) | Error::Conflict(_) | Error::NotFound(_)
            ),
            "the owner's writable connection refused the agent's write: {e:?}"
        );
    }
}

#[tokio::test]
async fn an_agent_write_on_a_read_only_connection_names_the_setting() {
    let pool = mem_pool().await;
    let user = seed_user(&pool).await;
    let ws = seed_ws(&pool).await;
    let conn = seed_guarded(
        &pool,
        Some(ws),
        &user,
        ConnectionKind::Mysql,
        Environment::Dev,
        true,
    )
    .await;
    let svc = service(&pool);
    for q in [
        req("DELETE FROM t"),
        // An agent's confirmation is not a person's (S6-304).
        QueryRequest {
            confirm_write: true,
            ..req("UPDATE t SET a = 1")
        },
    ] {
        let err = with_request_credential(AGENT, svc.run(&conn, &user, &q))
            .await
            .expect_err("a read-only connection refuses an agent's write");
        let msg = err.to_string();
        if q.confirm_write {
            assert!(matches!(err, Error::Forbidden(_)), "{err:?}");
            continue;
        }
        assert!(matches!(err, Error::Conflict(_)), "{err:?}");
        assert!(msg.contains(WRITE_BLOCKED_PREFIX), "{msg}");
        assert!(msg.contains("read-only connection 't'"), "{msg}");
        assert!(
            msg.contains("turn off Read-only in the connection's settings"),
            "{msg}"
        );
        assert!(!msg.contains("confirm the write"), "{msg}");
    }
    // A read on the same connection still runs.
    if let Some(Err(e)) = agent_run(&svc, &conn, &user, &req("SELECT 1")).await {
        assert!(
            !matches!(e, Error::Forbidden(_) | Error::Conflict(_)),
            "{e:?}"
        );
    }
}

#[tokio::test]
async fn an_agent_write_on_a_production_connection_names_the_environment() {
    let pool = mem_pool().await;
    let user = seed_user(&pool).await;
    let ws = seed_ws(&pool).await;
    let prod = seed_guarded(
        &pool,
        Some(ws.clone()),
        &user,
        ConnectionKind::Mysql,
        Environment::Prod,
        false,
    )
    .await;
    let both = seed_guarded(
        &pool,
        Some(ws),
        &user,
        ConnectionKind::Mysql,
        Environment::Prod,
        true,
    )
    .await;
    let svc = service(&pool);
    let msg = with_request_credential(AGENT, svc.run(&prod, &user, &req("DROP TABLE t")))
        .await
        .expect_err("production refuses")
        .to_string();
    assert!(msg.contains("production connection 't'"), "{msg}");
    assert!(
        msg.contains("set an environment other than Production in the connection's settings"),
        "{msg}"
    );
    let msg = with_request_credential(AGENT, svc.run(&both, &user, &req("DROP TABLE t")))
        .await
        .expect_err("production + read-only refuses")
        .to_string();
    assert!(
        msg.contains("turn off Read-only and set an environment other than Production"),
        "{msg}"
    );
}

#[tokio::test]
async fn a_person_still_gets_the_typed_confirmation_prompt() {
    // The UI's own run: the message the Explorer keys its typed confirm on is
    // unchanged for a person, and their confirmation still lifts the guard.
    let pool = mem_pool().await;
    let user = seed_user(&pool).await;
    let ws = seed_ws(&pool).await;
    let conn = seed_guarded(
        &pool,
        Some(ws),
        &user,
        ConnectionKind::Mysql,
        Environment::Dev,
        true,
    )
    .await;
    let svc = service(&pool);
    let person = RequestCredential::default();
    let msg = with_request_credential(person, svc.run(&conn, &user, &req("DELETE FROM t")))
        .await
        .expect_err("unconfirmed")
        .to_string();
    assert!(
        msg.contains(WRITE_BLOCKED_PREFIX) && msg.contains("confirm the write to run it"),
        "{msg}"
    );
    let confirmed = QueryRequest {
        confirm_write: true,
        ..req("DELETE FROM t")
    };
    let out = with_request_credential(person, run_bounded(&svc, &conn, &user, &confirmed)).await;
    if let Some(Err(e)) = out {
        assert!(
            !matches!(e, Error::Forbidden(_) | Error::Conflict(_)),
            "a person's confirmation lifts the guard: {e:?}"
        );
    }
}

#[tokio::test]
async fn the_read_only_path_still_refuses_an_agent_write_on_a_writable_connection() {
    let pool = mem_pool().await;
    let user = seed_user(&pool).await;
    let ws = seed_ws(&pool).await;
    let conn = seed_conn(&pool, Some(ws), &user, ConnectionKind::Mysql).await;
    let svc = service(&pool);
    let err = with_request_credential(
        AGENT,
        svc.run_read_only(&conn, &user, &req("DELETE FROM t")),
    )
    .await
    .expect_err("the read-only MCP path never writes");
    assert!(
        matches!(err, Error::Forbidden(ref m) if m.starts_with(MCP_READ_ONLY_PREFIX)),
        "{err:?}"
    );
}
