//! End-to-end tests for the Redis driver against a live server.
//!
//! Each test is `#[ignore]` by default and additionally guarded by
//! `OTTO_DBV_E2E`. Expects the dev-stack Redis on 127.0.0.1:16379 (requirepass
//! `ottoredis`), which `dev/dbviewer/docker-compose.yml` seeds once (after the
//! server is healthy) from `dev/dbviewer/seed/redis/seed.txt`:
//!
//! ```sh
//! docker compose -f dev/dbviewer/docker-compose.yml up -d
//! ```
//!
//! The seed loads a small, mixed-type keyspace on db0 (verbatim from `seed.txt`):
//!
//! ```text
//! SET   app:name "Otto Shop"
//! SET   app:version "1.4.2"
//! SET   session:abc123 "{...json...}"
//! SET   session:def456 "{...json...}"
//! SETEX cache:product:1 3600 "Mechanical Keyboard"
//! HSET  customer:1 email ada@example.com name "Ada Lovelace" country GB
//! HSET  customer:2 email alan@example.com name "Alan Turing"  country GB
//! RPUSH queue:emails "welcome:1" "welcome:2" "receipt:4"
//! SADD  countries GB US FI DE
//! ZADD  leaderboard 4200 ada 3900 grace 5100 linus
//! INCR  counter:visits            # x3 -> counter:visits == 3
//! ```
//!
//! `redis_prefix_filter_and_cap` needs a prefix-heavy keyspace that `seed.txt`
//! deliberately does NOT carry, so it SELF-SEEDS its own `user:*` keys and 2000
//! `bigns:*` keys at the start and removes them again before returning — keeping
//! the whole suite runnable against the stock docker stack.
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

/// Number of `bigns:*` keys `redis_prefix_filter_and_cap` seeds — several times
/// the driver's per-listing cap (`KEY_LIST_CAP` = 500), so the broad-filter path
/// is forced to truncate.
const BIGNS_SEED_COUNT: usize = 2000;
/// The `user:*` keys the same test seeds for its narrow-filter assertions.
const USER_SEED_KEYS: &[&str] = &["user:alice", "user:bob", "user:carol", "user:dave"];

/// Seed `redis_prefix_filter_and_cap`'s fixture keys through the driver's own
/// command path (a single `MSET` on db0). Idempotent — a re-run just overwrites,
/// so a previous crash that skipped teardown does not perturb the counts.
async fn seed_prefix_keys(d: &RedisDriver, cfg: &ResolvedConfig) {
    let mut stmt = String::from("MSET");
    for i in 0..BIGNS_SEED_COUNT {
        stmt.push_str(&format!(" bigns:{i} 1"));
    }
    for k in USER_SEED_KEYS {
        stmt.push_str(&format!(" {k} 1"));
    }
    d.run(cfg, &query(&stmt)).await.expect("seed MSET failed");
}

/// Best-effort removal of everything `seed_prefix_keys` created (one `UNLINK`).
async fn cleanup_prefix_keys(d: &RedisDriver, cfg: &ResolvedConfig) {
    let mut stmt = String::from("UNLINK");
    for i in 0..BIGNS_SEED_COUNT {
        stmt.push_str(&format!(" bigns:{i}"));
    }
    for k in USER_SEED_KEYS {
        stmt.push_str(&format!(" {k}"));
    }
    let _ = d.run(cfg, &query(&stmt)).await;
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
        .schema_children(&cfg, &path, None)
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

/// Prefix filter narrows a huge keyspace; broad matches are capped with a hint,
/// and every key node carries a (bulk-looked-up) type. Validates the behaviour
/// behind the Redis "filter by prefix / limit results" tree feature.
#[tokio::test]
#[ignore]
async fn redis_prefix_filter_and_cap() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }

    let d = RedisDriver::default();
    let cfg = cfg();
    let ks = NodePath::parse("kdb:0");

    // Prefix filtering + capping needs a prefix-heavy keyspace the shared seed
    // omits. Seed the fixture here so the test is self-contained against the
    // stock docker stack, and tear it down before returning.
    seed_prefix_keys(&d, &cfg).await;

    // Gather every listing up front, THEN clean up, THEN assert. A panic inside an
    // assertion would otherwise skip the teardown and leak the 2000 `bigns:*` keys.
    let users = d
        .schema_children(&cfg, &ks, Some("user:"))
        .await
        .expect("filter user:");
    let big = d
        .schema_children(&cfg, &ks, Some("bigns:"))
        .await
        .expect("filter bigns:");
    let overview = d
        .schema_children(&cfg, &ks, None)
        .await
        .expect("overview");

    cleanup_prefix_keys(&d, &cfg).await;

    // Narrow filter → flat list of only user:* keys, typed, no truncation hint.
    assert!(!users.is_empty(), "user: filter should match seeded keys");
    assert!(
        users.iter().all(|n| n.kind == NodeKind::Key),
        "a filtered listing is flat keys (no namespaces)"
    );
    assert!(
        users.iter().all(|n| n.label.starts_with("user:")),
        "every result must match the prefix"
    );
    assert!(
        users.iter().all(|n| n.detail.as_deref() == Some("string")),
        "bulk TYPE should label each key 'string'"
    );
    println!("filter user: -> {} keys", users.len());

    // Broad filter (2000 keys) → capped at the per-listing cap (500) with a
    // trailing truncation hint (a passive Folder node marked with ⋯).
    let keys = big.iter().filter(|n| n.kind == NodeKind::Key).count();
    let hint = big.iter().find(|n| n.kind == NodeKind::Folder);
    assert_eq!(keys, 500, "should fill the 500-key cap for 2000 matches");
    assert!(
        big.iter()
            .filter(|n| n.kind == NodeKind::Key)
            .all(|n| n.label.starts_with("bigns:")),
        "every capped result must still match the prefix"
    );
    let hint = hint.expect("a truncation hint should be appended when capped");
    assert!(hint.label.starts_with('⋯'), "hint label marked with ⋯");
    assert!(!hint.has_children, "the hint is passive (not expandable)");
    println!("filter bigns: -> {keys} keys + hint {:?}", hint.label);

    // Overview (no filter) still groups by namespace prefix.
    assert!(
        overview
            .iter()
            .any(|n| n.kind == NodeKind::KeyNamespace && n.label.starts_with("bigns")),
        "overview should include a bigns namespace group"
    );
    println!("overview -> {} nodes", overview.len());
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
    // Completion is commands-only by design — it must NOT scan the keyspace for
    // live key prefixes (that stalled typing for seconds on large databases).
    assert!(
        completions
            .items
            .iter()
            .all(|c| c.kind == CompletionKind::Command),
        "completion should be commands-only (no key-prefix Field items)"
    );
}
