//! Proof Packs — the evidence layer.
//!
//! An agent may not declare a task "done" on assertion alone. Every meaningful
//! unit of agent work carries a **Proof Pack**: a bundle of inspectable evidence
//! artifacts (diff, tests, build/lint, screenshots, api/db samples, ci, review
//! findings, self-review, human approval) whose **status is derived from the
//! evidence, not claimed by the agent**.
//!
//! This module is the contract of record: the domain types plus the three pure
//! derivation functions ([`derive_status`], [`compute_risk`], [`compute_badges`])
//! that are the single source of truth for what a pack's status and badges are.
//! It has no I/O — `otto-state` persists these, `otto-server` assembles them.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::Id;

/// A pack with `risk_score >= RISKY_THRESHOLD` (or any risky file) earns the
/// `risky_change` badge.
pub const RISKY_THRESHOLD: u8 = 50;
/// Hard cap on stored artifact content (2 MiB). Larger content is truncated with
/// a trailing note and `metadata.truncated = true`.
pub const STORE_CAP: usize = 2 * 1024 * 1024;
/// Cap on the inline preview returned in list/detail responses (8 KiB).
pub const PREVIEW_CAP: usize = 8 * 1024;
/// Hard cap on a single media blob (screenshot/video), 25 MiB. Larger ⇒ 413.
pub const MEDIA_CAP: usize = 25 * 1024 * 1024;
/// Per-artifact content copied into an immutable snapshot bundle (64 KiB). The
/// artifact's `content_sha256` of the FULL content is also embedded, so the
/// snapshot stays small while remaining tamper-evident.
pub const SNAPSHOT_ARTIFACT_CAP: usize = 64 * 1024;
/// Done-contract score at/above which a pack earns the "release-ready" treatment
/// in the UI. The gate uses `status`, not this score; this is presentational.
pub const DONE_READY_THRESHOLD: u8 = 80;
/// PR-consistency score (0..100) at/above which the check passes (absent a hard
/// fail like a false "tests pass" claim).
pub const PR_CONSISTENCY_THRESHOLD: u8 = 70;

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

/// Derived status of a proof pack. `Waived` is the only human override; every
/// other value is computed by [`derive_status`] from the artifact set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProofStatus {
    /// No evidence at all.
    Missing,
    /// Some evidence, but the required set for this work-item kind isn't met.
    Partial,
    /// Required evidence present and nothing failed.
    Passed,
    /// At least one artifact failed (e.g. a test command exited non-zero).
    Failed,
    /// A human explicitly waived the proof requirement.
    Waived,
}

impl ProofStatus {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "missing" => Some(Self::Missing),
            "partial" => Some(Self::Partial),
            "passed" => Some(Self::Passed),
            "failed" => Some(Self::Failed),
            "waived" => Some(Self::Waived),
            _ => None,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Partial => "partial",
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::Waived => "waived",
        }
    }
}

/// The kind of an evidence artifact. Mirrors the requested MVP kind list plus
/// `SelfReview` (agent self-review is distinct from review-findings).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProofArtifactKind {
    Command,
    Log,
    Screenshot,
    /// Video / screencast evidence of a working UI (stored as a blob).
    Video,
    Diff,
    Ci,
    Api,
    Db,
    /// Kafka (message broker) read evidence — a consumed message sample.
    Kafka,
    Review,
    Approval,
    /// PR-description consistency-check result (claims vs. actual change).
    PrCheck,
    SelfReview,
}

impl ProofArtifactKind {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "command" => Some(Self::Command),
            "log" => Some(Self::Log),
            "screenshot" => Some(Self::Screenshot),
            "video" => Some(Self::Video),
            "diff" => Some(Self::Diff),
            "ci" => Some(Self::Ci),
            "api" => Some(Self::Api),
            "db" => Some(Self::Db),
            "kafka" => Some(Self::Kafka),
            "review" => Some(Self::Review),
            "approval" => Some(Self::Approval),
            "pr_check" => Some(Self::PrCheck),
            "self_review" => Some(Self::SelfReview),
            _ => None,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Command => "command",
            Self::Log => "log",
            Self::Screenshot => "screenshot",
            Self::Video => "video",
            Self::Diff => "diff",
            Self::Ci => "ci",
            Self::Api => "api",
            Self::Db => "db",
            Self::Kafka => "kafka",
            Self::Review => "review",
            Self::Approval => "approval",
            Self::PrCheck => "pr_check",
            Self::SelfReview => "self_review",
        }
    }
    /// True for media kinds whose content is a binary blob (not inline text).
    pub fn is_media(&self) -> bool {
        matches!(self, Self::Screenshot | Self::Video)
    }
}

/// Status of a single artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProofArtifactStatus {
    Passed,
    Failed,
    Pending,
    /// Neutral evidence (a diff, a logged sample) — neither pass nor fail.
    Info,
}

impl ProofArtifactStatus {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "passed" => Some(Self::Passed),
            "failed" => Some(Self::Failed),
            "pending" => Some(Self::Pending),
            "info" => Some(Self::Info),
            _ => None,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::Pending => "pending",
            Self::Info => "info",
        }
    }
}

/// What kind of work item a pack is attached to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemKind {
    Session,
    GoalLoop,
    Review,
    WorkflowRun,
    Task,
    Manual,
}

impl WorkItemKind {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "session" => Some(Self::Session),
            "goal_loop" => Some(Self::GoalLoop),
            "review" => Some(Self::Review),
            "workflow_run" => Some(Self::WorkflowRun),
            "task" => Some(Self::Task),
            "manual" => Some(Self::Manual),
            _ => None,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::GoalLoop => "goal_loop",
            Self::Review => "review",
            Self::WorkflowRun => "workflow_run",
            Self::Task => "task",
            Self::Manual => "manual",
        }
    }
}

/// A UI badge derived from a pack + its artifacts. A pack may carry several.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProofBadge {
    NoProof,
    TestsPassed,
    TestsFailed,
    HumanApproved,
    RiskyChange,
    CiMissing,
    /// CI reported success (a `ci` artifact passed).
    CiPassed,
    /// CI reported failure (a `ci` artifact failed).
    CiFailed,
    /// CI is still running (a `ci` artifact is pending).
    CiPending,
    DbApiVerified,
    /// UI evidence present (a screenshot/video artifact).
    UiVerified,
    /// PR description is inconsistent with the actual change.
    PrInconsistent,
    ReviewUnresolved,
    Waived,
}

impl ProofBadge {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NoProof => "no_proof",
            Self::TestsPassed => "tests_passed",
            Self::TestsFailed => "tests_failed",
            Self::HumanApproved => "human_approved",
            Self::RiskyChange => "risky_change",
            Self::CiMissing => "ci_missing",
            Self::CiPassed => "ci_passed",
            Self::CiFailed => "ci_failed",
            Self::CiPending => "ci_pending",
            Self::DbApiVerified => "db_api_verified",
            Self::UiVerified => "ui_verified",
            Self::PrInconsistent => "pr_inconsistent",
            Self::ReviewUnresolved => "review_unresolved",
            Self::Waived => "waived",
        }
    }
}

// ---------------------------------------------------------------------------
// Structs
// ---------------------------------------------------------------------------

/// A proof pack: one per work item (`UNIQUE(work_item_kind, work_item_id)`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofPack {
    pub id: Id,
    pub workspace_id: Id,
    pub work_item_kind: WorkItemKind,
    pub work_item_id: String,
    pub title: String,
    pub status: ProofStatus,
    pub summary: String,
    pub risk_score: u8,
    /// Done-contract readiness 0..100 (derived; see [`compute_done_contract`]).
    /// Persisted on recompute for cheap list/summary sorting; the detail view
    /// recomputes the full contract live.
    #[serde(default)]
    pub done_score: u8,
    /// Optional parent pack (e.g. a goal-loop pack parenting the session/review/
    /// workflow packs it spawns).
    pub parent_pack_id: Option<Id>,
    /// Repo this pack's work touched, when resolvable — drives per-repo proof
    /// policy ([`RepoProofConfig`]) and CI/report lookups.
    #[serde(default)]
    pub repo_id: Option<Id>,
    /// PR number once the pack becomes PR-linked (for CI refresh + report).
    #[serde(default)]
    pub pr_number: Option<i64>,
    pub waived_by: Option<Id>,
    pub waived_reason: Option<String>,
    /// When the pack was waived (RFC3339), set alongside `waived_by`.
    #[serde(default)]
    pub waived_at: Option<String>,
    pub created_by: Id,
    pub created_at: String,
    pub updated_at: String,
}

/// One evidence artifact within a pack.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofArtifact {
    pub id: Id,
    pub proof_pack_id: Id,
    pub workspace_id: Id,
    pub kind: ProofArtifactKind,
    pub title: String,
    /// Inline text (capped `STORE_CAP`), a URL, or a file ref. The flavor is in
    /// `metadata.ref_kind` ∈ `inline | url | file`.
    pub content_ref: Option<String>,
    pub status: ProofArtifactStatus,
    pub metadata: Value,
    /// SHA-256 (hex) of the FULL stored content at write time — tamper-evidence
    /// that survives into snapshots (which copy only a capped preview). `None`
    /// for artifacts with no inline content (url/blob/none refs).
    #[serde(default)]
    pub content_sha256: Option<String>,
    pub created_by: Id,
    pub created_at: String,
    pub updated_at: String,
}

// ---------------------------------------------------------------------------
// Test-command recognition (D2 — non-gameable "passed")
// ---------------------------------------------------------------------------

/// True if `cmd` looks like a recognized test runner. A `command` artifact only
/// counts as *test evidence* (and so toward `passed`) when its command matches —
/// a trivial green no-op like `true` does not earn `passed`.
pub fn looks_like_test_command(cmd: &str) -> bool {
    let c = cmd.to_lowercase();
    const NEEDLES: &[&str] = &[
        "cargo test",
        "cargo nextest",
        "go test",
        "npm test",
        "npm run test",
        "npm run check",
        "yarn test",
        "pnpm test",
        "jest",
        "vitest",
        "playwright test",
        "pytest",
        "python -m pytest",
        "go vet",        // build/lint runners that still gate quality
        "cargo clippy",
        "svelte-check",
        "ctest",
        "gradle test",
        "mvn test",
        "rspec",
        "phpunit",
    ];
    NEEDLES.iter().any(|n| c.contains(n))
}

/// True if `a` is a `command` artifact that represents an actual test run
/// (recognized runner). Build/lint commands are recognized too — they all gate
/// quality — but the metadata `test_kind` records which it was.
pub fn is_test_artifact(a: &ProofArtifact) -> bool {
    a.kind == ProofArtifactKind::Command && looks_like_test_command(&a.title)
}

// ---------------------------------------------------------------------------
// Required-kind policy (D4)
// ---------------------------------------------------------------------------

/// The evidence a pack of a given work-item kind needs to reach `Passed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequiredSpec {
    /// A code change: requires a `diff` AND ≥1 passing recognized-test command.
    CodeChange,
    /// A review: requires ≥1 `review` artifact and none failed.
    Review,
    /// A workflow run: ≥1 node artifact, none failed, every approval present is passed.
    WorkflowRun,
    /// Lenient: ≥1 artifact and none failed.
    Lenient,
}

pub fn required_kinds(kind: WorkItemKind) -> RequiredSpec {
    match kind {
        WorkItemKind::Session | WorkItemKind::GoalLoop | WorkItemKind::Task => {
            RequiredSpec::CodeChange
        }
        WorkItemKind::Review => RequiredSpec::Review,
        WorkItemKind::WorkflowRun => RequiredSpec::WorkflowRun,
        WorkItemKind::Manual => RequiredSpec::Lenient,
    }
}

// ---------------------------------------------------------------------------
// Derivation (the single source of truth)
// ---------------------------------------------------------------------------

fn any_failed(arts: &[ProofArtifact]) -> bool {
    arts.iter().any(|a| a.status == ProofArtifactStatus::Failed)
}

fn has_kind(arts: &[ProofArtifact], k: ProofArtifactKind) -> bool {
    arts.iter().any(|a| a.kind == k)
}

/// Whether the required-evidence set for this pack is satisfied (ignoring
/// failures, which are handled separately).
fn required_met(pack: &ProofPack, arts: &[ProofArtifact]) -> bool {
    match required_kinds(pack.work_item_kind) {
        RequiredSpec::CodeChange => {
            has_kind(arts, ProofArtifactKind::Diff)
                && arts
                    .iter()
                    .any(|a| is_test_artifact(a) && a.status == ProofArtifactStatus::Passed)
        }
        RequiredSpec::Review => has_kind(arts, ProofArtifactKind::Review),
        RequiredSpec::WorkflowRun => {
            !arts.is_empty()
                && arts
                    .iter()
                    .filter(|a| a.kind == ProofArtifactKind::Approval)
                    .all(|a| a.status == ProofArtifactStatus::Passed)
        }
        RequiredSpec::Lenient => arts
            .iter()
            .any(|a| a.status != ProofArtifactStatus::Failed),
    }
}

/// Derive the pack status from its artifacts. `Waived` short-circuits.
pub fn derive_status(pack: &ProofPack, arts: &[ProofArtifact]) -> ProofStatus {
    if pack.waived_by.is_some() {
        return ProofStatus::Waived;
    }
    if arts.is_empty() {
        return ProofStatus::Missing;
    }
    if any_failed(arts) {
        return ProofStatus::Failed;
    }
    if required_met(pack, arts) {
        ProofStatus::Passed
    } else {
        ProofStatus::Partial
    }
}

/// Read a usize-ish field from a diff artifact's metadata.
fn meta_usize(a: &ProofArtifact, key: &str) -> usize {
    a.metadata
        .get(key)
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize
}

/// Compute the 0..100 risk score from the artifact set. See spec §4.2.
pub fn compute_risk(arts: &[ProofArtifact]) -> u8 {
    let mut risk: i32 = 0;

    // Size + risky files from the diff artifact (if any).
    let diff = arts.iter().find(|a| a.kind == ProofArtifactKind::Diff);
    let mut migration_touched = false;
    if let Some(d) = diff {
        let loc = meta_usize(d, "additions") + meta_usize(d, "deletions");
        risk += ((loc / 20) as i32).min(40);
        let risky_files = d
            .metadata
            .get("risky_files")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        risk += ((risky_files as i32) * 8).min(32);
        if let Some(arr) = d.metadata.get("risky_files").and_then(|v| v.as_array()) {
            migration_touched = arr.iter().any(|f| {
                f.as_str()
                    .map(|s| s.contains("migrations/") || s.ends_with(".sql"))
                    .unwrap_or(false)
            });
        }
    }

    // Failing tests.
    if arts
        .iter()
        .any(|a| is_test_artifact(a) && a.status == ProofArtifactStatus::Failed)
    {
        risk += 25;
    }
    // Unresolved review findings.
    if arts
        .iter()
        .any(|a| a.kind == ProofArtifactKind::Review && a.status == ProofArtifactStatus::Failed)
    {
        risk += 15;
    }
    // Untested change (a diff but no test command at all).
    if diff.is_some() && !arts.iter().any(is_test_artifact) {
        risk += 10;
    }
    if migration_touched {
        risk += 10;
    }

    risk.clamp(0, 100) as u8
}

/// Compute the badge set for a pack. See spec §4.3 + decisions D9/D10/D11.
pub fn compute_badges(pack: &ProofPack, arts: &[ProofArtifact]) -> Vec<ProofBadge> {
    let mut out = Vec::new();

    if pack.status == ProofStatus::Waived {
        out.push(ProofBadge::Waived);
    }
    if arts.is_empty() && pack.status != ProofStatus::Waived {
        out.push(ProofBadge::NoProof);
    }

    let test_arts: Vec<&ProofArtifact> = arts.iter().filter(|a| is_test_artifact(a)).collect();
    if !test_arts.is_empty() {
        if test_arts
            .iter()
            .any(|a| a.status == ProofArtifactStatus::Failed)
        {
            out.push(ProofBadge::TestsFailed);
        } else if test_arts
            .iter()
            .all(|a| a.status == ProofArtifactStatus::Passed)
        {
            out.push(ProofBadge::TestsPassed);
        }
    }

    if arts
        .iter()
        .any(|a| a.kind == ProofArtifactKind::Approval && a.status == ProofArtifactStatus::Passed)
    {
        out.push(ProofBadge::HumanApproved);
    }

    if pack.risk_score >= RISKY_THRESHOLD || has_risky_file(arts) {
        out.push(ProofBadge::RiskyChange);
    }

    // D9: only when code changed but no CI result attached.
    if has_kind(arts, ProofArtifactKind::Diff) && !has_kind(arts, ProofArtifactKind::Ci) {
        out.push(ProofBadge::CiMissing);
    }

    // CI outcome (when a ci artifact is present). Failed wins, then pending,
    // then passed — mirrors how a human reads a check-run summary.
    let ci: Vec<&ProofArtifact> = arts.iter().filter(|a| a.kind == ProofArtifactKind::Ci).collect();
    if !ci.is_empty() {
        if ci.iter().any(|a| a.status == ProofArtifactStatus::Failed) {
            out.push(ProofBadge::CiFailed);
        } else if ci.iter().any(|a| a.status == ProofArtifactStatus::Pending) {
            out.push(ProofBadge::CiPending);
        } else if ci.iter().any(|a| a.status == ProofArtifactStatus::Passed) {
            out.push(ProofBadge::CiPassed);
        }
    }

    // D10: requires an explicit Passed mark, not the default Info. Kafka joins
    // the db/api "data verified" family.
    if arts.iter().any(|a| {
        matches!(
            a.kind,
            ProofArtifactKind::Db | ProofArtifactKind::Api | ProofArtifactKind::Kafka
        ) && a.status == ProofArtifactStatus::Passed
    }) {
        out.push(ProofBadge::DbApiVerified);
    }

    // UI evidence present (a screenshot or video artifact, any non-failed status).
    if arts
        .iter()
        .any(|a| a.kind.is_media() && a.status != ProofArtifactStatus::Failed)
    {
        out.push(ProofBadge::UiVerified);
    }

    // PR description inconsistent with the actual change.
    if arts
        .iter()
        .any(|a| a.kind == ProofArtifactKind::PrCheck && a.status == ProofArtifactStatus::Failed)
    {
        out.push(ProofBadge::PrInconsistent);
    }

    // Review with unresolved findings.
    if arts
        .iter()
        .any(|a| a.kind == ProofArtifactKind::Review && a.status == ProofArtifactStatus::Failed)
    {
        out.push(ProofBadge::ReviewUnresolved);
    }

    out
}

fn has_risky_file(arts: &[ProofArtifact]) -> bool {
    arts.iter()
        .filter(|a| a.kind == ProofArtifactKind::Diff)
        .any(|d| {
            d.metadata
                .get("risky_files")
                .and_then(|v| v.as_array())
                .map(|a| !a.is_empty())
                .unwrap_or(false)
        })
}

/// Truncate `content` to `PREVIEW_CAP` for list/detail responses, returning
/// `(preview, truncated)`.
pub fn preview(content: &str) -> (String, bool) {
    if content.len() <= PREVIEW_CAP {
        (content.to_string(), false)
    } else {
        let cut = content
            .char_indices()
            .take_while(|(i, _)| *i <= PREVIEW_CAP)
            .last()
            .map(|(i, _)| i)
            .unwrap_or(PREVIEW_CAP);
        (content[..cut].to_string(), true)
    }
}

// ---------------------------------------------------------------------------
// Integrity hashing (R1 — tamper-evidence)
// ---------------------------------------------------------------------------

/// SHA-256 (lowercase hex) of `content`. Used to stamp `ProofArtifact.content_sha256`
/// and to make snapshots tamper-evident.
pub fn content_sha256(content: &str) -> String {
    let mut h = Sha256::new();
    h.update(content.as_bytes());
    format!("{:x}", h.finalize())
}

/// SHA-256 (lowercase hex) of raw bytes (media blobs).
pub fn bytes_sha256(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    format!("{:x}", h.finalize())
}

/// Recursively sort object keys so a `Value` serializes deterministically
/// regardless of serde_json's `preserve_order` feature.
fn canonicalize(v: &Value) -> Value {
    match v {
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            let mut out = serde_json::Map::new();
            for k in keys {
                out.insert(k.clone(), canonicalize(&m[k]));
            }
            Value::Object(out)
        }
        Value::Array(a) => Value::Array(a.iter().map(canonicalize).collect()),
        other => other.clone(),
    }
}

/// Stable SHA-256 of a JSON bundle (key-order-independent). The immutable
/// snapshot's tamper-evidence key.
pub fn bundle_sha256(v: &Value) -> String {
    content_sha256(&serde_json::to_string(&canonicalize(v)).unwrap_or_default())
}

// ---------------------------------------------------------------------------
// Evidence status mappers (R2/R5/R6 — pure, so they're testable & non-gameable)
// ---------------------------------------------------------------------------

/// Map a provider CI aggregate state to an artifact status.
pub fn ci_artifact_status(state: &str) -> ProofArtifactStatus {
    match state.trim().to_lowercase().as_str() {
        "success" | "passed" | "passing" | "ok" | "green" => ProofArtifactStatus::Passed,
        "failure" | "failed" | "failing" | "error" | "red" | "canceled" | "cancelled" => {
            ProofArtifactStatus::Failed
        }
        "pending" | "running" | "in_progress" | "queued" | "expected" | "waiting" => {
            ProofArtifactStatus::Pending
        }
        // "none" / unknown — neutral.
        _ => ProofArtifactStatus::Info,
    }
}

/// Map an HTTP response code to an API-evidence artifact status. `0` = no
/// response (network error) → `Info` is misleading; treat as `Failed`.
pub fn http_evidence_status(code: u16) -> ProofArtifactStatus {
    match code {
        0 => ProofArtifactStatus::Failed,
        200..=299 => ProofArtifactStatus::Passed,
        c if c >= 400 => ProofArtifactStatus::Failed,
        _ => ProofArtifactStatus::Info, // 1xx / 3xx
    }
}

/// DB / Kafka read evidence: an error fails it; otherwise it's verified.
pub fn read_evidence_status(has_error: bool) -> ProofArtifactStatus {
    if has_error {
        ProofArtifactStatus::Failed
    } else {
        ProofArtifactStatus::Passed
    }
}

/// A compact, serialisable CI summary (mirrors otto-git `CiStatus`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CiSummary {
    pub state: String,
    pub total: u32,
    pub passed: u32,
    pub failed: u32,
    pub url: Option<String>,
}

// ---------------------------------------------------------------------------
// Per-repo proof policy (R3) + done contract (R8)
// ---------------------------------------------------------------------------

/// Per-repository proof requirements, stored as JSON on `repos.proof_config_json`.
/// Every flag defaults to `false` ⇒ a repo with no config behaves exactly as v1.
/// Flags can only STRENGTHEN the work-item-kind defaults (never relax them), so
/// the trust layer cannot be turned down via config.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoProofConfig {
    /// Require a passing, recognized test command before a pack can be `passed`.
    #[serde(default)]
    pub require_test: bool,
    /// The repo's canonical test command (used by session auto-test when set).
    #[serde(default)]
    pub test_cmd: Option<String>,
    /// Require a green CI artifact.
    #[serde(default)]
    pub require_ci: bool,
    /// Require a passing PR-description consistency check.
    #[serde(default)]
    pub require_pr_consistency: bool,
    /// Require a resolved review artifact.
    #[serde(default)]
    pub require_review: bool,
}

/// The EXTRA, repo-opted-in requirements layered on top of a work-item kind's
/// built-in requirements. The kind defaults live in [`derive_status`]; this only
/// ever adds, so `derive_status_with_policy(p, a, &default) == derive_status(p, a)`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DoneContractPolicy {
    pub require_test: bool,
    pub require_ci: bool,
    pub require_pr_consistency: bool,
    pub require_review: bool,
}

impl DoneContractPolicy {
    /// The default (no-extra) policy for a kind. The kind's intrinsic
    /// requirements are enforced by `derive_status`, so the default carries no
    /// extras — guaranteeing the legacy-equivalence invariant.
    pub fn for_kind(_kind: WorkItemKind) -> Self {
        Self::default()
    }

    /// Layer a repo's opted-in extras on top (strengthen-only).
    pub fn with_repo(mut self, cfg: &RepoProofConfig) -> Self {
        self.require_test |= cfg.require_test;
        self.require_ci |= cfg.require_ci;
        self.require_pr_consistency |= cfg.require_pr_consistency;
        self.require_review |= cfg.require_review;
        self
    }

    /// Whether any opted-in extra requirement lacks a *passing* artifact.
    fn extra_unmet(&self, arts: &[ProofArtifact]) -> bool {
        if self.require_test
            && !arts
                .iter()
                .any(|a| is_test_artifact(a) && a.status == ProofArtifactStatus::Passed)
        {
            return true;
        }
        if self.require_ci
            && !arts
                .iter()
                .any(|a| a.kind == ProofArtifactKind::Ci && a.status == ProofArtifactStatus::Passed)
        {
            return true;
        }
        if self.require_pr_consistency
            && !arts.iter().any(|a| {
                a.kind == ProofArtifactKind::PrCheck && a.status == ProofArtifactStatus::Passed
            })
        {
            return true;
        }
        if self.require_review
            && !arts.iter().any(|a| {
                a.kind == ProofArtifactKind::Review && a.status == ProofArtifactStatus::Passed
            })
        {
            return true;
        }
        false
    }
}

/// Derive status under a policy: the legacy status, then — only if it would be
/// `Passed` — capped to `Partial` when a repo-opted-in extra requirement is unmet.
/// A failing required artifact already makes the legacy status `Failed`.
pub fn derive_status_with_policy(
    pack: &ProofPack,
    arts: &[ProofArtifact],
    policy: &DoneContractPolicy,
) -> ProofStatus {
    match derive_status(pack, arts) {
        ProofStatus::Passed if policy.extra_unmet(arts) => ProofStatus::Partial,
        other => other,
    }
}

/// One line of the done-contract checklist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractItem {
    pub key: String,
    pub label: String,
    pub required: bool,
    pub satisfied: bool,
    pub weight: u8,
    pub detail: String,
}

/// The "done contract": an explainable readiness score + itemized checklist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoneContract {
    /// 0..100 weighted readiness over the *required* items (waived ⇒ 100).
    pub score: u8,
    /// Count of items currently satisfied.
    pub satisfied: u32,
    /// Count of items that are required.
    pub required: u32,
    pub items: Vec<ContractItem>,
}

/// Compute the done contract for a pack. Deterministic & pure (the score is the
/// single source of truth for "how ready is this, and what's missing").
pub fn compute_done_contract(
    pack: &ProofPack,
    arts: &[ProofArtifact],
    policy: &DoneContractPolicy,
) -> DoneContract {
    let spec = required_kinds(pack.work_item_kind);
    let code_change = spec == RequiredSpec::CodeChange;
    let nonempty = !arts.is_empty();
    let no_failures = nonempty && !any_failed(arts);

    let has_passing_test = arts
        .iter()
        .any(|a| is_test_artifact(a) && a.status == ProofArtifactStatus::Passed);
    let has_diff = has_kind(arts, ProofArtifactKind::Diff);
    let ci_passed = arts
        .iter()
        .any(|a| a.kind == ProofArtifactKind::Ci && a.status == ProofArtifactStatus::Passed);
    let review_ok = has_kind(arts, ProofArtifactKind::Review)
        && !arts
            .iter()
            .any(|a| a.kind == ProofArtifactKind::Review && a.status == ProofArtifactStatus::Failed);
    let pr_ok = has_kind(arts, ProofArtifactKind::PrCheck)
        && !arts.iter().any(|a| {
            a.kind == ProofArtifactKind::PrCheck && a.status == ProofArtifactStatus::Failed
        });
    let ui_ok = arts
        .iter()
        .any(|a| a.kind.is_media() && a.status != ProofArtifactStatus::Failed);
    let data_ok = arts.iter().any(|a| {
        matches!(
            a.kind,
            ProofArtifactKind::Db | ProofArtifactKind::Api | ProofArtifactKind::Kafka
        ) && a.status == ProofArtifactStatus::Passed
    });
    let self_review = has_kind(arts, ProofArtifactKind::SelfReview);
    let human_ok = arts
        .iter()
        .any(|a| a.kind == ProofArtifactKind::Approval && a.status == ProofArtifactStatus::Passed);

    let item = |key: &str, label: &str, required: bool, satisfied: bool, weight: u8, detail: &str| {
        ContractItem {
            key: key.into(),
            label: label.into(),
            required,
            satisfied,
            weight,
            detail: detail.into(),
        }
    };

    let items = vec![
        item(
            "diff",
            "Code diff captured",
            code_change,
            has_diff,
            15,
            if has_diff { "diff present" } else { "no diff artifact" },
        ),
        item(
            "tests",
            "Tests passed",
            code_change || policy.require_test,
            has_passing_test,
            25,
            if has_passing_test { "passing test command" } else { "no passing recognized test" },
        ),
        item(
            "no_failures",
            "No failed evidence",
            true,
            no_failures,
            20,
            if !nonempty { "no evidence yet" } else if no_failures { "nothing failed" } else { "a failed artifact" },
        ),
        item(
            "ci",
            "CI green",
            policy.require_ci,
            ci_passed,
            10,
            if ci_passed { "CI passed" } else { "no green CI" },
        ),
        item(
            "review",
            "Review resolved",
            spec == RequiredSpec::Review || policy.require_review,
            review_ok,
            10,
            if review_ok { "review resolved" } else { "no resolved review" },
        ),
        item(
            "pr_consistency",
            "PR matches change",
            policy.require_pr_consistency,
            pr_ok,
            10,
            if pr_ok { "PR description consistent" } else { "no passing PR check" },
        ),
        item("ui_evidence", "UI screenshot/video", false, ui_ok, 5,
            if ui_ok { "UI media attached" } else { "no UI evidence" }),
        item("data_evidence", "API/DB/Kafka verified", false, data_ok, 5,
            if data_ok { "data read verified" } else { "no data evidence" }),
        item("self_review", "Agent self-review", false, self_review, 5,
            if self_review { "self-review present" } else { "no self-review" }),
        item("human_approval", "Human approved", false, human_ok, 5,
            if human_ok { "human approval recorded" } else { "no human approval" }),
    ];

    // Score over the *required* weight. `no_failures` is required for every kind
    // (weight 20), so the required weight is always ≥ 20; `.max(1)` is a
    // belt-and-braces guard against a future all-optional contract.
    let req_weight: u32 = items.iter().filter(|i| i.required).map(|i| i.weight as u32).sum();
    let req_sat: u32 = items
        .iter()
        .filter(|i| i.required && i.satisfied)
        .map(|i| i.weight as u32)
        .sum();
    let (num, den) = if req_weight > 0 {
        (req_sat, req_weight)
    } else {
        // No required items → score by present optional weight.
        (
            items.iter().filter(|i| i.satisfied).map(|i| i.weight as u32).sum(),
            items.iter().map(|i| i.weight as u32).sum(),
        )
    };
    let den = den.max(1);
    let score = if pack.waived_by.is_some() {
        100
    } else {
        ((num * 100 + den / 2) / den) as u8
    };

    DoneContract {
        score,
        satisfied: items.iter().filter(|i| i.satisfied).count() as u32,
        required: items.iter().filter(|i| i.required).count() as u32,
        items,
    }
}

/// Convenience: just the score (persisted on the pack for cheap sorting).
pub fn done_score(pack: &ProofPack, arts: &[ProofArtifact], policy: &DoneContractPolicy) -> u8 {
    compute_done_contract(pack, arts, policy).score
}

// ---------------------------------------------------------------------------
// PR description consistency (R7 — catch misalignment before the PR)
// ---------------------------------------------------------------------------

/// Inputs to the PR-consistency check. `description`/`title` should already be
/// redacted by the caller (the check works fine on redacted text).
#[derive(Debug, Clone, Default)]
pub struct PrConsistencyInput {
    pub title: String,
    pub description: String,
    pub files_changed: Vec<String>,
    pub additions: u32,
    pub deletions: u32,
    pub has_passing_tests: bool,
    pub has_failing_tests: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrConsistencyCheck {
    pub key: String,
    pub label: String,
    pub passed: bool,
    pub weight: u8,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrConsistencyReport {
    pub score: u8,
    pub passed: bool,
    /// A hard fail (e.g. a false "tests pass" claim) forces `passed=false`
    /// regardless of score — the core misalignment guard.
    pub hard_fail: bool,
    pub checks: Vec<PrConsistencyCheck>,
}

/// Tokens worth mentioning for a changed path: its top directory and file stem.
fn path_tokens(path: &str) -> Vec<String> {
    let p = path.trim_start_matches("./").to_lowercase();
    let mut out = Vec::new();
    if let Some(top) = p.split('/').next() {
        if top.len() >= 3 {
            out.push(top.to_string());
        }
    }
    if let Some(base) = p.rsplit('/').next() {
        let stem = base.split('.').next().unwrap_or(base);
        if stem.len() >= 3 {
            out.push(stem.to_string());
        }
    }
    out
}

/// Whether the description asserts that tests pass.
fn claims_tests_pass(desc: &str) -> bool {
    let d = desc.to_lowercase();
    const CLAIMS: &[&str] = &[
        "tests pass",
        "all tests pass",
        "tests passing",
        "tests green",
        "test suite passes",
        "passing tests",
        "all green",
        "ci green",
        "ci passes",
        "tests succeed",
    ];
    CLAIMS.iter().any(|c| d.contains(c))
}

/// Run the deterministic PR-description consistency check.
pub fn check_pr_consistency(input: &PrConsistencyInput) -> PrConsistencyReport {
    let desc = input.description.trim();
    let title = input.title.trim();
    let loc = input.additions + input.deletions;
    let dl = desc.to_lowercase();

    // 1. Description is non-trivial.
    let desc_ok = desc.len() >= 40;

    // 2. Title is present and a sane length.
    let title_ok = (8..=140).contains(&title.chars().count());

    // 3. The description mentions at least one changed area.
    let mentions = if input.files_changed.is_empty() {
        true
    } else {
        input
            .files_changed
            .iter()
            .flat_map(|f| path_tokens(f))
            .any(|t| dl.contains(&t))
    };

    // 4. A "testing" section when code actually changed.
    let testing_ok = if loc == 0 {
        true
    } else {
        ["test", "tested", "verif", "ci ", "ci.", "coverage"]
            .iter()
            .any(|c| dl.contains(c))
    };

    // 5. No false "tests pass" claim (HARD).
    let claims = claims_tests_pass(desc);
    let false_claim = claims && (input.has_failing_tests || !input.has_passing_tests);
    let no_false_claim = !false_claim;

    let checks = vec![
        PrConsistencyCheck {
            key: "description".into(),
            label: "Description is substantive".into(),
            passed: desc_ok,
            weight: 20,
            detail: if desc_ok { "≥40 chars".into() } else { "too short / empty".into() },
        },
        PrConsistencyCheck {
            key: "title".into(),
            label: "Title is sane".into(),
            passed: title_ok,
            weight: 10,
            detail: if title_ok { "ok".into() } else { "empty or implausible length".into() },
        },
        PrConsistencyCheck {
            key: "mentions_change".into(),
            label: "Mentions the actual change".into(),
            passed: mentions,
            weight: 25,
            detail: if mentions {
                "references a changed area".into()
            } else {
                "does not reference any changed file/dir".into()
            },
        },
        PrConsistencyCheck {
            key: "testing_section".into(),
            label: "Has testing notes for a code change".into(),
            passed: testing_ok,
            weight: 15,
            detail: if testing_ok { "ok".into() } else { "code changed but no testing notes".into() },
        },
        PrConsistencyCheck {
            key: "no_false_test_claim".into(),
            label: "No false 'tests pass' claim".into(),
            passed: no_false_claim,
            weight: 30,
            detail: if no_false_claim {
                "claims match evidence".into()
            } else {
                "claims tests pass but evidence shows failing/absent tests".into()
            },
        },
    ];

    let score: u32 = checks.iter().filter(|c| c.passed).map(|c| c.weight as u32).sum();
    let score = score.min(100) as u8;
    let passed = !false_claim && score >= PR_CONSISTENCY_THRESHOLD;

    PrConsistencyReport {
        score,
        passed,
        hard_fail: false_claim,
        checks,
    }
}

// ---------------------------------------------------------------------------
// Report rendering (R9 — exportable Markdown / HTML)
// ---------------------------------------------------------------------------

/// Everything a report needs (borrowed; the caller owns the data).
pub struct ReportView<'a> {
    pub pack: &'a ProofPack,
    pub artifacts: &'a [ProofArtifact],
    pub contract: &'a DoneContract,
    pub badges: &'a [String],
    pub generated_at: &'a str,
}

fn artifact_preview(a: &ProofArtifact) -> String {
    match &a.content_ref {
        Some(c) => {
            let (p, _) = preview(c);
            p
        }
        None => String::new(),
    }
}

/// Render a self-contained Markdown evidence report.
pub fn render_report_md(v: &ReportView) -> String {
    let p = v.pack;
    let mut s = String::new();
    s.push_str(&format!("# Proof Pack — {}\n\n", if p.title.is_empty() { &p.work_item_id } else { &p.title }));
    s.push_str(&format!(
        "- **Status:** {}\n- **Done contract:** {}/100\n- **Risk:** {}/100\n- **Work item:** {} `{}`\n- **Generated:** {}\n\n",
        p.status.as_str(), v.contract.score, p.risk_score, p.work_item_kind.as_str(), p.work_item_id, v.generated_at
    ));
    if !v.badges.is_empty() {
        s.push_str(&format!("**Badges:** {}\n\n", v.badges.join(", ")));
    }
    if !p.summary.is_empty() {
        s.push_str(&format!("{}\n\n", p.summary));
    }
    if let (Some(by), Some(reason)) = (&p.waived_by, &p.waived_reason) {
        s.push_str(&format!(
            "> ⚠️ **Waived** by `{}`{}: {}\n\n",
            by,
            p.waived_at.as_deref().map(|t| format!(" at {t}")).unwrap_or_default(),
            reason
        ));
    }

    s.push_str("## Done contract\n\n");
    for it in &v.contract.items {
        let mark = if it.satisfied { "x" } else { " " };
        let req = if it.required { " *(required)*" } else { "" };
        s.push_str(&format!("- [{}] {}{} — {}\n", mark, it.label, req, it.detail));
    }
    s.push('\n');

    s.push_str("## Evidence\n\n");
    for a in v.artifacts {
        s.push_str(&format!(
            "### {} · {} · `{}`\n\n",
            a.kind.as_str(),
            a.title,
            a.status.as_str()
        ));
        if let Some(sha) = &a.content_sha256 {
            s.push_str(&format!("`sha256:{}`\n\n", sha));
        }
        let pv = artifact_preview(a);
        if !pv.is_empty() && !a.kind.is_media() {
            s.push_str("```\n");
            s.push_str(&pv);
            s.push_str("\n```\n\n");
        } else if a.kind.is_media() {
            s.push_str(&format!("_(media: {})_\n\n", a.content_ref.as_deref().unwrap_or("")));
        }
    }
    s
}

fn esc_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Render a self-contained HTML evidence report (inline CSS, no external assets).
pub fn render_report_html(v: &ReportView) -> String {
    let p = v.pack;
    let title = if p.title.is_empty() { &p.work_item_id } else { &p.title };
    let mut body = String::new();
    body.push_str(&format!("<h1>Proof Pack — {}</h1>", esc_html(title)));
    body.push_str(&format!(
        "<p class=meta><b>Status:</b> {} &middot; <b>Done contract:</b> {}/100 &middot; <b>Risk:</b> {}/100 &middot; <b>{}</b> <code>{}</code> &middot; {}</p>",
        esc_html(p.status.as_str()), v.contract.score, p.risk_score,
        esc_html(p.work_item_kind.as_str()), esc_html(&p.work_item_id), esc_html(v.generated_at)
    ));
    if !v.badges.is_empty() {
        body.push_str("<p>");
        for b in v.badges {
            body.push_str(&format!("<span class=badge>{}</span> ", esc_html(b)));
        }
        body.push_str("</p>");
    }
    if !p.summary.is_empty() {
        body.push_str(&format!("<p>{}</p>", esc_html(&p.summary)));
    }
    if let (Some(by), Some(reason)) = (&p.waived_by, &p.waived_reason) {
        body.push_str(&format!(
            "<p class=waived>⚠️ Waived by <code>{}</code>{}: {}</p>",
            esc_html(by),
            p.waived_at.as_deref().map(|t| format!(" at {}", esc_html(t))).unwrap_or_default(),
            esc_html(reason)
        ));
    }

    body.push_str("<h2>Done contract</h2><ul class=contract>");
    for it in &v.contract.items {
        body.push_str(&format!(
            "<li class=\"{}\">{} {}{} — {}</li>",
            if it.satisfied { "ok" } else { "miss" },
            if it.satisfied { "✓" } else { "✗" },
            esc_html(&it.label),
            if it.required { " (required)" } else { "" },
            esc_html(&it.detail)
        ));
    }
    body.push_str("</ul>");

    body.push_str("<h2>Evidence</h2>");
    for a in v.artifacts {
        body.push_str(&format!(
            "<div class=art><h3>{} · {} · <code>{}</code></h3>",
            esc_html(a.kind.as_str()),
            esc_html(&a.title),
            esc_html(a.status.as_str())
        ));
        if let Some(sha) = &a.content_sha256 {
            body.push_str(&format!("<p class=sha><code>sha256:{}</code></p>", esc_html(sha)));
        }
        let pv = artifact_preview(a);
        if a.kind.is_media() {
            body.push_str(&format!("<p class=media>media: {}</p>", esc_html(a.content_ref.as_deref().unwrap_or(""))));
        } else if !pv.is_empty() {
            body.push_str(&format!("<pre>{}</pre>", esc_html(&pv)));
        }
        body.push_str("</div>");
    }

    format!(
        "<!doctype html><html><head><meta charset=utf-8><title>Proof Pack — {}</title>\
<style>body{{font:14px/1.5 -apple-system,Segoe UI,sans-serif;max-width:860px;margin:2rem auto;padding:0 1rem;color:#1c1c1e}}\
code{{background:#f2f2f7;padding:1px 4px;border-radius:4px}}.meta{{color:#555}}\
.badge{{display:inline-block;background:#eef;border-radius:10px;padding:1px 8px;font-size:12px;margin:2px}}\
.contract{{list-style:none;padding:0}}.contract li{{padding:2px 0}}.contract .ok{{color:#1a7f37}}.contract .miss{{color:#a40e26}}\
.art{{border:1px solid #e5e5ea;border-radius:8px;padding:8px 12px;margin:10px 0}}.art h3{{margin:.3rem 0;font-size:14px}}\
pre{{background:#1c1c1e;color:#e5e5ea;padding:10px;border-radius:6px;overflow:auto;max-height:420px;white-space:pre-wrap}}\
.waived{{background:#fff3cd;border:1px solid #ffe69c;padding:8px;border-radius:6px}}.sha{{color:#888;font-size:12px}}</style>\
</head><body>{}</body></html>",
        esc_html(title),
        body
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn pack(kind: WorkItemKind) -> ProofPack {
        ProofPack {
            id: "p1".into(),
            workspace_id: "w1".into(),
            work_item_kind: kind,
            work_item_id: "wi1".into(),
            title: "t".into(),
            status: ProofStatus::Missing,
            summary: String::new(),
            risk_score: 0,
            done_score: 0,
            parent_pack_id: None,
            repo_id: None,
            pr_number: None,
            waived_by: None,
            waived_reason: None,
            waived_at: None,
            created_by: "u1".into(),
            created_at: "2026-06-26T00:00:00Z".into(),
            updated_at: "2026-06-26T00:00:00Z".into(),
        }
    }

    fn art(kind: ProofArtifactKind, title: &str, status: ProofArtifactStatus, meta: Value) -> ProofArtifact {
        ProofArtifact {
            id: "a".into(),
            proof_pack_id: "p1".into(),
            workspace_id: "w1".into(),
            kind,
            title: title.into(),
            content_ref: None,
            status,
            metadata: meta,
            content_sha256: None,
            created_by: "otto".into(),
            created_at: "2026-06-26T00:00:00Z".into(),
            updated_at: "2026-06-26T00:00:00Z".into(),
        }
    }

    #[test]
    fn enum_roundtrips() {
        for s in ["missing", "partial", "passed", "failed", "waived"] {
            assert_eq!(ProofStatus::parse(s).unwrap().as_str(), s);
        }
        for s in [
            "command", "log", "screenshot", "video", "diff", "ci", "api", "db", "kafka", "review",
            "approval", "pr_check", "self_review",
        ] {
            assert_eq!(ProofArtifactKind::parse(s).unwrap().as_str(), s);
        }
        for s in ["passed", "failed", "pending", "info"] {
            assert_eq!(ProofArtifactStatus::parse(s).unwrap().as_str(), s);
        }
        for s in ["session", "goal_loop", "review", "workflow_run", "task", "manual"] {
            assert_eq!(WorkItemKind::parse(s).unwrap().as_str(), s);
        }
    }

    #[test]
    fn looks_like_test_command_pos_neg() {
        assert!(looks_like_test_command("cargo test --workspace"));
        assert!(looks_like_test_command("npm test"));
        assert!(looks_like_test_command("go test ./..."));
        assert!(looks_like_test_command("npx playwright test"));
        assert!(!looks_like_test_command("true"));
        assert!(!looks_like_test_command("echo done"));
        assert!(!looks_like_test_command("ls -la"));
    }

    #[test]
    fn status_missing_when_no_artifacts() {
        let p = pack(WorkItemKind::Session);
        assert_eq!(derive_status(&p, &[]), ProofStatus::Missing);
    }

    #[test]
    fn status_waived_overrides() {
        let mut p = pack(WorkItemKind::Session);
        p.waived_by = Some("u1".into());
        // even with a failing artifact, waived wins
        let arts = vec![art(ProofArtifactKind::Command, "cargo test", ProofArtifactStatus::Failed, json!({}))];
        assert_eq!(derive_status(&p, &arts), ProofStatus::Waived);
    }

    #[test]
    fn status_failed_on_any_failure() {
        let p = pack(WorkItemKind::Session);
        let arts = vec![
            art(ProofArtifactKind::Diff, "diff", ProofArtifactStatus::Info, json!({})),
            art(ProofArtifactKind::Command, "cargo test", ProofArtifactStatus::Failed, json!({})),
        ];
        assert_eq!(derive_status(&p, &arts), ProofStatus::Failed);
    }

    #[test]
    fn code_change_passed_requires_diff_and_passing_test() {
        let p = pack(WorkItemKind::Session);
        // diff only -> partial
        let diff_only = vec![art(ProofArtifactKind::Diff, "diff", ProofArtifactStatus::Info, json!({}))];
        assert_eq!(derive_status(&p, &diff_only), ProofStatus::Partial);
        // diff + passing test -> passed
        let full = vec![
            art(ProofArtifactKind::Diff, "diff", ProofArtifactStatus::Info, json!({})),
            art(ProofArtifactKind::Command, "cargo test --workspace", ProofArtifactStatus::Passed, json!({})),
        ];
        assert_eq!(derive_status(&p, &full), ProofStatus::Passed);
    }

    #[test]
    fn passed_is_not_gameable_by_noop_command() {
        let p = pack(WorkItemKind::Session);
        // a passing but non-test command does not satisfy CodeChange
        let arts = vec![
            art(ProofArtifactKind::Diff, "diff", ProofArtifactStatus::Info, json!({})),
            art(ProofArtifactKind::Command, "true", ProofArtifactStatus::Passed, json!({})),
        ];
        assert_eq!(derive_status(&p, &arts), ProofStatus::Partial);
    }

    #[test]
    fn review_passed_with_review_artifact() {
        let p = pack(WorkItemKind::Review);
        let clean = vec![art(ProofArtifactKind::Review, "review", ProofArtifactStatus::Passed, json!({}))];
        assert_eq!(derive_status(&p, &clean), ProofStatus::Passed);
        let unresolved = vec![art(ProofArtifactKind::Review, "review", ProofArtifactStatus::Failed, json!({}))];
        assert_eq!(derive_status(&p, &unresolved), ProofStatus::Failed);
    }

    #[test]
    fn workflow_passed_requires_approval_passed_if_present() {
        let p = pack(WorkItemKind::WorkflowRun);
        let approved = vec![
            art(ProofArtifactKind::Log, "node", ProofArtifactStatus::Passed, json!({})),
            art(ProofArtifactKind::Approval, "approval", ProofArtifactStatus::Passed, json!({})),
        ];
        assert_eq!(derive_status(&p, &approved), ProofStatus::Passed);
        // a pending approval -> not yet passed (partial)
        let pending = vec![
            art(ProofArtifactKind::Log, "node", ProofArtifactStatus::Passed, json!({})),
            art(ProofArtifactKind::Approval, "approval", ProofArtifactStatus::Pending, json!({})),
        ];
        assert_eq!(derive_status(&p, &pending), ProofStatus::Partial);
    }

    #[test]
    fn risk_clamps_and_responds_to_size_and_risky_files() {
        assert_eq!(compute_risk(&[]), 0);
        // size 40 (cap) + risky 32 (cap, 4 files) + migration 10 + review-failed 15
        // + untested 10 = 107 -> clamps to 100.
        let big = vec![
            art(
                ProofArtifactKind::Diff,
                "diff",
                ProofArtifactStatus::Info,
                json!({"additions": 100000, "deletions": 100000,
                       "risky_files": ["a/migrations/0077.sql", "b/auth.rs", "c/policy.rs", "d/Cargo.lock"]}),
            ),
            art(ProofArtifactKind::Review, "review", ProofArtifactStatus::Failed, json!({})),
        ];
        assert_eq!(compute_risk(&big), 100);
        // small change, no risky files, but untested -> +10
        let small = vec![art(
            ProofArtifactKind::Diff,
            "diff",
            ProofArtifactStatus::Info,
            json!({"additions": 10, "deletions": 0, "risky_files": []}),
        )];
        assert_eq!(compute_risk(&small), 10);
    }

    #[test]
    fn badges_cover_all_states() {
        // no proof
        let p = pack(WorkItemKind::Session);
        assert!(compute_badges(&p, &[]).contains(&ProofBadge::NoProof));

        // tests passed + ci missing (diff present, no ci)
        let mut pp = pack(WorkItemKind::Session);
        pp.status = ProofStatus::Passed;
        let arts = vec![
            art(ProofArtifactKind::Diff, "diff", ProofArtifactStatus::Info, json!({"risky_files": []})),
            art(ProofArtifactKind::Command, "cargo test", ProofArtifactStatus::Passed, json!({})),
        ];
        let b = compute_badges(&pp, &arts);
        assert!(b.contains(&ProofBadge::TestsPassed));
        assert!(b.contains(&ProofBadge::CiMissing));

        // tests failed
        let failed = vec![art(ProofArtifactKind::Command, "cargo test", ProofArtifactStatus::Failed, json!({}))];
        assert!(compute_badges(&p, &failed).contains(&ProofBadge::TestsFailed));

        // human approved
        let appr = vec![art(ProofArtifactKind::Approval, "a", ProofArtifactStatus::Passed, json!({}))];
        assert!(compute_badges(&p, &appr).contains(&ProofBadge::HumanApproved));

        // db/api verified requires Passed, not Info
        let db_info = vec![art(ProofArtifactKind::Db, "q", ProofArtifactStatus::Info, json!({}))];
        assert!(!compute_badges(&p, &db_info).contains(&ProofBadge::DbApiVerified));
        let db_pass = vec![art(ProofArtifactKind::Db, "q", ProofArtifactStatus::Passed, json!({}))];
        assert!(compute_badges(&p, &db_pass).contains(&ProofBadge::DbApiVerified));

        // review unresolved
        let rev = vec![art(ProofArtifactKind::Review, "r", ProofArtifactStatus::Failed, json!({}))];
        assert!(compute_badges(&p, &rev).contains(&ProofBadge::ReviewUnresolved));

        // risky
        let risky = vec![art(ProofArtifactKind::Diff, "d", ProofArtifactStatus::Info, json!({"risky_files": ["a/migrations/x.sql"]}))];
        assert!(compute_badges(&p, &risky).contains(&ProofBadge::RiskyChange));

        // waived
        let mut wp = pack(WorkItemKind::Session);
        wp.status = ProofStatus::Waived;
        wp.waived_by = Some("u".into());
        assert!(compute_badges(&wp, &[]).contains(&ProofBadge::Waived));
    }

    #[test]
    fn preview_caps() {
        let short = "hello";
        assert_eq!(preview(short), ("hello".to_string(), false));
        let long = "x".repeat(PREVIEW_CAP + 100);
        let (pv, trunc) = preview(&long);
        assert!(trunc);
        assert!(pv.len() <= PREVIEW_CAP + 4);
    }

    // -- v2 ---------------------------------------------------------------

    #[test]
    fn evidence_status_mappers() {
        assert_eq!(ci_artifact_status("success"), ProofArtifactStatus::Passed);
        assert_eq!(ci_artifact_status("FAILURE"), ProofArtifactStatus::Failed);
        assert_eq!(ci_artifact_status("in_progress"), ProofArtifactStatus::Pending);
        assert_eq!(ci_artifact_status("none"), ProofArtifactStatus::Info);
        assert_eq!(ci_artifact_status("weird"), ProofArtifactStatus::Info);

        assert_eq!(http_evidence_status(200), ProofArtifactStatus::Passed);
        assert_eq!(http_evidence_status(204), ProofArtifactStatus::Passed);
        assert_eq!(http_evidence_status(404), ProofArtifactStatus::Failed);
        assert_eq!(http_evidence_status(500), ProofArtifactStatus::Failed);
        assert_eq!(http_evidence_status(301), ProofArtifactStatus::Info);
        assert_eq!(http_evidence_status(0), ProofArtifactStatus::Failed);

        assert_eq!(read_evidence_status(false), ProofArtifactStatus::Passed);
        assert_eq!(read_evidence_status(true), ProofArtifactStatus::Failed);
    }

    #[test]
    fn new_badges() {
        let p = pack(WorkItemKind::Session);
        // ci passed/failed/pending
        let ci_ok = vec![art(ProofArtifactKind::Ci, "CI", ProofArtifactStatus::Passed, json!({}))];
        assert!(compute_badges(&p, &ci_ok).contains(&ProofBadge::CiPassed));
        let ci_bad = vec![art(ProofArtifactKind::Ci, "CI", ProofArtifactStatus::Failed, json!({}))];
        assert!(compute_badges(&p, &ci_bad).contains(&ProofBadge::CiFailed));
        let ci_pend = vec![art(ProofArtifactKind::Ci, "CI", ProofArtifactStatus::Pending, json!({}))];
        assert!(compute_badges(&p, &ci_pend).contains(&ProofBadge::CiPending));
        // ui verified
        let ui = vec![art(ProofArtifactKind::Screenshot, "shot", ProofArtifactStatus::Info, json!({}))];
        assert!(compute_badges(&p, &ui).contains(&ProofBadge::UiVerified));
        let vid = vec![art(ProofArtifactKind::Video, "clip", ProofArtifactStatus::Info, json!({}))];
        assert!(compute_badges(&p, &vid).contains(&ProofBadge::UiVerified));
        // kafka joins data-verified
        let kafka = vec![art(ProofArtifactKind::Kafka, "topic", ProofArtifactStatus::Passed, json!({}))];
        assert!(compute_badges(&p, &kafka).contains(&ProofBadge::DbApiVerified));
        // pr inconsistent
        let pr = vec![art(ProofArtifactKind::PrCheck, "pr", ProofArtifactStatus::Failed, json!({}))];
        assert!(compute_badges(&p, &pr).contains(&ProofBadge::PrInconsistent));
    }

    /// The regression invariant: with the default (no-extra) policy,
    /// `derive_status_with_policy == derive_status` for every fixture.
    #[test]
    fn policy_default_equals_legacy() {
        let kinds = [
            WorkItemKind::Session,
            WorkItemKind::GoalLoop,
            WorkItemKind::Task,
            WorkItemKind::Review,
            WorkItemKind::WorkflowRun,
            WorkItemKind::Manual,
        ];
        let fixtures: Vec<Vec<ProofArtifact>> = vec![
            vec![],
            vec![art(ProofArtifactKind::Diff, "diff", ProofArtifactStatus::Info, json!({}))],
            vec![
                art(ProofArtifactKind::Diff, "diff", ProofArtifactStatus::Info, json!({})),
                art(ProofArtifactKind::Command, "cargo test", ProofArtifactStatus::Passed, json!({})),
            ],
            vec![art(ProofArtifactKind::Command, "cargo test", ProofArtifactStatus::Failed, json!({}))],
            vec![art(ProofArtifactKind::Review, "rev", ProofArtifactStatus::Passed, json!({}))],
            vec![art(ProofArtifactKind::Review, "rev", ProofArtifactStatus::Failed, json!({}))],
            vec![
                art(ProofArtifactKind::Log, "node", ProofArtifactStatus::Passed, json!({})),
                art(ProofArtifactKind::Approval, "ap", ProofArtifactStatus::Pending, json!({})),
            ],
        ];
        for k in kinds {
            let p = pack(k);
            let pol = DoneContractPolicy::for_kind(k);
            for arts in &fixtures {
                assert_eq!(
                    derive_status_with_policy(&p, arts, &pol),
                    derive_status(&p, arts),
                    "kind {:?} drifted from legacy",
                    k
                );
            }
        }
    }

    #[test]
    fn policy_require_ci_caps_passed_to_partial() {
        // A code change that would legacy-pass (diff + passing test) but the repo
        // requires CI and there's no green CI → capped to Partial.
        let p = pack(WorkItemKind::Session);
        let arts = vec![
            art(ProofArtifactKind::Diff, "diff", ProofArtifactStatus::Info, json!({})),
            art(ProofArtifactKind::Command, "cargo test", ProofArtifactStatus::Passed, json!({})),
        ];
        assert_eq!(derive_status(&p, &arts), ProofStatus::Passed);
        let pol = DoneContractPolicy::default().with_repo(&RepoProofConfig {
            require_ci: true,
            ..Default::default()
        });
        assert_eq!(derive_status_with_policy(&p, &arts, &pol), ProofStatus::Partial);
        // add green CI → back to passed
        let mut with_ci = arts.clone();
        with_ci.push(art(ProofArtifactKind::Ci, "CI", ProofArtifactStatus::Passed, json!({})));
        assert_eq!(derive_status_with_policy(&p, &with_ci, &pol), ProofStatus::Passed);
    }

    #[test]
    fn policy_never_relaxes_failed_or_missing() {
        let p = pack(WorkItemKind::Session);
        let pol = DoneContractPolicy::default().with_repo(&RepoProofConfig {
            require_test: true,
            require_ci: true,
            ..Default::default()
        });
        // failed stays failed
        let failed = vec![art(ProofArtifactKind::Command, "cargo test", ProofArtifactStatus::Failed, json!({}))];
        assert_eq!(derive_status_with_policy(&p, &failed, &pol), ProofStatus::Failed);
        // missing stays missing
        assert_eq!(derive_status_with_policy(&p, &[], &pol), ProofStatus::Missing);
    }

    #[test]
    fn done_contract_scoring() {
        let pol = DoneContractPolicy::default();
        // Empty session pack: required diff+tests+no_failures, none satisfied → 0.
        let p = pack(WorkItemKind::Session);
        let empty = compute_done_contract(&p, &[], &pol);
        assert_eq!(empty.score, 0);
        assert!(empty.required >= 3);

        // diff + passing test → required (diff15+tests25+no_failures20=60) all met → 100.
        let full = vec![
            art(ProofArtifactKind::Diff, "diff", ProofArtifactStatus::Info, json!({})),
            art(ProofArtifactKind::Command, "cargo test", ProofArtifactStatus::Passed, json!({})),
        ];
        let c = compute_done_contract(&p, &full, &pol);
        assert_eq!(c.score, 100);

        // diff only → satisfied required = diff(15)+no_failures(20)=35 of 60
        // → round(35*100/60) = 58.
        let diff_only = vec![art(ProofArtifactKind::Diff, "diff", ProofArtifactStatus::Info, json!({}))];
        let c2 = compute_done_contract(&p, &diff_only, &pol);
        assert_eq!(c2.score, 58u8);

        // waived → 100 regardless.
        let mut wp = pack(WorkItemKind::Session);
        wp.waived_by = Some("u".into());
        assert_eq!(compute_done_contract(&wp, &[], &pol).score, 100);
    }

    #[test]
    fn done_contract_extra_requirements_count() {
        // require_ci makes 'ci' a required item.
        let p = pack(WorkItemKind::Manual);
        let base = compute_done_contract(&p, &[], &DoneContractPolicy::default());
        let strict = compute_done_contract(
            &p,
            &[],
            &DoneContractPolicy::default().with_repo(&RepoProofConfig {
                require_ci: true,
                require_test: true,
                ..Default::default()
            }),
        );
        assert!(strict.required > base.required);
    }

    #[test]
    fn pr_consistency_good_and_false_claim() {
        // A good description that references the change and has testing notes.
        let good = check_pr_consistency(&PrConsistencyInput {
            title: "Add proof snapshots".into(),
            description: "This adds immutable snapshots to the proof module. \
                          Testing: ran cargo test for otto-core."
                .into(),
            files_changed: vec!["crates/otto-core/src/proof.rs".into()],
            additions: 120,
            deletions: 4,
            has_passing_tests: true,
            has_failing_tests: false,
        });
        assert!(good.passed, "score {} checks {:?}", good.score, good.checks);
        assert!(!good.hard_fail);

        // A false "tests pass" claim with failing tests → hard fail.
        let liar = check_pr_consistency(&PrConsistencyInput {
            title: "Fix the bug".into(),
            description: "Fixed it in proof.rs. All tests pass now and CI is green.".into(),
            files_changed: vec!["crates/otto-core/src/proof.rs".into()],
            additions: 5,
            deletions: 1,
            has_passing_tests: false,
            has_failing_tests: true,
        });
        assert!(liar.hard_fail);
        assert!(!liar.passed);
    }

    #[test]
    fn pr_consistency_unmentioned_change_lowers_score() {
        let r = check_pr_consistency(&PrConsistencyInput {
            title: "misc".into(),
            description: "Some unrelated prose that never names the files at all here.".into(),
            files_changed: vec!["crates/otto-keychain/src/lib.rs".into()],
            additions: 50,
            deletions: 0,
            has_passing_tests: false,
            has_failing_tests: false,
        });
        // mentions_change (25) fails → score capped below threshold path.
        assert!(r.checks.iter().any(|c| c.key == "mentions_change" && !c.passed));
    }

    #[test]
    fn hashing_is_stable_and_order_independent() {
        assert_eq!(content_sha256("hello"), content_sha256("hello"));
        assert_ne!(content_sha256("hello"), content_sha256("world"));
        let a = json!({"b": 1, "a": [1, 2, {"y": 9, "x": 8}]});
        let b = json!({"a": [1, 2, {"x": 8, "y": 9}], "b": 1});
        assert_eq!(bundle_sha256(&a), bundle_sha256(&b));
        let c = json!({"a": [1, 2, {"x": 8, "y": 10}], "b": 1});
        assert_ne!(bundle_sha256(&a), bundle_sha256(&c));
    }

    #[test]
    fn reports_render_key_sections() {
        let mut p = pack(WorkItemKind::Session);
        p.title = "Stricter proof".into();
        p.status = ProofStatus::Passed;
        let arts = vec![art(
            ProofArtifactKind::Command,
            "cargo test --workspace",
            ProofArtifactStatus::Passed,
            json!({}),
        )];
        let contract = compute_done_contract(&p, &arts, &DoneContractPolicy::default());
        let badges = vec!["tests_passed".to_string()];
        let view = ReportView {
            pack: &p,
            artifacts: &arts,
            contract: &contract,
            badges: &badges,
            generated_at: "2026-06-26T12:00:00Z",
        };
        let md = render_report_md(&view);
        assert!(md.contains("# Proof Pack"));
        assert!(md.contains("Done contract"));
        assert!(md.contains("cargo test --workspace"));
        let html = render_report_html(&view);
        assert!(html.starts_with("<!doctype html>"));
        assert!(html.contains("Done contract"));
        // HTML escaping of angle brackets in content.
        let mut p2 = pack(WorkItemKind::Manual);
        p2.summary = "<script>alert(1)</script>".into();
        let c2 = compute_done_contract(&p2, &[], &DoneContractPolicy::default());
        let v2 = ReportView { pack: &p2, artifacts: &[], contract: &c2, badges: &[], generated_at: "t" };
        let h2 = render_report_html(&v2);
        assert!(!h2.contains("<script>alert(1)</script>"));
        assert!(h2.contains("&lt;script&gt;"));
    }
}
