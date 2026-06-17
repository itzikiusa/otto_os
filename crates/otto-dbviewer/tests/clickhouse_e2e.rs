//! End-to-end tests for the ClickHouse driver against a live server.
//!
//! Each test is `#[ignore]` by default and additionally guarded by the
//! `OTTO_DBV_E2E` env var so they never run in a normal `cargo test`. Run with:
//!
//! ```sh
//! OTTO_DBV_E2E=1 cargo test -p otto-dbviewer --test clickhouse_e2e -- --ignored --nocapture
//! ```
//!
//! Expects a ClickHouse 127.0.0.1:18123 (HTTP) seeded with database `analytics`
//! holding `events` (event_id, event_type, user_id, path, ts, revenue_cents; 5
//! rows), `daily_sales`, and a view `revenue_by_type`. Auth: otto/ottopw.

use otto_dbviewer::driver::Driver;
use otto_dbviewer::drivers::clickhouse::ClickhouseDriver;
use otto_dbviewer::types::{
    CompletionContext, CompletionKind, Engine, NodePath, QueryRequest, ResolvedConfig, TlsConfig,
};
use serde_json::json;

fn cfg() -> ResolvedConfig {
    ResolvedConfig {
        engine: Engine::Clickhouse,
        host: "127.0.0.1".into(),
        port: 18123,
        user: Some("otto".into()),
        password: Some("ottopw".into()),
        database: Some("analytics".into()),
        tls: TlsConfig::default(),
        params: json!({}),
    }
}

fn query(stmt: &str) -> QueryRequest {
    QueryRequest {
        statement: stmt.into(),
        max_rows: None,
        params: None,
        node: None,
    }
}

/// `test()` connects and reports the server version.
#[tokio::test]
#[ignore]
async fn clickhouse_connect() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }

    let d = ClickhouseDriver::default();
    let test = d.test(&cfg()).await.expect("test() should not error");
    assert!(test.ok, "expected ok connection, got: {}", test.message);
    assert!(
        test.server_version.is_some(),
        "expected a server_version, got none"
    );
    eprintln!(
        "connected: version={:?} latency_ms={:?}",
        test.server_version, test.latency_ms
    );
}

/// schema_root → analytics, expand to the events table, expand that to columns
/// including event_type.
#[tokio::test]
#[ignore]
async fn clickhouse_schema_tree() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }

    let d = ClickhouseDriver::default();
    let cfg = cfg();

    // schema_root lists the analytics database.
    let roots = d.schema_root(&cfg).await.expect("schema_root");
    assert!(
        roots.iter().any(|n| n.label == "analytics"),
        "schema_root should contain 'analytics', got: {:?}",
        roots.iter().map(|n| &n.label).collect::<Vec<_>>()
    );

    // expand db:analytics → lists the events table.
    let db_path = NodePath::parse("db:analytics");
    let tables = d
        .schema_children(&cfg, &db_path)
        .await
        .expect("schema_children of db:analytics");
    assert!(
        tables.iter().any(|n| n.label == "events"),
        "db:analytics should contain table 'events', got: {:?}",
        tables.iter().map(|n| &n.label).collect::<Vec<_>>()
    );

    // expand db:analytics/table:events → columns include event_type.
    let events_path = NodePath::parse("db:analytics/table:events");
    let columns = d
        .schema_children(&cfg, &events_path)
        .await
        .expect("schema_children of table:events");
    assert!(
        columns.iter().any(|n| n.label == "event_type"),
        "events columns should include 'event_type', got: {:?}",
        columns.iter().map(|n| &n.label).collect::<Vec<_>>()
    );
}

/// object_detail of events: event_type column and an engine in `extra`.
#[tokio::test]
#[ignore]
async fn clickhouse_object_detail() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }

    let d = ClickhouseDriver::default();
    let cfg = cfg();

    let events_path = NodePath::parse("db:analytics/table:events");
    let detail = d
        .object_detail(&cfg, &events_path)
        .await
        .expect("object_detail of events");
    assert!(
        detail.columns.iter().any(|c| c.name == "event_type"),
        "events should have an 'event_type' column, got: {:?}",
        detail.columns.iter().map(|c| &c.name).collect::<Vec<_>>()
    );
    assert!(
        detail.extra.get("engine").is_some(),
        "events object_detail extra should carry the engine, got: {}",
        detail.extra
    );
    eprintln!(
        "events: {} columns, row_count={:?}, engine extra={}",
        detail.columns.len(),
        detail.row_count,
        detail.extra
    );
}

/// run() count rows in events; expect 5.
#[tokio::test]
#[ignore]
async fn clickhouse_run() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }

    let d = ClickhouseDriver::default();
    let cfg = cfg();

    let result = d
        .run(&cfg, &query("SELECT count() AS c FROM analytics.events"))
        .await
        .expect("run count()");
    assert_eq!(result.rows.len(), 1, "count() should return exactly one row");
    let cell = &result.rows[0][0];
    let count_ok = cell.as_i64() == Some(5)
        || cell.as_u64() == Some(5)
        || cell.as_str() == Some("5")
        || cell.as_f64() == Some(5.0);
    assert!(count_ok, "expected count == 5, got cell: {cell:?}");
}

/// completion offers the SELECT keyword, the count function, and the events
/// table.
#[tokio::test]
#[ignore]
async fn clickhouse_completion() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }

    let d = ClickhouseDriver::default();
    let cfg = cfg();

    let ctx = CompletionContext {
        prefix: String::new(),
        database: Some("analytics".into()),
        node: None,
    };
    let comp = d.completion(&cfg, &ctx).await.expect("completion");
    assert!(
        comp.items.iter().any(|i| {
            i.kind == CompletionKind::Keyword && i.label.eq_ignore_ascii_case("SELECT")
        }),
        "completion should include keyword SELECT"
    );
    assert!(
        comp.items
            .iter()
            .any(|i| i.kind == CompletionKind::Function && i.label == "count"),
        "completion should include function count"
    );
    assert!(
        comp.items
            .iter()
            .any(|i| i.kind == CompletionKind::Table && i.label == "events"),
        "completion should include table events"
    );
}

/// The session time zone is applied: an `Europe/Amsterdam` profile makes
/// `timezone()` return `Europe/Amsterdam`, and the default returns `UTC`.
#[tokio::test]
#[ignore]
async fn clickhouse_timezone() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }

    let d = ClickhouseDriver::default();

    // Explicit Europe/Amsterdam → session_timezone=Europe/Amsterdam.
    let mut tz_cfg = cfg();
    tz_cfg.params = json!({ "timezone": "Europe/Amsterdam" });
    let res = d
        .run(&tz_cfg, &query("SELECT timezone()"))
        .await
        .expect("run(SELECT timezone()) with Europe/Amsterdam");
    assert_eq!(res.rows.len(), 1, "should be a single row");
    assert_eq!(
        res.rows[0][0].as_str(),
        Some("Europe/Amsterdam"),
        "timezone() should be Europe/Amsterdam; got: {:?}",
        res.rows[0][0]
    );

    // No timezone param → defaults to UTC.
    let res = d
        .run(&cfg(), &query("SELECT timezone()"))
        .await
        .expect("run(SELECT timezone()) default");
    assert_eq!(res.rows.len(), 1, "should be a single row");
    assert_eq!(
        res.rows[0][0].as_str(),
        Some("UTC"),
        "default timezone() should be UTC; got: {:?}",
        res.rows[0][0]
    );
}

/// The NATIVE TCP transport (port 19000 → docker's 9000) drives the full driver
/// surface: `test()` reports a version, `run` counts the seeded rows,
/// `schema_root` lists the database, and `object_detail` carries the columns.
/// Same docker server as the HTTP tests, reached over the native protocol.
#[tokio::test]
#[ignore]
async fn clickhouse_native_e2e() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }

    // Native protocol, plaintext: docker forwards 9000 → 127.0.0.1:19000.
    let native_cfg = ResolvedConfig {
        engine: Engine::Clickhouse,
        host: "127.0.0.1".into(),
        port: 19000,
        user: Some("otto".into()),
        password: Some("ottopw".into()),
        database: Some("analytics".into()),
        tls: TlsConfig::default(),
        params: json!({}),
    };

    let d = ClickhouseDriver::default();

    // test() connects over native and reports the server version.
    let test = d.test(&native_cfg).await.expect("native test() should not error");
    assert!(test.ok, "native test() expected ok, got: {}", test.message);
    assert!(
        test.server_version.is_some(),
        "native test() expected a server_version, got none"
    );
    eprintln!(
        "native connected: version={:?} latency_ms={:?}",
        test.server_version, test.latency_ms
    );

    // run() count rows in events; expect 5, over native.
    let result = d
        .run(&native_cfg, &query("SELECT count() AS c FROM analytics.events"))
        .await
        .expect("native run count()");
    assert_eq!(
        result.rows.len(),
        1,
        "native count() should return exactly one row"
    );
    let cell = &result.rows[0][0];
    let count_ok = cell.as_i64() == Some(5)
        || cell.as_u64() == Some(5)
        || cell.as_str() == Some("5")
        || cell.as_f64() == Some(5.0);
    assert!(count_ok, "native expected count == 5, got cell: {cell:?}");
    // The column carries its CH type hint from the decoded native block.
    assert_eq!(result.columns.len(), 1, "one column");
    assert_eq!(result.columns[0].name, "c", "column should be named c");

    // schema_root lists the analytics database over native.
    let roots = d.schema_root(&native_cfg).await.expect("native schema_root");
    assert!(
        roots.iter().any(|n| n.label == "analytics"),
        "native schema_root should contain 'analytics', got: {:?}",
        roots.iter().map(|n| &n.label).collect::<Vec<_>>()
    );

    // object_detail of events over native carries the event_type column.
    let events_path = NodePath::parse("db:analytics/table:events");
    let detail = d
        .object_detail(&native_cfg, &events_path)
        .await
        .expect("native object_detail of events");
    assert!(
        detail.columns.iter().any(|c| c.name == "event_type"),
        "native events should have an 'event_type' column, got: {:?}",
        detail.columns.iter().map(|c| &c.name).collect::<Vec<_>>()
    );
    assert!(
        detail.extra.get("engine").is_some(),
        "native events object_detail extra should carry the engine, got: {}",
        detail.extra
    );
    eprintln!(
        "native events: {} columns, row_count={:?}, engine extra={}",
        detail.columns.len(),
        detail.row_count,
        detail.extra
    );
}
