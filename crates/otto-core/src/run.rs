//! Run with Otto — the flagship "one button" flow.
//!
//! An [`OttoRun`] is the source-agnostic unit of work: a source item (Jira /
//! Confluence / GitHub issue or PR / Slack or Telegram thread / Product task /
//! Review finding / Failing test / Scheduled-task report) is normalized into a run
//! and driven through a fixed pipeline whose stages ARE the [`RunStatus`] values.
//!
//! Everything here is pure (no I/O): the state machine, the enum string forms, and
//! the free-text [`parse_source_ref`] auto-detector. The engine in `otto-server`
//! cannot drift from this ordering because it advances via [`RunStatus::next_on_success`].

use serde::{Deserialize, Serialize};

use crate::id::Id;
use chrono::{DateTime, Utc};

/// The pipeline stage machine. The `status` column on a run is exactly one of these.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    /// Created, not yet picked up.
    Queued,
    /// Fetching + normalizing the source into a `ResolvedSource`.
    ResolvingSource,
    /// Assembling the Context Packet (the task prompt).
    BuildingContext,
    /// Creating the isolated branch/worktree.
    Provisioning,
    /// The agent (single) or goal loop is working.
    Executing,
    /// Assembling the Proof Pack from the produced diff.
    Proving,
    /// Running AI review on the branch.
    Reviewing,
    /// Paused for human approval (the only pause point).
    AwaitingApproval,
    /// Generating the PR draft (+ best-effort push).
    DraftingPr,
    /// Done — a PR draft (and optionally an opened PR) is ready.
    Completed,
    /// A stage errored; `error` carries the reason.
    Failed,
    /// A human rejected the change at the approval gate.
    Rejected,
    /// Cancelled by a user.
    Cancelled,
}

impl RunStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::ResolvingSource => "resolving_source",
            Self::BuildingContext => "building_context",
            Self::Provisioning => "provisioning",
            Self::Executing => "executing",
            Self::Proving => "proving",
            Self::Reviewing => "reviewing",
            Self::AwaitingApproval => "awaiting_approval",
            Self::DraftingPr => "drafting_pr",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Rejected => "rejected",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "queued" => Self::Queued,
            "resolving_source" => Self::ResolvingSource,
            "building_context" => Self::BuildingContext,
            "provisioning" => Self::Provisioning,
            "executing" => Self::Executing,
            "proving" => Self::Proving,
            "reviewing" => Self::Reviewing,
            "awaiting_approval" => Self::AwaitingApproval,
            "drafting_pr" => Self::DraftingPr,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "rejected" => Self::Rejected,
            "cancelled" => Self::Cancelled,
            _ => return None,
        })
    }

    /// The next stage on success. `None` for terminal states AND for
    /// `AwaitingApproval` (which only advances on an explicit approve).
    pub fn next_on_success(&self) -> Option<RunStatus> {
        Some(match self {
            Self::Queued => Self::ResolvingSource,
            Self::ResolvingSource => Self::BuildingContext,
            Self::BuildingContext => Self::Provisioning,
            Self::Provisioning => Self::Executing,
            Self::Executing => Self::Proving,
            Self::Proving => Self::Reviewing,
            Self::Reviewing => Self::AwaitingApproval,
            // AwaitingApproval is resolved by approve/reject, not by the engine.
            Self::AwaitingApproval => return None,
            Self::DraftingPr => Self::Completed,
            Self::Completed | Self::Failed | Self::Rejected | Self::Cancelled => return None,
        })
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Rejected | Self::Cancelled
        )
    }

    pub fn is_active(&self) -> bool {
        !self.is_terminal()
    }

    /// On daemon restart, may this stage be safely re-driven? Stages that spawned
    /// live background work (agent PTYs / review agents) that no longer exists —
    /// `Executing`, `Reviewing` — must NOT be re-driven (mirrors goal_loop
    /// `fail_running`); they are failed instead. `AwaitingApproval` is left alone
    /// (it waits for a human). The short, idempotent stages re-drive cleanly.
    pub fn is_resumable_on_boot(&self) -> bool {
        matches!(
            self,
            Self::Queued
                | Self::ResolvingSource
                | Self::BuildingContext
                | Self::Provisioning
                | Self::Proving
                | Self::DraftingPr
        )
    }
}

/// Where the source item came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Jira,
    Confluence,
    GithubPr,
    GithubIssue,
    Channel,
    ProductStory,
    Finding,
    Test,
    ScheduledReport,
}

impl SourceKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Jira => "jira",
            Self::Confluence => "confluence",
            Self::GithubPr => "github_pr",
            Self::GithubIssue => "github_issue",
            Self::Channel => "channel",
            Self::ProductStory => "product_story",
            Self::Finding => "finding",
            Self::Test => "test",
            Self::ScheduledReport => "scheduled_report",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "jira" => Self::Jira,
            "confluence" => Self::Confluence,
            "github_pr" => Self::GithubPr,
            "github_issue" => Self::GithubIssue,
            "channel" => Self::Channel,
            "product_story" => Self::ProductStory,
            "finding" => Self::Finding,
            "test" => Self::Test,
            "scheduled_report" => Self::ScheduledReport,
            _ => return None,
        })
    }
}

/// How the run executes the change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunMode {
    /// A single headless agent on an `otto-run/<id>` worktree (default).
    #[default]
    SingleAgent,
    /// A full goal loop (Plan→Execute→Evaluate→Digest) on a `goal-loop/<id>` worktree.
    GoalLoop,
}

impl RunMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SingleAgent => "single_agent",
            Self::GoalLoop => "goal_loop",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "single_agent" => Self::SingleAgent,
            "goal_loop" => Self::GoalLoop,
            _ => return None,
        })
    }
}

/// Which surface launched the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunOrigin {
    Slack,
    Telegram,
    Webhook,
    Ui,
    Mcp,
    Api,
}

impl RunOrigin {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Slack => "slack",
            Self::Telegram => "telegram",
            Self::Webhook => "webhook",
            Self::Ui => "ui",
            Self::Mcp => "mcp",
            Self::Api => "api",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "slack" => Self::Slack,
            "telegram" => Self::Telegram,
            "webhook" => Self::Webhook,
            "ui" => Self::Ui,
            "mcp" => Self::Mcp,
            "api" => Self::Api,
            _ => return None,
        })
    }
}

/// One run row (read DTO; mirrors the `otto_runs` table).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OttoRun {
    pub id: Id,
    pub workspace_id: Id,
    pub title: String,
    pub source_kind: SourceKind,
    pub source_ref: String,
    pub source_url: Option<String>,
    pub goal: String,
    pub mode: RunMode,
    pub provider: String,
    pub repo_id: Option<String>,
    pub repo_path: Option<String>,
    pub base_branch: Option<String>,
    pub branch: Option<String>,
    pub worktree_path: Option<String>,
    pub base_commit: Option<String>,
    pub status: RunStatus,
    pub error: Option<String>,
    pub origin_kind: RunOrigin,
    pub origin_chat: Option<String>,
    pub origin_thread: Option<String>,
    pub origin_user: Option<String>,
    pub callback_url: Option<String>,
    pub goal_loop_id: Option<String>,
    pub review_id: Option<String>,
    pub proof_pack_id: Option<String>,
    pub proof_status: Option<String>,
    pub risk_score: Option<i64>,
    pub findings_total: i64,
    pub findings_blocking: i64,
    pub pr_draft_json: Option<String>,
    pub pr_url: Option<String>,
    pub auto_open_pr: bool,
    pub approval_decision: Option<String>,
    pub approved_by: Option<String>,
    pub approved_at: Option<String>,
    pub result_summary: Option<String>,
    pub context_summary: Option<String>,
    pub created_by: Id,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// One timeline entry for a run (audit + the Slack/feed source).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunEvent {
    pub id: Id,
    pub run_id: Id,
    pub workspace_id: Id,
    pub kind: String,
    pub status: Option<String>,
    pub message: String,
    pub detail: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
}

/// Request to launch a run (REST / webhook / Slack / UI all build one of these).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct LaunchRunReq {
    /// Explicit source kind; if absent we auto-detect from `source_ref`/`url`.
    #[serde(default)]
    pub source_kind: Option<SourceKind>,
    /// The handle or free-text ref (Jira key, page id, finding id, …).
    #[serde(default)]
    pub source_ref: Option<String>,
    /// A full URL (GitHub/Confluence); auto-detected when present.
    #[serde(default)]
    pub url: Option<String>,
    /// For channel-triggered runs: the message text that seeds the goal.
    #[serde(default)]
    pub seed_text: Option<String>,
    #[serde(default)]
    pub mode: Option<RunMode>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub repo_id: Option<String>,
    #[serde(default)]
    pub auto_open_pr: Option<bool>,
    /// Optional explicit title; otherwise derived from the source.
    #[serde(default)]
    pub title: Option<String>,
}

/// The normalized source content an adapter produces.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedSource {
    pub title: String,
    pub body_md: String,
    pub goal: String,
    pub source_url: Option<String>,
    pub repo_hint: Option<String>,
    pub metadata: serde_json::Value,
}

/// Approve/reject a run at the approval gate.
#[derive(Debug, Clone, Deserialize)]
pub struct ApproveRunReq {
    /// `"approve"` or `"reject"`.
    pub decision: String,
    #[serde(default)]
    pub note: Option<String>,
}

/// Pure free-text source detector. Returns `(kind, canonical_ref, url?)`.
///
/// - `PROJ-123`            → Jira (ref = the key)
/// - `…/pull/42`           → GithubPr (ref = `owner/repo#42`, url = full)
/// - `…/issues/42`         → GithubIssue (ref = `owner/repo#42`, url = full)
/// - `…/pages/12345`       → Confluence (ref = the page id, url = full)
/// - `finding:<id>` etc.   → explicit-prefixed kinds (ref = the id)
pub fn parse_source_ref(input: &str) -> Option<(SourceKind, String, Option<String>)> {
    let s = input.trim();
    if s.is_empty() {
        return None;
    }

    // Explicit prefixes always win.
    for (prefix, kind) in [
        ("finding:", SourceKind::Finding),
        ("story:", SourceKind::ProductStory),
        ("test:", SourceKind::Test),
        ("report:", SourceKind::ScheduledReport),
        ("jira:", SourceKind::Jira),
        ("confluence:", SourceKind::Confluence),
    ] {
        if let Some(rest) = s.strip_prefix(prefix) {
            let rest = rest.trim();
            if rest.is_empty() {
                return None;
            }
            return Some((kind, rest.to_string(), None));
        }
    }

    // GitHub PR / issue URL.
    if let Some(rest) = s.split("github.com/").nth(1) {
        let parts: Vec<&str> = rest.split('/').collect();
        if parts.len() >= 4 {
            let owner = parts[0];
            let repo = parts[1];
            let kind = match parts[2] {
                "pull" => Some(SourceKind::GithubPr),
                "issues" => Some(SourceKind::GithubIssue),
                _ => None,
            };
            if let Some(kind) = kind {
                let num: String = parts[3].chars().take_while(|c| c.is_ascii_digit()).collect();
                if !owner.is_empty() && !repo.is_empty() && !num.is_empty() {
                    return Some((kind, format!("{owner}/{repo}#{num}"), Some(s.to_string())));
                }
            }
        }
    }

    // Confluence page URL (`/pages/<id>`).
    if s.contains("/pages/") {
        if let Some(rest) = s.split("/pages/").nth(1) {
            let id: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if !id.is_empty() {
                return Some((SourceKind::Confluence, id, Some(s.to_string())));
            }
        }
    }

    // Bare Jira key: UPPER-ALNUM project + '-' + digits.
    if is_jira_key(s) {
        return Some((SourceKind::Jira, s.to_string(), None));
    }

    None
}

fn is_jira_key(s: &str) -> bool {
    let Some((proj, num)) = s.split_once('-') else {
        return false;
    };
    !proj.is_empty()
        && proj.chars().next().is_some_and(|c| c.is_ascii_uppercase())
        && proj
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        && !num.is_empty()
        && num.chars().all(|c| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn happy_path_ordering_is_total_and_stops_at_approval() {
        // The full success chain from Queued lands at AwaitingApproval, which has
        // no auto-next (it waits for a human).
        let mut s = RunStatus::Queued;
        let mut seen = vec![s];
        while let Some(next) = s.next_on_success() {
            s = next;
            seen.push(s);
            assert!(seen.len() < 20, "cycle in next_on_success");
        }
        assert_eq!(s, RunStatus::AwaitingApproval);
        assert_eq!(
            seen,
            vec![
                RunStatus::Queued,
                RunStatus::ResolvingSource,
                RunStatus::BuildingContext,
                RunStatus::Provisioning,
                RunStatus::Executing,
                RunStatus::Proving,
                RunStatus::Reviewing,
                RunStatus::AwaitingApproval,
            ]
        );
        // Past the gate: DraftingPr → Completed.
        assert_eq!(
            RunStatus::DraftingPr.next_on_success(),
            Some(RunStatus::Completed)
        );
        assert_eq!(RunStatus::Completed.next_on_success(), None);
    }

    #[test]
    fn terminal_and_boot_classification() {
        for t in [
            RunStatus::Completed,
            RunStatus::Failed,
            RunStatus::Rejected,
            RunStatus::Cancelled,
        ] {
            assert!(t.is_terminal());
            assert!(!t.is_active());
            assert!(!t.is_resumable_on_boot());
        }
        // Live-work stages must NOT be re-driven on boot.
        assert!(!RunStatus::Executing.is_resumable_on_boot());
        assert!(!RunStatus::Reviewing.is_resumable_on_boot());
        assert!(!RunStatus::AwaitingApproval.is_resumable_on_boot());
        // Short idempotent stages re-drive.
        for r in [
            RunStatus::Queued,
            RunStatus::ResolvingSource,
            RunStatus::BuildingContext,
            RunStatus::Provisioning,
            RunStatus::Proving,
            RunStatus::DraftingPr,
        ] {
            assert!(r.is_resumable_on_boot());
            assert!(r.is_active());
        }
    }

    #[test]
    fn status_roundtrips() {
        for s in [
            RunStatus::Queued,
            RunStatus::ResolvingSource,
            RunStatus::BuildingContext,
            RunStatus::Provisioning,
            RunStatus::Executing,
            RunStatus::Proving,
            RunStatus::Reviewing,
            RunStatus::AwaitingApproval,
            RunStatus::DraftingPr,
            RunStatus::Completed,
            RunStatus::Failed,
            RunStatus::Rejected,
            RunStatus::Cancelled,
        ] {
            assert_eq!(RunStatus::parse(s.as_str()), Some(s));
        }
        assert_eq!(RunStatus::parse("nope"), None);
    }

    #[test]
    fn enum_roundtrips() {
        for k in [
            SourceKind::Jira,
            SourceKind::Confluence,
            SourceKind::GithubPr,
            SourceKind::GithubIssue,
            SourceKind::Channel,
            SourceKind::ProductStory,
            SourceKind::Finding,
            SourceKind::Test,
            SourceKind::ScheduledReport,
        ] {
            assert_eq!(SourceKind::parse(k.as_str()), Some(k));
        }
        for m in [RunMode::SingleAgent, RunMode::GoalLoop] {
            assert_eq!(RunMode::parse(m.as_str()), Some(m));
        }
        assert_eq!(RunMode::default(), RunMode::SingleAgent);
        for o in [
            RunOrigin::Slack,
            RunOrigin::Telegram,
            RunOrigin::Webhook,
            RunOrigin::Ui,
            RunOrigin::Mcp,
            RunOrigin::Api,
        ] {
            assert_eq!(RunOrigin::parse(o.as_str()), Some(o));
        }
    }

    #[test]
    fn parse_source_ref_detects_each_kind() {
        assert_eq!(
            parse_source_ref("PROJ-123"),
            Some((SourceKind::Jira, "PROJ-123".into(), None))
        );
        assert_eq!(
            parse_source_ref("  AB12-7 "),
            Some((SourceKind::Jira, "AB12-7".into(), None))
        );
        // not a jira key
        assert_eq!(parse_source_ref("hello-world"), None);
        assert_eq!(parse_source_ref("proj-1"), None); // lowercase project

        let pr = parse_source_ref("https://github.com/acme/widgets/pull/42").unwrap();
        assert_eq!(pr.0, SourceKind::GithubPr);
        assert_eq!(pr.1, "acme/widgets#42");
        assert!(pr.2.is_some());

        let iss = parse_source_ref("https://github.com/acme/widgets/issues/9").unwrap();
        assert_eq!(iss.0, SourceKind::GithubIssue);
        assert_eq!(iss.1, "acme/widgets#9");

        let conf =
            parse_source_ref("https://x.atlassian.net/wiki/spaces/ENG/pages/65540/Spec").unwrap();
        assert_eq!(conf.0, SourceKind::Confluence);
        assert_eq!(conf.1, "65540");

        assert_eq!(
            parse_source_ref("finding:01HF"),
            Some((SourceKind::Finding, "01HF".into(), None))
        );
        assert_eq!(
            parse_source_ref("story:abc"),
            Some((SourceKind::ProductStory, "abc".into(), None))
        );
        assert_eq!(
            parse_source_ref("test:run1"),
            Some((SourceKind::Test, "run1".into(), None))
        );
        assert_eq!(
            parse_source_ref("report:r9"),
            Some((SourceKind::ScheduledReport, "r9".into(), None))
        );
        assert_eq!(parse_source_ref("   "), None);
    }
}
