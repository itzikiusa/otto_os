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
