//! Pure normalization: the "policy" (risk) heuristic. Status normalization lives
//! on [`otto_state::WorkStatus::from_source`] (kept next to the enum it produces).

use otto_state::{RiskLevel, WorkKind};

/// Sensitive keywords that bump any work item to [`RiskLevel::High`] — the
/// "policy" axis Mission Control surfaces. A reviewer/operator can still override
/// via PATCH (including up to [`RiskLevel::Critical`]).
const SENSITIVE: &[&str] = &[
    "security",
    "payment",
    "auth",
    "secret",
    "credential",
    "wallet",
    "compliance",
    "fraud",
    "prod",
    "production",
    "deploy",
    "migration",
];

/// Default risk for a freshly-derived work item, from its kind + title. Applied
/// on CREATE only (it is human-governable thereafter).
pub fn risk(kind: WorkKind, title: &str) -> RiskLevel {
    let t = title.to_ascii_lowercase();
    if SENSITIVE.iter().any(|k| t.contains(k)) {
        return RiskLevel::High;
    }
    match kind {
        WorkKind::GoalLoop
        | WorkKind::Swarm
        | WorkKind::Pr
        | WorkKind::Session
        | WorkKind::ExternalTrigger
        | WorkKind::Review => RiskLevel::Medium,
        WorkKind::Workflow | WorkKind::ProductStory => RiskLevel::Low,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sensitive_titles_are_high() {
        assert_eq!(
            risk(WorkKind::Review, "Security review of payment changes"),
            RiskLevel::High
        );
        assert_eq!(
            risk(WorkKind::Workflow, "Deploy to production"),
            RiskLevel::High
        );
    }

    #[test]
    fn defaults_by_kind() {
        assert_eq!(
            risk(WorkKind::Workflow, "Nightly repo health check"),
            RiskLevel::Low
        );
        assert_eq!(
            risk(WorkKind::GoalLoop, "Fix Kafka consumer test"),
            RiskLevel::Medium
        );
        assert_eq!(
            risk(WorkKind::Session, "Refactor login UI"),
            RiskLevel::Medium
        );
    }
}
