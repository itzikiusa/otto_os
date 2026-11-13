//! The Processes panel on a MongoDB connection (moved out of `mongodb.rs`,
//! which may not grow): the listing and Kill, recognized on the single-
//! statement path before the command parser.
//!
//! - Listing: the exact [`MONGO_CURRENT_OP`] statement — a JSON aggregate the
//!   classifier already reads as a READ, so it passes `read_only: true` and a
//!   Read-only connection — runs `$currentOp` on `admin` (`allUsers: true`,
//!   no idle connections). A login without the `inprog` privilege is refused
//!   that; it is retried for the login's own operations and the result says the
//!   list is limited.
//! - Kill: exactly `{"killOp": 1, "op": <opid>}`. The classifier cannot parse
//!   it as a read, so the service has already treated it as a write (Read-only
//!   refusal, production typed confirm, no agent confirm, history row). The
//!   opid is checked and handed to `killOp` as typed BSON, never spliced into
//!   text; a read-only execution (the MCP path) refuses it here as well.

use super::*;
use crate::service::process_list::MONGO_CURRENT_OP;

/// Longest command text a row carries (the rest is cut, marked with `…`).
const COMMAND_CHARS: usize = 2000;
/// The listing's column names, in row order (`ui/.../process-list.ts`).
const COLUMNS: [&str; 9] = [
    "opid",
    "user",
    "client",
    "ns",
    "op",
    "secs_running",
    "waiting_for_lock",
    "desc",
    "command",
];
/// The `message` of a listing limited to the login's own operations.
pub(super) const OWN_OPS_ONLY: &str = "This login may not list other users' operations \
     (it lacks the inprog privilege), so only its own are shown.";

/// [`MongoDriver::run_one`], unless `req` is one of the panel's two statements.
pub(super) async fn or_run_one(
    driver: &MongoDriver,
    cfg: &ResolvedConfig,
    req: &QueryRequest,
    comment: Option<&str>,
) -> Result<QueryResult> {
    let stmt = req.statement.trim();
    if !req.explain && stmt == MONGO_CURRENT_OP {
        return list(driver, cfg, req.max_rows.unwrap_or(500)).await;
    }
    if !req.explain {
        if let Some(op) = kill_target(stmt) {
            return kill(driver, cfg, op?).await;
        }
    }
    driver.run_one(cfg, req, comment).await
}

fn current_op_stage(all_users: bool, max_rows: usize) -> Vec<Document> {
    vec![
        doc! { "$currentOp": { "allUsers": all_users, "idleConnections": false } },
        doc! { "$limit": i64::try_from(max_rows.clamp(1, 5000)).unwrap_or(500) },
    ]
}

async fn list(driver: &MongoDriver, cfg: &ResolvedConfig, max_rows: usize) -> Result<QueryResult> {
    let started = Instant::now();
    let client = driver.connect(cfg).await?;
    let admin = client.database("admin");
    let (docs, limited) = match current_ops(&admin, current_op_stage(true, max_rows)).await {
        Ok(docs) => (docs, false),
        Err(e) if is_unauthorized(&e) => (
            current_ops(&admin, current_op_stage(false, max_rows))
                .await
                .map_err(types::upstream)?,
            true,
        ),
        Err(e) => return Err(types::upstream(e)),
    };
    let rows: Vec<Vec<Value>> = docs.iter().map(process_row).collect();
    Ok(QueryResult {
        columns: COLUMNS
            .iter()
            .map(|name| Column {
                name: (*name).to_string(),
                type_hint: None,
                binary: false,
            })
            .collect(),
        stats: QueryStats {
            duration_ms: started.elapsed().as_millis() as u64,
            row_count: rows.len(),
            bytes_read: None,
        },
        rows,
        message: limited.then(|| OWN_OPS_ONLY.to_string()),
        ..QueryResult::empty()
    })
}

/// One `$currentOp` document as a row of [`COLUMNS`]. Every field is
/// optional (an op of another kind, a mongos, a server version may omit it):
/// a missing one is `null`, never an error.
pub(super) fn process_row(d: &Document) -> Vec<Value> {
    let text = |k: &str| {
        d.get_str(k)
            .map_or(Value::Null, |s| Value::String(s.into()))
    };
    let users: Vec<String> = match d.get_array("effectiveUsers") {
        Ok(list) => list
            .iter()
            .filter_map(|u| u.as_document())
            .filter_map(|u| {
                let name = u.get_str("user").ok()?;
                Some(match u.get_str("db") {
                    Ok(db) => format!("{name}@{db}"),
                    Err(_) => name.to_string(),
                })
            })
            .collect(),
        Err(_) => Vec::new(),
    };
    // `microsecs_running` is the precise one; `secs_running` is whole seconds.
    let seconds = match d.get("microsecs_running") {
        Some(Bson::Int64(us)) => Some(*us as f64 / 1e6),
        Some(Bson::Int32(us)) => Some(f64::from(*us) / 1e6),
        _ => match d.get("secs_running") {
            Some(Bson::Int64(s)) => Some(*s as f64),
            Some(Bson::Int32(s)) => Some(f64::from(*s)),
            _ => None,
        },
    };
    let client = d.get_str("client").or_else(|_| d.get_str("client_s"));
    let command = d
        .get_document("command")
        .map(|c| bounded(&bson_to_json(&Bson::Document(c.clone())).to_string()))
        .map_or(Value::Null, Value::String);
    vec![
        d.get("opid").map_or(Value::Null, bson_to_json),
        if users.is_empty() {
            Value::Null
        } else {
            Value::String(users.join(", "))
        },
        client.map_or(Value::Null, |c| Value::String(c.into())),
        text("ns"),
        text("op"),
        seconds.map_or(Value::Null, |s| json!((s * 10.0).round() / 10.0)),
        d.get_bool("waitingForLock")
            .map_or(Value::Null, Value::Bool),
        text("desc"),
        command,
    ]
}

/// `s` cut to [`COMMAND_CHARS`] characters (on a char boundary).
fn bounded(s: &str) -> String {
    match s.char_indices().nth(COMMAND_CHARS) {
        Some((at, _)) => format!("{}…", &s[..at]),
        None => s.to_string(),
    }
}

/// The opid of a kill statement: `None` when `stmt` is not shaped as one
/// (it then takes the normal path and its usual parse error), `Some(Err)`
/// when it is but the opid is not a valid one.
pub(super) fn kill_target(stmt: &str) -> Option<Result<Bson>> {
    let value: Value = serde_json::from_str(stmt).ok()?;
    let obj = value.as_object()?;
    if obj.len() != 2 || obj.get("killOp").and_then(Value::as_i64) != Some(1) {
        return None;
    }
    let op = obj.get("op")?;
    Some(opid(op).ok_or_else(|| {
        types::invalid(
            "killOp needs the opid from the process list: a non-negative integer, or \
             \"<shard>:<integer>\" on a mongos",
        )
    }))
}

/// A valid opid as the BSON `killOp` takes: a mongod's number (Int32 when it
/// fits, as the server reports it) or a mongos's `"<shard>:<number>"` string.
fn opid(v: &Value) -> Option<Bson> {
    if let Some(n) = v.as_i64().filter(|n| *n >= 0) {
        return Some(i32::try_from(n).map_or(Bson::Int64(n), Bson::Int32));
    }
    let s = v.as_str()?;
    let (shard, n) = s.rsplit_once(':')?;
    let shard_ok = !shard.is_empty()
        && shard
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'));
    let n_ok = !n.is_empty() && n.len() <= 19 && n.bytes().all(|b| b.is_ascii_digit());
    (shard_ok && n_ok).then(|| Bson::String(s.to_string()))
}

async fn kill(driver: &MongoDriver, cfg: &ResolvedConfig, op: Bson) -> Result<QueryResult> {
    // The service classified the statement as a write before it got here; a
    // read-only execution must still never reach `killOp`.
    if cfg
        .params
        .get("__read_only_execution")
        .and_then(Value::as_bool)
        == Some(true)
    {
        return Err(otto_core::Error::Forbidden(
            "killOp is not allowed in a read-only run".into(),
        ));
    }
    let started = Instant::now();
    let client = driver.connect(cfg).await?;
    // A refusal (`Unauthorized`, no `killop` privilege) is the server's own
    // message, surfaced as-is.
    let reply = client
        .database("admin")
        .run_command(doc! { "killOp": 1, "op": op.clone() })
        .await
        .map_err(types::upstream)?;
    let info = reply.get_str("info").unwrap_or("killOp sent").to_string();
    Ok(QueryResult {
        columns: vec![Column {
            name: "result".into(),
            type_hint: None,
            binary: false,
        }],
        rows: vec![vec![Value::String(info)]],
        stats: QueryStats {
            duration_ms: started.elapsed().as_millis() as u64,
            row_count: 1,
            bytes_read: None,
        },
        message: Some(format!("killOp sent for op {}", bson_to_json(&op))),
        ..QueryResult::empty()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_recorded_current_op_document_maps_to_a_row() {
        let d = doc! {
            "type": "op",
            "opid": 4242_i32,
            "active": true,
            "secs_running": 3_i64,
            "microsecs_running": 3_456_789_i64,
            "op": "query",
            "ns": "shop.orders",
            "command": { "find": "orders", "filter": { "status": "open" }, "$db": "shop" },
            "client": "10.0.0.5:51234",
            "desc": "conn17",
            "waitingForLock": false,
            "effectiveUsers": [ { "user": "app", "db": "admin" } ],
        };
        let row = process_row(&d);
        assert_eq!(row.len(), COLUMNS.len());
        assert_eq!(row[0], json!(4242));
        assert_eq!(row[1], json!("app@admin"));
        assert_eq!(row[2], json!("10.0.0.5:51234"));
        assert_eq!(row[3], json!("shop.orders"));
        assert_eq!(row[4], json!("query"));
        assert_eq!(row[5], json!(3.5));
        assert_eq!(row[6], json!(false));
        assert_eq!(row[7], json!("conn17"));
        let cmd = row[8].as_str().expect("command text");
        assert!(cmd.contains("\"find\":\"orders\""), "{cmd}");
    }

    #[test]
    fn missing_fields_are_null_and_a_mongos_row_keeps_its_string_opid() {
        let d = doc! { "opid": "shard01:77", "client_s": "10.0.0.9:4000", "secs_running": 2_i32 };
        let row = process_row(&d);
        assert_eq!(row[0], json!("shard01:77"));
        assert_eq!(row[1], Value::Null);
        assert_eq!(row[2], json!("10.0.0.9:4000"));
        assert_eq!(row[3], Value::Null);
        assert_eq!(row[5], json!(2.0));
        assert_eq!(row[6], Value::Null);
        assert_eq!(row[8], Value::Null);
        assert!(process_row(&Document::new()).iter().all(Value::is_null));
    }

    #[test]
    fn a_long_command_is_cut_on_a_char_boundary() {
        let long = "é".repeat(COMMAND_CHARS + 10);
        let cut = bounded(&long);
        assert_eq!(cut.chars().count(), COMMAND_CHARS + 1);
        assert!(cut.ends_with('…'));
        assert_eq!(bounded("short"), "short");
    }

    #[test]
    fn kill_takes_only_a_typed_opid() {
        let ok = |s: &str| kill_target(s).expect("a kill").expect("a valid opid");
        assert_eq!(ok(r#"{"killOp":1,"op":42}"#), Bson::Int32(42));
        assert_eq!(ok(r#"{"op":42,"killOp":1}"#), Bson::Int32(42));
        assert_eq!(
            ok(r#"{"killOp":1,"op":4294967296}"#),
            Bson::Int64(4_294_967_296)
        );
        assert_eq!(
            ok(r#"{"killOp":1,"op":"shard-01:42"}"#),
            Bson::String("shard-01:42".into())
        );
        for bad in [
            r#"{"killOp":1,"op":-1}"#,
            r#"{"killOp":1,"op":1.5}"#,
            r#"{"killOp":1,"op":"42"}"#,
            r#"{"killOp":1,"op":"shard:42; x"}"#,
            r#"{"killOp":1,"op":":42"}"#,
            r#"{"killOp":1,"op":{"$gt":0}}"#,
            r#"{"killOp":1,"op":null}"#,
        ] {
            assert!(
                kill_target(bad).expect("shaped as a kill").is_err(),
                "{bad}"
            );
        }
        // Not shaped as a kill: the normal path (and its parse error) applies.
        for other in [
            r#"{"killOp":1}"#,
            r#"{"killOp":0,"op":42}"#,
            r#"{"killOp":1,"op":42,"comment":"x"}"#,
            "db.orders.find({})",
            MONGO_CURRENT_OP,
        ] {
            assert!(kill_target(other).is_none(), "{other}");
        }
    }

    #[test]
    fn the_listing_asks_for_all_users_then_falls_back_to_own() {
        assert_eq!(
            current_op_stage(true, 500)[0],
            doc! { "$currentOp": { "allUsers": true, "idleConnections": false } }
        );
        assert_eq!(
            current_op_stage(false, 500)[0],
            doc! { "$currentOp": { "allUsers": false, "idleConnections": false } }
        );
        assert_eq!(current_op_stage(true, 0)[1], doc! { "$limit": 1_i64 });
        assert_eq!(
            current_op_stage(true, 1_000_000)[1],
            doc! { "$limit": 5000_i64 }
        );
        assert!(OWN_OPS_ONLY.contains("only its own"));
    }
}
