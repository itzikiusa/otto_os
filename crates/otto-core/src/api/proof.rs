//! Proof Packs; Proof Packs v2 — snapshots, evidence capture, CI, PR-check, reports, config.
//!
//! Split out of `api.rs` by area (re-exported from there, so every path is
//! still `otto_core::api::*`). Mirrored by `ui/src/lib/api/types/proof.ts`.
use super::*;

// ---------------------------------------------------------------------------
// Proof Packs
// ---------------------------------------------------------------------------

/// A proof pack with its derived badge list (snake_case strings) and artifact
/// count — the list/summary row shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofPackResp {
    #[serde(flatten)]
    pub pack: crate::proof::ProofPack,
    /// Derived badges as snake_case strings (see `ProofBadge`).
    pub badges: Vec<String>,
    pub artifact_count: u32,
}

/// What a diff assembly did (error-handling-6-02). `NotARepo` and
/// `DiffFailed` write no artifact; a HEAD that moves mid-assembly is a 409.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum DiffAssembly {
    /// A `diff` artifact was written (or refreshed).
    Assembled { files_changed: usize },
    /// The working tree has no changes vs the base: nothing to record.
    NoChanges,
    /// `cwd` is not a git repository (no `HEAD`).
    NotARepo,
    /// A git command failed; each entry is `"<command>: <error>"`.
    DiffFailed { commands: Vec<String> },
}

/// One requested command of an assemble call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssembledCommand {
    pub cmd: String,
    /// `passed` / `failed` when its artifact was stored; absent on `error`.
    pub status: Option<crate::proof::ProofArtifactStatus>,
    /// Why the command's artifact could not be stored.
    pub error: Option<String>,
}

/// What `POST /proof-packs/{id}/assemble` did, beside the recomputed pack.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssembleSummary {
    /// Absent when the request had no `cwd` (nothing to assemble).
    pub diff: Option<DiffAssembly>,
    pub commands: Vec<AssembledCommand>,
}

/// `POST /proof-packs/{id}/assemble` — the `ProofPackResp` fields plus
/// `assembled`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssembleResp {
    #[serde(flatten)]
    pub pack: ProofPackResp,
    pub assembled: AssembleSummary,
}

/// One artifact plus a capped inline preview for list/detail rendering.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofArtifactView {
    #[serde(flatten)]
    pub artifact: crate::proof::ProofArtifact,
    /// Capped preview (`PREVIEW_CAP`) of inline content; absent for url/file refs.
    pub preview: Option<String>,
    pub truncated: bool,
}

/// `GET /proof-packs/{id}` — the pack, its badges, artifacts, and any children.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofPackDetailResp {
    pub pack: crate::proof::ProofPack,
    pub badges: Vec<String>,
    pub artifacts: Vec<ProofArtifactView>,
    pub children: Vec<ProofPackResp>,
    /// The done-contract, recomputed LIVE so the meter is accurate even before
    /// the pack's next recompute.
    pub done_contract: crate::proof::DoneContract,
    /// Immutable snapshots taken of this pack (metadata only; newest first).
    #[serde(default)]
    pub snapshots: Vec<ProofSnapshotMeta>,
}

/// `POST /workspaces/{id}/proof-packs`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateProofPackReq {
    /// `session | goal_loop | review | workflow_run | task | manual`.
    pub work_item_kind: String,
    pub work_item_id: String,
    pub title: Option<String>,
    pub parent_pack_id: Option<String>,
    /// Optionally link the pack to a registered repo, so that repo's proof policy
    /// (`RepoProofConfig` — "test command required", "CI green required", …)
    /// applies on recompute. Strengthen-only: linking can never *relax* a pack's
    /// requirements, only add to them. Ignored if the repo can't be resolved.
    #[serde(default)]
    pub repo_id: Option<String>,
}

/// `POST /proof-packs/{id}/artifacts`. Provide at most one of `content`/`content_url`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddArtifactReq {
    /// One of the `ProofArtifactKind` snake_case strings.
    pub kind: String,
    pub title: String,
    /// Inline text content (redacted + capped on store).
    pub content: Option<String>,
    /// An external URL (CI build, screenshot) — stored as `ref_kind=url`.
    pub content_url: Option<String>,
    /// One of the `ProofArtifactStatus` strings; defaults to `info`.
    pub status: Option<String>,
    pub metadata: Option<Value>,
}

/// One command to run during `/assemble`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssembleCmd {
    pub cmd: String,
    /// `test | build | lint`; inferred if omitted.
    pub kind: Option<String>,
}

/// `POST /proof-packs/{id}/assemble` — re-run auto-assembly then recompute.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssembleReq {
    /// Working directory to assemble a diff from (and run commands in).
    pub cwd: Option<String>,
    /// Optional base ref for the diff (defaults to the working tree vs HEAD).
    pub base: Option<String>,
    /// Optional commands to run and capture as `command` artifacts.
    pub commands: Option<Vec<AssembleCmd>>,
}

/// `POST /proof-packs/{id}/waive`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WaiveReq {
    pub reason: String,
}

/// One row of `GET /workspaces/{id}/proof-summary` — cheap badge lookup keyed by
/// work item (the UI maps `"<kind>:<work_item_id>"` -> this).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofSummaryRow {
    pub work_item_kind: String,
    pub work_item_id: String,
    pub proof_pack_id: String,
    pub status: String,
    pub risk_score: u8,
    #[serde(default)]
    pub done_score: u8,
    pub badges: Vec<String>,
}

/// `GET /workspaces/{id}/proof-summary`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofSummaryResp {
    pub rows: Vec<ProofSummaryRow>,
}

// ---------------------------------------------------------------------------
// Proof Packs v2 — snapshots, evidence capture, CI, PR-check, reports, config
// ---------------------------------------------------------------------------

/// Snapshot metadata (no bundle/reports) — list rows + detail embed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofSnapshotMeta {
    pub id: String,
    pub proof_pack_id: String,
    pub seq: i64,
    pub sha256: String,
    pub status: String,
    pub done_score: u8,
    pub risk_score: u8,
    pub note: String,
    pub created_by: String,
    pub created_at: String,
}

/// A full immutable snapshot — `GET /proof-snapshots/{id}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofSnapshotResp {
    #[serde(flatten)]
    pub meta: ProofSnapshotMeta,
    /// The frozen, canonical evidence bundle (pack + capped artifacts + contract).
    pub bundle: Value,
    pub report_md: String,
    pub report_html: String,
}

/// `POST /proof-packs/{id}/snapshot`
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CreateSnapshotReq {
    #[serde(default)]
    pub note: Option<String>,
}

/// `POST /proof-packs/{id}/media` — attach screenshot/video evidence (base64).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachMediaReq {
    /// `screenshot | video`.
    pub kind: String,
    pub title: String,
    /// MIME type (validated against an allow-list).
    pub mime: String,
    /// Base64-encoded bytes (capped at `MEDIA_CAP`).
    pub data_base64: String,
    #[serde(default)]
    pub metadata: Option<Value>,
}

/// `POST /proof-packs/{id}/evidence/api` — record an HTTP request/response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiEvidenceReq {
    pub title: String,
    pub method: String,
    pub url: String,
    pub status: u16,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub request: Option<String>,
    #[serde(default)]
    pub response: Option<String>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

/// `POST /proof-packs/{id}/evidence/db` — record a DB read result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbEvidenceReq {
    pub title: String,
    #[serde(default)]
    pub engine: Option<String>,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub columns: Option<Vec<String>>,
    #[serde(default)]
    pub row_count: Option<u64>,
    #[serde(default)]
    pub sample: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

/// `POST /proof-packs/{id}/evidence/kafka` — record a Kafka read result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KafkaEvidenceReq {
    pub title: String,
    pub topic: String,
    #[serde(default)]
    pub message_count: Option<u64>,
    #[serde(default)]
    pub sample: Option<String>,
    #[serde(default)]
    pub truncated: Option<bool>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

/// `POST /proof-packs/{id}/pr-check` — run the PR-description consistency check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrCheckReq {
    pub title: String,
    pub description: String,
    /// Base ref for the diff (defaults to the working tree vs HEAD).
    #[serde(default)]
    pub base: Option<String>,
    /// Working dir to diff (defaults to the pack's repo path / cwd).
    #[serde(default)]
    pub cwd: Option<String>,
}

/// `POST /proof-packs/{id}/ci-refresh` — fetch live CI for the linked PR.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CiRefreshReq {
    /// Override repo id (defaults to the pack's `repo_id`).
    #[serde(default)]
    pub repo_id: Option<String>,
    /// Override PR number (defaults to the pack's `pr_number`).
    #[serde(default)]
    pub pr_number: Option<i64>,
}

/// `GET|PUT /repos/{id}/proof-config`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoProofConfigResp {
    pub repo_id: String,
    #[serde(flatten)]
    pub config: crate::proof::RepoProofConfig,
}

// ── Conversation view (design docs/design/conversation-view.md §4.3) ─────────

/// `POST /sessions/{id}/tasks` — add a task to a session's board from the UI /
/// Mission Control. The row is `source: "user"`, `nudge_pending: true`; the
/// nudge sweep hands it to the agent once the session is idle.
#[derive(Debug, Clone, Deserialize)]
pub struct CreateAgentTaskReq {
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
}

/// `POST /sessions/{id}/inbox` — drop a pasted image next to the session so the
/// composer can reference it as `[Image: <path>]`. `image/*` only, ≤ 10 MB.
#[derive(Debug, Clone, Deserialize)]
pub struct InboxUploadReq {
    pub filename: String,
    pub mime: String,
    pub data_b64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboxUploadResp {
    pub path: String,
}

/// `POST /workspaces/{wid}/history/import` — adopt an on-disk transcript as a
/// `reconnectable` session row so the normal resume path continues it.
#[derive(Debug, Clone, Deserialize)]
pub struct HistoryImportReq {
    /// `claude` | `codex`.
    pub provider: String,
    pub transcript_path: String,
}

// ── API client storage gauge (perf N2) ───────────────────────────────────────

/// `GET /workspaces/{wid}/api-client/storage`: how much the workspace's API
/// history and automation run reports hold. Counts and byte sizes only —
/// retention stays opt-in; the History list uses this to offer presets.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiClientStorage {
    /// `api_history` rows.
    pub history_rows: i64,
    /// Stored bytes of those rows' request + response snapshots.
    pub history_bytes: i64,
    /// `api_automation_runs` rows (all automations).
    pub run_rows: i64,
    /// Step results recorded across those runs.
    pub step_rows: i64,
    /// Stored bytes of the run headers + step results.
    pub run_bytes: i64,
    /// The largest run count held by any one automation.
    pub max_runs_per_automation: i64,
}
