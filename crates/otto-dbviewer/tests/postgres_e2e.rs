//! End-to-end tests for the PostgreSQL driver against a live server.
//!
//! Requires the dev Postgres (docker `postgres:17` on 127.0.0.1:15432, user
//! otto / pass ottopw, database `shopdb` seeded from
//! dev/dbviewer/seed/postgres/01_schema.sql). Each test is `#[ignore]` by default
//! and additionally guarded by `OTTO_DBV_E2E`. Run with:
//!   OTTO_DBV_E2E=1 cargo test -p otto-dbviewer --test postgres_e2e -- --ignored --nocapture

use std::sync::Arc;
use std::time::Duration;

use otto_dbviewer::driver::Driver;
use otto_dbviewer::drivers::postgres::PostgresDriver;
use otto_dbviewer::types::{
    CancelToken, CompletionContext, CompletionKind, Engine, NodePath, QueryRequest, ResolvedConfig,
    TlsConfig,
};
use serde_json::json;

fn cfg() -> ResolvedConfig {
    ResolvedConfig {
        engine: Engine::Postgres,
        host: "127.0.0.1".into(),
        port: 15432,
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

fn gated() -> bool {
    std::env::var("OTTO_DBV_E2E").is_err()
}

#[tokio::test]
#[ignore]
async fn postgres_connect() {
    if gated() {
        return;
    }
    let d = PostgresDriver::default();
    let test = d.test(&cfg()).await.expect("test() should not error");
    assert!(test.ok, "test().ok should be true; message: {}", test.message);
    assert!(test.server_version.is_some(), "server_version should be Some");
    eprintln!("server_version = {:?}", test.server_version);
}

/// schema_root → the database's schemas (public present); expand public → the
/// Tables folder → orders; expand orders → columns include customer_id.
#[tokio::test]
#[ignore]
async fn postgres_schema_tree() {
    if gated() {
        return;
    }
    let d = PostgresDriver::default();
    let cfg = cfg();

    let roots = d.schema_root(&cfg).await.expect("schema_root");
    assert!(
        roots.iter().any(|n| n.label == "public"),
        "schema_root should contain 'public'; got: {:?}",
        roots.iter().map(|n| &n.label).collect::<Vec<_>>()
    );
    // public first (public before reporting).
    assert_eq!(roots.first().map(|n| n.label.as_str()), Some("public"));

    let public = NodePath::parse("db:public");
    let folders = d
        .schema_children(&cfg, &public, None)
        .await
        .expect("schema_children(db:public)");
    let tables_folder = folders
        .iter()
        .find(|n| n.label == "Tables")
        .expect("a Tables folder");
    // Functions folder shown because the seed has customer_order_count.
    assert!(
        folders.iter().any(|n| n.label == "Functions"),
        "expected a Functions folder; got: {:?}",
        folders.iter().map(|n| &n.label).collect::<Vec<_>>()
    );

    let tables = d
        .schema_children(&cfg, &NodePath::parse(&tables_folder.id), None)
        .await
        .expect("schema_children(folder:tables)");
    assert!(
        tables.iter().any(|n| n.label == "orders"),
        "Tables should include 'orders'; got: {:?}",
        tables.iter().map(|n| &n.label).collect::<Vec<_>>()
    );

    let orders_path = NodePath::parse("db:public/table:orders");
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

/// object_detail of orders: customer_id column, PK id, FK → customers, and a DDL
/// synthesized from the catalog (contains the FK constraint text).
#[tokio::test]
#[ignore]
async fn postgres_object_detail() {
    if gated() {
        return;
    }
    let d = PostgresDriver::default();
    let detail = d
        .object_detail(&cfg(), &NodePath::parse("db:public/table:orders"))
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
        detail.foreign_keys.iter().any(|fk| fk.ref_table == "customers"),
        "orders should have an FK referencing 'customers'; got: {:?}",
        detail
            .foreign_keys
            .iter()
            .map(|fk| (&fk.name, &fk.ref_table))
            .collect::<Vec<_>>()
    );
    let ddl = detail.ddl.expect("orders DDL");
    assert!(
        ddl.contains("CREATE TABLE") && ddl.contains("customer_id"),
        "DDL should be a synthesized CREATE TABLE; got: {ddl}"
    );
}

/// run a SELECT against customers; first row email is ada@example.com.
#[tokio::test]
#[ignore]
async fn postgres_run_select() {
    if gated() {
        return;
    }
    let d = PostgresDriver::default();
    let res = d
        .run(&cfg(), &query("SELECT id, email FROM customers ORDER BY id"))
        .await
        .expect("run(SELECT)");
    assert!(res.rows.len() >= 4, "expected >= 4 rows; got {}", res.rows.len());
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

/// A true multi-statement batch returns TWO results (first on top, second in
/// more_results), each with its statement preview.
#[tokio::test]
#[ignore]
async fn postgres_run_multi_statement_batch() {
    if gated() {
        return;
    }
    let d = PostgresDriver::default();
    let res = d
        .run(&cfg(), &query("SELECT 1 AS a; SELECT 2 AS b"))
        .await
        .expect("run(batch)");
    assert_eq!(res.rows[0][0].as_i64(), Some(1), "first result = SELECT 1");
    assert_eq!(res.statement.as_deref(), Some("SELECT 1 AS a"));
    assert_eq!(res.more_results.len(), 1, "one trailing result");
    assert_eq!(res.more_results[0].rows[0][0].as_i64(), Some(2));
    assert_eq!(res.more_results[0].statement.as_deref(), Some("SELECT 2 AS b"));
}

/// After `FROM`, completion offers the tables ranked above keywords; in a WHERE
/// the in-scope table's columns come back index-first (PK > UNIQUE > INDEX > plain).
#[tokio::test]
#[ignore]
async fn postgres_completion_index_first() {
    if gated() {
        return;
    }
    let d = PostgresDriver::default();
    let cfg = cfg();

    let from_ctx = CompletionContext {
        database: Some("public".into()),
        prefix: "SELECT * FROM ".into(),
        suffix: String::new(),
        node: None,
    };
    let comp = d.completion(&cfg, &from_ctx).await.expect("completion(FROM)");
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

    let where_ctx = CompletionContext {
        database: Some("public".into()),
        prefix: "SELECT * FROM customers WHERE ".into(),
        suffix: String::new(),
        node: None,
    };
    let comp = d.completion(&cfg, &where_ctx).await.expect("completion(WHERE)");
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

/// EXPLAIN (FORMAT JSON) returns a plan row (smoke test for the plan path).
#[tokio::test]
#[ignore]
async fn postgres_explain_format_json() {
    if gated() {
        return;
    }
    let d = PostgresDriver::default();
    let res = d
        .run(&cfg(), &query("EXPLAIN (FORMAT JSON) SELECT * FROM customers"))
        .await
        .expect("run(EXPLAIN FORMAT JSON)");
    assert!(!res.rows.is_empty(), "EXPLAIN should return a plan row");
}

/// The session time zone is applied on connect via `SET TIME ZONE`.
#[tokio::test]
#[ignore]
async fn postgres_timezone() {
    if gated() {
        return;
    }
    let d = PostgresDriver::default();
    let mut tz_cfg = cfg();
    tz_cfg.params = json!({ "timezone": "America/New_York" });
    let res = d
        .run(&tz_cfg, &query("SELECT current_setting('TimeZone')"))
        .await
        .expect("run(current_setting TimeZone)");
    assert_eq!(
        res.rows[0][0].as_str(),
        Some("America/New_York"),
        "session TimeZone should be America/New_York; got: {:?}",
        res.rows[0][0]
    );
}

/// A running query can be cancelled server-side via `pg_cancel_backend(pid)` on a
/// separate connection: a `SELECT pg_sleep(5)` is cancelled once its backend pid
/// is captured, and returns an error promptly (well under the 5s sleep).
#[tokio::test]
#[ignore]
async fn postgres_cancel() {
    if gated() {
        return;
    }
    let d = Arc::new(PostgresDriver::default());
    let token = CancelToken::new();

    let run_d = d.clone();
    let run_token = token.clone();
    let run = tokio::spawn(async move {
        run_d
            .run_tracked(&cfg(), &query("SELECT pg_sleep(5)"), &run_token)
            .await
    });

    // Wait for the backend pid to be captured (it's set right after acquire).
    let mut handle = None;
    for _ in 0..60 {
        if let Some(h) = token.handle() {
            handle = Some(h);
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let handle = handle.expect("backend pid should be captured");
    d.cancel(&cfg(), &handle).await.expect("cancel should be Ok");

    let res = tokio::time::timeout(Duration::from_secs(4), run)
        .await
        .expect("run finished well before the 5s sleep")
        .expect("join");
    assert!(res.is_err(), "a cancelled query should return an error");
}
