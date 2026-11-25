//! Live Database Watch & Component-Test Watcher (`crates/otto-ide/src/db_watch`).
//!
//! Connects code-level database access detection (SQL tables, MongoDB
//! collections, Redis key prefixes, ClickHouse tables) with saved Otto
//! connections and live local ephemeral Testcontainers / `miniredis` instances,
//! computing structured `Before -> After` row/document/key diffs across debug
//! steps and test runs.

pub mod containers;
pub mod detect;
pub mod diff;
pub mod watch;
pub use watch::WatchRegistry;

use otto_core::{Error, Id, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use self::containers::DbWatchRegistry;

/// Maximum rows/documents/keys returned in a single IDE DB Watch snapshot.
pub const MAX_DB_WATCH_LIMIT: usize = 150;

/// Database engines supported by the IDE Database Watcher (mirrors `otto-dbviewer`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IdeDbEngine {
    Mysql,
    Postgres,
    Mongodb,
    Redis,
    Clickhouse,
}

impl IdeDbEngine {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Mysql => "mysql",
            Self::Postgres => "postgres",
            Self::Mongodb => "mongodb",
            Self::Redis => "redis",
            Self::Clickhouse => "clickhouse",
        }
    }
}

/// Kind of database object referenced in source code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdeDetectedDbKind {
    Table,
    Collection,
    KeyPrefix,
}

/// A database table, collection, or Redis key prefix detected in project code.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdeDetectedDbAccess {
    pub engine: IdeDbEngine,
    pub object_name: String,
    pub kind: IdeDetectedDbKind,
    pub operations: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database_hint: Option<String>,
    pub source_file: String,
    pub line: u32,
    /// Rank input: 100 paused frame's file, 60 other stack file, 40 the paused
    /// file's folder, 30 active file, 10 anywhere else (see `db_watch::detect`).
    #[serde(default)]
    pub score: u32,
}

/// A detected object as the detect route answers it: with a target in the
/// request, `verified` says whether the name is one of its real objects.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdeDetectedDbObject {
    #[serde(flatten)]
    pub access: IdeDetectedDbAccess,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verified: Option<bool>,
}

/// A saved Otto database connection profile visible to the caller.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdeSavedDbTarget {
    pub target_kind: String,
    pub connection_id: String,
    pub name: String,
    pub engine: IdeDbEngine,
    pub host: String,
    pub port: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<String>,
    /// A loopback endpoint with no SSH tunnel (an SSH-tunnelled 127.0.0.1 is remote).
    #[serde(default)]
    pub local: bool,
}

/// Source of a live local ephemeral database target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IdeEphemeralSource {
    Testcontainers,
    Process,
    /// Any other local Docker DB container (no `org.testcontainers` label):
    /// possibly another project's, so it never outranks a saved connection.
    Container,
}

/// A live local Testcontainer or in-process `miniredis` endpoint discovered server-side.
/// Credentials stay strictly inside [`DbWatchRegistry`] and never cross the wire.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdeEphemeralDbTarget {
    pub target_kind: String,
    pub ephemeral_id: String,
    pub name: String,
    pub engine: IdeDbEngine,
    pub host: String,
    pub port: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
    pub source: IdeEphemeralSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container_image: Option<String>,
}

/// `POST /api/v1/ide/db-watch/detect` request body.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct IdeDbWatchDetectReq {
    pub root: String,
    #[serde(default)]
    pub connection_id: Option<String>,
    #[serde(default)]
    pub active_file: Option<String>,
    #[serde(default)]
    pub additional_roots: Vec<String>,
    /// With a target, each detected object gets `verified`.
    #[serde(default)]
    pub target: Option<IdeDbWatchTargetRef>,
    #[serde(default)]
    pub database: Option<String>,
    #[serde(default)]
    pub debug_session_id: Option<String>,
}

/// `POST /api/v1/ide/db-watch/detect` response body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdeDbWatchDetectResp {
    pub detected_objects: Vec<IdeDetectedDbObject>,
    pub saved_connections: Vec<IdeSavedDbTarget>,
    pub ephemeral_targets: Vec<IdeEphemeralDbTarget>,
}

/// Tabular snapshot payload captured from a database target.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct IdeDbWatchSnapshotItem {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
}

/// Single changed column/field within a modified row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdeDbFieldChange {
    pub column: String,
    pub before: Value,
    pub after: Value,
    /// The raw values differ; the wire carries masked ones.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub masked: bool,
}

/// Kind of row-level change between two snapshots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IdeDbDiffKind {
    Added,
    Modified,
    Removed,
}

/// Structured diff row emitted by [`diff::diff_snapshots`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdeDbRowDiff {
    pub key: String,
    pub kind: IdeDbDiffKind,
    pub row: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_row: Option<Vec<Value>>,
    pub changes: Vec<IdeDbFieldChange>,
    /// A cell of `row` / `previous_row` is masked.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub masked: bool,
}

/// Summary of differences between a baseline snapshot and the latest snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdeDbWatchDiffSummary {
    pub added_count: usize,
    pub modified_count: usize,
    pub removed_count: usize,
    pub unchanged_count: usize,
    pub diffs: Vec<IdeDbRowDiff>,
}

/// Internal target resolved by `otto-ide` and handed to `IdeCtx::execute_db_watch_query`.
///
/// This, [`DbWatchQueryExec`] and [`DbWatchExecOutcome`] mirror
/// `otto_dbviewer`'s `WatchEndpoint` / `WatchExecReq` / `WatchExecOutcome`
/// field for field: the host converts them through serde.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DbWatchQueryTarget {
    Saved {
        connection_id: Id,
    },
    EphemeralLoopback {
        port: u16,
        user: Option<String>,
        password: Option<String>,
        database: Option<String>,
    },
}

/// What a [`DbWatchQueryExec`] asks the executor for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DbWatchOp {
    /// Run `statement` → `columns` / `rows` (raw, never a history row).
    #[default]
    Read,
    /// The target's watchable objects → `objects` (+ `database`).
    ListObjects,
    /// `object_name` → `primary_key` + `row_estimate`.
    Describe,
}

/// Internal query execution command handed to `IdeCtx::execute_db_watch_query`.
#[derive(Clone, Serialize, Deserialize)]
pub struct DbWatchQueryExec {
    pub target: DbWatchQueryTarget,
    pub engine: IdeDbEngine,
    pub database: Option<String>,
    pub object_name: String,
    pub statement: String,
    pub limit: usize,
    #[serde(default)]
    pub op: DbWatchOp,
}

/// Result returned by `IdeCtx::execute_db_watch_query`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DbWatchExecOutcome {
    pub target_label: String,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    pub duration_ms: u64,
    #[serde(default)]
    pub objects: Vec<String>,
    #[serde(default)]
    pub database: Option<String>,
    #[serde(default)]
    pub primary_key: Vec<String>,
    #[serde(default)]
    pub row_estimate: Option<i64>,
}

fn is_safe_db_ident(raw: &str) -> bool {
    let trimmed = raw.trim();
    !trimmed.is_empty()
        && trimmed.len() <= 128
        && trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

fn validate_ident<'a>(raw: &'a str, what: &str) -> Result<&'a str> {
    let trimmed = raw.trim();
    if !is_safe_db_ident(trimmed) {
        return Err(Error::Invalid(format!("invalid {what} '{trimmed}'")));
    }
    Ok(trimmed)
}

/// Build a safe, engine-appropriate read-only query for watching `object_name`.
pub fn build_default_watch_query(
    engine: IdeDbEngine,
    database: Option<&str>,
    object_name: &str,
    limit: usize,
) -> Result<String> {
    let capped = limit.clamp(1, MAX_DB_WATCH_LIMIT);
    match engine {
        IdeDbEngine::Mysql | IdeDbEngine::Clickhouse => {
            let obj = validate_ident(object_name, "table name")?;
            let qualified = if let Some((db_part, tbl_part)) = obj.split_once('.') {
                format!(
                    "`{}`.`{}`",
                    validate_ident(db_part, "database")?,
                    validate_ident(tbl_part, "table")?
                )
            } else if let Some(db) = database.map(str::trim).filter(|s| !s.is_empty()) {
                format!("`{}`.`{}`", validate_ident(db, "database")?, obj)
            } else {
                format!("`{obj}`")
            };
            Ok(format!("SELECT * FROM {qualified} LIMIT {capped}"))
        }
        IdeDbEngine::Postgres => {
            let obj = validate_ident(object_name, "table name")?;
            let qualified = if let Some((schema_part, tbl_part)) = obj.split_once('.') {
                format!(
                    "\"{}\".\"{}\"",
                    validate_ident(schema_part, "schema")?,
                    validate_ident(tbl_part, "table")?
                )
            } else {
                format!("\"{obj}\"")
            };
            Ok(format!("SELECT * FROM {qualified} LIMIT {capped}"))
        }
        IdeDbEngine::Mongodb => {
            let coll = validate_ident(object_name, "collection name")?;
            Ok(format!("db.{coll}.find({{}}).limit({capped})"))
        }
        IdeDbEngine::Redis => {
            let trimmed = object_name.trim();
            if trimmed.is_empty() || trimmed.len() > 160 || trimmed.chars().any(char::is_control) {
                return Err(Error::Invalid("invalid Redis key pattern".into()));
            }
            let pattern = if trimmed.contains('*') || trimmed.contains('?') {
                trimmed.replace('"', "\\\"")
            } else {
                format!("{}*", trimmed.replace('"', "\\\""))
            };
            Ok(format!("SCAN 0 MATCH \"{pattern}\" COUNT {capped}"))
        }
    }
}

// ── Watch engine wire types (`/api/v1/ide/db-watch/watches`, `…/objects`) ──

/// Which database a watch reads. `Auto` = the newest live local ephemeral
/// target of `engine` (never a saved connection).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum IdeDbWatchTargetRef {
    Saved { connection_id: String },
    Ephemeral { ephemeral_id: String },
    Auto { engine: IdeDbEngine },
}

/// `column = value` equality on a SQL table / Mongo collection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdeDbWatchFilter {
    pub column: String,
    pub value: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdeDbWatchItemSpec {
    pub object: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<IdeDbWatchFilter>,
}

/// `{debug_session_id}` or `{run_id}`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeDbWatchBind {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub debug_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdeDbWatchCompare {
    #[default]
    PreviousStop,
    Start,
}

#[derive(Debug, Clone, Deserialize)]
pub struct IdeDbWatchStartReq {
    pub root: String,
    pub target: IdeDbWatchTargetRef,
    #[serde(default)]
    pub database: Option<String>,
    pub items: Vec<IdeDbWatchItemSpec>,
    #[serde(default)]
    pub bind: Option<IdeDbWatchBind>,
    #[serde(default)]
    pub confirm_production: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct IdeDbWatchUpdateReq {
    #[serde(default)]
    pub items: Option<Vec<IdeDbWatchItemSpec>>,
    #[serde(default)]
    pub bind: Option<IdeDbWatchBind>,
    #[serde(default)]
    pub confirm_production: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct IdeDbWatchSnapshotNowReq {
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub compare: IdeDbWatchCompare,
    #[serde(default)]
    pub for_agent: bool,
}

/// `GET …/watches/{id}` query.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct IdeDbWatchReadQuery {
    #[serde(default)]
    pub compare: IdeDbWatchCompare,
    #[serde(default)]
    pub rows: Option<String>,
    #[serde(default)]
    pub for_agent: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdeDbWatchTargetInfo {
    /// `saved` | `ephemeral`.
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ephemeral_id: Option<String>,
    pub engine: IdeDbEngine,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<String>,
    pub local: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdeDbWatchItemInfo {
    pub object: String,
    pub kind: IdeDetectedDbKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<IdeDbWatchFilter>,
    pub primary_key: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row_estimate: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdeDbWatchCheckpointReason {
    Start,
    Stop,
    RunEnd,
    SessionEnd,
    Manual,
}

impl IdeDbWatchCheckpointReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Stop => "stop",
            Self::RunEnd => "run_end",
            Self::SessionEnd => "session_end",
            Self::Manual => "manual",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdeDbWatchCheckpointInfo {
    pub seq: u32,
    pub reason: IdeDbWatchCheckpointReason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    pub captured_at_ms: i64,
    pub duration_ms: u64,
    pub changed_rows: usize,
    pub changed_items: usize,
    pub failed_items: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdeDbWatch {
    pub id: String,
    pub root: String,
    pub target: IdeDbWatchTargetInfo,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
    pub items: Vec<IdeDbWatchItemInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bind: Option<IdeDbWatchBind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bound_ended_at_ms: Option<i64>,
    pub checkpoint_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest: Option<IdeDbWatchCheckpointInfo>,
    pub busy: bool,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdeDbWatchItemDiff {
    pub object: String,
    pub engine: IdeDbEngine,
    pub executed_query: String,
    pub window_note: String,
    pub window_full: bool,
    pub columns: Vec<String>,
    pub added_count: usize,
    pub modified_count: usize,
    pub removed_count: usize,
    pub unchanged_count: usize,
    pub diffs: Vec<IdeDbRowDiff>,
    pub more_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<IdeDbWatchSnapshotItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdeDbWatchReadResp {
    pub watch: IdeDbWatch,
    pub compare: IdeDbWatchCompare,
    pub from_seq: Option<u32>,
    pub to_seq: Option<u32>,
    pub items: Vec<IdeDbWatchItemDiff>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdeDbWatchListResp {
    pub watches: Vec<IdeDbWatch>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct IdeDbWatchListQuery {
    #[serde(default)]
    pub root: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct IdeDbWatchObjectsReq {
    pub root: String,
    pub target: IdeDbWatchTargetRef,
    #[serde(default)]
    pub database: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdeDbWatchObject {
    pub name: String,
    pub kind: IdeDetectedDbKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdeDbWatchObjectsResp {
    pub target: IdeDbWatchTargetInfo,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
    pub objects: Vec<IdeDbWatchObject>,
    pub truncated: bool,
}

impl IdeDbEngine {
    /// The object kind this engine watches.
    pub fn object_kind(self) -> IdeDetectedDbKind {
        match self {
            Self::Mongodb => IdeDetectedDbKind::Collection,
            Self::Redis => IdeDetectedDbKind::KeyPrefix,
            _ => IdeDetectedDbKind::Table,
        }
    }
}

/// A filter value as an SQL literal. Not a bound parameter (no driver binds
/// them): the value is a JSON number / bool / null, or a string of letters,
/// digits and `_ . : @ -` only, which no quoting rule can break out of.
fn filter_literal(value: &Value) -> Result<Option<String>> {
    Ok(match value {
        Value::Null => None,
        Value::Bool(b) => Some(if *b { "TRUE" } else { "FALSE" }.into()),
        Value::Number(n) => Some(n.to_string()),
        Value::String(s)
            if !s.is_empty()
                && s.len() <= 128
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':' | '@' | '-')) =>
        {
            Some(format!("'{s}'"))
        }
        _ => {
            return Err(Error::Invalid(
                "a row filter value is a number, true/false, null, or text of letters, digits and _ . : @ -"
                    .into(),
            ))
        }
    })
}

/// The read a watch item runs at every checkpoint: the newest
/// [`MAX_DB_WATCH_LIMIT`] rows by primary key (`ORDER BY pk DESC`), so an
/// INSERT on a big table is inside the window; no pk = no stable order.
pub fn build_watch_query(
    engine: IdeDbEngine,
    database: Option<&str>,
    object: &str,
    primary_key: &[String],
    filter: Option<&IdeDbWatchFilter>,
) -> Result<String> {
    let limit = MAX_DB_WATCH_LIMIT;
    let quote = |ident: &str| -> Result<String> {
        let ident = validate_ident(ident, "column")?;
        Ok(match engine {
            IdeDbEngine::Postgres => format!("\"{ident}\""),
            _ => format!("`{ident}`"),
        })
    };
    match engine {
        IdeDbEngine::Redis => {
            if filter.is_some() {
                return Err(Error::Invalid(
                    "a Redis item filters by its own key pattern (e.g. session:*)".into(),
                ));
            }
            build_default_watch_query(engine, database, object, limit)
        }
        IdeDbEngine::Mongodb => {
            let coll = validate_ident(object, "collection name")?;
            let mut query = serde_json::Map::new();
            if let Some(f) = filter {
                validate_ident(&f.column, "field")?;
                filter_literal(&f.value)?;
                query.insert(f.column.clone(), f.value.clone());
            }
            let sort = if primary_key.is_empty() {
                ""
            } else {
                ".sort({\"_id\": -1})"
            };
            Ok(format!(
                "db.{coll}.find({}){sort}.limit({limit})",
                Value::Object(query)
            ))
        }
        IdeDbEngine::Mysql | IdeDbEngine::Clickhouse | IdeDbEngine::Postgres => {
            // Postgres: `database` is the schema; qualify a bare table with it.
            let object = match (engine, database) {
                (IdeDbEngine::Postgres, Some(schema)) if !object.contains('.') => {
                    format!("{}.{object}", schema.trim())
                }
                _ => object.to_string(),
            };
            let base = build_default_watch_query(engine, database, &object, limit)?;
            let from = base
                .strip_prefix("SELECT * FROM ")
                .and_then(|s| s.strip_suffix(&format!(" LIMIT {limit}")))
                .unwrap_or_default()
                .to_string();
            let mut sql = match engine {
                // Server-side cap: no client deadline, so the pooled session survives.
                IdeDbEngine::Mysql => {
                    format!("SELECT /*+ MAX_EXECUTION_TIME(10000) */ * FROM {from}")
                }
                _ => format!("SELECT * FROM {from}"),
            };
            if let Some(f) = filter {
                let col = quote(&f.column)?;
                match filter_literal(&f.value)? {
                    Some(lit) => sql.push_str(&format!(" WHERE {col} = {lit}")),
                    None => sql.push_str(&format!(" WHERE {col} IS NULL")),
                }
            }
            if !primary_key.is_empty() {
                let order = primary_key
                    .iter()
                    .map(|c| quote(c).map(|q| format!("{q} DESC")))
                    .collect::<Result<Vec<_>>>()?;
                sql.push_str(&format!(" ORDER BY {}", order.join(", ")));
            }
            sql.push_str(&format!(" LIMIT {limit}"));
            Ok(sql)
        }
    }
}

#[cfg(test)]
mod query_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_watch_read_orders_by_the_primary_key_newest_first() {
        let pk = vec!["game_id".to_string(), "account_id".to_string()];
        assert_eq!(
            build_watch_query(IdeDbEngine::Mysql, Some("pr_bo_1"), "session_info", &pk, None).unwrap(),
            "SELECT /*+ MAX_EXECUTION_TIME(10000) */ * FROM `pr_bo_1`.`session_info` ORDER BY `game_id` DESC, `account_id` DESC LIMIT 150"
        );
        assert_eq!(
            build_watch_query(
                IdeDbEngine::Postgres,
                Some("public"),
                "public.users",
                &["id".into()],
                None
            )
            .unwrap(),
            "SELECT * FROM \"public\".\"users\" ORDER BY \"id\" DESC LIMIT 150"
        );
        assert_eq!(
            build_watch_query(IdeDbEngine::Clickhouse, Some("shop"), "events", &[], None).unwrap(),
            "SELECT * FROM `shop`.`events` LIMIT 150"
        );
        assert_eq!(
            build_watch_query(IdeDbEngine::Mongodb, None, "orders", &["_id".into()], None).unwrap(),
            "db.orders.find({}).sort({\"_id\": -1}).limit(150)"
        );
    }

    #[test]
    fn a_filter_is_an_equality_on_a_checked_column_and_value() {
        let f = |column: &str, value: Value| IdeDbWatchFilter {
            column: column.into(),
            value,
        };
        let q = |filter: &IdeDbWatchFilter| {
            build_watch_query(
                IdeDbEngine::Mysql,
                Some("db"),
                "t",
                &["id".into()],
                Some(filter),
            )
        };
        assert!(q(&f("player_id", json!(77)))
            .unwrap()
            .contains("WHERE `player_id` = 77 ORDER BY"));
        assert!(q(&f("status", json!("open")))
            .unwrap()
            .contains("WHERE `status` = 'open'"));
        assert!(q(&f("closed_at", Value::Null))
            .unwrap()
            .contains("WHERE `closed_at` IS NULL"));
        assert_eq!(
            build_watch_query(
                IdeDbEngine::Mongodb,
                None,
                "orders",
                &[],
                Some(&f("status", json!("paid")))
            )
            .unwrap(),
            "db.orders.find({\"status\":\"paid\"}).limit(150)"
        );
        // Nothing can break out of the literal or the identifier.
        for bad in [
            f("id", json!("1' OR '1'='1")),
            f("id", json!("1 UNION SELECT * FROM credentials --")),
            f("id", json!("x\\")),
            f("id", json!("a;b")),
            f("id", json!("a/*")),
            f("id", json!({"$where": "sleep(1)"})),
            f("id", json!([1])),
            f("id`; DROP", json!(1)),
            f("id = 1 --", json!(1)),
        ] {
            assert!(q(&bad).is_err(), "{bad:?}");
            assert!(
                build_watch_query(IdeDbEngine::Mongodb, None, "o", &[], Some(&bad)).is_err(),
                "{bad:?}"
            );
        }
        assert!(
            build_watch_query(IdeDbEngine::Redis, None, "s:", &[], Some(&f("k", json!(1))))
                .is_err()
        );
    }
}
