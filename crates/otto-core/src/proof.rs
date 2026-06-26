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

use crate::Id;

/// A pack with `risk_score >= RISKY_THRESHOLD` (or any risky file) earns the
/// `risky_change` badge.
pub const RISKY_THRESHOLD: u8 = 50;
/// Hard cap on stored artifact content (2 MiB). Larger content is truncated with
/// a trailing note and `metadata.truncated = true`.
pub const STORE_CAP: usize = 2 * 1024 * 1024;
/// Cap on the inline preview returned in list/detail responses (8 KiB).
pub const PREVIEW_CAP: usize = 8 * 1024;

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
    Diff,
    Ci,
    Api,
    Db,
    Review,
    Approval,
    SelfReview,
}

impl ProofArtifactKind {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "command" => Some(Self::Command),
            "log" => Some(Self::Log),
            "screenshot" => Some(Self::Screenshot),
            "diff" => Some(Self::Diff),
            "ci" => Some(Self::Ci),
            "api" => Some(Self::Api),
            "db" => Some(Self::Db),
            "review" => Some(Self::Review),
            "approval" => Some(Self::Approval),
            "self_review" => Some(Self::SelfReview),
            _ => None,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Command => "command",
            Self::Log => "log",
            Self::Screenshot => "screenshot",
            Self::Diff => "diff",
            Self::Ci => "ci",
            Self::Api => "api",
            Self::Db => "db",
            Self::Review => "review",
            Self::Approval => "approval",
            Self::SelfReview => "self_review",
        }
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
    DbApiVerified,
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
            Self::DbApiVerified => "db_api_verified",
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
    /// Optional parent pack (e.g. a goal-loop pack parenting the session/review/
    /// workflow packs it spawns).
    pub parent_pack_id: Option<Id>,
    pub waived_by: Option<Id>,
    pub waived_reason: Option<String>,
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

    // D10: requires an explicit Passed mark, not the default Info.
    if arts.iter().any(|a| {
        matches!(a.kind, ProofArtifactKind::Db | ProofArtifactKind::Api)
            && a.status == ProofArtifactStatus::Passed
    }) {
        out.push(ProofBadge::DbApiVerified);
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
            parent_pack_id: None,
            waived_by: None,
            waived_reason: None,
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
        for s in ["command", "log", "screenshot", "diff", "ci", "api", "db", "review", "approval", "self_review"] {
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
}
