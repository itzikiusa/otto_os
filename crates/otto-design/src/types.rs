//! Domain rows, request DTOs and response shapes of the design graph. Mirrored
//! in TS (`ui/src/lib/api/types.ts`, "Design Hall") and documented in
//! `docs/contracts/api.md` § Design Hall — change all three together.

use chrono::{DateTime, Utc};
use otto_core::Id;
use serde::{Deserialize, Serialize};
use serde_json::Value;

// ---------------------------------------------------------------------------
// Closed vocabularies (validated in Rust; the columns are plain TEXT so a new
// value never needs a table rebuild)
// ---------------------------------------------------------------------------

/// The seven studios of the Hall.
pub const STUDIOS: &[&str] = &[
    "frames",
    "graphics",
    "site",
    "3d",
    "whiteboard",
    "brand",
    "spatial",
];

/// Artifact review status. `approved` is only reachable through
/// `POST …/approve` (it needs an `approved_version_id`).
pub const STATUSES: &[&str] = &["draft", "review", "approved", "shipped", "archived"];

/// Link relations (proposal §3.2).
pub const RELS: &[&str] = &[
    "embeds",
    "uses_component",
    "uses_tokens",
    "describes",
    "derived_from",
    "references",
    "implements",
    "variant_of",
    "resized_from",
    "published_as",
    "exported_to",
    "created_in",
];

/// Relations whose target is RENDERED inside the source: cycle rejection and
/// the render-depth cap apply to these only.
pub const RENDER_RELS: &[&str] = &["embeds", "uses_component", "uses_tokens"];

/// What a link can point at.
pub const DST_KINDS: &[&str] = &[
    "artifact",
    "story",
    "session",
    "swarm_project",
    "vault_note",
    "pr",
    "url",
    "attachment",
    "publish",
];

/// How a consumer follows a linked artifact that changes.
pub const POLICIES: &[&str] = &["follow_approved", "follow_latest", "pinned"];

/// Why a version exists. Only `autosave` versions are ever retention
/// candidates (and only through the opt-in prune).
pub const VERSION_KINDS: &[&str] = &["autosave", "named", "agent", "import", "sync", "restore"];

/// Captured learning signals (proposal §6.2). `variant_chosen` is the
/// client-recorded pick (a tray "Apply"); `variant_accepted` is recorded by the
/// server when `POST …/variants/{v}/accept` fast-forwards main — the learner
/// treats both the same. `agent_draft` marks a version a design-assist turn
/// committed (main or a variant branch).
pub const SIGNAL_KINDS: &[&str] = &[
    "variant_chosen",
    "variant_accepted",
    "variant_rejected",
    "agent_draft",
    "edit_after_draft",
    "review_comment",
    "critique_finding",
    "a11y_fix",
    "brand_correction",
    "rule_feedback",
    "status_change",
    "shipped",
];

/// Who authored a version / emitted a signal.
pub const AUTHOR_KINDS: &[&str] = &["user", "agent", "system"];

/// Render nesting cap (A embeds B embeds C …): deeper chains are reported and
/// not rendered past this depth.
pub const MAX_RENDER_DEPTH: usize = 4;

/// Is `v` one of `set`?
pub fn one_of(set: &[&str], v: &str) -> bool {
    set.contains(&v)
}

// ---------------------------------------------------------------------------
// Rows
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesignProject {
    pub id: Id,
    pub workspace_id: Id,
    pub name: String,
    pub description: String,
    pub epic_story_id: Option<Id>,
    pub swarm_project_id: Option<Id>,
    pub brand_kit_id: Option<Id>,
    pub cover_artifact_id: Option<Id>,
    pub archived: bool,
    pub meta: Value,
    pub created_by: Id,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Non-archived artifacts filed in this project.
    pub artifact_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesignArtifact {
    pub id: Id,
    pub project_id: Option<Id>,
    pub workspace_id: Id,
    pub studio: String,
    pub format: String,
    pub mime: String,
    pub title: String,
    pub status: String,
    pub head_version_id: Option<Id>,
    /// `seq` of the head version (the "v12" badge), joined in.
    pub head_seq: Option<i64>,
    pub approved_version_id: Option<Id>,
    pub tags: Vec<String>,
    /// sha256 of the PNG thumbnail blob (`GET …/thumbnail`), if any.
    pub thumb_blob: Option<String>,
    pub meta: Value,
    /// `product_attachment` | `canvas_scene` for imported legacy rows.
    pub source_kind: Option<String>,
    pub source_id: Option<Id>,
    pub created_by: Id,
    pub created_by_kind: String,
    pub created_session_id: Option<Id>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesignVersion {
    pub id: Id,
    pub artifact_id: Id,
    pub seq: i64,
    pub parent_version_id: Option<Id>,
    pub branch: String,
    pub blob_sha256: String,
    pub size_bytes: i64,
    pub kind: String,
    pub author_kind: String,
    pub author_id: String,
    pub session_id: Option<Id>,
    pub message: String,
    pub provenance: Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesignLink {
    pub id: Id,
    pub src_artifact_id: Id,
    pub src_version_id: Option<Id>,
    pub src_node: Option<String>,
    pub dst_kind: String,
    pub dst_id: String,
    pub dst_node: Option<String>,
    pub rel: String,
    pub policy: String,
    pub pinned_version_id: Option<Id>,
    pub origin: String,
    pub broken: bool,
    pub meta: Value,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesignSignal {
    pub id: Id,
    pub workspace_id: Id,
    pub artifact_id: Id,
    pub version_id: Option<Id>,
    pub kind: String,
    pub actor_kind: String,
    pub actor_id: String,
    pub session_id: Option<Id>,
    pub payload: Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesignPublish {
    pub id: Id,
    pub artifact_id: Id,
    pub version_id: Id,
    pub target: String,
    pub url: Option<String>,
    pub pinned_set: Value,
    pub created_by: Id,
    pub created_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Requests
// ---------------------------------------------------------------------------

/// `POST /design/projects`.
#[derive(Debug, Deserialize)]
pub struct CreateProjectReq {
    pub workspace_id: Id,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub epic_story_id: Option<Id>,
    #[serde(default)]
    pub swarm_project_id: Option<Id>,
    #[serde(default)]
    pub brand_kit_id: Option<Id>,
    #[serde(default)]
    pub meta: Option<Value>,
}

/// `PATCH /design/projects/{id}` — omitted fields are unchanged; an empty
/// string clears an optional id.
#[derive(Debug, Default, Deserialize)]
pub struct UpdateProjectReq {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub epic_story_id: Option<String>,
    #[serde(default)]
    pub swarm_project_id: Option<String>,
    #[serde(default)]
    pub brand_kit_id: Option<String>,
    #[serde(default)]
    pub cover_artifact_id: Option<String>,
    #[serde(default)]
    pub archived: Option<bool>,
    /// Shallow-merged into the stored `meta` object.
    #[serde(default)]
    pub meta: Option<Value>,
}

/// `POST /design/artifacts`. Content is `content` (UTF-8 text formats) or
/// `content_b64` (any format); omitted → the format's empty document (binary
/// formats require content). `derived_from` forks an existing version.
#[derive(Debug, Deserialize)]
pub struct CreateArtifactReq {
    pub workspace_id: Id,
    #[serde(default)]
    pub project_id: Option<Id>,
    /// Defaults to the format's home studio.
    #[serde(default)]
    pub studio: Option<String>,
    pub format: String,
    pub title: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub meta: Option<Value>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub content_b64: Option<String>,
    /// Link the new artifact to this product story (`implements`).
    #[serde(default)]
    pub story_id: Option<Id>,
    /// Fork: start from this artifact's version (`derived_from`, pinned).
    #[serde(default)]
    pub derived_from: Option<ForkFrom>,
    #[serde(default)]
    pub author_kind: Option<String>,
    #[serde(default)]
    pub session_id: Option<Id>,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ForkFrom {
    pub artifact_id: Id,
    /// Version id; omitted → the source's approved version, else its head.
    #[serde(default)]
    pub version_id: Option<Id>,
}

/// `PATCH /design/artifacts/{id}` — metadata only (content goes through
/// `PUT …/content`). An empty `project_id` unfiles the artifact.
#[derive(Debug, Default, Deserialize)]
pub struct UpdateArtifactReq {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub studio: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    /// Shallow-merged into the stored `meta` object.
    #[serde(default)]
    pub meta: Option<Value>,
    /// A PNG thumbnail (base64, ≤ 2 MB) stored as a blob.
    #[serde(default)]
    pub thumb_b64: Option<String>,
}

/// `PUT /design/artifacts/{id}/content`.
#[derive(Debug, Default, Deserialize)]
pub struct ContentPutReq {
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub content_b64: Option<String>,
    /// Optimistic concurrency: the head version id the editor loaded (`""` =
    /// "I expect no version yet"). A mismatch is a **409**; omitted =
    /// unconditional save.
    #[serde(default)]
    pub base_version: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    /// `user` (default) | `agent`.
    #[serde(default)]
    pub author_kind: Option<String>,
    #[serde(default)]
    pub session_id: Option<Id>,
    /// Stored verbatim (bounded) on the version: `{refs, why, template, …}`.
    #[serde(default)]
    pub provenance: Option<Value>,
}

/// `POST /design/artifacts/{id}/versions` — a NAMED commit. With content it
/// commits those bytes; without, it snapshots the working copy (what an agent
/// edited in place), or re-labels the head when nothing changed.
#[derive(Debug, Default, Deserialize)]
pub struct CommitReq {
    pub message: String,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub content_b64: Option<String>,
    #[serde(default)]
    pub base_version: Option<String>,
    #[serde(default)]
    pub author_kind: Option<String>,
    #[serde(default)]
    pub session_id: Option<Id>,
    #[serde(default)]
    pub provenance: Option<Value>,
}

/// `POST /design/artifacts/{id}/approve`. Omitted `version_id` = head.
#[derive(Debug, Default, Deserialize)]
pub struct ApproveReq {
    #[serde(default)]
    pub version_id: Option<Id>,
}

/// `POST /design/artifacts/{id}/links` — an explicit link.
#[derive(Debug, Deserialize)]
pub struct CreateLinkReq {
    pub rel: String,
    pub dst_kind: String,
    pub dst_id: String,
    #[serde(default)]
    pub dst_node: Option<String>,
    #[serde(default)]
    pub src_node: Option<String>,
    /// Defaults: render rels → `follow_approved`; `derived_from` /
    /// `references` → `pinned`; everything else → `follow_latest`.
    #[serde(default)]
    pub policy: Option<String>,
    #[serde(default)]
    pub pinned_version_id: Option<Id>,
    #[serde(default)]
    pub meta: Option<Value>,
}

/// `POST /design/signals`.
#[derive(Debug, Deserialize)]
pub struct SignalReq {
    pub artifact_id: Id,
    pub kind: String,
    #[serde(default)]
    pub version_id: Option<Id>,
    #[serde(default)]
    pub actor_kind: Option<String>,
    #[serde(default)]
    pub session_id: Option<Id>,
    #[serde(default)]
    pub payload: Option<Value>,
}

/// `POST /design/admin/prune` — opt-in retention. Dry run unless `apply`.
#[derive(Debug, Default, Deserialize)]
pub struct PruneReq {
    #[serde(default)]
    pub artifact_id: Option<Id>,
    #[serde(default)]
    pub apply: bool,
    /// Autosave squash window (default 600 s = 10 minutes).
    #[serde(default)]
    pub window_secs: Option<i64>,
}

// ---------------------------------------------------------------------------
// Responses
// ---------------------------------------------------------------------------

/// `GET /design/artifacts/{id}`.
#[derive(Debug, Clone, Serialize)]
pub struct ArtifactDetail {
    pub artifact: DesignArtifact,
    pub head: Option<DesignVersion>,
    pub approved: Option<DesignVersion>,
    pub links_out: i64,
    pub links_in: i64,
    /// Where the editable working copy lives (text formats only) — agents edit
    /// this file in place, then `POST …/versions` snapshots it.
    pub work_path: Option<String>,
    /// Local path of the thumbnail PNG (a blob), so multimodal agents can
    /// look at what the artifact renders like.
    pub thumbnail_path: Option<String>,
    /// With `?content=true`: the UTF-8 source of the requested version (head
    /// by default, `?version=` otherwise), capped at 256 KiB. `null` for
    /// binary formats or when not requested.
    pub content: Option<String>,
    /// The version `content` was read from.
    pub content_version_id: Option<Id>,
    /// `content` was cut at the cap.
    pub content_truncated: bool,
}

/// One unresolved `otto://design/…` reference found in a document.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BrokenRef {
    pub uri: String,
    pub src_node: Option<String>,
    /// `missing_artifact` | `missing_version` | `missing_node` | `malformed`.
    pub reason: String,
}

/// What link extraction found on a save.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LinkReport {
    /// Extracted links stored (including broken ones, which carry a badge).
    pub extracted: usize,
    pub broken: Vec<BrokenRef>,
    /// Render links NOT stored because they would close a cycle.
    pub cycles: Vec<String>,
    /// Render targets whose chain exceeds the depth cap (stored; rendering
    /// stops at `MAX_RENDER_DEPTH`).
    pub depth_exceeded: Vec<String>,
}

/// Result of a content save / named commit / create.
#[derive(Debug, Clone, Serialize)]
pub struct SaveResult {
    pub artifact: DesignArtifact,
    pub version: DesignVersion,
    /// `false` when the content was byte-identical to the head (no new
    /// version was written; `version` is the unchanged head).
    pub created: bool,
    pub links: LinkReport,
}

/// `GET /design/artifacts/{id}/links`.
#[derive(Debug, Clone, Serialize)]
pub struct LinksResp {
    pub links: Vec<DesignLink>,
    /// Every artifact on the other end of `links` (for titles/badges).
    pub artifacts: Vec<DesignArtifact>,
}

/// One `GET /design/search` hit.
#[derive(Debug, Clone, Serialize)]
pub struct SearchHit {
    pub artifact: DesignArtifact,
    /// A short excerpt around the match (empty for filter-only searches).
    pub snippet: String,
    /// Higher is better.
    pub score: f64,
    /// How many OTHER artifacts reference / derive from / embed this one.
    pub reference_count: i64,
    /// Product stories this artifact `implements`.
    pub story_ids: Vec<Id>,
}

/// `POST /design/admin/import` (and the startup run's log line).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImportReport {
    pub attachments_scanned: usize,
    pub scenes_scanned: usize,
    pub created: usize,
    pub synced: usize,
    pub unchanged: usize,
    pub skipped: usize,
    pub links_created: usize,
    pub errors: Vec<String>,
}

/// `POST /design/admin/prune`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PruneReport {
    pub applied: bool,
    pub artifacts_scanned: usize,
    /// Version ids that are (or, on a dry run, would be) removed.
    pub versions: Vec<Id>,
    /// Blobs no longer referenced by any version/thumbnail (removed on apply).
    pub blobs: Vec<String>,
}
