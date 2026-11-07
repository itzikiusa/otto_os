//! A review-to-fix goal for the existing loop engine, with explicit evidence criteria.
use otto_core::domain::*;

pub fn build(
    review: &Review,
    target: f64,
    provider: &str,
) -> Result<(GoalLoopDefinition, GoalLoopConfig), String> {
    if !target.is_finite() || !(1.0..=10.0).contains(&target) {
        return Err("Target score must be between 1 and 10".into());
    }
    let plan = review
        .scope_plan
        .as_ref()
        .ok_or("Re-run this legacy review to assess scope before fixing")?;
    let comments: Vec<_> = review
        .comments
        .iter()
        .filter(|c| c.state != CommentState::Declined)
        .collect();
    let mut criteria = Vec::new();
    for comment in &comments {
        criteria.push(AcceptanceCriterion {
            id: format!("finding-{}", comment.id),
            text: format!("Account for review finding {}", comment.id),
            verify: format!("Independently inspect current code for this original claim: {}. Record exactly fixed with file:line and actual check/trace evidence, still_open with concrete trigger, or not_relevant with causal scope reason. Never accept the executor's assertion alone. {}", comment.body, if comment.severity == CommentSeverity::Info { "Optional low-priority findings may stay open when explicitly deferred with reason and cost; they are accounted for and do not block the target." } else { "A must-fix or should-fix finding still_open must not pass this criterion." }),
            verify_kind: "agent".into(), verify_cmd: None,
        });
    }
    for (id, text, verify) in [
        ("fresh-review", "A fresh independent review has no open must-fix or should-fix issues and no unaccounted findings", "Re-read the full causal diff and affected consumers after fixes, through every selected relevant lens. Verify every previously reported issue and each newly found issue; record dispositions and file:line evidence. Questions and optional lows must be explicitly accounted for, not silently discarded. Do not equate missing output to verification.".to_string()),
        ("critical-checks", "Relevant critical-path checks pass with recorded evidence", "Run the relevant repository checks for changed critical paths. Record exact commands, exit status and output summary. Tests must fail for the original defect; do not demand exhaustive hypothetical branch coverage. Skipped or unavailable material checks leave this criterion unmet.".to_string()),
        ("quality-target", "Evidence-backed reviewer assessment meets the configured target", format!("Give an explicitly subjective score out of 10 per selected lens, with evidence and remaining limitations, then an overall assessment. Target {target:.1}/10. The score is reviewer judgement, NOT an objective measurement. Do not manufacture or raise scores just to stop; any open must-fix/should-fix or unaccounted finding prevents this criterion passing, regardless of score. Explain why evidence supports the assessment.")),
    ] {
        criteria.push(AcceptanceCriterion { id: id.into(), text: text.into(), verify, verify_kind: "agent".into(), verify_cmd: None });
    }
    let definition = GoalLoopDefinition {
        title: format!("Fix PR #{} review findings", review.pr_number),
        summary: format!("Original review {} at {}. Target {target:.1}/10 (subjective reviewer assessment). Scope: {}\nSelected lenses and rationale:\n{}\nOriginal findings:\n{}", review.id, plan.head_sha, plan.summary,
            serde_json::to_string(&plan.lenses).unwrap_or_default(), serde_json::to_string(&comments).unwrap_or_default()),
        objectives: vec!["Batch related fixes by cause into cohesive substantial iterations; address must-fix and should-fix before optional polish.".into(), "Each iteration verifies EVERY prior finding then performs a fresh independent review.".into()],
        acceptance_criteria: criteria,
        constraints: vec!["Local isolated-worktree edits only. Never stage, commit, push, publish, merge, open PRs, or send messages.".into(), "Respect existing user changes. Only fix issues newly caused by the reviewed PR; summarize preexisting widened issues once without opening finding threads.".into(), "Budget exhaustion or unavailable verification is incomplete, never success. Scores must not override unresolved findings.".into()],
        out_of_scope: vec!["Unrelated preexisting defects, exhaustive test coverage, style-only rewrites, remote publication.".into()],
        success_signal: format!("All findings accounted for with evidence, no open must/should issues, fresh assessment ≥ {target:.1}/10 and critical checks pass."),
    };
    let mut cfg = GoalLoopConfig {
        require_review: true,
        source_links: vec![format!("review:{}", review.id)],
        ..Default::default()
    };
    cfg.executors[0].provider = provider.into();
    cfg.executors[0].prompt_extra = "Treat the review as hypotheses to verify, not orders to blindly patch. Batch all cohesive high/medium fixes, and preserve evidence for the independent evaluator.".into();
    for role in [
        &mut cfg.planner,
        &mut cfg.evaluator,
        &mut cfg.digester,
        &mut cfg.definer,
    ] {
        role.provider = provider.into();
        role.model.clear();
    }
    cfg.evaluator.prompt.push_str(" Independently verify each prior finding against current code before fresh review. Report exact disposition/evidence for every ID. Freshly discovered issues enter next iteration feedback. Never claim command execution from a source trace or inflate a score. Missing or blank evidence means met=false.");
    Ok((definition, cfg))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn review() -> Review {
        serde_json::from_value(serde_json::json!({"id":"r1","repo_id":"repo","pr_number":3,"status":"done","error":null,"comments":[{"id":"c1","review_id":"r1","path":"auth.rs","line":2,"severity":"bug","body":"Permission bypass at auth.rs:2","state":"draft","posted":false,"created_at":"2026-11-07T00:00:00Z"}],"agents":[],"created_at":"2026-11-07T00:00:00Z","scope_plan":{"head_sha":"abc","summary":"Permission change","related_modules":["auth"],"risks":["bypass"],"lenses":[{"name":"Security","reason":"Permission boundary changed","instructions":"Trace caller permissions"}]}})).unwrap()
    }
    #[test]
    fn creates_one_verification_criterion_per_finding_and_reuses_local_only_loop() {
        let (goal, cfg) = build(&review(), 9.5, "codex").unwrap();
        assert_eq!(goal.acceptance_criteria.len(), 4);
        assert_eq!(goal.acceptance_criteria[0].id, "finding-c1");
        assert!(!cfg.allow_commits);
        assert!(cfg.require_review);
        assert_eq!(cfg.executors.len(), 1);
        assert_eq!(cfg.evaluator.provider, "codex");
        assert_eq!(cfg.source_links, vec!["review:r1"]);
        assert!(build(&review(), f64::NAN, "codex").is_err());
        assert!(build(&review(), 10.1, "codex").is_err());
    }
}
