//! API request/response DTOs for `/api/v1`.
//!
//! These types are mirrored by `ui/src/lib/api/types/*.ts`, and live in
//! `api/<area>.rs` files along the same seams; this file re-exports them all,
//! so every path stays `otto_core::api::*`. Endpoint shapes are
//! documented in `docs/contracts/api.md`; the WS protocol in `docs/contracts/ws.md`.

pub mod recap;
pub mod rooms;
pub use recap::*;
pub use rooms::*;
// Area files, split along the `ui/src/lib/api/types/*.ts` seams.
mod activity;
mod api_client;
mod auth;
mod connections;
mod context;
mod git;
mod goal_loops;
mod improve;
mod integrations;
mod notifications;
mod orchestrator;
mod problem;
mod review;
mod security;
mod sessions;
mod skills_eval;
mod usage;
pub use activity::*;
pub use api_client::*;
pub use auth::*;
pub use connections::*;
pub use context::*;
pub use git::*;
pub use goal_loops::*;
pub use improve::*;
pub use integrations::*;
pub use notifications::*;
pub use orchestrator::*;
pub use problem::*;
pub use review::*;
pub use security::*;
pub use sessions::*;
pub use skills_eval::*;
pub use usage::*;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::domain::{
    Connection, ConnectionKind, Environment, GitProviderKind, IssueProviderKind, MatrixPrompt,
    ReviewAgentCfg, ScoreWeights, Session, SessionKind, User, Workspace, WorkspaceRole,
};
use crate::Id;

/// Deserialize a present-or-absent nullable field into a double-`Option`.
/// `Some(Some(v))` ⇒ an explicit value; `Some(None)` ⇒ an explicit JSON `null`;
/// `None` ⇒ the key was absent. Pair with `#[serde(default)]` so an absent key
/// stays `None` while a present `null` becomes `Some(None)`.
fn de_double_option<'de, D, T>(de: D) -> std::result::Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Some(Option::<T>::deserialize(de)?))
}

#[cfg(test)]
mod update_issue_account_tests {
    use super::*;

    /// S17-18: `token_expires_at` is tri-state — absent keeps, `null` clears.
    #[test]
    fn token_expiry_absent_null_and_value_are_distinct() {
        let absent: UpdateIssueAccountReq = serde_json::from_str(r#"{"label":"x"}"#).unwrap();
        assert_eq!(absent.token_expires_at, None);
        let cleared: UpdateIssueAccountReq =
            serde_json::from_str(r#"{"token_expires_at":null}"#).unwrap();
        assert_eq!(cleared.token_expires_at, Some(None));
        let set: UpdateIssueAccountReq =
            serde_json::from_str(r#"{"token_expires_at":"2026-01-02T00:00:00Z"}"#).unwrap();
        assert!(matches!(set.token_expires_at, Some(Some(_))));
    }
}
