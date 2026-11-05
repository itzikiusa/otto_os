//! Agent UI control — the server-owned grant record kept in
//! `session.meta.ui_control` (a Threads thread keeps the same record on its
//! row): pure helpers shared by the bridge and the grant route.

use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::{json, Value};

/// The grant record's key in `session.meta`.
pub const META_KEY: &str = "ui_control";

/// `Some(true)` granted, `Some(false)` explicitly denied/revoked, `None` never decided.
pub fn grant_state(meta: &Value) -> Option<bool> {
    meta.get(META_KEY)?.get("enabled")?.as_bool()
}

/// When the last grant decision was made (RFC 3339), if recorded.
pub fn grant_decided_at(meta: &Value) -> Option<DateTime<Utc>> {
    meta.get(META_KEY)?
        .get("granted_at")?
        .as_str()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|t| t.with_timezone(&Utc))
}

/// A Deny/Stop within `quiet` of `now` — answer without prompting again.
pub fn recently_denied(meta: &Value, now: DateTime<Utc>, quiet: Duration) -> bool {
    grant_state(meta) == Some(false)
        && grant_decided_at(meta).is_some_and(|t| {
            now.signed_duration_since(t)
                .to_std()
                .map(|d| d < quiet)
                .unwrap_or(true)
        })
}

/// The grant record written by `POST /sessions/{id}/ui-control`.
pub fn grant_record(enabled: bool, by: &str) -> Value {
    json!({ META_KEY: {
        "enabled": enabled,
        "granted_at": Utc::now().to_rfc3339(),
        "granted_by": by,
    }})
}
