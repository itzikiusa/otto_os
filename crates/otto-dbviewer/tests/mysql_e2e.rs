//! End-to-end tests for the MySQL driver against a live server.
//!
//! Requires a MySQL seeded as described in the seed reference (docker MySQL on
//! 127.0.0.1:13306, user otto / ottopw, database `shopdb`). Each test is
//! `#[ignore]` by default and additionally guarded by `OTTO_DBV_E2E`. Run with:
//!   OTTO_DBV_E2E=1 cargo test -p otto-dbviewer --test mysql_e2e -- --ignored --nocapture

use otto_dbviewer::driver::Driver;
use otto_dbviewer::drivers::mysql::MysqlDriver;
use otto_dbviewer::types::{
    CancelToken, CompletionContext, CompletionKind, Engine, NodePath, QueryHandle, QueryRequest,
    ResolvedConfig, TlsConfig,
};
use serde_json::json;

fn cfg() -> ResolvedConfig {
    ResolvedConfig {
        lifecycle: None,
        engine: Engine::Mysql,
        host: "127.0.0.1".into(),
        port: 13306,
        user: Some("otto".into()),
        password: Some("ottopw".into()),
        database: Some("shopdb".into()),
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
async fn mysql_connect() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }

    let d = MysqlDriver::default();
    let test = d.test(&cfg()).await.expect("test() should not error");
    assert!(
        test.ok,
        "test().ok should be true; message: {}",
        test.message
    );
    assert!(
        test.server_version.is_some(),
        "server_version should be Some"
    );
    eprintln!("server_version = {:?}", test.server_version);
}

/// schema_root → shopdb, expand to the Tables folder → orders, expand orders
/// → columns include customer_id.
#[tokio::test]
#[ignore]
async fn mysql_schema_tree() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }

    let d = MysqlDriver::default();
    let cfg = cfg();

    // schema_root contains shopdb.
    let roots = d.schema_root(&cfg).await.expect("schema_root");
    assert!(
        roots.iter().any(|n| n.label == "shopdb"),
        "schema_root should contain 'shopdb'; got: {:?}",
        roots.iter().map(|n| &n.label).collect::<Vec<_>>()
    );

    // expand shopdb -> folder:tables.
    let shopdb = NodePath::parse("db:shopdb");
    let folders = d
        .schema_children(&cfg, &shopdb, None)
        .await
        .expect("schema_children(db)");
    let tables_folder = folders
        .iter()
        .find(|n| n.id.ends_with("folder:tables"))
        .expect("a 'Tables' folder");

    // expand folder:tables -> contains table 'orders'.
    let tables_path = NodePath::parse(&tables_folder.id);
    let tables = d
        .schema_children(&cfg, &tables_path, None)
        .await
        .expect("schema_children(folder:tables)");
    assert!(
        tables.iter().any(|n| n.label == "orders"),
        "tables folder should contain 'orders'; got: {:?}",
        tables.iter().map(|n| &n.label).collect::<Vec<_>>()
    );

    // expand db:shopdb/table:orders -> columns include customer_id.
    let orders_path = NodePath::parse("db:shopdb/table:orders");
    let columns = d
        .schema_children(&cfg, &orders_path, None)
        .await
        .expect("schema_children(table:orders)");
    assert!(
        columns.iter().any(|n| n.label == "customer_id"),
        "orders columns should include 'customer_id'; got: {:?}",
        columns.iter().map(|n| &n.label).collect::<Vec<_>>()
    );
}

/// object_detail of orders: customer_id column, primary key id, FK → customers.
#[tokio::test]
#[ignore]
async fn mysql_object_detail() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }

    let d = MysqlDriver::default();
    let cfg = cfg();

    let orders_path = NodePath::parse("db:shopdb/table:orders");
    let detail = d
        .object_detail(&cfg, &orders_path)
        .await
        .expect("object_detail(orders)");
    assert!(
        detail.columns.iter().any(|c| c.name == "customer_id"),
        "orders should have a 'customer_id' column; got: {:?}",
        detail.columns.iter().map(|c| &c.name).collect::<Vec<_>>()
    );
    assert!(
        detail.primary_key.iter().any(|c| c == "id"),
        "orders primary_key should contain 'id'; got: {:?}",
        detail.primary_key
    );
    assert!(
        detail
            .foreign_keys
            .iter()
            .any(|fk| fk.ref_table == "customers"),
        "orders should have a foreign_key referencing 'customers'; got: {:?}",
        detail
            .foreign_keys
            .iter()
            .map(|fk| (&fk.name, &fk.ref_table))
            .collect::<Vec<_>>()
    );
}

/// run a SELECT against customers; first row email is ada@example.com.
#[tokio::test]
#[ignore]
async fn mysql_run_select() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }

    let d = MysqlDriver::default();
    let cfg = cfg();

    let res = d
        .run(&cfg, &query("SELECT id, email FROM customers ORDER BY id"))
        .await
        .expect("run(SELECT)");
    assert!(
        res.rows.len() >= 4,
        "expected >= 4 rows; got {}",
        res.rows.len()
    );
    let email_idx = res
        .columns
        .iter()
        .position(|c| c.name == "email")
        .expect("email column index");
    assert_eq!(
        res.rows[0][email_idx].as_str(),
        Some("ada@example.com"),
        "first row email should be ada@example.com; got: {:?}",
        res.rows[0][email_idx]
    );
}

/// A true multi-statement batch (`SELECT 1; SELECT 2`) returns TWO results: the
/// first on top, the second in `more_results`, each carrying its statement
/// preview. Proves §2.2 end-to-end against the live server.
#[tokio::test]
#[ignore]
async fn mysql_run_multi_statement_batch() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }

    let d = MysqlDriver::default();
    let res = d
        .run(&cfg(), &query("SELECT 1 AS a; SELECT 2 AS b"))
        .await
        .expect("run(batch)");
    // First statement on top.
    assert_eq!(res.rows[0][0].as_i64(), Some(1), "first result = SELECT 1");
    assert_eq!(res.statement.as_deref(), Some("SELECT 1 AS a"));
    // Second statement in more_results.
    assert_eq!(res.more_results.len(), 1, "one trailing result");
    assert_eq!(res.more_results[0].rows[0][0].as_i64(), Some(2));
    assert_eq!(
        res.more_results[0].statement.as_deref(),
        Some("SELECT 2 AS b")
    );
    // A single statement doesn't gain the batch fields.
    let one = d
        .run(&cfg(), &query("SELECT 1 AS a"))
        .await
        .expect("run(single)");
    assert!(one.more_results.is_empty() && one.statement.is_none());
}

/// A batch that fails mid-way returns the completed results plus a terminal
/// `errored` entry (a 200 with partial results, not an error).
#[tokio::test]
#[ignore]
async fn mysql_batch_partial_on_error() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }

    let d = MysqlDriver::default();
    let res = d
        .run(
            &cfg(),
            &query("SELECT 1 AS a; SELECT * FROM no_such_table_xyz; SELECT 3"),
        )
        .await
        .expect("batch returns Ok with a partial result, not Err");
    // First statement succeeded (top-level); the failure is the terminal entry.
    assert_eq!(res.rows[0][0].as_i64(), Some(1));
    assert_eq!(
        res.more_results.len(),
        1,
        "stopped at the failing statement"
    );
    let failed = &res.more_results[0];
    assert!(failed.errored, "second entry flagged errored");
    assert!(
        !failed.message.as_deref().unwrap_or("").is_empty(),
        "carries the engine error"
    );
}

/// The governed (access-enforced, read-only transaction) path keeps the same
/// partial-batch contract: completed results + an `errored` terminal entry.
#[tokio::test]
#[ignore]
async fn mysql_governed_batch_partial_on_error() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }

    let mut c = cfg();
    c.params = json!({ "__read_only_execution": true });
    let d = MysqlDriver::default();
    let res = d
        .run(
            &c,
            &query("SELECT 1 AS a; SELECT * FROM no_such_table_xyz; SELECT 3"),
        )
        .await
        .expect("governed batch returns Ok with a partial result, not Err");
    assert_eq!(res.rows[0][0].as_i64(), Some(1));
    assert_eq!(
        res.more_results.len(),
        1,
        "stopped at the failing statement"
    );
    assert!(res.more_results[0].errored, "second entry flagged errored");
    // A single failing statement is still the request's error.
    assert!(d
        .run(&c, &query("SELECT * FROM no_such_table_xyz"))
        .await
        .is_err());
}

/// After `FROM`, completion offers the tables (orders, customers) ranked above
/// keywords.
#[tokio::test]
#[ignore]
async fn mysql_completion() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }

    let d = MysqlDriver::default();
    let cfg = cfg();

    let ctx = CompletionContext {
        database: Some("shopdb".into()),
        prefix: "SELECT * FROM ".into(),
        suffix: String::new(),
        node: None,
    };
    let comp = d.completion(&cfg, &ctx).await.expect("completion");
    let orders = comp
        .items
        .iter()
        .find(|i| i.kind == CompletionKind::Table && i.label == "orders")
        .expect("completion should include a Table 'orders'");
    let kw = comp
        .items
        .iter()
        .find(|i| i.kind == CompletionKind::Keyword)
        .expect("completion should still include keywords");
    assert!(
        orders.score.unwrap_or(0) > kw.score.unwrap_or(0),
        "tables must rank above keywords right after FROM"
    );
}

/// In a `WHERE`, columns of the in-scope table come back index-first:
/// id (PK) > email (UNIQUE) > country (INDEX) > full_name (plain).
#[tokio::test]
#[ignore]
async fn mysql_completion_where_index_first() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }
    let d = MysqlDriver::default();
    let cfg = cfg();
    let ctx = CompletionContext {
        database: Some("shopdb".into()),
        prefix: "SELECT * FROM customers WHERE ".into(),
        suffix: String::new(),
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
    assert!(score("id") > score("email"), "PK before UNIQUE");
    assert!(score("email") > score("country"), "UNIQUE before INDEX");
    assert!(score("country") > score("full_name"), "INDEX before plain");
}

/// A qualified `c.` only offers that alias's table columns, not the joined one.
#[tokio::test]
#[ignore]
async fn mysql_completion_qualified() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }
    let d = MysqlDriver::default();
    let cfg = cfg();
    let ctx = CompletionContext {
        database: Some("shopdb".into()),
        prefix: "SELECT * FROM orders o JOIN customers c ON o.id = c.id WHERE c.".into(),
        suffix: String::new(),
        node: None,
    };
    let comp = d.completion(&cfg, &ctx).await.expect("completion");
    let cols: Vec<&str> = comp
        .items
        .iter()
        .filter(|i| i.kind == CompletionKind::Column)
        .map(|i| i.label.as_str())
        .collect();
    assert!(
        cols.contains(&"email"),
        "customers.email expected: {cols:?}"
    );
    assert!(
        !cols.contains(&"total_cents"),
        "must not leak orders columns through c.: {cols:?}"
    );
}

/// TIMESTAMP columns decode to their `YYYY-MM-DD HH:MM:SS` text, not base64
/// gibberish. sqlx's `NaiveDateTime` is only `compatible()` with DATETIME —
/// TIMESTAMP needs a `DateTime<Utc>` attempt or the cell falls through to the
/// raw-bytes fallback and renders the binary wire payload base64'd
/// (`B+oHBwYQOQA=`-style). Uses a TEMPORARY table inside one batch so the
/// column is a true TIMESTAMP (literals/NOW() come back as DATETIME).
#[tokio::test]
#[ignore]
async fn mysql_timestamp_column_decodes_as_text() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }

    let d = MysqlDriver::default();
    let res = d
        .run(
            &cfg(),
            &query(
                "CREATE TEMPORARY TABLE ts_probe (\
                     ts TIMESTAMP NULL, dt DATETIME NULL, \
                     ts3 TIMESTAMP(3) NULL, dt6 DATETIME(6) NULL); \
                 INSERT INTO ts_probe VALUES \
                     ('2026-07-06 16:57:00', '2026-07-06 16:57:00', \
                      '2026-07-06 16:57:00.123', '2026-07-06 16:57:00.123456'), \
                     (NULL, NULL, NULL, NULL); \
                 SELECT ts, dt, ts3, dt6 FROM ts_probe ORDER BY ts IS NULL",
            ),
        )
        .await
        .expect("run(timestamp batch)");
    let select = res
        .more_results
        .last()
        .expect("SELECT result should be the last batch entry");
    assert_eq!(
        select.rows[0][0].as_str(),
        Some("2026-07-06 16:57:00"),
        "TIMESTAMP cell should decode to its text form; got: {:?}",
        select.rows[0][0]
    );
    assert_eq!(
        select.rows[0][1].as_str(),
        Some("2026-07-06 16:57:00"),
        "DATETIME cell should decode to its text form; got: {:?}",
        select.rows[0][1]
    );
    // Fractional-seconds precision (fsp 1..=6 — MySQL tops out at microseconds)
    // must survive: chrono's Display renders the trimmed `%.f` part.
    assert_eq!(
        select.rows[0][2].as_str(),
        Some("2026-07-06 16:57:00.123"),
        "TIMESTAMP(3) keeps milliseconds; got: {:?}",
        select.rows[0][2]
    );
    assert_eq!(
        select.rows[0][3].as_str(),
        Some("2026-07-06 16:57:00.123456"),
        "DATETIME(6) keeps microseconds; got: {:?}",
        select.rows[0][3]
    );
    assert!(
        select.rows[1].iter().all(|c| c.is_null()),
        "NULL temporals stay null; got: {:?}",
        select.rows[1]
    );
}

/// The session time zone is applied on connect: a `+03:00` profile yields
/// `@@session.time_zone == +03:00`, and the default (no param) yields `+00:00`.
#[tokio::test]
#[ignore]
async fn mysql_timezone() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }

    let d = MysqlDriver::default();

    // Explicit +03:00 → SET time_zone = '+03:00'.
    let mut tz_cfg = cfg();
    tz_cfg.params = json!({ "timezone": "+03:00" });
    let res = d
        .run(&tz_cfg, &query("SELECT @@session.time_zone"))
        .await
        .expect("run(SELECT @@session.time_zone) with +03:00");
    assert_eq!(res.rows.len(), 1, "should be a single row");
    assert_eq!(
        res.rows[0][0].as_str(),
        Some("+03:00"),
        "session time_zone should be +03:00; got: {:?}",
        res.rows[0][0]
    );

    // No timezone param → defaults to UTC (+00:00).
    let res = d
        .run(&cfg(), &query("SELECT @@session.time_zone"))
        .await
        .expect("run(SELECT @@session.time_zone) default");
    assert_eq!(res.rows.len(), 1, "should be a single row");
    assert_eq!(
        res.rows[0][0].as_str(),
        Some("+00:00"),
        "default session time_zone should be +00:00; got: {:?}",
        res.rows[0][0]
    );
}

/// shopdb exposes a `Triggers` folder → `trg_orders_clamp_total`; its object
/// detail carries the event/timing/table (in `extra`) + the SHOW CREATE DDL.
#[tokio::test]
#[ignore]
async fn mysql_triggers_browse() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }
    let d = MysqlDriver::default();
    let cfg = cfg();

    // shopdb → folders include "Triggers".
    let shopdb = NodePath::parse("db:shopdb");
    let folders = d
        .schema_children(&cfg, &shopdb, None)
        .await
        .expect("db folders");
    assert!(
        folders.iter().any(|n| n.label == "Triggers"),
        "shopdb should expose a Triggers folder; got: {:?}",
        folders.iter().map(|n| &n.label).collect::<Vec<_>>()
    );

    // Triggers folder → the seeded trigger.
    let trig_folder = NodePath::parse("db:shopdb/folder:triggers");
    let trigs = d
        .schema_children(&cfg, &trig_folder, None)
        .await
        .expect("triggers");
    let trg = trigs
        .iter()
        .find(|n| n.label == "trg_orders_clamp_total")
        .expect("trg_orders_clamp_total present");

    // object_detail → DDL + extra.{table,event,timing}.
    let detail = d
        .object_detail(&cfg, &NodePath::parse(&trg.id))
        .await
        .expect("trigger detail");
    let ddl = detail.ddl.unwrap_or_default().to_uppercase();
    assert!(
        ddl.contains("TRIGGER"),
        "trigger DDL should mention TRIGGER; got: {ddl}"
    );
    assert_eq!(
        detail.extra.get("table").and_then(|v| v.as_str()),
        Some("orders")
    );
    assert_eq!(
        detail.extra.get("event").and_then(|v| v.as_str()),
        Some("INSERT")
    );
}

/// EXPLAIN FORMAT=JSON on a full-scan SELECT yields a plan whose table node is
/// flagged as a full table scan.
#[tokio::test]
#[ignore]
async fn mysql_query_plan_flags_full_scan() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }
    let d = MysqlDriver::default();
    // No index on `status` → a filtered scan of orders.
    let plan = d
        .query_plan(
            &cfg(),
            "SELECT * FROM orders WHERE total_cents > 0",
            Some("shopdb"),
        )
        .await
        .expect("query_plan");
    assert_eq!(plan.engine, "mysql");
    // The orders access node should be present with an object name.
    let has_table = plan
        .root
        .children
        .iter()
        .any(|c| c.object.as_deref() == Some("orders"));
    assert!(
        has_table,
        "plan should reference the orders table; root: {:?}",
        plan.root
    );
    // A full scan (access_type ALL) is warned.
    let full_scan = plan
        .root
        .children
        .iter()
        .any(|c| c.warnings.iter().any(|w| w.contains("full table scan")));
    assert!(
        full_scan,
        "expected a full-table-scan warning; root: {:?}",
        plan.root
    );
}

/// SH-02: text that merely LOOKS like JSON stays the string it is — only the
/// native JSON type is parsed. (sqlx's JSON type accepts every text column, so
/// the old per-cell cascade returned `'null'` as NULL, `'123'` as a number, and
/// a 30-digit id as a lossy float.)
#[tokio::test]
#[ignore]
async fn mysql_text_that_looks_like_json_stays_text() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }
    let d = MysqlDriver::default();
    let cfg = cfg();
    let stmt = "SELECT v, CAST(v AS JSON) AS j FROM (\
        SELECT CAST('null' AS CHAR) AS v UNION ALL SELECT '123' UNION ALL SELECT 'true' \
        UNION ALL SELECT '[1,2]' UNION ALL SELECT '{\"a\":1}' UNION ALL SELECT '1e5' \
        UNION ALL SELECT '123456789012345678901234567890') t";
    let res = d.run(&cfg, &query(stmt)).await.expect("run");
    let texts: Vec<_> = res.rows.iter().map(|r| r[0].clone()).collect();
    assert_eq!(
        texts,
        vec![
            json!("null"),
            json!("123"),
            json!("true"),
            json!("[1,2]"),
            json!("{\"a\":1}"),
            json!("1e5"),
            json!("123456789012345678901234567890"),
        ]
    );
    // The native JSON column still decodes to JSON values.
    assert_eq!(res.rows[0][1], json!(null));
    assert_eq!(res.rows[1][1], json!(123));
    assert_eq!(res.rows[3][1], json!([1, 2]));
}

/// SH-06: a read the auto-LIMIT can't touch (UNION) stops pulling at the cap
/// instead of draining the whole server result, and the pool stays usable.
#[tokio::test]
#[ignore]
async fn mysql_uncapped_read_stops_at_the_row_cap() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }
    let d = MysqlDriver::default();
    let cfg = cfg();
    // A cross join of the catalog: far more rows than the cap, and the UNION
    // keeps the auto-LIMIT injector away.
    let stmt = "SELECT 1 AS n UNION ALL SELECT a.ORDINAL_POSITION \
        FROM information_schema.columns a CROSS JOIN information_schema.columns b";
    let req = QueryRequest {
        statement: stmt.into(),
        max_rows: Some(100),
        ..Default::default()
    };
    let started = std::time::Instant::now();
    let res = d.run(&cfg, &req).await.expect("run");
    assert_eq!(res.rows.len(), 100);
    assert!(res.truncated);
    eprintln!("capped UNION in {:?}", started.elapsed());
    // The next query on the pool is unaffected by the abandoned result.
    let next = d.run(&cfg, &query("SELECT 7")).await.expect("next run");
    assert_eq!(next.rows[0][0], json!(7));
}

/// DB2-01: an auto-limited read that hits the row cap keeps its pooled
/// session. Back-to-back Runs are racy to compare (sqlx returns a released
/// session on a spawned task after a ping, so an immediate acquire can open a
/// second connection — see `postgres_truncated_read_keeps_its_session`): each
/// read's own connection id comes from its cancel token, with a settle between
/// Runs, and the session must still be connected afterwards.
#[tokio::test]
#[ignore]
async fn mysql_truncated_read_keeps_its_session() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }

    let d = MysqlDriver::default();
    let cfg = cfg();
    let mut ids = std::collections::BTreeSet::new();
    for _ in 0..5 {
        let token = CancelToken::new();
        let res = d
            .run_tracked(
                &cfg,
                &QueryRequest {
                    statement: "SELECT id FROM customers".into(),
                    max_rows: Some(1),
                    ..Default::default()
                },
                &token,
            )
            .await
            .expect("truncated read");
        assert!(res.truncated, "max_rows 1 over a multi-row table truncates");
        assert_eq!(res.auto_limited, Some(1));
        match token.handle() {
            Some(QueryHandle::MysqlConnId(id)) => ids.insert(id),
            other => panic!("the read must record its connection id; got {other:?}"),
        };
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    assert_eq!(
        ids.len(),
        1,
        "a truncated, server-bounded read must not close its pooled session \
         (each Run reused the previous connection); ids seen: {ids:?}"
    );
    let id = *ids.first().unwrap();
    let alive = d
        .run(
            &cfg,
            &query(&format!(
                "SELECT COUNT(*) AS c FROM information_schema.PROCESSLIST WHERE ID = {id}"
            )),
        )
        .await
        .expect("processlist");
    let alive_n = alive.rows[0][0]
        .as_i64()
        .or_else(|| alive.rows[0][0].as_str().and_then(|s| s.parse().ok()));
    assert_eq!(
        alive_n,
        Some(1),
        "connection {id} must still be open; got {:?}",
        alive.rows[0][0]
    );
}

/// The observable counter establishes SERVER-side bounding, not merely a
/// truncated display. The backend id also pins same-session batch execution.
#[tokio::test]
#[ignore = "requires an isolated SQL fixture (OTTO_DBV_E2E)"]
async fn mysql_batch_bounds_select_on_the_same_session() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }
    let d = MysqlDriver::default();
    let result = d.run(&cfg(), &QueryRequest {
        statement: "SET @otto_seen=0; SELECT @otto_seen:=@otto_seen+1 AS n, CONNECTION_ID() AS backend FROM information_schema.columns; SELECT @otto_seen AS seen, CONNECTION_ID() AS backend".into(),
        max_rows: Some(2),
        ..Default::default()
    }).await.unwrap();
    assert_eq!(result.more_results.len(), 2);
    let preview = &result.more_results[0];
    let after = &result.more_results[1];
    assert_eq!(preview.rows.len(), 2);
    assert!(preview.truncated);
    assert_eq!(after.rows[0][0].to_string().trim_matches('"'), "3");
    assert_eq!(preview.rows[0][1], after.rows[0][1]);
}

#[tokio::test]
#[ignore = "requires an isolated SQL fixture (OTTO_DBV_E2E)"]
async fn mysql_batch_preserves_explicit_limit_forms() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }
    let driver = MysqlDriver::default();
    for clause in ["LIMIT\n2"] {
        let result = driver.run(&cfg(), &QueryRequest {
            statement: format!("SELECT CONNECTION_ID() AS backend FROM information_schema.columns {clause}; SELECT CONNECTION_ID() AS backend"),
            max_rows: Some(2), ..Default::default()
        }).await.unwrap();
        assert!(!result.rows.is_empty(), "{clause}");
        assert_eq!(result.more_results.len(), 1, "{clause}");
        assert_eq!(
            result.rows[0][0], result.more_results[0].rows[0][0],
            "{clause}"
        );
    }
}

/// Native commands must use COM_QUERY and shape rows from EXECUTE/admin commands.
#[tokio::test]
#[ignore = "requires an isolated SQL fixture (OTTO_DBV_E2E)"]
async fn mysql_native_commands_and_dynamic_result_rows() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }
    let driver = MysqlDriver::default();
    let result = driver.run(&cfg(), &query(
        "USE shopdb; PREPARE otto_parity FROM 'SELECT 42 AS answer'; EXECUTE otto_parity; DEALLOCATE PREPARE otto_parity; SELECT DATABASE()"
    )).await.unwrap();
    assert_eq!(result.more_results.len(), 4, "{result:?}");
    assert_eq!(result.more_results[1].rows, vec![vec![json!(42)]]);
    assert_eq!(result.more_results[1].columns[0].name, "answer");
    assert_eq!(result.more_results[3].rows[0][0], json!("shopdb"));
}

/// CTE DML must retain its acknowledgement, despite its WITH prefix.
#[tokio::test]
#[ignore = "requires an isolated SQL fixture (OTTO_DBV_E2E)"]
async fn mysql_cte_write_affected_rows() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }
    let driver = MysqlDriver::default();
    let result = driver.run(&cfg(), &query(
        "CREATE TEMPORARY TABLE otto_cte_parity (id INT); INSERT INTO otto_cte_parity VALUES (1), (2); WITH ids AS (SELECT 1 AS id) UPDATE otto_cte_parity t JOIN ids ON ids.id=t.id SET t.id=3; SELECT id FROM otto_cte_parity ORDER BY id"
    )).await.unwrap();
    assert_eq!(result.more_results[1].rows_affected, Some(1), "{result:?}");
    assert_eq!(
        result.more_results[2].rows,
        vec![vec![json!(2)], vec![json!(3)]]
    );
}

#[tokio::test]
#[ignore = "requires an isolated SQL fixture (OTTO_DBV_E2E)"]
async fn mysql_time_duration_and_precise_values() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }
    let driver = MysqlDriver::default();
    let result = driver.run(&cfg(), &query(
        "SELECT CAST('-125:30:15.123456' AS TIME(6)), CAST('838:59:59' AS TIME), CAST('18446744073709551615' AS UNSIGNED), CAST('12345678901234567890.123456' AS DECIMAL(30,6)), CAST(NULL AS TIME), X'00FF', CAST('-01:02:03' AS TIME)"
    )).await.unwrap();
    assert_eq!(
        result.rows[0],
        vec![
            json!("-125:30:15.123456"),
            json!("838:59:59"),
            json!("18446744073709551615"),
            json!("12345678901234567890.123456"),
            json!(null),
            json!("AP8="),
            json!("-01:02:03")
        ]
    );
}

/// Provision via a separate raw connection so the CALL reader is tested
/// independently from client-script DELIMITER support. The unique routine is
/// removed even when the assertion later fails.
#[tokio::test]
#[ignore = "requires an isolated SQL fixture (OTTO_DBV_E2E)"]
async fn mysql_call_keeps_each_result_boundary() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }
    use sqlx::Connection as _;
    let mut conn = sqlx::MySqlConnection::connect("mysql://otto:ottopw@127.0.0.1:13306/shopdb")
        .await
        .unwrap();
    let name = format!(
        "otto_parity_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "CREATE PROCEDURE `{name}`() BEGIN SELECT 1 AS a UNION ALL SELECT 2; SELECT 'second' AS b, 3 AS c; END"
    ))).execute(&mut conn).await.unwrap();
    let driver = MysqlDriver::default();
    let response = driver
        .run(
            &cfg(),
            &QueryRequest {
                statement: format!("CALL `{name}`(); SELECT 'after' AS d"),
                max_rows: Some(1),
                ..Default::default()
            },
        )
        .await;
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("DROP PROCEDURE `{name}`")))
        .execute(&mut conn)
        .await
        .unwrap();
    let result = response.unwrap();
    assert_eq!(result.rows, vec![vec![json!(1)]]);
    assert!(result.truncated);
    assert_eq!(result.more_results.len(), 3, "{result:?}");
    assert_eq!(result.more_results[0].columns[0].name, "b");
    assert_eq!(
        result.more_results[0].rows,
        vec![vec![json!("second"), json!(3)]]
    );
    assert_eq!(result.more_results[1].rows_affected, Some(0));
    assert_eq!(result.more_results[2].rows, vec![vec![json!("after")]]);
}

#[tokio::test]
#[ignore = "requires an isolated SQL fixture (OTTO_DBV_E2E)"]
async fn mysql_empty_select_keeps_columns() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }
    let result = MysqlDriver::default()
        .run(
            &cfg(),
            &query("SELECT 42 AS answer, 'text' AS label WHERE FALSE"),
        )
        .await
        .unwrap();
    assert!(result.rows.is_empty());
    assert_eq!(
        result
            .columns
            .iter()
            .map(|col| col.name.as_str())
            .collect::<Vec<_>>(),
        vec!["answer", "label"]
    );
}

#[tokio::test]
#[ignore = "requires an isolated SQL fixture (OTTO_DBV_E2E)"]
async fn mysql_timeout_covers_lowercase_commands_and_batches() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }
    let driver = MysqlDriver::default();
    for sql in [
        "select sleep(10)",
        "DO SLEEP(10)",
        "SELECT 1; DO SLEEP(10); SELECT 2",
    ] {
        let started = std::time::Instant::now();
        let error = driver
            .run(
                &cfg(),
                &QueryRequest {
                    statement: sql.into(),
                    timeout_ms: Some(100),
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("timed out"), "{sql}: {error}");
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "{sql}"
        );
        assert_eq!(
            driver.run(&cfg(), &query("SELECT 7")).await.unwrap().rows[0][0],
            json!(7)
        );
    }
}

#[tokio::test]
#[ignore = "requires an isolated SQL fixture (OTTO_DBV_E2E)"]
async fn mysql_delimiter_routine_script_executes_as_native_batch() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }
    let name = format!(
        "otto_script_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let driver = MysqlDriver::default();
    let result = driver.run(&cfg(), &query(&format!(
        "DELIMITER $$\nCREATE PROCEDURE `{name}`() BEGIN SELECT 11 AS first_result; SELECT 22 AS second_value; END$$\nDELIMITER ;\nCALL `{name}`();"
    ))).await;
    // Clean this test's own uniquely named routine even if CALL failed.
    driver
        .run(
            &cfg(),
            &query(&format!("DROP PROCEDURE IF EXISTS `{name}`")),
        )
        .await
        .unwrap();
    let result = result.unwrap();
    assert_eq!(result.more_results.len(), 3, "{result:?}");
    assert_eq!(result.more_results[0].rows, vec![vec![json!(11)]]);
    assert_eq!(result.more_results[1].rows, vec![vec![json!(22)]]);
    assert_eq!(result.more_results[2].rows_affected, Some(0));
}

#[derive(Clone, Default)]
struct ExportBuffer(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
impl std::io::Write for ExportBuffer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
#[ignore = "requires an isolated SQL fixture (OTTO_DBV_E2E)"]
async fn mysql_streaming_export_preserves_empty_schema_and_native_values() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }
    use otto_dbviewer::export::ExportFormat;
    let driver = MysqlDriver::default();
    for (sql, limit, expected) in [
        ("SELECT 1 AS answer WHERE FALSE", None, "answer\n"),
        ("SELECT 1 AS answer", Some(0), "answer\n"),
        (
            "SELECT CAST('-125:30:15.123456' AS TIME(6)) AS duration",
            None,
            "duration\n-125:30:15.123456\n",
        ),
    ] {
        let buffer = ExportBuffer::default();
        driver
            .export_to_writer(
                &cfg(),
                sql,
                None,
                ExportFormat::CsvWithNames,
                limit,
                otto_dbviewer::export::ExportWriter::Direct(Box::new(buffer.clone())),
            )
            .await
            .unwrap();
        assert_eq!(
            String::from_utf8(buffer.0.lock().unwrap().clone()).unwrap(),
            expected
        );
    }
    let error = driver
        .export_to_writer(
            &cfg(),
            "SELECT 1; SELECT 2",
            None,
            ExportFormat::Csv,
            None,
            otto_dbviewer::export::ExportWriter::Direct(Box::new(ExportBuffer::default())),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("one row-returning statement"));
}

#[tokio::test]
#[ignore = "requires an isolated SQL fixture (OTTO_DBV_E2E)"]
async fn mysql_batch_transaction_rolls_back_and_session_variables_do_not_leak() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }
    let driver = MysqlDriver::default();
    let result = driver.run(&cfg(), &query(
        "CREATE TEMPORARY TABLE otto_tx_parity (id INT) ENGINE=InnoDB; SET @otto_parity=99; START TRANSACTION; INSERT INTO otto_tx_parity VALUES (7); ROLLBACK; SELECT COUNT(*) AS count, @otto_parity AS variable FROM otto_tx_parity"
    )).await.unwrap();
    assert_eq!(
        result.more_results.last().unwrap().rows[0],
        vec![json!(0), json!(99)]
    );
    let later = driver
        .run(&cfg(), &query("SELECT @otto_parity"))
        .await
        .unwrap();
    assert_eq!(later.rows[0][0], json!(null));
}

#[tokio::test]
#[ignore = "requires an isolated SQL fixture (OTTO_DBV_E2E)"]
async fn mysql_governed_read_retires_nontransactional_session_state() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }
    let driver = MysqlDriver::default();
    let mut cfg = cfg();
    cfg.params = json!({"__read_only_execution": true});
    let result = driver
        .run(&cfg, &query("SELECT @otto_governed_parity:=99"))
        .await
        .unwrap();
    assert_eq!(result.rows[0][0], json!(99));
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let later = driver
        .run(&cfg, &query("SELECT @otto_governed_parity"))
        .await
        .unwrap();
    assert_eq!(later.rows[0][0], json!(null));
}

#[tokio::test]
#[ignore = "requires an isolated SQL fixture (OTTO_DBV_E2E)"]
async fn mysql_call_preserves_empty_result_metadata_and_ack() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }
    let name = format!(
        "otto_empty_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let driver = MysqlDriver::default();
    let response = driver.run(&cfg(), &query(&format!(
        "DELIMITER $$\nCREATE PROCEDURE `{name}`() BEGIN SELECT 1 AS empty_first WHERE FALSE; SELECT 'x' AS empty_second, 2 AS other WHERE FALSE; SELECT 3 AS last_result; END$$\nDELIMITER ;\nCALL `{name}`();"
    ))).await;
    driver
        .run(
            &cfg(),
            &query(&format!("DROP PROCEDURE IF EXISTS `{name}`")),
        )
        .await
        .unwrap();
    let result = response.unwrap();
    assert_eq!(result.more_results.len(), 4, "{result:?}");
    let first = &result.more_results[0];
    assert!(first.rows.is_empty());
    assert_eq!(first.columns[0].name, "empty_first");
    assert_eq!(
        first.rows_affected, None,
        "empty SELECT is not a command acknowledgement"
    );
    let second = &result.more_results[1];
    assert!(second.rows.is_empty());
    assert_eq!(
        second
            .columns
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>(),
        vec!["empty_second", "other"]
    );
    assert_eq!(second.rows_affected, None);
    assert_eq!(result.more_results[2].rows, vec![vec![json!(3)]]);
    assert!(result.more_results[3].columns.is_empty());
    assert_eq!(
        result.more_results[3].rows_affected,
        Some(0),
        "CALL acknowledgement stays distinct"
    );
}

#[tokio::test]
#[ignore = "requires isolated MySQL fixture (OTTO_DBV_E2E)"]
async fn mysql_batch_uses_current_mode_after_set_and_execute() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }
    let driver = MysqlDriver::default();
    for change in [
        "SET SESSION sql_mode='NO_BACKSLASH_ESCAPES'",
        "PREPARE mode_stmt FROM 'SET SESSION sql_mode=\"NO_BACKSLASH_ESCAPES\"'; EXECUTE mode_stmt",
    ] {
        let sql = format!(
            r"{change}; SELECT 'ends\' AS value; SET SESSION sql_mode=''; SELECT 'normal\\' AS value; SELECT 77 AS last_result"
        );
        let result = driver.run(&cfg(), &query(&sql)).await.unwrap();
        let rows: Vec<_> = result
            .more_results
            .iter()
            .filter(|r| !r.rows.is_empty())
            .map(|r| r.rows.clone())
            .collect();
        assert_eq!(
            rows,
            vec![
                vec![vec![json!("ends\\")]],
                vec![vec![json!("normal\\")]],
                vec![vec![json!(77)]]
            ],
            "{result:?}"
        );
        assert!(!result.more_results.iter().any(|r| r.errored), "{result:?}");
    }
}

#[tokio::test]
#[ignore = "requires isolated MySQL fixture (OTTO_DBV_E2E)"]
async fn mysql_native_boundaries_survive_ansi_quotes_chunk() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }
    let driver = MysqlDriver::default();
    let result = driver.run(&cfg(), &query(r#"SET SESSION sql_mode='ANSI_QUOTES'; SELECT 1 AS "ends\"; SELECT 'second' AS last_result;"#)).await.unwrap();
    assert_eq!(result.more_results.len(), 2, "{result:?}");
    assert_eq!(result.more_results[0].columns[0].name, "ends\\");
    assert_eq!(result.more_results[0].rows, vec![vec![json!(1)]]);
    assert_eq!(result.more_results[1].columns[0].name, "last_result");
    assert_eq!(result.more_results[1].rows, vec![vec![json!("second")]]);
}
