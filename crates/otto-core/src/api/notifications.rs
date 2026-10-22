//! Notifications.
//!
//! Split out of `api.rs` by area (re-exported from there, so every path is
//! still `otto_core::api::*`). Mirrored by `ui/src/lib/api/types/notifications.ts`.
use super::*;

// ---------------------------------------------------------------------------
// Notifications
// ---------------------------------------------------------------------------

/// `GET /api/v1/notifications/settings` and `PUT` body (all fields required on PUT).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationSettings {
    /// Notify this many days before a credential expires.
    pub expiry_threshold_days: u32,
    /// Raise native OS notifications for warn/error notices.
    pub native_enabled: bool,
    /// Emit notices for session-progress events (finished / awaiting-input / exited).
    pub session_events: bool,
    /// Also raise a native banner for "Session awaiting input" (`:waiting`,
    /// severity info) when the user is not watching that session (review
    /// A2). The UI decides "watching"; the server severity is unchanged.
    #[serde(default = "default_true")]
    pub native_on_waiting: bool,
}

impl Default for NotificationSettings {
    fn default() -> Self {
        Self {
            expiry_threshold_days: 3,
            native_enabled: true,
            session_events: true,
            native_on_waiting: true,
        }
    }
}
