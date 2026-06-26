//! Live end-to-end tests for the MongoDB driver.
//!
//! Each test is `#[ignore]` by default and additionally guarded by
//! `OTTO_DBV_E2E`. Requires a seeded MongoDB on `127.0.0.1:17017` (user
//! `otto`/`ottopw`, authSource `admin`):
//!
//! ```sh
//! docker run -d --name otto-dbv-mongo -p 17017:27017 \
//!   -e MONGO_INITDB_ROOT_USERNAME=otto \
//!   -e MONGO_INITDB_ROOT_PASSWORD=ottopw mongo:8
//! # then seed `shopdb` (customers/products/orders) + `analytics` (events),
//! # with a unique index on customers.email and >=4 customers.
//! ```
//!
//! Run: `OTTO_DBV_E2E=1 cargo test -p otto-dbviewer --test mongodb_e2e -- --ignored --nocapture`

use otto_dbviewer::driver::Driver;
use otto_dbviewer::drivers::mongodb::MongoDriver;
use otto_dbviewer::types::{
    CompletionContext, CompletionKind, Engine, NodePath, QueryRequest, ResolvedConfig, TlsConfig,
};
use serde_json::json;

fn cfg() -> ResolvedConfig {
    ResolvedConfig {
        engine: Engine::Mongodb,
        host: "127.0.0.1".into(),
        port: 17017,
        user: Some("otto".into()),
        password: Some("ottopw".into()),
        database: Some("shopdb".into()),
        tls: TlsConfig::default(),
        params: json!({ "auth_source": "admin" }),
    }
}

/// `test()` connects and reports the server version from buildInfo.
#[tokio::test]
#[ignore]
async fn mongo_connect() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }

    let d = MongoDriver::default();
    let test = d.test(&cfg()).await.expect("test() should not Err");
    assert!(test.ok, "connection test failed: {}", test.message);
    assert!(
        test.server_version.is_some(),
        "expected a server version from buildInfo"
    );
    println!(
        "test ok, version={:?} latency={:?}ms",
        test.server_version, test.latency_ms
    );
}

/// schema_root → shopdb, expand to the customers collection, expand that to
/// sampled fields including email.
#[tokio::test]
#[ignore]
async fn mongo_schema_tree() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }

    let d = MongoDriver::default();
    let cfg = cfg();

    // schema_root contains shopdb.
    let roots = d.schema_root(&cfg).await.expect("schema_root");
    assert!(
        roots.iter().any(|n| n.label == "shopdb"),
        "schema_root should list shopdb, got: {:?}",
        roots.iter().map(|n| &n.label).collect::<Vec<_>>()
    );

    // expand db:shopdb → contains the customers collection.
    let db_path = NodePath::parse("db:shopdb");
    let colls = d
        .schema_children(&cfg, &db_path, None)
        .await
        .expect("schema_children(db)");
    assert!(
        colls.iter().any(|n| n.label == "customers"),
        "shopdb should contain a 'customers' collection, got: {:?}",
        colls.iter().map(|n| &n.label).collect::<Vec<_>>()
    );

    // expand the collection → sampled fields include email.
    let coll_path = NodePath::parse("db:shopdb/coll:customers");
    let fields = d
        .schema_children(&cfg, &coll_path, None)
        .await
        .expect("schema_children(coll)");
    assert!(
        fields.iter().any(|n| n.label == "email"),
        "customers fields should include 'email', got: {:?}",
        fields.iter().map(|n| &n.label).collect::<Vec<_>>()
    );
}

/// object_detail of customers: a unique index (on email) and the exact
/// collStats document count as `row_count`.
#[tokio::test]
#[ignore]
async fn mongo_object_detail() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }

    let d = MongoDriver::default();
    let cfg = cfg();

    let coll_path = NodePath::parse("db:shopdb/coll:customers");
    let detail = d
        .object_detail(&cfg, &coll_path)
        .await
        .expect("object_detail");
    assert!(
        !detail.indexes.is_empty(),
        "customers should report indexes"
    );
    assert!(
        detail.indexes.iter().any(|i| i.unique),
        "customers should have a unique index (on email)"
    );
    // object_detail reports the exact collStats count for a collection (cheap +
    // exact in Mongo, unlike SQL's opt-in estimates); the seed has 4 customers.
    assert_eq!(
        detail.row_count,
        Some(4),
        "customers row_count should be the collStats count (4), got {:?}",
        detail.row_count
    );
}

/// run() find returns >=4 docs including the 'email' field, via both the
/// `db.coll.find(...)` shorthand and the JSON command form.
#[tokio::test]
#[ignore]
async fn mongo_run() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }

    let d = MongoDriver::default();
    let cfg = cfg();

    // db.customers.find({}) shorthand.
    let req = QueryRequest {
        statement: "db.customers.find({})".into(),
        max_rows: Some(100),
        ..Default::default()
    };
    let result = d.run(&cfg, &req).await.expect("run(find)");
    assert!(
        result.rows.len() >= 4,
        "find should return >=4 rows, got {}",
        result.rows.len()
    );
    assert!(
        result.columns.iter().any(|c| c.name == "email"),
        "find result columns should include 'email', got: {:?}",
        result.columns.iter().map(|c| &c.name).collect::<Vec<_>>()
    );

    // JSON command form.
    let json_req = QueryRequest {
        statement: r#"{"collection":"customers","op":"find","filter":{}}"#.into(),
        max_rows: Some(100),
        ..Default::default()
    };
    let json_result = d.run(&cfg, &json_req).await.expect("run(json find)");
    assert!(json_result.rows.len() >= 4, "json find should return >=4 rows");
}

/// completion offers the $match operator and the customers collection.
#[tokio::test]
#[ignore]
async fn mongo_completion() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }

    let d = MongoDriver::default();
    let cfg = cfg();

    let ctx = CompletionContext {
        prefix: "$".into(),
        suffix: String::new(),
        database: Some("shopdb".into()),
        node: Some("db:shopdb/coll:customers".into()),
    };
    let completion = d.completion(&cfg, &ctx).await.expect("completion");
    assert!(
        completion
            .items
            .iter()
            .any(|i| i.label == "$match" && i.kind == CompletionKind::Operator),
        "completion should include Operator $match"
    );
    assert!(
        completion
            .items
            .iter()
            .any(|i| i.label == "customers" && i.kind == CompletionKind::Collection),
        "completion should include Collection customers"
    );
}

/// `db.` offers collections; `db.orders.` offers methods (find/aggregate).
#[tokio::test]
#[ignore]
async fn mongo_completion_collection_and_method() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }
    let d = MongoDriver::default();
    let cfg = cfg();

    let coll_ctx = CompletionContext {
        prefix: "db.".into(),
        suffix: String::new(),
        database: Some("shopdb".into()),
        node: None,
    };
    let c = d.completion(&cfg, &coll_ctx).await.expect("completion");
    assert!(
        c.items.iter().any(|i| i.label == "orders" && i.kind == CompletionKind::Collection),
        "db. should list collections"
    );
    assert!(
        !c.items.iter().any(|i| i.kind == CompletionKind::Command),
        "db. is not a method position"
    );

    let method_ctx = CompletionContext {
        prefix: "db.orders.".into(),
        suffix: String::new(),
        database: Some("shopdb".into()),
        node: None,
    };
    let m = d.completion(&cfg, &method_ctx).await.expect("completion");
    assert!(
        m.items.iter().any(|i| i.label == "find" && i.kind == CompletionKind::Command),
        "db.orders. should offer methods"
    );
}

/// Inside `db.orders.find({ ` the indexed fields (`_id`, `customerId`, `status`)
/// rank above plain ones.
#[tokio::test]
#[ignore]
async fn mongo_completion_fields_index_first() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }
    let d = MongoDriver::default();
    let cfg = cfg();
    let ctx = CompletionContext {
        prefix: "db.orders.find({ ".into(),
        suffix: String::new(),
        database: Some("shopdb".into()),
        node: None,
    };
    let c = d.completion(&cfg, &ctx).await.expect("completion");
    let field = |label: &str| {
        c.items
            .iter()
            .find(|i| i.kind == CompletionKind::Field && i.label == label)
            .unwrap_or_else(|| panic!("missing field {label}: {:?}", c.items))
            .score
            .unwrap_or(0)
    };
    assert!(field("customerId") > field("totalCents"), "indexed field ranks first");
    assert!(field("status") > field("totalCents"), "indexed field ranks first");
}

/// Embedded-field support: `db.profiles.find({ ` offers the parent `address`
/// AND the indexed embedded path `address.city`, both ranked as indexes.
#[tokio::test]
#[ignore]
async fn mongo_completion_embedded_paths() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }
    let d = MongoDriver::default();
    let cfg = cfg();
    let ctx = CompletionContext {
        prefix: "db.profiles.find({ ".into(),
        suffix: String::new(),
        database: Some("shopdb".into()),
        node: None,
    };
    let c = d.completion(&cfg, &ctx).await.expect("completion");
    let labels: Vec<&str> = c
        .items
        .iter()
        .filter(|i| i.kind == CompletionKind::Field)
        .map(|i| i.label.as_str())
        .collect();
    assert!(labels.contains(&"address"), "parent path offered: {labels:?}");
    assert!(
        labels.contains(&"address.city"),
        "indexed embedded path offered: {labels:?}"
    );
    // The indexed embedded path out-ranks a plain leaf like `address.zip`.
    let score = |label: &str| {
        c.items
            .iter()
            .find(|i| i.kind == CompletionKind::Field && i.label == label)
            .map(|i| i.score.unwrap_or(0))
            .unwrap_or(i32::MIN)
    };
    assert!(
        score("address.city") > score("prefs.theme"),
        "indexed embedded path ranks above a plain embedded path"
    );
}
