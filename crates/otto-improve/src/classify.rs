//! Decide whether a proposed edit auto-applies or queues for approval.
//!
//! Guardrail first, then the autonomy policy:
//!   * A SKILL edit is "allow-listed" only if its `target_ref` is in the
//!     workspace's `skill_allowlist`. A non-allow-listed skill edit ALWAYS
//!     queues (this is the empty-allow-list "propose-only" guarantee and the
//!     shared-skill blast-radius guard).
//!   * MEMORY edits are workspace-local and not gated by the skill allow-list;
//!     they follow the autonomy policy directly.
//!   * Then: auto => apply; propose => queue; tiered => apply iff risk == low.

use otto_core::domain::{Autonomy, ImprovementRisk, ImprovementTarget};

use crate::proposal::ProposedEdit;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    Apply,
    Queue,
}

/// Is this edit permitted to auto-apply by the allow-list guardrail?
pub fn allowlisted(edit: &ProposedEdit, skill_allowlist: &[String]) -> bool {
    match edit.target_type {
        ImprovementTarget::Memory => true,
        ImprovementTarget::Skill => skill_allowlist.iter().any(|s| s == &edit.target_ref),
    }
}

/// Final disposition for an edit.
pub fn decide(edit: &ProposedEdit, skill_allowlist: &[String], autonomy: Autonomy) -> Disposition {
    if !allowlisted(edit, skill_allowlist) {
        return Disposition::Queue;
    }
    match autonomy {
        Autonomy::Propose => Disposition::Queue,
        Autonomy::Auto => Disposition::Apply,
        Autonomy::Tiered => match edit.risk {
            ImprovementRisk::Low => Disposition::Apply,
            ImprovementRisk::Structural => Disposition::Queue,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proposal::{EditPatch, ProposedEdit};
    use otto_core::domain::{ImprovementEditKind, ImprovementRisk, ImprovementTarget};

    fn edit(target: ImprovementTarget, target_ref: &str, risk: ImprovementRisk) -> ProposedEdit {
        ProposedEdit {
            id: "e".into(),
            target_type: target,
            target_ref: target_ref.into(),
            kind: ImprovementEditKind::Add,
            risk,
            rationale: String::new(),
            evidence: vec![],
            dedup_checked: true,
            dedup_quote: None,
            patch: EditPatch { before: None, after: "x".into() },
        }
    }

    #[test]
    fn non_allowlisted_skill_always_queues_even_on_auto() {
        let e = edit(ImprovementTarget::Skill, "other-skill", ImprovementRisk::Low);
        assert_eq!(decide(&e, &[], Autonomy::Auto), Disposition::Queue);
    }

    #[test]
    fn allowlisted_low_skill_applies_on_tiered() {
        let list = vec!["support-triage-router".to_string()];
        let e = edit(ImprovementTarget::Skill, "support-triage-router", ImprovementRisk::Low);
        assert_eq!(decide(&e, &list, Autonomy::Tiered), Disposition::Apply);
    }

    #[test]
    fn allowlisted_structural_skill_queues_on_tiered() {
        let list = vec!["support-triage-router".to_string()];
        let e = edit(ImprovementTarget::Skill, "support-triage-router", ImprovementRisk::Structural);
        assert_eq!(decide(&e, &list, Autonomy::Tiered), Disposition::Queue);
    }

    #[test]
    fn memory_low_applies_on_tiered_without_allowlist() {
        let e = edit(ImprovementTarget::Memory, "MEMORY.md", ImprovementRisk::Low);
        assert_eq!(decide(&e, &[], Autonomy::Tiered), Disposition::Apply);
    }

    #[test]
    fn propose_queues_everything() {
        let list = vec!["support-triage-router".to_string()];
        let e = edit(ImprovementTarget::Skill, "support-triage-router", ImprovementRisk::Low);
        assert_eq!(decide(&e, &list, Autonomy::Propose), Disposition::Queue);
    }
}
