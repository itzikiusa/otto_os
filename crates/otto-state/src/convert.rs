//! Row-conversion helpers shared by the repositories.

use chrono::{DateTime, Utc};
use otto_core::{Error, Result};

/// Parse an RFC3339 TEXT column into a UTC timestamp.
pub fn ts(s: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .map_err(|e| Error::Internal(format!("bad timestamp '{s}': {e}")))
}

/// Parse a JSON TEXT column.
pub fn json(s: &str) -> Result<serde_json::Value> {
    serde_json::from_str(s).map_err(|e| Error::Internal(format!("bad json column: {e}")))
}

/// Format a timestamp for storage.
pub fn fmt(t: DateTime<Utc>) -> String {
    t.to_rfc3339()
}

/// Map a sqlx error, translating row-not-found to NotFound.
pub fn dberr(context: &str) -> impl Fn(sqlx::Error) -> Error + '_ {
    move |e| match e {
        sqlx::Error::RowNotFound => Error::NotFound(context.to_string()),
        other => Error::Internal(format!("{context}: {other}")),
    }
}

/// Like [`dberr`], but maps a UNIQUE-constraint violation to a `Conflict` (409)
/// with `conflict_msg`, rather than burying it as an `Internal` (500). Use this
/// on inserts/renames where a duplicate key is a caller-facing condition (e.g.
/// `UNIQUE(workspace_id, name)`). Detection is specific to the sqlx
/// unique-violation code, so other database errors still surface as `Internal`.
pub fn dberr_unique<'a>(
    context: &'a str,
    conflict_msg: &'a str,
) -> impl Fn(sqlx::Error) -> Error + 'a {
    move |e| {
        if let sqlx::Error::Database(db) = &e {
            if db.is_unique_violation() {
                return Error::Conflict(conflict_msg.to_string());
            }
        }
        dberr(context)(e)
    }
}
