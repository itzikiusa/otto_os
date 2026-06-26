//! Review-findings workflow domain types — the contract for turning multi-agent
//! code-review output into a tracked security/code-quality workflow that closes
//! the loop with evidence.
//!
//! Two axes live on a finding, with **disjoint writers**:
//!   * [`FindingStatus`] — the human WORKFLOW disposition (this file). Written only
//!     by the action endpoints; the engine never touches it.
//!   * the engine DETECTION lifecycle (`state`, owned by otto-state) — written only
//!     by the review engine. Exposed here read-only as `Finding::state` + a derived
//!     `regressed` flag.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::Id;

// ---------------------------------------------------------------------------
// Status — the 6-value workflow disposition + machine-checked transitions
// ---------------------------------------------------------------------------

/// The workflow status of a finding (the required 6 values).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingStatus {
    Open,
    Accepted,
    FalsePositive,
    Fixed,
    Verified,
    Waived,
}

impl FindingStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Accepted => "accepted",
            Self::FalsePositive => "false_positive",
            Self::Fixed => "fixed",
            Self::Verified => "verified",
            Self::Waived => "waived",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "open" => Self::Open,
            "accepted" => Self::Accepted,
            "false_positive" => Self::FalsePositive,
            "fixed" => Self::Fixed,
            "verified" => Self::Verified,
            "waived" => Self::Waived,
            _ => return None,
        })
    }

    /// Whether `self -> to` is a legal workflow transition (§15.2 of the design).
    ///
    /// `fix` enters via `* -> accepted`; the async fix-completion stamps
    /// `accepted -> fixed`. `verify` runs from {accepted, fixed, verified} and
    /// lands on `verified`. `reject` (approval) is the `* -> false_positive`
    /// override. Everything terminal can be reopened to `open`.
    pub fn can_transition(self, to: FindingStatus) -> bool {
        use FindingStatus::*;
        matches!(
            (self, to),
            // open
            (Open, Accepted)
                | (Open, FalsePositive)
                | (Open, Waived)
                // accepted
                | (Accepted, Fixed)
                | (Accepted, Verified)
                | (Accepted, FalsePositive)
                | (Accepted, Waived)
                | (Accepted, Open)
                // fixed
                | (Fixed, Verified)
                | (Fixed, FalsePositive)
                | (Fixed, Waived)
                | (Fixed, Open)
                // verified (re-verify is idempotent; FP override; reopen on regress)
                | (Verified, Verified)
                | (Verified, FalsePositive)
                | (Verified, Open)
                // terminal-ish → reopen only
                | (FalsePositive, Open)
                | (Waived, Open)
        )
    }
}

// ---------------------------------------------------------------------------
// Severity — normalized 5-level scale (total, unknown -> info)
// ---------------------------------------------------------------------------

/// Normalized finding severity. Reviewer agents emit a grab-bag of tokens
/// (`info|warn|bug`, `blocker|major|minor|nit`, `critical|high|…`); [`normalize`]
/// folds them all into these five, on both write and read. (§15.4)
///
/// [`normalize`]: FindingSeverity::normalize
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingSeverity {
    Critical,
    High,
    Medium,
    Low,
    Info,
}

impl FindingSeverity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Critical => "critical",
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
            Self::Info => "info",
        }
    }

    /// Total mapping from any reviewer token to a normalized severity. Unknown
    /// inputs fold to `info` (never panics, never fails to parse — fixes the
    /// legacy `'bug'/'warn'` rows that aren't enum members).
    pub fn normalize(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "critical" | "blocker" => Self::Critical,
            "bug" | "high" | "error" | "major" => Self::High,
            "warn" | "warning" | "medium" => Self::Medium,
            "minor" | "low" => Self::Low,
            _ => Self::Info, // "nit" | "info" | unknown
        }
    }
}

/// Normalize a finding's `(line, line_end)` anchor into a sane range before it is
/// persisted. Returns the cleaned `(line, line_end)`:
/// - a `line_end` without a `line` is meaningless → both dropped;
/// - a `line_end` that is missing, equal to, or before `line` collapses to a
///   single-line anchor (`line_end = None`);
/// - a genuine multi-line range (`line_end > line`) is preserved.
///
/// Keeps review findings honestly anchored so the board/Proof Pack never show an
/// inverted or phantom line range.
pub fn normalize_line_range(line: Option<u32>, line_end: Option<u32>) -> (Option<u32>, Option<u32>) {
    match (line, line_end) {
        (None, _) => (None, None),
        (Some(start), Some(end)) if end > start => (Some(start), Some(end)),
        (Some(start), _) => (Some(start), None),
    }
}

// ---------------------------------------------------------------------------
// Finding — the full workflow DTO (all 11 required fields + workflow state)
// ---------------------------------------------------------------------------

/// A review finding as the workflow tracks it. `finding_id` = `id`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub id: Id,
    pub review_id: Id,
    pub workspace_id: Id,
    pub repo_id: Id,
    pub pr_number: Option<u64>,
    pub fingerprint: String,
    // --- the 11 required fields ---
    pub severity: FindingSeverity,        // (2)
    pub category: Option<String>,         // (3)
    pub path: Option<String>,             // (4) file
    pub line: Option<u32>,                // (4) range start
    pub line_end: Option<u32>,            // (4) range end
    pub title: String,
    pub body: String,
    pub evidence: String,                 // (5)
    pub agent_reasoning_summary: String,  // (6)
    pub suggested_fix: Option<String>,    // (7)
    pub status: FindingStatus,            // (8)
    pub linked_commit: Option<String>,    // (9)
    pub linked_test: Option<String>,      // (10)
    pub reviewer: String,                 // (11) current disposition owner
    // --- workflow state / gates / artifacts ---
    /// Engine DETECTION lifecycle (read-only here): open|fixing|resolved|regressed|declined.
    pub state: String,
    /// Derived: the detection axis currently reads `regressed` (reappeared after closure).
    pub regressed: bool,
    pub requires_human_approval: bool,
    pub approval_decision: Option<String>,
    pub approved_by: Option<String>,
    pub approved_at: Option<String>,
    pub jira_key: Option<String>,
    pub jira_url: Option<String>,
    pub produced_by_agent: Option<String>,
    pub repo_rule_id: Option<Id>,
    pub fix_session_id: Option<Id>,
    pub occurrence_count: i64,
    pub created_at: String,
    pub updated_at: String,
}

/// One immutable audit-trail entry for a finding — the spine of "closing the loop
/// with evidence". One row per action / transition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FindingEvent {
    pub id: Id,
    pub finding_id: Id,
    pub kind: String,
    pub actor: String,
    pub from_status: Option<String>,
    pub to_status: Option<String>,
    pub detail: serde_json::Value,
    pub created_at: String,
}

/// A finding plus its full event timeline (the `GET /findings/{id}` response).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FindingDetail {
    pub finding: Finding,
    pub events: Vec<FindingEvent>,
}

/// The response of an action that may spawn a live agent session (fix / verify /
/// regression-test). `session_id` is present when an openable session was started.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FindingActionResp {
    pub finding: Finding,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<Id>,
}

// ---------------------------------------------------------------------------
// Repo rule — durable lesson fed into the Context Engine
// ---------------------------------------------------------------------------

/// A repo rule generalized from a finding, materialized into future agent
/// sessions' instruction files via the Context Engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoRule {
    pub id: Id,
    pub workspace_id: Id,
    pub title: String,
    pub body: String,
    pub category: Option<String>,
    pub severity: Option<String>,
    pub glob: Option<String>,
    pub source_finding_id: Option<Id>,
    pub enabled: bool,
    pub created_by: Id,
    pub created_at: String,
    pub updated_at: String,
}

// ---------------------------------------------------------------------------
// Proof Pack — the assembled evidence bundle (namespaced `Review*` to avoid the
// parallel feat/proof-packs branch's generic ProofPack).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReviewProofPackSummary {
    pub total: u64,
    pub by_status: BTreeMap<String, u64>,
    pub by_severity: BTreeMap<String, u64>,
    pub verified: u64,
    pub fixed: u64,
    pub open: u64,
    pub with_commit: u64,
    pub with_test: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewProofPackEntry {
    pub finding: Finding,
    pub events: Vec<FindingEvent>,
}

/// The live-assembled evidence bundle for a review.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewProofPack {
    pub review_id: Id,
    pub workspace_id: Id,
    pub generated_at: String,
    pub summary: ReviewProofPackSummary,
    pub findings: Vec<ReviewProofPackEntry>,
    pub repo_rules: Vec<RepoRule>,
}

/// The persisted-snapshot response of `POST /reviews/{id}/proof-pack/export`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewProofPackExport {
    pub id: Id,
    pub review_id: Id,
    pub format: String,
    pub markdown: String,
    pub created_at: String,
}

// ---------------------------------------------------------------------------
// Tests — the transition machine + severity normalization
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use FindingStatus::*;

    #[test]
    fn status_round_trips() {
        for s in [Open, Accepted, FalsePositive, Fixed, Verified, Waived] {
            assert_eq!(FindingStatus::parse(s.as_str()), Some(s));
        }
        assert_eq!(FindingStatus::parse("garbage"), None);
    }

    #[test]
    fn transition_table_matches_design_15_2() {
        // Legal edges (exact §15.2 set).
        let legal = [
            (Open, Accepted),
            (Open, FalsePositive),
            (Open, Waived),
            (Accepted, Fixed),
            (Accepted, Verified),
            (Accepted, FalsePositive),
            (Accepted, Waived),
            (Accepted, Open),
            (Fixed, Verified),
            (Fixed, FalsePositive),
            (Fixed, Waived),
            (Fixed, Open),
            (Verified, Verified),
            (Verified, FalsePositive),
            (Verified, Open),
            (FalsePositive, Open),
            (Waived, Open),
        ];
        for (a, b) in legal {
            assert!(a.can_transition(b), "{a:?} -> {b:?} should be legal");
        }
        // A few representative ILLEGAL edges.
        let illegal = [
            (Open, Fixed),            // fix goes via accepted, not directly
            (Open, Verified),         // can't verify an untouched finding
            (Waived, Verified),
            (Waived, Fixed),
            (FalsePositive, Fixed),
            (FalsePositive, Accepted),
            (Verified, Fixed),
            (Verified, Accepted),
        ];
        for (a, b) in illegal {
            assert!(!a.can_transition(b), "{a:?} -> {b:?} should be illegal");
        }
    }

    #[test]
    fn line_range_anchoring_is_sane() {
        // No start line → no range at all (an end without a start is meaningless).
        assert_eq!(normalize_line_range(None, None), (None, None));
        assert_eq!(normalize_line_range(None, Some(10)), (None, None));
        // Start only → single-line anchor.
        assert_eq!(normalize_line_range(Some(7), None), (Some(7), None));
        // Proper multi-line range survives.
        assert_eq!(normalize_line_range(Some(7), Some(12)), (Some(7), Some(12)));
        // Degenerate range (end == start) collapses to a single line.
        assert_eq!(normalize_line_range(Some(7), Some(7)), (Some(7), None));
        // Inverted range (end < start) is dropped, keeping the start.
        assert_eq!(normalize_line_range(Some(12), Some(7)), (Some(12), None));
    }

    #[test]
    fn severity_normalize_is_total() {
        use FindingSeverity::*;
        assert_eq!(FindingSeverity::normalize("critical"), Critical);
        assert_eq!(FindingSeverity::normalize("Blocker"), Critical);
        assert_eq!(FindingSeverity::normalize("bug"), High);
        assert_eq!(FindingSeverity::normalize("HIGH"), High);
        assert_eq!(FindingSeverity::normalize("error"), High);
        assert_eq!(FindingSeverity::normalize("major"), High);
        assert_eq!(FindingSeverity::normalize("warn"), Medium);
        assert_eq!(FindingSeverity::normalize(" warning "), Medium);
        assert_eq!(FindingSeverity::normalize("medium"), Medium);
        assert_eq!(FindingSeverity::normalize("minor"), Low);
        assert_eq!(FindingSeverity::normalize("low"), Low);
        assert_eq!(FindingSeverity::normalize("nit"), Info);
        assert_eq!(FindingSeverity::normalize("info"), Info);
        assert_eq!(FindingSeverity::normalize("something-unknown"), Info);
        // round-trips through as_str
        for sv in [Critical, High, Medium, Low, Info] {
            assert_eq!(FindingSeverity::normalize(sv.as_str()), sv);
        }
    }
}
