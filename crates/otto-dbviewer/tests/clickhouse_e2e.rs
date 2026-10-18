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
        lifecycle: None,
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
        ..Default::default()
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
        .schema_children(&cfg, &db_path, None)
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
        .schema_children(&cfg, &events_path, None)
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
    assert_eq!(
        result.rows.len(),
        1,
        "count() should return exactly one row"
    );
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
        suffix: String::new(),
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

/// In a `WHERE`, ORDER BY / primary-key columns (`event_type`, `ts`) rank above
/// the non-key columns (`path`, `user_id`) — ClickHouse "indexes first".
#[tokio::test]
#[ignore]
async fn clickhouse_completion_where_primary_key_first() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }
    let d = ClickhouseDriver::default();
    let cfg = cfg();
    let ctx = CompletionContext {
        prefix: "SELECT * FROM events WHERE ".into(),
        suffix: String::new(),
        database: Some("analytics".into()),
        node: None,
    };
    let comp = d.completion(&cfg, &ctx).await.expect("completion");
    let score = |label: &str| {
        comp.items
            .iter()
            .find(|i| i.kind == CompletionKind::Column && i.label == label)
            .unwrap_or_else(|| panic!("missing column {label}: {:?}", comp.items))
            .score
            .unwrap_or(0)
    };
    assert!(
        score("event_type") > score("path"),
        "primary-key column should rank above a plain column"
    );
    assert!(score("ts") > score("user_id"), "ts is in the ORDER BY key");
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
        lifecycle: None,
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
    let test = d
        .test(&native_cfg)
        .await
        .expect("native test() should not error");
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
        .run(
            &native_cfg,
            &query("SELECT count() AS c FROM analytics.events"),
        )
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
    let roots = d
        .schema_root(&native_cfg)
        .await
        .expect("native schema_root");
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

/// Export truncation fix (spec §2.1 / R1): a full export (`max_rows: None`) must
/// stream ALL rows, not silently cap at the interactive default (1000). Uses
/// `numbers(5000)` so it's self-contained (no seed dependency) and asserts the
/// exported file carries all 5000 data rows.
#[tokio::test]
#[ignore]
async fn clickhouse_export_is_uncapped_over_1000_rows() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }
    use otto_dbviewer::export::ExportFormat;

    let d = ClickhouseDriver::default();
    let dir = tempfile::tempdir().expect("tempdir");
    let dest = dir.path().join("export.csv");

    let counts = d
        .export_to_path(
            &cfg(),
            "SELECT number FROM numbers(5000)",
            None,
            ExportFormat::CsvWithNames,
            None, // uncapped — the whole point
            &dest,
        )
        .await
        .expect("export_to_path should stream the full result");

    // CH's HTTP FORMAT path reports rows as 0 (bytes-only), so verify by counting
    // the written data lines instead of trusting counts.rows.
    let text = std::fs::read_to_string(&dest).expect("read export file");
    let total_lines = text.lines().count();
    let data_rows = total_lines.saturating_sub(1); // minus the header row
    assert_eq!(
        data_rows, 5000,
        "expected 5000 data rows, got {data_rows} (bytes={})",
        counts.bytes
    );
}

/// Isolated parity server by default; CI points at its seeded compose server.
fn parity_cfg(native: bool) -> ResolvedConfig {
    let mut c = cfg();
    let (port_var, default_port) = if native {
        ("OTTO_CLICKHOUSE_PARITY_NATIVE_PORT", 29000)
    } else {
        ("OTTO_CLICKHOUSE_PARITY_HTTP_PORT", 28123)
    };
    c.port = std::env::var(port_var)
        .map(|port| {
            port.parse::<u16>()
                .expect("ClickHouse parity port must be a u16")
        })
        .unwrap_or(default_port);
    c.database = Some("default".into());
    c.params = json!({"transport": if native { "native" } else { "http" }});
    c
}

/// A server-owned readonly=1 profile must remain usable without weakening it
/// to Otto's readonly=2 execution mode or requiring permission to change settings.
#[tokio::test]
#[ignore]
async fn clickhouse_parity_native_strict_readonly_profile() {
    if std::env::var("OTTO_DBV_PARITY").is_err() {
        return;
    }
    let user = format!(
        "otto_readonly_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let admin = ClickhouseDriver::default();
    let admin_cfg = parity_cfg(false);
    admin
        .run(
            &admin_cfg,
            &query(&format!(
        "CREATE USER {user} IDENTIFIED WITH no_password SETTINGS readonly=1, session_timezone='UTC'"
    )),
        )
        .await
        .unwrap();
    admin
        .run(
            &admin_cfg,
            &query(&format!("GRANT SELECT ON *.* TO {user}")),
        )
        .await
        .unwrap();
    let outcome = async {
        let driver = ClickhouseDriver::default();
        for enforced in [false, true] {
            let mut cfg = parity_cfg(true);
            cfg.user = Some(user.clone());
            cfg.password = None;
            cfg.params["__read_only_execution"] = json!(enforced);
            let result = driver
                .run(
                    &cfg,
                    &query("SELECT toUInt8(getSetting('readonly')) AS mode, timezone() AS tz"),
                )
                .await?;
            assert_eq!(result.rows[0], vec![json!(1), json!("UTC")]);
            assert!(driver.run(&cfg, &query("SET readonly=0")).await.is_err());
            cfg.params["timezone"] = json!("Europe/Paris");
            let error = driver.run(&cfg, &query("SELECT 1")).await.unwrap_err();
            assert!(error.to_string().contains("timezone"), "{error}");
        }
        Ok::<(), otto_core::Error>(())
    }
    .await;
    admin
        .run(&admin_cfg, &query(&format!("DROP USER {user}")))
        .await
        .unwrap();
    outcome.unwrap();
}

#[tokio::test]
#[ignore]
async fn clickhouse_parity_formats_types_and_batch_state() {
    if std::env::var("OTTO_DBV_PARITY").is_err() {
        return;
    }
    for native in [false, true] {
        let d = ClickhouseDriver::default();
        let c = parity_cfg(native);
        let result = d.run(&c, &query("SELECT toUInt64(18446744073709551615), CAST('yes' AS Enum8('yes'=1,'no'=2)), [CAST('yes' AS Enum8('yes'=1,'no'=2))] FORMAT CSV SETTINGS max_threads=1")).await.unwrap();
        assert_eq!(
            result.rows[0][0],
            json!("18446744073709551615"),
            "native={native}"
        );
        assert_eq!(result.rows[0][1], json!("yes"), "native={native}");
        assert_eq!(result.rows[0][2], json!(["yes"]), "native={native}");
        let result = d.run(&c, &query("SET max_threads=3; SELECT toUInt64(getSetting('max_threads')); USE system; SELECT currentDatabase()")).await.unwrap();
        assert!(!result.errored, "native={native}: {result:?}");
        assert_eq!(
            result.more_results[0].rows[0][0],
            json!("3"),
            "native={native}: {result:?}"
        );
        assert_eq!(
            result.more_results[2].rows[0][0],
            json!("system"),
            "native={native}: {result:?}"
        );
        let result = d
            .run(
                &c,
                &QueryRequest {
                    node: Some("db:system".into()),
                    ..query("SELECT currentDatabase()")
                },
            )
            .await
            .unwrap();
        assert_eq!(result.rows[0][0], json!("system"));
        let result = d.run(&c, &query("SELECT currentDatabase()")).await.unwrap();
        assert_eq!(result.rows[0][0], json!("default"));
        d.run(&c, &query("USE system")).await.unwrap();
        let result = d.run(&c, &query("SELECT currentDatabase()")).await.unwrap();
        assert_eq!(
            result.rows[0][0],
            json!("default"),
            "single USE must not mutate another operation's session"
        );
        let mut c = c.clone();
        c.params["timezone"] = json!("Europe/Amsterdam");
        let result = d.run(&c, &query("SELECT timezone()")).await.unwrap();
        assert_eq!(result.rows[0][0], json!("Europe/Amsterdam"));
    }
}

#[tokio::test]
#[ignore]
async fn clickhouse_parity_native_cancel_and_timeout() {
    if std::env::var("OTTO_DBV_PARITY").is_err() {
        return;
    }
    use otto_dbviewer::types::CancelToken;
    use std::sync::Arc;
    use std::time::Duration;
    let d = Arc::new(ClickhouseDriver::default());
    let c = parity_cfg(true);
    let token = CancelToken::default();
    let job = {
        let (d, c, token) = (Arc::clone(&d), c.clone(), token.clone());
        tokio::spawn(async move {
            d.run_tracked(
                &c,
                &query("SELECT /* parity_cancel_deadline */ sleepEachRow(0.1) FROM numbers(1000) SETTINGS max_block_size=10"),
                &token,
            )
            .await
        })
    };
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        !job.is_finished(),
        "query must still be running before cancellation"
    );
    let handle = token.handle().expect("native query handle");
    d.cancel(&c, &handle).await.unwrap();
    let result = tokio::time::timeout(Duration::from_secs(5), job)
        .await
        .expect("cancel must finish query")
        .unwrap();
    assert!(result.is_err());
    let started = std::time::Instant::now();
    let result = d
        .run(
            &c,
            &QueryRequest {
                timeout_ms: Some(1000),
                ..query("SELECT /* parity_cancel_deadline */ sleepEachRow(0.1) FROM numbers(1000) SETTINGS max_block_size=10")
            },
        )
        .await;
    assert!(result
        .as_ref()
        .unwrap_err()
        .to_string()
        .contains("timed out"));
    assert!(started.elapsed() >= Duration::from_millis(900));
    // KILL ASYNC acknowledges cancellation before a running block observes it.
    // Bound cleanup latency rather than assuming the acknowledgement is a join.
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let result = d.run(&c, &query("SELECT count() FROM system.processes WHERE startsWith(query, '/* otto-query:') AND position(query, 'parity_cancel_deadline') > 0")).await.unwrap();
            if result.rows[0][0] == json!("1") { break; } // verification query itself
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }).await.expect("cancelled query must leave system.processes promptly");
}

#[tokio::test]
#[ignore]
async fn clickhouse_parity_ddl_dml_and_readonly_isolation() {
    if std::env::var("OTTO_DBV_PARITY").is_err() {
        return;
    }
    for native in [false, true] {
        let d = ClickhouseDriver::default();
        let c = parity_cfg(native);
        let name = format!(
            "otto_parity_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let run = |statement: String| QueryRequest {
            statement,
            timeout_ms: Some(3000),
            ..Default::default()
        };
        d.run(
            &c,
            &run(format!("CREATE TABLE {name} (n UInt64) ENGINE=Memory")),
        )
        .await
        .unwrap();
        let result = d
            .run(&c, &run(format!("INSERT INTO {name} VALUES (1),(2)")))
            .await;
        assert!(result.is_ok(), "native={native}: {result:?}");
        let result = d
            .run(&c, &run(format!("EXISTS TABLE {name}")))
            .await
            .unwrap();
        assert_eq!(result.rows[0][0], json!(1));
        let result = d
            .run(&c, &run(format!("SELECT count() FROM {name}")))
            .await
            .unwrap();
        assert_eq!(result.rows[0][0], json!("2"));
        let mut readonly = c.clone();
        readonly.params["__read_only_execution"] = json!(true);
        d.run(&readonly, &run(format!("SELECT count() FROM {name}")))
            .await
            .unwrap();
        assert!(d
            .run(&readonly, &run(format!("DROP TABLE {name}")))
            .await
            .is_err());
        d.run(&c, &run(format!("DROP TABLE {name}"))).await.unwrap();
    }
}

#[tokio::test]
#[ignore]
async fn clickhouse_parity_native_export_streams_all_rows() {
    if std::env::var("OTTO_DBV_PARITY").is_err() {
        return;
    }
    use otto_dbviewer::export::ExportFormat;
    let d = ClickhouseDriver::default();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rows.csv");
    let counts = d
        .export_to_path(
            &parity_cfg(true),
            "SELECT number FROM numbers(5000)",
            Some("system"),
            ExportFormat::CsvWithNames,
            None,
            &path,
        )
        .await
        .unwrap();
    assert_eq!(counts.rows, 5000);
    assert_eq!(std::fs::read_to_string(path).unwrap().lines().count(), 5001);
}

#[tokio::test]
#[ignore]
async fn clickhouse_parity_row_caps_and_empty_metadata() {
    if std::env::var("OTTO_DBV_PARITY").is_err() {
        return;
    }
    for native in [false, true] {
        let d = ClickhouseDriver::default();
        let c = parity_cfg(native);
        let result = d
            .run(
                &c,
                &QueryRequest {
                    max_rows: Some(7),
                    ..query("SELECT number FROM numbers(1000) SETTINGS max_block_size=10")
                },
            )
            .await
            .unwrap();
        assert_eq!(result.rows.len(), 7);
        assert!(result.truncated);
        let result = d
            .run(&c, &query("SELECT toUInt64(1) AS id WHERE false"))
            .await
            .unwrap();
        assert!(result.rows.is_empty());
        assert_eq!(result.columns[0].name, "id");
    }
}

#[tokio::test]
#[ignore]
async fn clickhouse_parity_maps_json_and_large_json_export() {
    if std::env::var("OTTO_DBV_PARITY").is_err() {
        return;
    }
    use otto_dbviewer::export::ExportFormat;
    for native in [false, true] {
        let d = ClickhouseDriver::default();
        let c = parity_cfg(native);
        let result = d
            .run(&c, &query("SELECT map('x',1,'x',2) AS m"))
            .await
            .unwrap();
        assert_eq!(
            result.rows[0][0],
            json!([["x", 1], ["x", 2]]),
            "native={native}"
        );
        let sql = if native {
            "SELECT toJSONString(CAST('{\"a\":1}' AS JSON)) AS j"
        } else {
            "SELECT CAST('{\"a\":1}' AS JSON) AS j"
        };
        let result = d.run(&c, &query(sql)).await.unwrap();
        // The explicit native toJSONString workaround retains the value;
        // HTTP directly supports JSON with precision-safe quoted integers.
        let actual = match &result.rows[0][0] {
            serde_json::Value::String(s) => serde_json::from_str(s).unwrap(),
            value => value.clone(),
        };
        assert_eq!(
            actual,
            if native {
                json!({"a": 1})
            } else {
                json!({"a": "1"})
            },
            "native={native}"
        );
    }
    let d = ClickhouseDriver::default();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("large.json");
    d.export_to_path(
        &parity_cfg(false),
        "SELECT repeat('x', 1024) AS text FROM numbers(135000)",
        None,
        ExportFormat::Json,
        None,
        &path,
    )
    .await
    .unwrap();
    assert!(std::fs::metadata(&path).unwrap().len() > 128 * 1024 * 1024);
    // Stream validation avoids allocating the whole large result in the test.
    let reader = std::io::BufReader::new(std::fs::File::open(path).unwrap());
    serde_json::from_reader::<_, serde::de::IgnoredAny>(reader).unwrap();
}

#[tokio::test]
#[ignore]
async fn clickhouse_parity_export_caps_existing_limits_and_settings() {
    if std::env::var("OTTO_DBV_PARITY").is_err() {
        return;
    }
    use otto_dbviewer::export::ExportFormat;
    for native in [false, true] {
        for cap in [0, 7] {
            let d = ClickhouseDriver::default();
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("rows.json");
            let counts = d.export_to_path(&parity_cfg(native),
                "SELECT number, 'line\\nnext' AS text FROM numbers(1000) LIMIT 500 SETTINGS max_block_size=10",
                None, ExportFormat::Json, Some(cap), &path).await.unwrap();
            let rows: Vec<serde_json::Value> =
                serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
            assert_eq!(rows.len(), cap, "native={native}");
            assert_eq!(counts.rows as usize, cap);
            if cap > 0 {
                assert_eq!(rows[0]["text"], "line\nnext");
            }
        }
    }
}

#[tokio::test]
#[ignore]
async fn clickhouse_parity_unsupported_native_type_guidance() {
    if std::env::var("OTTO_DBV_PARITY").is_err() {
        return;
    }
    let d = ClickhouseDriver::default();
    let c = parity_cfg(true);
    let error = d
        .run(&c, &query("SELECT CAST(1 AS Dynamic) AS value"))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("HTTP transport"), "{error}");
    assert!(error.contains("toJSONString(column)"), "{error}");
    let result = d
        .run(
            &c,
            &query("SELECT toJSONString(CAST(1 AS Dynamic)) AS value"),
        )
        .await
        .unwrap();
    assert_eq!(result.rows[0][0], "1");
}

#[tokio::test]
#[ignore]
async fn clickhouse_parity_backup_status_and_nullable_maps() {
    if std::env::var("OTTO_DBV_PARITY").is_err() {
        return;
    }
    for native in [false, true] {
        let d = ClickhouseDriver::default();
        let c = parity_cfg(native);
        let result = d
            .run(&c, &query("BACKUP TABLE system.one TO Null"))
            .await
            .unwrap();
        assert_eq!(result.columns[0].name, "id");
        assert_eq!(result.columns[1].name, "status");
        assert_eq!(result.rows[0][1], "BACKUP_CREATED");
        let result = d
            .run(
                &c,
                &query("SELECT map('n', CAST(NULL AS Nullable(UInt8))) AS m"),
            )
            .await
            .unwrap();
        assert_eq!(result.rows[0][0], json!([["n", null]]));
        let result = d
            .run(&c, &query("SELECT CAST(map() AS Map(String, UInt8)) AS m"))
            .await
            .unwrap();
        assert_eq!(result.rows[0][0], json!([]));
    }
}

#[tokio::test]
#[ignore]
async fn clickhouse_parity_native_operations_do_not_queue_behind_other_tabs() {
    if std::env::var("OTTO_DBV_PARITY").is_err() {
        return;
    }
    use otto_dbviewer::types::CancelToken;
    use std::{sync::Arc, time::Duration};
    let d = Arc::new(ClickhouseDriver::default());
    let c = parity_cfg(true);
    let token = CancelToken::default();
    let slow = {
        let (d, c, token) = (Arc::clone(&d), c.clone(), token.clone());
        tokio::spawn(async move {
            d.run_tracked(
                &c,
                &query("SELECT sleepEachRow(0.1) FROM numbers(1000) SETTINGS max_block_size=10"),
                &token,
            )
            .await
        })
    };
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!slow.is_finished());
    let fast =
        tokio::time::timeout(Duration::from_secs(2), d.run(&c, &query("SELECT 7 AS n"))).await;
    d.cancel(&c, &token.handle().unwrap()).await.unwrap();
    assert!(tokio::time::timeout(Duration::from_secs(5), slow)
        .await
        .unwrap()
        .unwrap()
        .is_err());
    assert_eq!(
        fast.expect("another tab must not queue behind the slow operation")
            .unwrap()
            .rows[0][0],
        7
    );
}

#[tokio::test]
#[ignore]
async fn clickhouse_parity_export_failure_and_bad_credentials() {
    if std::env::var("OTTO_DBV_PARITY").is_err() {
        return;
    }
    use otto_dbviewer::export::ExportFormat;
    for native in [false, true] {
        let d = ClickhouseDriver::default();
        let c = parity_cfg(native);
        let dir = tempfile::tempdir().unwrap();
        for cap in [None, Some(30000)] {
            let path = dir.path().join(format!("failed-{cap:?}.csv"));
            let error = d.export_to_path(&c,
                "SELECT number, throwIf(number > 10000) AS failed FROM numbers(20000) SETTINGS max_block_size=1000",
                None, ExportFormat::Csv, cap, &path).await;
            assert!(
                error.is_err(),
                "native={native} cap={cap:?}: failed export must not report success"
            );
        }
        let mut bad = c.clone();
        bad.password = Some("deliberately-incorrect-parity-secret".into());
        let error = d
            .run(&bad, &query("SELECT 1"))
            .await
            .unwrap_err()
            .to_string();
        assert!(!error.contains("deliberately-incorrect-parity-secret"));
        assert!(d.run(&c, &query("SELECT 1")).await.is_ok());
    }
}
