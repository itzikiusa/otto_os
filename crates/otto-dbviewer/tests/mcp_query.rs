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
use otto_core::domain::{ConnectionKind, Environment};
use otto_core::secrets::SecretStore;
use otto_core::{Error, Id, Result};
use otto_dbviewer::types::QueryRequest;
use otto_dbviewer::DbViewerService;
use otto_state::{ConnectionsRepo, DbExplorerRepo, NewConnection, SqlitePool};
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

async fn mem_pool() -> SqlitePool {
    let opts = SqliteConnectOptions::new().in_memory(true).foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .expect("in-memory pool");
    sqlx::migrate!("../otto-state/migrations")
        .run(&pool)
        .await
        .expect("migrations");
    pool
}

async fn seed_user(pool: &SqlitePool) -> Id {
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

async fn seed_ws(pool: &SqlitePool) -> Id {
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

async fn seed_conn(pool: &SqlitePool, ws: Option<Id>, user: &Id, kind: ConnectionKind) -> Id {
    ConnectionsRepo::new(pool.clone())
        .create(NewConnection {
            workspace_id: ws,
            name: "t".into(),
            kind,
            // A dead local port: a read passes the gate, then fails at connect.
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
        .id
}

fn service(pool: &SqlitePool) -> DbViewerService {
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
