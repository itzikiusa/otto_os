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
        lifecycle: None,
        engine: Engine::Mongodb,
        host: "127.0.0.1".into(),
        port: std::env::var("OTTO_MONGO_PARITY_PORT")
            .ok()
            .and_then(|port| port.parse().ok())
            .unwrap_or(17017),
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
    assert!(
        json_result.rows.len() >= 4,
        "json find should return >=4 rows"
    );
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
        c.items
            .iter()
            .any(|i| i.label == "orders" && i.kind == CompletionKind::Collection),
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
        m.items
            .iter()
            .any(|i| i.label == "find" && i.kind == CompletionKind::Command),
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
    assert!(
        field("customerId") > field("totalCents"),
        "indexed field ranks first"
    );
    assert!(
        field("status") > field("totalCents"),
        "indexed field ranks first"
    );
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
    assert!(
        labels.contains(&"address"),
        "parent path offered: {labels:?}"
    );
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

/// `import_rows` bulk-inserts parsed rows as `insertMany` batches; a follow-up
/// find confirms the documents landed with their inferred types.
#[tokio::test]
#[ignore]
async fn mongo_import_rows_inserts_batches() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }
    let d = MongoDriver::default();
    let cfg = cfg();
    let coll = "e2e_import_scratch";
    let stmt = |s: &str| QueryRequest {
        statement: s.into(),
        ..Default::default()
    };

    // Clean slate — deleteMany IS a parsed console op (drop() is not; a silently
    // failed cleanup left duplicate _ids behind for the next run).
    d.run(&cfg, &stmt(&format!("db.{coll}.deleteMany({{}})")))
        .await
        .expect("cleanup deleteMany");

    let columns = vec!["_id".to_string(), "name".to_string(), "qty".to_string()];
    let rows = vec![
        vec![json!("imp-1"), json!("Ada"), json!(10)],
        vec![json!("imp-2"), json!("Grace"), json!(20)],
        vec![json!("imp-3"), json!("Linus"), json!(30)],
    ];
    let (inserted, batches) = d
        .import_rows(&cfg, coll, &columns, &rows, 2, None)
        .await
        .expect("import_rows");
    assert_eq!(inserted, 3, "3 documents inserted");
    assert_eq!(batches, 2, "batch size 2 over 3 rows → 2 batches");

    // Verify they are queryable, with the numeric type preserved.
    let res = d
        .run(&cfg, &stmt(&format!("db.{coll}.find({{}})")))
        .await
        .expect("find imported");
    assert_eq!(res.rows.len(), 3, "find should return the 3 imported docs");

    // Cleanup.
    let _ = d.run(&cfg, &stmt(&format!("db.{coll}.drop()"))).await;
}

/// `query_plan` returns a normalized plan for a find (a stage tree from
/// explain queryPlanner).
#[tokio::test]
#[ignore]
async fn mongo_query_plan_returns_stage_tree() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }
    let d = MongoDriver::default();
    let plan = d
        .query_plan(&cfg(), "db.orders.find({})", None)
        .await
        .expect("query_plan");
    assert_eq!(plan.engine, "mongodb");
    assert!(
        !plan.root.op.is_empty(),
        "plan root should carry a stage op"
    );
}

/// Unsupported native forms must execute through mongosh without dropping
/// options; fixture writes use a unique collection and are cleaned up.
#[tokio::test]
#[ignore = "requires OTTO_DBV_E2E Mongo fixture and mongosh"]
async fn mongo_shell_fallback_preserves_options_and_returns_values() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }
    let driver = MongoDriver::default();
    let mut config = cfg();
    if let Ok(port) = std::env::var("OTTO_MONGO_PARITY_PORT") {
        config.port = port.parse().unwrap();
    }
    let collection = format!("otto_parity_{}", std::process::id());
    let run = |statement: String| QueryRequest {
        statement,
        max_rows: Some(2),
        timeout_ms: Some(10_000),
        ..Default::default()
    };
    let inserted = driver
        .run(
            &config,
            &run(format!(
                "db.{collection}.updateOne({{k: 1}}, {{$set: {{value: 9}}}}, {{upsert: true}})"
            )),
        )
        .await
        .expect("upsert options preserved by shell");
    assert!(!inserted.rows.is_empty(), "write result is visible");
    let result = driver
        .run(
            &config,
            &run(format!(
                "db.{collection}.findOne({{k: 1}}, {{_id: 0, value: 1}})"
            )),
        )
        .await
        .expect("findOne shell fallback");
    assert!(result
        .rows
        .iter()
        .flatten()
        .any(|v| v.to_string().contains("value")));
    let count = driver
        .run(
            &config,
            &run(format!("db.{collection}.countDocuments({{k: 1}})")),
        )
        .await
        .expect("native count");
    assert_eq!(count.rows[0][0], json!(1));
    driver
        .run(
            &config,
            &run(format!(
                "db.{collection}.insertMany([{{k:2,late:17}},{{k:3}},{{k:4}},{{k:5}}])"
            )),
        )
        .await
        .expect("seed cursor");
    for format in [
        otto_dbviewer::export::ExportFormat::Json,
        otto_dbviewer::export::ExportFormat::Ndjson,
    ] {
        let file = tempfile::NamedTempFile::new().unwrap();
        driver
            .export_to_writer(
                &config,
                &format!("db.{collection}.find({{}}).sort({{k:1}})"),
                None,
                format,
                None,
                Box::new(file.reopen().unwrap()),
            )
            .await
            .expect("JSON export");
        let bytes = std::fs::read_to_string(file.path()).unwrap();
        let docs: Vec<serde_json::Value> =
            if matches!(format, otto_dbviewer::export::ExportFormat::Json) {
                serde_json::from_str(&bytes).unwrap()
            } else {
                bytes
                    .lines()
                    .map(|line| serde_json::from_str(line).unwrap())
                    .collect()
            };
        assert_eq!(docs[1]["late"], json!(17), "sparse key must survive export");
        assert!(
            docs[0]["_id"].get("$oid").is_some(),
            "ObjectId must keep its type"
        );
    }
    let distinct = driver
        .run(&config, &run(format!("db.{collection}.distinct('k')")))
        .await
        .expect("native distinct");
    assert_eq!(distinct.rows.len(), 2);
    assert!(distinct.truncated);
    let cursor_count = driver
        .run(&config, &run(format!("db.{collection}.find({{}}).count()")))
        .await
        .expect("native cursor count");
    assert_eq!(cursor_count.rows[0][0], json!(5));
    // Legacy count accepts $where; countDocuments' aggregation rejects it.
    // This distinguishes the server operations even when normal counts agree.
    let legacy = driver
        .run(
            &config,
            &run(format!(
                "db.{collection}.count({{$where: 'return this.k > 0'}})"
            )),
        )
        .await
        .expect("legacy count command semantics");
    assert_eq!(legacy.rows[0][0], json!(5));
    assert!(
        driver
            .run(
                &config,
                &run(format!(
                    "db.{collection}.countDocuments({{$where: 'return this.k > 0'}})"
                ))
            )
            .await
            .is_err(),
        "countDocuments must retain aggregation restrictions"
    );
    let json_count = driver
        .run(
            &config,
            &run(json!({
                "collection": collection, "op": "count", "filter": {"k": 1}
            })
            .to_string()),
        )
        .await
        .expect("JSON legacy count");
    assert_eq!(json_count.rows[0][0], json!(1));
    let plan = driver
        .run(
            &config,
            &run(format!("db.{collection}.find({{}}).explain()")),
        )
        .await
        .expect("native explain accepts timeout and cancellation comment");
    assert_eq!(plan.columns[0].name, "queryPlan");
    let exact = driver
        .run(
            &config,
            &run(format!("db.{collection}.find({{}}).limit(2)")),
        )
        .await
        .expect("native explicit limit");
    assert_eq!(exact.rows.len(), 2);
    assert!(
        !exact.truncated,
        "explicit limit is the entire requested result"
    );
    let cursor = driver
        .run(
            &config,
            &run(format!(
                "db.{collection}.find({{}}, {{_id:0,k:1}}).sort({{k:-1}}).skip(1)"
            )),
        )
        .await
        .expect("shell cursor");
    assert_eq!(cursor.rows.len(), 2);
    assert!(cursor.truncated);
    assert!(cursor.rows[0][0].as_str().unwrap().contains("4"));
    let count = driver
        .run(
            &config,
            &run(format!(
                "db.{collection}.countDocuments({{}}, {{skip:1, limit:2}})"
            )),
        )
        .await
        .expect("shell count options");
    assert!(count.rows[0][0].as_str().unwrap().contains("2"));
    let admin = driver
        .run(&config, &run("db.runCommand({ping: 1})".into()))
        .await
        .expect("administrative command fallback");
    assert!(admin.rows[0][0].as_str().unwrap().contains("ok"));
    let raw = driver
        .run(&config, &run("const answer = 42; print(answer);".into()))
        .await
        .expect("raw script preserved");
    assert_eq!(raw.rows[0][0], json!("42"));
    for script in [
        "do { print('42'); } while(false)",
        "switch(1) { case 1: print('42'); break; }",
        "if\t(true) { print('42'); }",
        "/* leading */ const x=42; print(x);",
    ] {
        let result = driver
            .run(&config, &run(script.into()))
            .await
            .unwrap_or_else(|error| panic!("raw script {script}: {error}"));
        assert_eq!(result.rows[0][0], json!("42"), "{script}");
    }
    let sibling = driver
        .run(
            &config,
            &run(
                "/* active database */\ndb.getSiblingDB('otto_parity_sibling')\n.getName()".into(),
            ),
        )
        .await
        .expect("single sibling DB expression returns its value");
    assert!(sibling.rows[0][0]
        .as_str()
        .unwrap()
        .contains("otto_parity_sibling"));
    let selected = driver
        .run(
            &config,
            &QueryRequest {
                node: Some("otto_parity_selected".into()),
                ..run("db.getName()".into())
            },
        )
        .await
        .expect("selected database in shell");
    assert!(selected.rows[0][0]
        .as_str()
        .unwrap()
        .contains("otto_parity_selected"));
    driver
        .run(&config, &run(format!("db.{collection}.drop()")))
        .await
        .expect("cleanup collection via shell fallback");
}

#[tokio::test]
#[ignore = "requires OTTO_DBV_E2E Mongo fixture and mongosh"]
async fn mongo_shell_timeout_bounds_long_running_scripts() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }
    let mut config = cfg();
    if let Ok(port) = std::env::var("OTTO_MONGO_PARITY_PORT") {
        config.port = port.parse().unwrap();
    }
    let started = std::time::Instant::now();
    let error = MongoDriver::default()
        .run(
            &config,
            &QueryRequest {
                statement: "print('started'); sleep(30000);".into(),
                timeout_ms: Some(1200),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("timed out"), "{error}");
    assert!(started.elapsed() < std::time::Duration::from_secs(10));
}

#[tokio::test]
#[ignore = "requires OTTO_DBV_E2E Mongo fixture"]
async fn mongo_cancellation_isolates_repeated_client_query_ids() {
    if std::env::var("OTTO_DBV_E2E").is_err() {
        return;
    }
    use otto_dbviewer::types::{CancelToken, QueryHandle};
    let config = cfg();
    let driver = std::sync::Arc::new(MongoDriver::default());
    let first_token = CancelToken::new();
    let second_token = CancelToken::new();
    let start = |token: CancelToken| {
        let driver = driver.clone();
        let config = config.clone();
        tokio::spawn(async move {
            driver
                .run_tracked(
                    &config,
                    &QueryRequest {
                        statement: "db.customers.find({$where: 'sleep(10000) || true'})".into(),
                        query_id: Some("same-client-id".into()),
                        ..Default::default()
                    },
                    &token,
                )
                .await
        })
    };
    let first = start(first_token.clone());
    let second = start(second_token.clone());
    let handles = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let (Some(a), Some(b)) = (first_token.handle(), second_token.handle()) {
                break (a, b);
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let (QueryHandle::MongoComment(a), QueryHandle::MongoComment(b)) = (&handles.0, &handles.1)
    else {
        panic!("missing Mongo tags")
    };
    assert_ne!(a, b);
    let client = mongodb::Client::with_uri_str(format!(
        "mongodb://otto:ottopw@127.0.0.1:{}/?authSource=admin",
        config.port
    ))
    .await
    .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let result = client.database("admin").run_command(mongodb::bson::doc! {
                "aggregate":1, "pipeline":[{"$currentOp":{"allUsers":true}}, {"$match":{"command.comment":{"$in":[a,b]}}}], "cursor":{}
            }).await.unwrap();
            if result.get_document("cursor").unwrap().get_array("firstBatch").unwrap().len() == 2 { break; }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }).await.expect("both tagged queries reached server");
    driver.cancel(&config, &handles.0).await.unwrap();
    let cancelled = tokio::time::timeout(std::time::Duration::from_secs(5), first)
        .await
        .unwrap()
        .unwrap();
    assert!(cancelled.is_err());
    assert!(
        !second.is_finished(),
        "cancelling first must leave second query running"
    );
    driver.cancel(&config, &handles.1).await.unwrap();
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(5), second)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
}
