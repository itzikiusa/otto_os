//! Request DTOs for the canvas router. Persistence structs (`CanvasScene`,
//! `CanvasSceneSummary`, `NewScene`, `SceneUpdate`) live in `otto_state::canvas`.

use serde::Deserialize;

/// Create a scene. `doc` is the full Scene JSON (opaque to Rust); when omitted
/// an empty scene document is stored.
#[derive(Debug, Deserialize)]
pub struct CreateSceneReq {
    pub title: String,
    #[serde(default)]
    pub doc: Option<serde_json::Value>,
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
    pub doc: Option<serde_json::Value>,
    #[serde(default)]
    pub thumbnail: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub section: Option<String>,
    /// Link/relink this scene to a product story.
    #[serde(default)]
    pub story_id: Option<String>,
}

/// The default empty Scene document stored when `CreateSceneReq.doc` is absent.
pub fn empty_doc(title: &str) -> serde_json::Value {
    serde_json::json!({
        "schema": 1,
        "title": title,
        "nodes": [],
        "edges": [],
        "slides": [],
        "appState": { "grid": true }
    })
}
