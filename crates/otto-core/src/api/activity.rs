//! Agent activity (live trail + task tracker).
//!
//! Split out of `api.rs` by area (re-exported from there, so every path is
//! still `otto_core::api::*`). Mirrored by `ui/src/lib/api/types/activity.ts`.
use super::*;

// ---------------------------------------------------------------------------
// Agent activity (live trail + task tracker)
// ---------------------------------------------------------------------------

/// `POST /workspaces/{wid}/sessions/{sid}/trail` — append one trail entry.
/// `source`/`kind` are lowercase strings (default `user`/`note`).
#[derive(Debug, Clone, Deserialize)]
pub struct AppendTrailReq {
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    /// `info | warn | error` (default `info`).
    #[serde(default)]
    pub level: Option<String>,
    pub summary: String,
    #[serde(default)]
    pub detail: Option<Value>,
}

/// One task in a [`PutTasksReq`].
#[derive(Debug, Clone, Deserialize)]
pub struct TaskInput {
    #[serde(default)]
    pub ext_id: Option<String>,
    pub title: String,
    /// `pending | in_progress | completed | blocked | cancelled`.
    pub status: String,
}

/// `PUT /workspaces/{wid}/sessions/{sid}/tasks` — replace the whole task list
/// (the task tracker is provider-synced; each push is the source of truth).
#[derive(Debug, Clone, Deserialize)]
pub struct PutTasksReq {
    pub tasks: Vec<TaskInput>,
}

/// One product-analysis lens offered in the Analysis tab. A curated subset of
/// the bundled product skills — only those that emit the Findings contract
/// (generative skills like `jira-story-writer`/`rfc-writer` are excluded).
/// Returned by `GET /workspaces/{id}/product/lenses`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProductLens {
    /// Library skill name (e.g. `"po-story-overview"`).
    pub skill: String,
    /// Human label shown in the UI checkbox row.
    pub label: String,
    /// One-line description of what the lens does.
    pub description: String,
    /// Whether the lens is checked by default in the Analysis tab.
    pub default_on: bool,
}
