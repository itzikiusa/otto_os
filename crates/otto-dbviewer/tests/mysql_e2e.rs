//! End-to-end tests for the MySQL driver against a live server.
//!
//! Requires a MySQL seeded as described in the seed reference (docker MySQL on
//! 127.0.0.1:13306, user otto / ottopw, database `shopdb`). Each test is
//! `#[ignore]` by default and additionally guarded by `OTTO_DBV_E2E`. Run with:
//!   OTTO_DBV_E2E=1 cargo test -p otto-dbviewer --test mysql_e2e -- --ignored --nocapture

use otto_dbviewer::driver::Driver;
use otto_dbviewer::drivers::mysql::MysqlDriver;
use otto_dbviewer::types::{
    CompletionContext, CompletionKind, Engine, NodePath, QueryRequest, ResolvedConfig, TlsConfig,
};
use serde_json::json;

fn cfg() -> ResolvedConfig {
    ResolvedConfig {
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
    assert!(test.ok, "test().ok should be true; message: {}", test.message);
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
    assert_eq!(res.more_results[0].statement.as_deref(), Some("SELECT 2 AS b"));
    // A single statement doesn't gain the batch fields.
    let one = d.run(&cfg(), &query("SELECT 1 AS a")).await.expect("run(single)");
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
        .run(&cfg(), &query("SELECT 1 AS a; SELECT * FROM no_such_table_xyz; SELECT 3"))
        .await
        .expect("batch returns Ok with a partial result, not Err");
    // First statement succeeded (top-level); the failure is the terminal entry.
    assert_eq!(res.rows[0][0].as_i64(), Some(1));
    assert_eq!(res.more_results.len(), 1, "stopped at the failing statement");
    let failed = &res.more_results[0];
    assert!(failed.errored, "second entry flagged errored");
    assert!(!failed.message.as_deref().unwrap_or("").is_empty(), "carries the engine error");
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
    assert!(cols.contains(&"email"), "customers.email expected: {cols:?}");
    assert!(
        !cols.contains(&"total_cents"),
        "must not leak orders columns through c.: {cols:?}"
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
    let folders = d.schema_children(&cfg, &shopdb, None).await.expect("db folders");
    assert!(
        folders.iter().any(|n| n.label == "Triggers"),
        "shopdb should expose a Triggers folder; got: {:?}",
        folders.iter().map(|n| &n.label).collect::<Vec<_>>()
    );

    // Triggers folder → the seeded trigger.
    let trig_folder = NodePath::parse("db:shopdb/folder:triggers");
    let trigs = d.schema_children(&cfg, &trig_folder, None).await.expect("triggers");
    let trg = trigs
        .iter()
        .find(|n| n.label == "trg_orders_clamp_total")
        .expect("trg_orders_clamp_total present");

    // object_detail → DDL + extra.{table,event,timing}.
    let detail = d.object_detail(&cfg, &NodePath::parse(&trg.id)).await.expect("trigger detail");
    let ddl = detail.ddl.unwrap_or_default().to_uppercase();
    assert!(ddl.contains("TRIGGER"), "trigger DDL should mention TRIGGER; got: {ddl}");
    assert_eq!(detail.extra.get("table").and_then(|v| v.as_str()), Some("orders"));
    assert_eq!(detail.extra.get("event").and_then(|v| v.as_str()), Some("INSERT"));
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
        .query_plan(&cfg(), "SELECT * FROM orders WHERE total_cents > 0", Some("shopdb"))
        .await
        .expect("query_plan");
    assert_eq!(plan.engine, "mysql");
    // The orders access node should be present with an object name.
    let has_table = plan.root.children.iter().any(|c| c.object.as_deref() == Some("orders"));
    assert!(has_table, "plan should reference the orders table; root: {:?}", plan.root);
    // A full scan (access_type ALL) is warned.
    let full_scan = plan
        .root
        .children
        .iter()
        .any(|c| c.warnings.iter().any(|w| w.contains("full table scan")));
    assert!(full_scan, "expected a full-table-scan warning; root: {:?}", plan.root);
}
