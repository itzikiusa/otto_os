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

use otto_core::{Error, Id, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use self::containers::DbWatchRegistry;

/// Maximum rows/documents/keys returned in a single IDE DB Watch snapshot.
pub const MAX_DB_WATCH_LIMIT: usize = 150;
/// Default rows/documents/keys returned when `limit` is omitted.
pub const DEFAULT_DB_WATCH_LIMIT: usize = 100;

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
}

/// `POST /api/v1/ide/db-watch/detect` response body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdeDbWatchDetectResp {
    pub detected_objects: Vec<IdeDetectedDbAccess>,
    pub saved_connections: Vec<IdeSavedDbTarget>,
    pub ephemeral_targets: Vec<IdeEphemeralDbTarget>,
}

/// Tabular snapshot payload captured from a database target.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IdeDbWatchSnapshotItem {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
}

/// `POST /api/v1/ide/db-watch/snapshot` request body.
#[derive(Debug, Clone, Deserialize)]
pub struct IdeDbWatchSnapshotReq {
    pub root: String,
    pub target_kind: String,
    #[serde(default)]
    pub connection_id: Option<String>,
    #[serde(default)]
    pub ephemeral_id: Option<String>,
    pub engine: IdeDbEngine,
    #[serde(default)]
    pub database: Option<String>,
    pub object_name: String,
    #[serde(default)]
    pub custom_query: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub previous: Option<IdeDbWatchSnapshotItem>,
}

/// Single changed column/field within a modified row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdeDbFieldChange {
    pub column: String,
    pub before: Value,
    pub after: Value,
}

/// Kind of row-level change between two snapshots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IdeDbDiffKind {
    Added,
    Modified,
    Removed,
}

/// Structured diff row emitted by [`diff::compute_snapshot_diff`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdeDbRowDiff {
    pub key: String,
    pub kind: IdeDbDiffKind,
    pub row: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_row: Option<Vec<Value>>,
    pub changes: Vec<IdeDbFieldChange>,
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

/// `POST /api/v1/ide/db-watch/snapshot` response body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdeDbWatchSnapshotResp {
    pub engine: IdeDbEngine,
    pub target_label: String,
    pub object_name: String,
    pub executed_query: String,
    pub captured_at_ms: i64,
    pub duration_ms: u64,
    pub snapshot: IdeDbWatchSnapshotItem,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff: Option<IdeDbWatchDiffSummary>,
}

/// Internal target resolved by `otto-ide` and handed to `IdeCtx::execute_db_watch_query`.
#[derive(Clone)]
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

/// Internal query execution command handed to `IdeCtx::execute_db_watch_query`.
#[derive(Clone)]
pub struct DbWatchQueryExec {
    pub target: DbWatchQueryTarget,
    pub engine: IdeDbEngine,
    pub database: Option<String>,
    pub object_name: String,
    pub statement: String,
    pub is_custom_query: bool,
    pub limit: usize,
}

/// Tabular snapshot result returned by `IdeCtx::execute_db_watch_query`.
#[derive(Debug, Clone)]
pub struct DbWatchExecOutcome {
    pub target_label: String,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    pub duration_ms: u64,
}

pub fn clamp_watch_limit(raw: Option<usize>) -> usize {
    raw.unwrap_or(DEFAULT_DB_WATCH_LIMIT)
        .clamp(1, MAX_DB_WATCH_LIMIT)
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
