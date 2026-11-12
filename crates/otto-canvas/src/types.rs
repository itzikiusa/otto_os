//! Request DTOs for the canvas router. Persistence structs (`CanvasScene`,
//! `CanvasSceneSummary`, `NewScene`, `SceneUpdate`) live in `otto_state::canvas`.

use serde::Deserialize;
use serde_json::value::RawValue;

/// Create a scene. `doc` is the full Scene JSON (opaque to Rust); when omitted
/// an empty scene document is stored. It is kept as the raw JSON text
/// (`RawValue`): validated once while scanning, never built into a
/// `serde_json::Value` tree nor re-serialised (F5 — a 25 MB board used to be
/// parsed + re-stringified on every 700 ms autosave).
#[derive(Debug, Deserialize)]
pub struct CreateSceneReq {
    pub title: String,
    #[serde(default)]
    pub doc: Option<Box<RawValue>>,
    #[serde(default)]
    pub story_id: Option<String>,
    /// Which agent drives "Ask AI" for this scene (defaults to `"claude"`).
    #[serde(default)]
    pub provider: Option<String>,
    /// Optional folder path used to group scenes in the UI.
    #[serde(default)]
    pub section: Option<String>,
}

/// Partial update. Any omitted field is left unchanged.
#[derive(Debug, Deserialize)]
pub struct UpdateSceneReq {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub doc: Option<Box<RawValue>>,
    #[serde(default)]
    pub thumbnail: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub section: Option<String>,
    /// Link/relink this scene to a product story.
    #[serde(default)]
    pub story_id: Option<String>,
    /// Optimistic-concurrency guard: the scene's `updated_at` as the writer
    /// last read it (RFC 3339). When the stored row is newer the PUT answers
    /// 409 instead of silently overwriting the other writer's board.
    #[serde(default)]
    pub expect_updated_at: Option<String>,
    /// `"agent"` for an agent's write (the `canvas_update_scene` MCP tool):
    /// the pre-save snapshot is an unthrottled `agent` version. Anything else
    /// is a user save (snapshot throttled per [`otto_state::USER_SNAPSHOT_EVERY_SECS`]).
    #[serde(default)]
    pub origin: Option<String>,
}

/// The default empty Scene document stored when `CreateSceneReq.doc` is absent.
/// Uses the file-backed canvas shape (`{type:"otto-canvas", format, source}`) —
/// the only shape Canvas Studio renders; the legacy Svelte-Flow skeleton this
/// used to emit left `format`/`source` undefined for fresh scenes.
pub fn empty_doc(title: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "otto-canvas",
        "version": 1,
        "format": "mermaid",
        "title": title,
        "source": "",
    })
}

/// Raw JSON text of a request `doc` (no re-serialisation; one move).
pub fn raw_doc(doc: Box<RawValue>) -> String {
    Box::<str>::from(doc).into_string()
}
