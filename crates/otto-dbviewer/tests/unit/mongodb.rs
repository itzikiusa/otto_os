use super::*;

fn script_cfg(params: Value) -> ResolvedConfig {
    ResolvedConfig {
        lifecycle: None,
        engine: crate::types::Engine::Mongodb,
        host: "127.0.0.1".into(),
        port: 27017,
        user: Some("root".into()),
        password: Some("p@ss/w".into()),
        database: None,
        tls: Default::default(),
        params,
    }
}

#[test]
fn capped_nested_document_carries_explicit_write_provenance() {
    let big = "x".repeat(types::MAX_CELL_CHARS + 7);
    let result = docs_to_result(
        vec![doc! {"_id": 1, "profile": {"bio": big, "name": "before"}}],
        false,
        Instant::now(),
    );
    let wire = serde_json::to_value(result).unwrap();
    assert_eq!(wire.get("cells_truncated"), Some(&Value::Bool(true)));
    let intact = docs_to_result(
        vec![doc! {"_id": 2, "bio": "ordinary…[truncated 7 chars]"}],
        false,
        Instant::now(),
    );
    assert!(serde_json::to_value(intact)
        .unwrap()
        .get("cells_truncated")
        .is_none());
}

#[test]
fn grid_expansion_counts_escaped_values_and_late_columns() {
    // Control characters expand to six JSON bytes each, and the last
    // document widens every preceding row by hundreds of null slots.
    let mut docs = (0..500)
        .map(|i| doc! {"_id": i, "text": "\u{0001}".repeat(16_000)})
        .collect::<Vec<_>>();
    let mut late = Document::new();
    for i in 0..498 {
        late.insert(format!("late_{i}"), i);
    }
    docs.push(late);
    let r = docs_to_result(docs, false, Instant::now());
    assert_eq!(r.columns.len(), 500);
    assert!(r.truncated);
    assert_eq!(r.truncated_reason, Some(types::TruncatedReason::Bytes));
    assert!(serde_json::to_vec(&r).unwrap().len() <= types::RESULT_BYTE_BUDGET);
    assert_eq!(r.rows[0][0], serde_json::json!(0));
    assert_eq!(r.rows[0][499], Value::Null);
}

#[test]
fn sparse_grid_expansion_obeys_cell_and_byte_budgets() {
    let docs = (0..100_000)
        .map(|i| {
            let mut d = doc! {"_id": i};
            d.insert(format!("field_{}", i % 499), i);
            d
        })
        .collect();
    let r = docs_to_result(docs, false, Instant::now());
    assert!(r.rows.len() * r.columns.len() <= 1_000_000);
    assert!(r.truncated);
    assert_eq!(r.truncated_reason, Some(types::TruncatedReason::Bytes));
    assert!(serde_json::to_vec(&r).unwrap().len() < 32 * 1024 * 1024);
    assert_eq!(r.rows[0][0], serde_json::json!(0));
}

/// Script output is collected within its caps as it streams: lines past
/// the line cap and bytes past the byte budget are counted as truncation,
/// never buffered; chunk boundaries never split a line; the tail survives
/// for error reports.
#[test]
fn script_output_is_bounded_while_streaming() {
    let mut out = ScriptOutput::new(3, 1024);
    out.push(b"one\ntw");
    out.push(b"o\r\nthree\nfour\nfive");
    let (lines, truncated, tail) = out.finish();
    assert_eq!(lines, vec!["one", "two", "three"]);
    assert!(truncated);
    assert!(tail.ends_with("four\nfive"));

    let mut exact = ScriptOutput::new(2, 1024);
    exact.push(b"a\nb\n");
    let (lines, truncated, _) = exact.finish();
    assert_eq!(lines, vec!["a", "b"]);
    assert!(!truncated, "exactly the cap is not truncation");

    let mut small = ScriptOutput::new(100, 4);
    small.push(b"abcdef\nxyz\n");
    let (lines, truncated, _) = small.finish();
    assert_eq!(lines, vec!["abcd"]);
    assert!(truncated);

    // The error tail stays bounded however much is written.
    let mut err = ScriptOutput::new(0, 0);
    for _ in 0..100 {
        err.push(&[b'x'; 4096]);
    }
    let (lines, _, tail) = err.finish();
    assert!(lines.is_empty());
    assert!(tail.len() <= 2 * SCRIPT_TAIL_BYTES);
}

/// A script honours the tab timeout (it used to run for up to 30 minutes
/// whatever the tab said); 0 / unset keeps the generous default.
#[test]
fn script_timeout_honours_the_tab_timeout() {
    use std::time::Duration;
    let default = Duration::from_secs(30 * 60);
    assert_eq!(script_timeout(None, default), default);
    assert_eq!(script_timeout(Some(0), default), default);
    assert_eq!(
        script_timeout(Some(90_000), default),
        Duration::from_secs(90)
    );
    assert_eq!(human_duration(default), "30 minutes");
    assert_eq!(human_duration(Duration::from_secs(90)), "1m 30s");
    assert_eq!(human_duration(Duration::from_secs(5)), "5s");
    assert_eq!(human_duration(Duration::from_millis(250)), "250ms");
}

/// The database selected in the tree wins over a profile `db` param for
/// every entry point (they all resolve through `resolve_db`); the profile
/// database is only the fallback.
#[test]
fn selected_database_wins_over_the_profile_default() {
    let mut cfg = script_cfg(serde_json::json!({}));
    cfg.database = Some("frb".into());
    assert_eq!(
        resolve_db(&cfg, Some("games_management")).unwrap(),
        "games_management"
    );
    assert_eq!(
        resolve_db(&cfg, Some("db:games_management/coll:users")).unwrap(),
        "games_management"
    );
    assert_eq!(resolve_db(&cfg, None).unwrap(), "frb");
    assert_eq!(resolve_db(&cfg, Some("  ")).unwrap(), "frb");
    // A database literally named `db` keeps its scope.
    assert_eq!(resolve_db(&cfg, Some("db")).unwrap(), "db");
    cfg.database = None;
    assert!(resolve_db(&cfg, None).is_err());
}

/// The script runner must dial EXACTLY what the native client dials:
/// host/port + percent-encoded credentials + authSource/replicaSet, the
/// selected database in the URI path, and — when the service opened an SSH
/// tunnel — the SOCKS5 proxy as Node-driver URI options.
#[test]
fn mongosh_invocation_builds_the_native_equivalent_uri() {
    let uri = mongosh_invocation(&script_cfg(json!({})), Some("db:promotions")).unwrap();
    assert_eq!(
        uri,
        "mongodb://root:p%40ss%2Fw@127.0.0.1:27017/promotions?authSource=admin"
    );

    // Tunnelled → SOCKS options appended to the existing query string.
    let uri = mongosh_invocation(
        &script_cfg(json!({ "__socks_port": 1080, "replica_set": "rs0" })),
        Some("db:promotions"),
    )
    .unwrap();
    assert!(uri.contains("replicaSet=rs0"), "uri: {uri}");
    assert!(
        uri.ends_with("&proxyHost=127.0.0.1&proxyPort=1080"),
        "uri: {uri}"
    );
}

/// A full `conn_string` wins verbatim (with `{secret}` substituted), and
/// the SOCKS options join with the right separator.
#[test]
fn mongosh_invocation_conn_string_wins_with_secret_substitution() {
    let mut cfg = script_cfg(json!({
        "conn_string": "mongodb+srv://u:{secret}@cluster.example.net/app?retryWrites=true",
        "__socks_port": 1081,
    }));
    cfg.password = Some("s3c".into());
    let uri = mongosh_invocation(&cfg, None).unwrap();
    assert_eq!(
            uri,
            "mongodb+srv://u:s3c@cluster.example.net/app?retryWrites=true&proxyHost=127.0.0.1&proxyPort=1081"
        );
}

/// The credential-bearing URI reaches mongosh through the 0600 script file
/// (`db = connect("…")`), never argv — the prelude must be a valid JS string
/// literal even when the URI holds quotes/backslashes.
#[test]
fn mongosh_prelude_json_escapes_the_uri() {
    assert_eq!(
        mongosh_script_prelude("mongodb://u:p@h:1/db", None),
        "db = connect(\"mongodb://u:p@h:1/db\");\n"
    );
    assert_eq!(
        mongosh_script_prelude("mongodb://u:p\"x\\@h:1/db", None),
        "db = connect(\"mongodb://u:p\\\"x\\\\@h:1/db\");\n"
    );
}

/// A conn_string names its own path database (here `frb`); a script run with another
/// database selected in the tree must run against the selected one, not the URI's.
#[test]
fn mongosh_prelude_switches_to_the_selected_database() {
    assert_eq!(
            mongosh_script_prelude("mongodb+srv://u:p@c.example.net/frb", Some("games_management")),
            "db = connect(\"mongodb+srv://u:p@c.example.net/frb\");\ndb = db.getSiblingDB(\"games_management\");\n"
        );
    assert_eq!(
        mongosh_script_prelude("mongodb://u:p@h:1/app", Some("  ")),
        "db = connect(\"mongodb://u:p@h:1/app\");\n"
    );
    assert_eq!(
        mongosh_script_prelude("mongodb://u:p@h:1/app", Some("we\"ird")),
        "db = connect(\"mongodb://u:p@h:1/app\");\ndb = db.getSiblingDB(\"we\\\"ird\");\n"
    );
}

#[test]
fn binary_uuid_subtype_renders_uuid_string() {
    use mongodb::bson::spec::BinarySubtype;
    let uuid = BsonUuid::new();
    let bin = mongodb::bson::Binary {
        subtype: BinarySubtype::Uuid,
        bytes: uuid.bytes().to_vec(),
    };
    assert_eq!(binary_to_string(&bin), uuid.to_string());
    // Non-UUID subtypes stay base64.
    let raw = mongodb::bson::Binary {
        subtype: BinarySubtype::Generic,
        bytes: vec![1, 2, 3],
    };
    assert_eq!(binary_to_string(&raw), base64_encode(&[1, 2, 3]));
    // A malformed 15-byte "uuid" must not panic — falls back to base64.
    let bad = mongodb::bson::Binary {
        subtype: BinarySubtype::Uuid,
        bytes: vec![0; 15],
    };
    assert_eq!(binary_to_string(&bad), base64_encode(&[0; 15]));
}

// --- SQL-dialect completion routing + collection resolution ----------------

#[test]
fn sql_routing_predicate() {
    use crate::complete::sql::current_statement;
    // SQL statements route to the SQL completion path…
    assert!(mongo_sql::looks_like_sql(current_statement(
        "SELECT * FROM customers WHERE "
    )));
    assert!(mongo_sql::looks_like_sql(current_statement(
        "db.x.find({}); SELECT * FROM c WHERE "
    )));
    // …native Mongo shorthand / JSON commands do NOT.
    assert!(!mongo_sql::looks_like_sql(current_statement(
        "db.customers.find({ "
    )));
    assert!(!mongo_sql::looks_like_sql(current_statement(
        "db.customers.aggregate([{ $match: { "
    )));
}

#[test]
fn resolve_collection_unqualified_uses_base() {
    let sctx = crate::complete::sql::analyze("SELECT * FROM customers WHERE ", "");
    assert_eq!(
        resolve_sql_collection(&sctx, None).as_deref(),
        Some("customers")
    );
}

#[test]
fn resolve_collection_alias_qualifier() {
    let sctx = crate::complete::sql::analyze("SELECT * FROM orders o WHERE o.", "");
    assert_eq!(
        resolve_sql_collection(&sctx, None).as_deref(),
        Some("orders")
    );
}

#[test]
fn resolve_collection_embedded_path_falls_back_to_base() {
    // `WHERE address.` — `address` is NOT a table; it's an embedded field path
    // on the base collection, so completion targets the base collection.
    let sctx = crate::complete::sql::analyze("SELECT * FROM profiles WHERE address.", "");
    assert_eq!(
        resolve_sql_collection(&sctx, None).as_deref(),
        Some("profiles")
    );
}

#[test]
fn resolve_collection_no_from_uses_tree_node() {
    let sctx = crate::complete::sql::analyze("WHERE ", "");
    assert_eq!(
        resolve_sql_collection(&sctx, Some("customers")).as_deref(),
        Some("customers")
    );
}

#[test]
fn shorthand_find_basic() {
    let p = parse_command("db.customers.find({})").unwrap();
    assert_eq!(p.collection, "customers");
    assert_eq!(p.op, MongoOp::Find);
}

#[test]
fn shorthand_find_with_filter_limit_sort() {
    let p =
        parse_command(r#"db.orders.find({"status":"paid"}).sort({"total":-1}).limit(5)"#).unwrap();
    assert_eq!(p.collection, "orders");
    assert_eq!(p.op, MongoOp::Find);
    assert_eq!(p.limit, Some(5));
    assert_eq!(p.filter.unwrap().get_str("status").unwrap(), "paid");
    // serde_json integers become BSON Int64.
    assert_eq!(p.sort.unwrap().get_i64("total").unwrap(), -1);
}

#[test]
fn shorthand_aggregate() {
    let p = parse_command(r#"db.events.aggregate([{"$match":{"k":1}},{"$count":"n"}])"#).unwrap();
    assert_eq!(p.collection, "events");
    assert_eq!(p.op, MongoOp::Aggregate);
    assert_eq!(p.pipeline.unwrap().len(), 2);
}

#[test]
fn shorthand_count() {
    let p = parse_command(r#"db.users.countDocuments({"active":true})"#).unwrap();
    assert_eq!(p.collection, "users");
    assert_eq!(p.op, MongoOp::Count);
}

#[test]
fn shorthand_replace_one() {
    // The grid's JSON-view document editor emits replaceOne by `_id`.
    let p = parse_command(
            r#"db.orders.replaceOne({"_id": {"$oid": "6725e2532c39b0477e55d679"}}, {"status": "paid", "items": [{"qty": 5}]})"#,
        )
        .unwrap();
    assert_eq!(p.collection, "orders");
    assert_eq!(p.op, MongoOp::ReplaceOne);
    assert!(p.filter.unwrap().contains_key("_id"));
    let replacement = p.update.unwrap();
    assert_eq!(replacement.get_str("status").unwrap(), "paid");
    assert!(replacement.contains_key("items"));
}

/// `\"` inside a string literal must not miscount bracket depth or split
/// args — `find({k:"a\"b)"})` is valid input.
#[test]
fn shorthand_honours_escaped_quotes_in_strings() {
    let p = parse_command(r#"db.c.find({k:"a\"b)"})"#).unwrap();
    assert_eq!(p.op, MongoOp::Find);
    assert_eq!(p.filter.unwrap().get_str("k").unwrap(), "a\"b)");
    // And in a two-arg call the top-level comma split ignores an escaped
    // quote followed by a comma inside the string.
    let p = parse_command(r#"db.c.find({k:"a\",b"}, {k:1})"#).unwrap();
    assert_eq!(p.filter.unwrap().get_str("k").unwrap(), "a\",b");
    assert!(p.projection.is_some());
}

#[test]
fn splits_multi_statement_paste_with_comments() {
    let src = r#"
            // header comment
            db.dashboards.deleteOne({ dashboardId: "x" });
            db.dashboards.insertOne({ dashboardId: "x", note: "a; b inside a string" });
        "#;
    let stmts = split_statements(src);
    assert_eq!(stmts.len(), 2);
    assert!(stmts[0].starts_with("db.dashboards.deleteOne"));
    assert!(stmts[1].starts_with("db.dashboards.insertOne"));
}

#[test]
fn single_statement_without_semicolon_is_one() {
    assert_eq!(split_statements("db.players.find({})").len(), 1);
    // Comment-only / blank input yields no statements.
    assert!(split_statements("// just a comment\n").is_empty());
}

#[test]
fn parses_mongosh_insert_with_date_and_backtick() {
    // The exact troublesome shape: unquoted keys, new Date(), a backtick SQL
    // template with parens + single quotes, nested array, trailing commas.
    let p = parse_command(
        "db.dashboards.insertOne({ dashboardId: \"player-activities\", createdAt: new Date(), \
             widgets: [{ id: \"w1\", query: `SELECT a FROM t WHERE x IN ('A','B')` },], })",
    )
    .unwrap();
    assert_eq!(p.collection, "dashboards");
    assert_eq!(p.op, MongoOp::InsertOne);
    let docs = p.documents.unwrap();
    assert_eq!(docs.len(), 1);
    let doc = &docs[0];
    assert_eq!(doc.get_str("dashboardId").unwrap(), "player-activities");
    // new Date() decoded to a real BSON DateTime, not a string/object.
    assert!(matches!(doc.get("createdAt"), Some(Bson::DateTime(_))));
    // Backtick SQL preserved verbatim inside the nested widget.
    let w0 = doc.get_array("widgets").unwrap()[0].as_document().unwrap();
    assert!(w0.get_str("query").unwrap().contains("IN ('A','B')"));
}

#[test]
fn json_command_form() {
    let p = parse_command(
        r#"{"collection":"products","op":"find","filter":{"price":{"$gt":10}},"limit":3}"#,
    )
    .unwrap();
    assert_eq!(p.collection, "products");
    assert_eq!(p.op, MongoOp::Find);
    assert_eq!(p.limit, Some(3));
}

#[test]
fn json_command_pipeline_defaults_to_aggregate() {
    let p = parse_command(r#"{"collection":"e","pipeline":[{"$count":"n"}]}"#).unwrap();
    assert_eq!(p.op, MongoOp::Aggregate);
    assert_eq!(p.pipeline.unwrap().len(), 1);
}

#[test]
fn bson_to_json_simple_doc() {
    let oid = mongodb::bson::oid::ObjectId::new();
    let doc = doc! {
        "_id": oid,
        "email": "a@b.com",
        "n": 42i32,
        "active": true,
        "tags": ["x", "y"],
    };
    let v = bson_to_json(&Bson::Document(doc));
    let obj = v.as_object().unwrap();
    assert_eq!(obj.get("_id").unwrap(), &Value::String(oid.to_hex()));
    assert_eq!(obj.get("email").unwrap(), "a@b.com");
    assert_eq!(obj.get("n").unwrap(), &json!(42));
    assert_eq!(obj.get("active").unwrap(), &Value::Bool(true));
    assert_eq!(obj.get("tags").unwrap(), &json!(["x", "y"]));
}

#[test]
fn structure_sample_is_bounded_by_bytes_not_document_count() {
    // Small documents: the flat sample is already cheap, keep it.
    assert_eq!(structure_sample_size(Some(1_024)), SAMPLE_SIZE);
    assert_eq!(structure_sample_size(Some(20_480)), SAMPLE_SIZE);
    // Fat documents (the `lobby_format_history` case, ~370KB): a flat 100
    // would pull ~37MB per pass. Bound it by the byte budget instead.
    let n = structure_sample_size(Some(370_000));
    assert!(n < SAMPLE_SIZE, "expected a reduced sample, got {n}");
    assert!(
        n * 370_000 <= STRUCTURE_SAMPLE_BYTES,
        "over budget: {n} docs"
    );
    // Never collapse to nothing — a heterogeneous collection still needs a
    // few documents to show its shape, even when each one is enormous.
    assert_eq!(
        structure_sample_size(Some(50_000_000)),
        STRUCTURE_SAMPLE_MIN
    );
    // Unknown / nonsense stats fall back to the old behaviour.
    assert_eq!(structure_sample_size(None), SAMPLE_SIZE);
    assert_eq!(structure_sample_size(Some(0)), SAMPLE_SIZE);
}

#[test]
fn bson_to_json_typed_preserves_oid_and_date() {
    let oid = mongodb::bson::oid::ObjectId::new();
    let dt = mongodb::bson::DateTime::from_millis(1_565_191_869_123);
    let doc = doc! {
        "_id": oid,
        "createdDate": dt,
        "name": "x",
        "nested": { "innerDate": dt },
    };
    let v = bson_to_json_typed(&Bson::Document(doc));
    let obj = v.as_object().unwrap();
    // ObjectId → {"$oid": hex}; the UI renders this as ObjectId("…").
    assert_eq!(obj.get("_id").unwrap(), &json!({ "$oid": oid.to_hex() }));
    // DateTime → {"$date": iso}; nested dates are typed too (recursive).
    assert!(obj.get("createdDate").unwrap().get("$date").is_some());
    assert!(obj
        .get("nested")
        .unwrap()
        .get("innerDate")
        .unwrap()
        .get("$date")
        .is_some());
    // Plain scalars are unchanged.
    assert_eq!(obj.get("name").unwrap(), "x");
}

// ---- server-side cancel helpers -----------------------------------------

#[test]
fn mongo_comment_tag_is_namespaced_by_query_id() {
    assert_eq!(mongo_comment_tag("q-42"), "otto:q-42");
    assert_eq!(mongo_comment_tag(""), "otto:");
}

#[test]
fn current_op_pipeline_matches_comment_and_projects_opid() {
    let p = current_op_pipeline("otto:q-42");
    assert_eq!(
        p,
        vec![
            doc! { "$currentOp": { "allUsers": true, "localOps": true } },
            doc! { "$match": { "command.comment": "otto:q-42" } },
            doc! { "$project": { "opid": 1 } },
        ]
    );
    // The unprivileged retry differs ONLY in `allUsers`.
    let scoped = current_op_pipeline_scoped("otto:q-42", false);
    assert_eq!(
        scoped[0],
        doc! { "$currentOp": { "allUsers": false, "localOps": true } }
    );
    assert_eq!(scoped[1..], p[1..]);
}

#[test]
fn opids_from_current_op_keeps_raw_bson_and_skips_rows_without_one() {
    // mongod reports a number, mongos a "shard:n" string — both go back to
    // killOp verbatim; a row without `opid` is ignored.
    let docs = vec![
        doc! { "opid": 1234 },
        doc! { "opid": "shard01:987" },
        doc! { "desc": "conn12" },
    ];
    assert_eq!(
        opids_from_current_op(&docs),
        vec![Bson::Int32(1234), Bson::String("shard01:987".into())]
    );
    assert!(opids_from_current_op(&[]).is_empty());
}

#[test]
fn is_unauthorized_matches_privilege_refusals_only() {
    assert!(is_unauthorized(&"Command failed: Unauthorized"));
    assert!(is_unauthorized(
            &"not authorized on admin to execute command { aggregate: 1, pipeline: [ { $currentOp: … } ] }"
        ));
    assert!(!is_unauthorized(&"connection reset by peer"));
}

// ---- keyset pagination --------------------------------------------------

#[test]
fn keyset_filter_rejects_explicit_limit_other_sort_and_id_filter() {
    // An explicit `.limit(n)` is never paged (neither by offset nor keyset).
    assert!(keyset_filter(None, None, true, None).unwrap().is_none());
    // A sort on another field (or descending `_id`) needs the offset path.
    assert!(keyset_filter(None, Some(&doc! { "age": -1 }), false, None)
        .unwrap()
        .is_none());
    assert!(keyset_filter(None, Some(&doc! { "_id": -1 }), false, None)
        .unwrap()
        .is_none());
    assert!(
        keyset_filter(None, Some(&doc! { "_id": 1, "age": 1 }), false, None)
            .unwrap()
            .is_none()
    );
    // A filter already pinning `_id` keeps the order the user asked for.
    let f = doc! { "_id": { "$in": [1, 2] } };
    assert!(keyset_filter(Some(&f), None, false, None)
        .unwrap()
        .is_none());
}

#[test]
fn keyset_filter_forces_id_sort_on_page_one() {
    // Eligible without a cursor: the filter is untouched, the sort becomes
    // `{_id: 1}` — page 1 included, so every page shares one order.
    let f = doc! { "country": "US" };
    let (filter, sort) = keyset_filter(Some(&f), None, false, None).unwrap().unwrap();
    assert_eq!(filter, f);
    assert_eq!(sort, doc! { "_id": 1 });
    // No filter at all ⇒ an empty filter document.
    let (filter, sort) = keyset_filter(None, None, false, None).unwrap().unwrap();
    assert_eq!(filter, doc! {});
    assert_eq!(sort, doc! { "_id": 1 });
    // An explicit `{_id: 1}` sort is eligible too (any numeric 1).
    assert!(
        keyset_filter(None, Some(&doc! { "_id": 1_i64 }), false, None)
            .unwrap()
            .is_some()
    );
    assert!(keyset_filter(None, Some(&doc! { "_id": 1.0 }), false, None)
        .unwrap()
        .is_some());
}

#[test]
fn keyset_filter_with_cursor_ands_a_typed_gt_on_id() {
    let oid = mongodb::bson::oid::ObjectId::new();
    let f = doc! { "$or": [{ "a": 1 }, { "b": 2 }] };
    let cursor = json!({ "$oid": oid.to_hex() });
    let (filter, sort) = keyset_filter(Some(&f), None, false, Some(&cursor))
        .unwrap()
        .unwrap();
    // `$and` keeps a top-level `$or` intact, and the cursor is decoded to a
    // real ObjectId (not compared as a string / sub-document).
    assert_eq!(filter, doc! { "$and": [f, { "_id": { "$gt": oid } }] });
    assert_eq!(sort, doc! { "_id": 1 });
    // No filter + cursor ⇒ `$and: [{}, …]` (harmless, and keeps one shape).
    let cursor = json!(41);
    let (filter, _) = keyset_filter(None, None, false, Some(&cursor))
        .unwrap()
        .unwrap();
    assert_eq!(filter, doc! { "$and": [{}, { "_id": { "$gt": 41_i64 } }] });
    // A cursor that cannot be decoded is the caller's error, not a silent skip.
    let bad = json!({ "$oid": "nope" });
    assert!(keyset_filter(None, None, false, Some(&bad)).is_err());
}

#[test]
fn last_row_id_reads_the_id_column_of_the_last_row() {
    let started = Instant::now();
    let a = mongodb::bson::oid::ObjectId::new();
    let b = mongodb::bson::oid::ObjectId::new();
    let r = docs_to_result(
        vec![doc! { "_id": a, "n": 1 }, doc! { "n": 2, "_id": b }],
        true,
        started,
    );
    // `_id` is pinned to column 0 whatever the document order was.
    assert_eq!(last_row_id(&r), Some(json!({ "$oid": b.to_hex() })));
    // No `_id` column (projected away) / no rows ⇒ no cursor.
    let r = docs_to_result(vec![doc! { "n": 1 }], true, started);
    assert!(last_row_id(&r).is_none());
    assert!(last_row_id(&QueryResult::empty()).is_none());
}

// ---- type fidelity ------------------------------------------------------

#[test]
fn bson_to_json_typed_gates_int64_at_2_pow_53() {
    // Within ±2^53 a Long is exact in a JS number — keep it plain.
    assert_eq!(bson_to_json_typed(&Bson::Int64(42)), json!(42));
    assert_eq!(
        bson_to_json_typed(&Bson::Int64(1 << 53)),
        json!(9007199254740992_i64)
    );
    assert_eq!(
        bson_to_json_typed(&Bson::Int64(-(1 << 53))),
        json!(-9007199254740992_i64)
    );
    // Beyond it the digits would round in the webview — sentinel, both signs.
    assert_eq!(
        bson_to_json_typed(&Bson::Int64((1 << 53) + 1)),
        json!({ "$numberLong": "9007199254740993" })
    );
    assert_eq!(
        bson_to_json_typed(&Bson::Int64(i64::MIN)),
        json!({ "$numberLong": "-9223372036854775808" })
    );
    // …and it decodes back to the same Int64.
    let back = json_to_bson(&json!({ "$numberLong": "9007199254740993" })).unwrap();
    assert_eq!(back, Bson::Int64(9007199254740993));
    // Int32 is never a sentinel.
    assert_eq!(bson_to_json_typed(&Bson::Int32(7)), json!(7));
}

#[test]
fn bson_to_json_typed_binary_uuid_vs_generic() {
    let uuid = BsonUuid::new();
    let v = bson_to_json_typed(&Bson::Binary(BsonBinary::from(uuid)));
    assert_eq!(v, json!({ "$uuid": uuid.to_string() }));
    // Legacy UUID subtype (3) renders the same way.
    let legacy = BsonBinary {
        subtype: BinarySubtype::UuidOld,
        bytes: uuid.bytes().to_vec(),
    };
    assert_eq!(
        bson_to_json_typed(&Bson::Binary(legacy)),
        json!({ "$uuid": uuid.to_string() })
    );
    // Generic bytes → canonical `$binary` with a 2-hex lowercase subtype.
    let raw = BsonBinary {
        subtype: BinarySubtype::Generic,
        bytes: b"hello".to_vec(),
    };
    assert_eq!(
        bson_to_json_typed(&Bson::Binary(raw)),
        json!({ "$binary": { "base64": "aGVsbG8=", "subType": "00" } })
    );
    let user = BsonBinary {
        subtype: BinarySubtype::UserDefined(0x80),
        bytes: vec![1, 2],
    };
    assert_eq!(
        bson_to_json_typed(&Bson::Binary(user))["$binary"]["subType"],
        json!("80")
    );
}

#[test]
fn binary_and_uuid_round_trip_through_typed_json() {
    let raw = Bson::Binary(BsonBinary {
        subtype: BinarySubtype::Md5,
        bytes: vec![0, 255, 16],
    });
    assert_eq!(json_to_bson(&bson_to_json_typed(&raw)).unwrap(), raw);
    let uuid = Bson::Binary(BsonBinary::from(BsonUuid::new()));
    assert_eq!(json_to_bson(&bson_to_json_typed(&uuid)).unwrap(), uuid);
    // A missing subType defaults to generic; a bad one is an error.
    assert_eq!(
        json_to_bson(&json!({ "$binary": { "base64": "AQI=" } })).unwrap(),
        Bson::Binary(BsonBinary {
            subtype: BinarySubtype::Generic,
            bytes: vec![1, 2]
        })
    );
    assert!(json_to_bson(&json!({ "$binary": { "base64": "AQI=", "subType": "zz" } })).is_err());
    assert!(json_to_bson(&json!({ "$binary": { "base64": "not base64!" } })).is_err());
}

#[test]
fn timestamp_round_trips_through_typed_json() {
    let ts = Bson::Timestamp(BsonTimestamp {
        time: 1_700_000_000,
        increment: 7,
    });
    let v = bson_to_json_typed(&ts);
    assert_eq!(
        v,
        json!({ "$timestamp": { "t": 1_700_000_000_u32, "i": 7 } })
    );
    assert_eq!(json_to_bson(&v).unwrap(), ts);
    // Nested inside a document/array it is typed and decoded the same way.
    let doc = doc! { "ops": [ts.clone()] };
    let back = json_to_bson(&bson_to_json_typed(&Bson::Document(doc.clone()))).unwrap();
    assert_eq!(back, Bson::Document(doc));
    // Out-of-range / missing fields are errors.
    assert!(json_to_bson(&json!({ "$timestamp": { "t": 1 } })).is_err());
    assert!(json_to_bson(&json!({ "$timestamp": { "t": 1, "i": 4294967296_u64 } })).is_err());
}

#[test]
fn parses_update_one_with_set_unset_rename_and_typed_sentinels() {
    let p = parse_command(
            r#"db.c.updateOne({_id:{"$oid":"5f1d7f3e2c4b1a0001234567"}}, {"$set":{"a.b":1,"big":{"$numberLong":"9007199254740993"},"at":{"$date":"2024-01-02T03:04:05Z"}},"$unset":{"x":""},"$rename":{"o":"n"}})"#,
        )
        .unwrap();
    assert_eq!(p.collection, "c");
    assert_eq!(p.op, MongoOp::UpdateOne);
    // The `_id` filter decodes to a real ObjectId.
    let filter = p.filter.unwrap();
    assert!(matches!(filter.get("_id"), Some(Bson::ObjectId(_))));
    // All three operators survive as separate top-level keys…
    let update = p.update.unwrap();
    let set = update.get_document("$set").unwrap();
    assert_eq!(set.get("a.b"), Some(&Bson::Int64(1)));
    assert_eq!(set.get("big"), Some(&Bson::Int64(9007199254740993)));
    assert!(matches!(set.get("at"), Some(Bson::DateTime(_))));
    assert_eq!(
        update.get_document("$unset").unwrap().get_str("x").unwrap(),
        ""
    );
    assert_eq!(
        update
            .get_document("$rename")
            .unwrap()
            .get_str("o")
            .unwrap(),
        "n"
    );
}

// --- DB2-03: bulk schema-graph sample ---------------------------------

#[test]
fn graph_sample_pipeline_ships_only_names_and_types() {
    let p = graph_sample_pipeline(100);
    let stages: Vec<&str> = p
        .iter()
        .map(|s| s.keys().next().map(String::as_str).unwrap_or(""))
        .collect();
    assert_eq!(
        stages,
        ["$sample", "$project", "$unwind", "$group", "$sort", "$limit"]
    );
    assert_eq!(p[0], doc! { "$sample": { "size": 100_i64 } });
    // The fold happens server-side: one row per field, never a document.
    let group = p[3].get_document("$group").unwrap();
    assert_eq!(group.get_str("_id").unwrap(), "$kv.k");
    assert_eq!(
        group.get_document("t").unwrap(),
        &doc! { "$addToSet": { "$type": "$kv.v" } }
    );
    assert_eq!(
        p[5].get_i64("$limit").unwrap(),
        MAX_DOC_COLUMNS as i64,
        "the field list is capped like a read's columns"
    );
}

#[test]
fn type_aliases_match_the_explorer_labels() {
    // Every `$type` alias the server can emit lands on the label
    // `bson_type_name` gives the same value.
    let cases: &[(&str, Bson)] = &[
        ("double", Bson::Double(1.0)),
        ("string", Bson::String("s".into())),
        ("object", Bson::Document(doc! {})),
        ("array", Bson::Array(vec![])),
        ("bool", Bson::Boolean(true)),
        ("null", Bson::Null),
        ("int", Bson::Int32(1)),
        ("long", Bson::Int64(1)),
        ("objectId", Bson::ObjectId(Default::default())),
        ("date", Bson::DateTime(BsonDateTime::from_millis(0))),
        ("decimal", Bson::Decimal128(Decimal128::from_bytes([0; 16]))),
        (
            "timestamp",
            Bson::Timestamp(BsonTimestamp {
                time: 0,
                increment: 0,
            }),
        ),
        (
            "binData",
            Bson::Binary(BsonBinary {
                subtype: BinarySubtype::Generic,
                bytes: vec![],
            }),
        ),
        ("minKey", Bson::MinKey),
        ("maxKey", Bson::MaxKey),
    ];
    for (alias, value) in cases {
        assert_eq!(
            mongo_type_alias_label(alias),
            bson_type_name(value),
            "{alias}"
        );
    }
}

#[test]
fn grouped_rows_become_graph_columns() {
    let rows = vec![
        doc! { "_id": "_id", "t": ["objectId"], "p": 0_i64 },
        doc! { "_id": "qty", "t": ["int", "long"], "p": 1_i64 },
        doc! { "_id": "note", "t": ["string", "null"], "p": 2_i64 },
        doc! { "_id": "gone", "t": ["null"], "p": 3_i64 },
        doc! { "_id": "tags", "t": ["array"], "p": 4_i64 },
        doc! { "t": ["string"] }, // no name → skipped
    ];
    let cols = graph_columns(&rows);
    let shape: Vec<(&str, &str, bool, bool)> = cols
        .iter()
        .map(|c| {
            (
                c.name.as_str(),
                c.data_type.as_str(),
                c.nullable,
                c.primary_key,
            )
        })
        .collect();
    assert_eq!(
        shape,
        [
            ("_id", "objectId", false, true),
            ("qty", "int32|int64", false, false),
            ("note", "string", true, false),
            ("gone", "null", true, false),
            ("tags", "array", false, false),
        ]
    );
    assert!(cols.iter().all(|c| !c.foreign_key));
}

#[test]
fn graph_columns_put_id_first() {
    let rows = vec![
        doc! { "_id": "a", "t": ["string"] },
        doc! { "_id": "_id", "t": ["int"] },
    ];
    let names: Vec<String> = graph_columns(&rows).into_iter().map(|c| c.name).collect();
    assert_eq!(names, ["_id", "a"]);
}
