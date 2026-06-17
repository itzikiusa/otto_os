//! End-to-end tests for the Redis driver against a live server.
//!
//! Each test is `#[ignore]` by default and additionally guarded by
//! `OTTO_DBV_E2E`. Expects a Redis seeded by the harness on 127.0.0.1:16379
//! (requirepass `ottoredis`):
//!
//! ```sh
//! docker run -d --name otto-dbv-redis -p 16379:6379 redis:8 \
//!   redis-server --requirepass ottoredis
//! redis-cli -p 16379 -a ottoredis <<'SEED'
//!   SET app:name "Otto Shop"
//!   SET app:version 1.0.0
//!   SET session:abc123 active
//!   HSET customer:1 email ada@example.com name "Ada Lovelace" country GB
//!   RPUSH queue:emails welcome receipt newsletter
//!   SADD countries GB US DE FR
//!   ZADD leaderboard 100 ada 90 alan 80 grace
//!   SET counter:visits 42
//! SEED
//! ```
//!
//! Run: `OTTO_DBV_E2E=1 cargo test -p otto-dbviewer --test redis_e2e -- --ignored --nocapture`

use otto_dbviewer::driver::Driver;
use otto_dbviewer::drivers::redis::RedisDriver;
use otto_dbviewer::types::{
    CompletionKind, Engine, NodeKind, NodePath, QueryRequest, ResolvedConfig, TlsConfig,
};
use serde_json::json;

fn cfg() -> ResolvedConfig {
    ResolvedConfig {
        engine: Engine::Redis,
        host: "127.0.0.1".into(),
        port: 16379,
        user: None,
        password: Some("ottoredis".into()),
        database: Some("0".into()),
        tls: TlsConfig::default(),
        params: json!({}),
    }
}

fn query(stmt: &str) -> QueryRequest {
    QueryRequest {
        statement: stmt.into(),
        max_rows: None,
        ..Default::default()
    }
}

/// `test()` PINGs and reports the server version.
#[tokio::test]
#[ignore]
async fn redis_connect() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }

    let d = RedisDriver::default();
    let test = d.test(&cfg()).await.expect("test() returned Err");
    assert!(test.ok, "expected test ok, got: {}", test.message);
    assert!(
        test.server_version.is_some(),
        "expected a server version from INFO server"
    );
    println!(
        "test ok: version={:?} latency={:?}ms",
        test.server_version, test.latency_ms
    );
}

/// schema_root has a Keyspace node, and it expands to children.
#[tokio::test]
#[ignore]
async fn redis_schema_tree() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }

    let d = RedisDriver::default();
    let cfg = cfg();

    let roots = d.schema_root(&cfg).await.expect("schema_root failed");
    assert!(!roots.is_empty(), "schema_root should not be empty");
    let keyspace = roots
        .iter()
        .find(|n| n.kind == NodeKind::Keyspace)
        .expect("schema_root should contain a Keyspace node");

    // Expand the keyspace node — should yield namespaces/keys.
    let path = NodePath::parse(&keyspace.id);
    let children = d
        .schema_children(&cfg, &path)
        .await
        .expect("schema_children(keyspace) failed");
    assert!(
        !children.is_empty(),
        "expanding the keyspace should yield children; got none"
    );
    println!(
        "keyspace {} -> {} child node(s)",
        keyspace.label,
        children.len()
    );
}

/// object_detail of a string key reports extra.type == "string".
#[tokio::test]
#[ignore]
async fn redis_object_detail() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }

    let d = RedisDriver::default();
    let cfg = cfg();

    let path = NodePath::parse("kdb:0/key:app:name");
    let detail = d
        .object_detail(&cfg, &path)
        .await
        .expect("object_detail failed");
    assert_eq!(
        detail.extra.get("type").and_then(|v| v.as_str()),
        Some("string"),
        "object_detail extra.type should be 'string', got: {:?}",
        detail.extra
    );
}

/// run() executes GET and HGETALL commands against the seeded keys.
#[tokio::test]
#[ignore]
async fn redis_run() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }

    let d = RedisDriver::default();
    let cfg = cfg();

    // GET app:name -> single cell "Otto Shop".
    let got = d.run(&cfg, &query("GET app:name")).await.expect("GET failed");
    assert_eq!(got.rows.len(), 1, "GET should return a single row");
    assert_eq!(
        got.rows[0][0],
        json!("Otto Shop"),
        "GET app:name should be 'Otto Shop'"
    );

    // HGETALL customer:1 -> contains the seeded email somewhere.
    let hash = d
        .run(&cfg, &query("HGETALL customer:1"))
        .await
        .expect("HGETALL failed");
    let blob = serde_json::to_string(&hash.rows).unwrap();
    assert!(
        blob.contains("ada@example.com"),
        "HGETALL customer:1 should include ada@example.com, got: {blob}"
    );
}

/// completion offers the GET command and a live key prefix.
#[tokio::test]
#[ignore]
async fn redis_completion() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }

    let d = RedisDriver::default();
    let cfg = cfg();

    let completions = d
        .completion(&cfg, &Default::default())
        .await
        .expect("completion failed");
    assert!(
        completions
            .items
            .iter()
            .any(|c| c.label == "GET" && c.kind == CompletionKind::Command),
        "completion should include the GET command"
    );
    assert!(
        completions.items.iter().any(|c| {
            c.kind == CompletionKind::Field
                && (c.label.starts_with("customer") || c.label.starts_with("session"))
        }),
        "completion should include a live key prefix (customer:/session:)"
    );
}
