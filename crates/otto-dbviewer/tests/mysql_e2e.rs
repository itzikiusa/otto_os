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
        .schema_children(&cfg, &shopdb)
        .await
        .expect("schema_children(db)");
    let tables_folder = folders
        .iter()
        .find(|n| n.id.ends_with("folder:tables"))
        .expect("a 'Tables' folder");

    // expand folder:tables -> contains table 'orders'.
    let tables_path = NodePath::parse(&tables_folder.id);
    let tables = d
        .schema_children(&cfg, &tables_path)
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
        .schema_children(&cfg, &orders_path)
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

/// completion offers the SELECT keyword and the orders table.
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
        node: None,
    };
    let comp = d.completion(&cfg, &ctx).await.expect("completion");
    assert!(
        comp.items.iter().any(|i| {
            i.kind == CompletionKind::Keyword && i.label.eq_ignore_ascii_case("SELECT")
        }),
        "completion should include a Keyword 'SELECT'"
    );
    assert!(
        comp.items
            .iter()
            .any(|i| i.kind == CompletionKind::Table && i.label == "orders"),
        "completion should include a Table 'orders'"
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
