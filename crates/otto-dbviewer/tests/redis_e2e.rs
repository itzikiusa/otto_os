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
        lifecycle: None,
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
    let got = d
        .run(&cfg, &query("GET app:name"))
        .await
        .expect("GET failed");
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
    let overview = d.schema_children(&cfg, &ks, None).await.expect("overview");

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

/// Live parity verification for L13–L16 against an isolated Redis container
/// (`OTTO_REDIS_PARITY_PORT`, defaulting to 26379 or 16379).
#[tokio::test]
#[ignore]
async fn redis_parity_quotes_batches_transactions_select_and_blocking() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }

    let port = std::env::var("OTTO_REDIS_PARITY_PORT")
        .ok()
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(26379);
    let password = std::env::var("OTTO_REDIS_PARITY_PASSWORD").ok();
    let cfg = ResolvedConfig {
        lifecycle: None,
        engine: Engine::Redis,
        host: "127.0.0.1".into(),
        port,
        user: None,
        password,
        database: Some("0".into()),
        tls: TlsConfig::default(),
        params: json!({}),
    };
    let d = RedisDriver::default();

    // 1. L14 sdssplitargs quote & escape parity against real Redis server.
    let quotes_batch = d
        .run(
            &cfg,
            &query("ECHO \"a\\q\"\nECHO 'a\\\\b'\nECHO \"\\X41\"\nECHO \"\\x41\"\nECHO foo\"bar\""),
        )
        .await
        .expect("quote batch");
    assert_eq!(quotes_batch.rows, vec![vec![json!("aq")]]);
    assert_eq!(quotes_batch.more_results.len(), 4);
    assert_eq!(
        quotes_batch.more_results[0].rows,
        vec![vec![json!("a\\\\b")]]
    );
    assert_eq!(quotes_batch.more_results[1].rows, vec![vec![json!("X41")]]);
    assert_eq!(quotes_batch.more_results[2].rows, vec![vec![json!("A")]]);
    assert_eq!(
        quotes_batch.more_results[3].rows,
        vec![vec![json!("foobar")]]
    );

    // Adjacent unquoted token after closing quote must fail with validation error.
    let adj_err = d
        .run(&cfg, &query("ECHO \"foo\"bar"))
        .await
        .expect_err("closing quote followed by token must error");
    assert!(adj_err
        .to_string()
        .contains("closing quote must be followed by whitespace"));

    // 2. L16 MULTI/EXEC transaction on the same operation-isolated connection,
    // including nested runtime error element inside EXEC array.
    let tx = d
        .run(
            &cfg,
            &query(
                "DEL parity:tx:k parity:tx:str\nSET parity:tx:str hello\nMULTI\nINCR parity:tx:k\nLPUSH parity:tx:str bad\nGET parity:tx:k\nEXEC",
            ),
        )
        .await
        .expect("MULTI/EXEC batch");
    assert_eq!(tx.more_results.len(), 6);
    assert_eq!(tx.more_results[1].rows, vec![vec![json!("OK")]]); // MULTI
    assert_eq!(tx.more_results[2].rows, vec![vec![json!("QUEUED")]]); // INCR
    assert_eq!(tx.more_results[3].rows, vec![vec![json!("QUEUED")]]); // LPUSH
    assert_eq!(tx.more_results[4].rows, vec![vec![json!("QUEUED")]]); // GET
    let exec_rows = &tx.more_results[5].rows;
    assert_eq!(exec_rows.len(), 3);
    assert_eq!(exec_rows[0], vec![json!(1)]);
    assert!(
        exec_rows[1][0]
            .as_str()
            .unwrap_or_default()
            .contains("WRONGTYPE"),
        "runtime error inside EXEC should be rendered as a row cell: {:?}",
        exec_rows[1]
    );
    assert_eq!(exec_rows[2], vec![json!("1")]);

    // 3. L16 Operation isolation: an uncommitted MULTI or SELECT in one run()
    // must NOT leak to the next run() call or cached metadata connections.
    let _ = d
        .run(
            &cfg,
            &query("SELECT 7\nSET parity:db7:only yes\nMULTI\nINCR parity:db7:uncommitted"),
        )
        .await
        .expect("uncommitted MULTI in db7");
    let next_call = d
        .run(&cfg, &query("GET parity:db7:only\nEXEC"))
        .await
        .expect("subsequent run on db0");
    assert_eq!(
        next_call.rows,
        vec![vec![json!("(nil)")]],
        "subsequent run() must start on db0, not leaked db7"
    );
    assert!(
        next_call.more_results[0].errored,
        "EXEC without MULTI on fresh connection must error"
    );

    // 4. L16 Blocking command (BLPOP + XREAD BLOCK) with >30s configured timeout_ms.
    let mut blpop_req = query("RPUSH parity:list item-1\nBLPOP parity:list 1\nXREAD COUNT 1 BLOCK 50 STREAMS parity:empty:stream 0-0");
    blpop_req.timeout_ms = Some(45_000);
    let blpop_res = d.run(&cfg, &blpop_req).await.expect("BLPOP + XREAD BLOCK");
    assert_eq!(blpop_res.more_results.len(), 2);
    assert_eq!(
        blpop_res.more_results[0].rows,
        vec![vec![json!("parity:list")], vec![json!("item-1")]]
    );
    assert_eq!(
        blpop_res.more_results[1].rows,
        vec![vec![json!("(nil)")]],
        "XREAD BLOCK timeout returns (nil)"
    );

    // 5. L16 Bounded client-socket cleanup on cancellation: cancelling an in-flight
    // blocking `run()` drops the isolated connection and removes its client entry
    // from Redis `CLIENT LIST` within a bounded window (< 1s).
    let _ = d.run(&cfg, &query("DEL parity:cancel:list")).await;
    let cancel_req = query("CLIENT SETNAME otto-cancel-probe\nBLPOP parity:cancel:list 20");
    let timed_out = tokio::time::timeout(
        std::time::Duration::from_millis(150),
        d.run(&cfg, &cancel_req),
    )
    .await;
    assert!(
        timed_out.is_err(),
        "BLPOP on empty list should block until cancelled"
    );
    let mut cleaned = false;
    for _ in 0..20 {
        let cl = d
            .run(&cfg, &query("CLIENT LIST"))
            .await
            .expect("CLIENT LIST");
        let text = cl.rows[0][0].as_str().unwrap_or_default();
        if !text.contains("name=otto-cancel-probe") {
            cleaned = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    assert!(
        cleaned,
        "cancelled isolated run() connection should disappear from CLIENT LIST within 500ms"
    );

    // 6. L16 Genuine >30s blocking proof: `BLPOP parity:block:31s 40` with
    // `timeout_ms: Some(45_000)` survives past the 30s default transport timeout
    // and wakes up when a producer pushes after 31s.
    let _ = d.run(&cfg, &query("DEL parity:block:31s")).await;
    let pusher_cfg = cfg.clone();
    let pusher = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(31)).await;
        let pd = RedisDriver::default();
        pd.run(&pusher_cfg, &query("RPUSH parity:block:31s after-31s"))
            .await
            .expect("delayed RPUSH at 31s");
    });
    let mut long_block = query("BLPOP parity:block:31s 40");
    long_block.timeout_ms = Some(45_000);
    let long_started = std::time::Instant::now();
    let long_res = d
        .run(&cfg, &long_block)
        .await
        .expect("BLPOP >30s with timeout_ms=45000 should succeed at 31s");
    let elapsed = long_started.elapsed();
    pusher.await.expect("pusher task");
    assert!(
        elapsed >= std::time::Duration::from_secs(30),
        "BLPOP should have blocked >30s (elapsed={elapsed:?})"
    );
    assert_eq!(
        long_res.rows,
        vec![vec![json!("parity:block:31s")], vec![json!("after-31s")]]
    );

    // Cleanup
    let _ = d
        .run(
            &cfg,
            &query("DEL parity:tx:k parity:tx:str parity:list parity:cancel:list parity:block:31s\nSELECT 7\nDEL parity:db7:only parity:db7:uncommitted"),
        )
        .await;
}
