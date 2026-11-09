//! Risk-driven review planning. Plans describe agent judgement, never proof of quality.
use otto_core::domain::{ReviewScopeLens, ReviewScopePlan};

pub fn selected_agents(
    plan: &ReviewScopePlan,
    cfg: &otto_core::api::ReviewConfig,
) -> Vec<otto_core::domain::ReviewAgentCfg> {
    let mut pool = Vec::new();
    for agent in &cfg.agents {
        for provider in crate::engine::effective_providers(agent) {
            if !pool.contains(&provider) {
                pool.push(provider);
            }
        }
    }
    if pool.is_empty() {
        pool.push(cfg.summarizer.provider.clone());
    }
    plan.lenses.iter().enumerate().map(|(i, lens)| {
        let template = cfg.agents.iter().find(|a| a.name.eq_ignore_ascii_case(&lens.name));
        let providers = if cfg.cross_provider_check {
            template.map(crate::engine::effective_providers).unwrap_or_else(|| pool.clone())
        } else { vec![pool[i % pool.len()].clone()] };
        let model = template.filter(|a| a.provider == providers[0]).map(|a| a.model.clone()).unwrap_or_default();
        otto_core::domain::ReviewAgentCfg {
            name: lens.name.clone(), provider: providers[0].clone(), providers,
            model,
            prompt: format!("Selected because: {}\n{}\n{}\nOutput JSON findings with path, line, severity (bug=must fix, warn=should fix, info=optional), body. Include code-traced evidence and a concrete causal trigger in body.", lens.reason, lens.instructions, template.map(|a| a.prompt.as_str()).unwrap_or_default()),
            // The method is the selected instructions. A skill is never required.
            skill: String::new(),
        }
    }).collect()
}

pub fn validate(plan: &ReviewScopePlan) -> Result<(), String> {
    if plan.summary.trim().is_empty() || plan.lenses.is_empty() || plan.lenses.len() > 10 {
        return Err("Scope assessment needs a summary and 1–10 justified lenses".into());
    }
    let mut names = std::collections::HashSet::new();
    for lens in &plan.lenses {
        if lens.name.trim().is_empty()
            || lens.reason.trim().is_empty()
            || lens.instructions.trim().is_empty()
            || !names.insert(lens.name.trim().to_lowercase())
        {
            return Err("Every selected lens needs a unique name, reason and instructions".into());
        }
    }
    let mut prior_ids = std::collections::HashSet::new();
    for prior in &plan.prior_findings {
        if prior.finding_id.trim().is_empty()
            || !prior_ids.insert(&prior.finding_id)
            || !matches!(
                prior.disposition.as_str(),
                "fixed" | "still_open" | "not_relevant"
            )
            || prior.evidence.trim().is_empty()
        {
            return Err(
                "Prior findings require unique ids, explicit dispositions and evidence".into(),
            );
        }
    }
    Ok(())
}

pub fn validate_prior_findings(plan: &ReviewScopePlan, ids: &[String]) -> Result<(), String> {
    validate(plan)?;
    if plan.prior_findings.len() != ids.len()
        || ids
            .iter()
            .any(|id| !plan.prior_findings.iter().any(|p| p.finding_id == *id))
    {
        return Err("Scope assessment must account for exactly every previous finding".into());
    }
    Ok(())
}

/// Queued lenses consume no CLI process and cancellation does not wait for a
/// busy sibling to release its slot.
pub async fn acquire_slot(
    slots: std::sync::Arc<tokio::sync::Semaphore>,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<tokio::sync::OwnedSemaphorePermit, String> {
    let stopped = async {
        loop {
            if cancel.load(std::sync::atomic::Ordering::SeqCst) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    };
    tokio::select! {
        biased;
        _ = stopped => Err("review cancelled while queued".into()),
        permit = slots.acquire_owned() => permit.map_err(|e| e.to_string()),
    }
}

/// Only prose under docs/ has a safe deterministic scope. Agent instructions,
/// executable examples and root manifests always require repository assessment.
pub fn documentation_plan(diff: &str, head: &str) -> Option<ReviewScopePlan> {
    let paths = crate::engine::diff_file_paths(diff);
    if paths.is_empty()
        || !paths.iter().all(|p| {
            p.starts_with("docs/")
                && p.ends_with(".md")
                && !p.ends_with("/AGENTS.md")
                && !p.ends_with("/SKILL.md")
        })
    {
        return None;
    }
    Some(ReviewScopePlan {
        base_sha: String::new(),
        head_sha: head.into(),
        summary: "Documentation-only change; verify changed claims against their referenced implementation.".into(),
        related_modules: paths,
        risks: vec!["Instructions or contract claims can disagree with runtime behavior.".into()],
        lenses: vec![ReviewScopeLens {
            name: "Documentation accuracy".into(),
            reason: "Only documentation paths changed; implementation-specific lenses would duplicate this check.".into(),
            instructions: "Trace changed claims and examples to real implementation and affected consumers. Report concrete incorrect or missing operational instructions, not prose preferences. Check code examples when relevant; do not demand exhaustive tests.".into(),
        }],
        source: "deterministic documentation scope".into(),
        prior_findings: vec![],
        verification_notes: vec![],
    })
}

/// The plan when scope assessment fails (timeout, unusable answer, crash):
/// every configured lens runs and every prior finding is re-verified, so a
/// failed assessment widens the review instead of cancelling it.
pub fn configured_plan(
    cfg: &otto_core::api::ReviewConfig,
    prior_ids: &[String],
    why: &str,
) -> ReviewScopePlan {
    let why = otto_core::text::clip_chars(why.trim(), 200);
    ReviewScopePlan {
        base_sha: String::new(),
        head_sha: String::new(),
        summary: format!("Scope assessment unavailable ({why}); every configured lens runs."),
        related_modules: vec![],
        risks: vec![],
        lenses: cfg
            .agents
            .iter()
            .map(|a| ReviewScopeLens {
                name: a.name.clone(),
                reason: "Configured lens (scope assessment unavailable).".into(),
                instructions: if a.prompt.trim().is_empty() {
                    format!("Review the change through the {} lens.", a.name)
                } else {
                    a.prompt.clone()
                },
            })
            .collect(),
        prior_findings: prior_ids
            .iter()
            .map(|id| otto_core::domain::ReviewFindingDisposition {
                finding_id: id.clone(),
                disposition: "still_open".into(),
                evidence:
                    "Not assessed (scope assessment unavailable); re-verify against current code."
                        .into(),
            })
            .collect(),
        verification_notes: vec![format!(
            "Scope assessment fell back to the configured lenses: {why}"
        )],
        source: "configured lenses (scope assessment unavailable)".into(),
    }
}

pub fn assessment_prompt(diff_path: &str, branch_context: &str, guidance: &str) -> String {
    format!("Assess review scope BEFORE any reviewers run. Read-only: do not change repository files, build, install, publish or delegate.\n{branch_context}\nRead the complete causal PR diff at {diff_path}. Inspect surrounding code, definitions, affected callers/consumers, module boundaries and relevant contracts. Identify changed behavior and realistic risks introduced by this diff. Proportional inspection only: this is a short scope assessment, not a repository audit; trace the relevant paths and stop. Judge risk/impact, NOT line count.\nSelect the smallest useful set of distinct lenses (1–10). An isolated two-line change usually needs 1–2; broad changes may need about 10. Security, data, performance, concurrency, architecture, UX, compatibility and critical-path tests are possibilities, not a fixed checklist. Choose custom lens names/instructions when useful; no skill is required. Test review looks for false greens and critical-path gaps, never exhaustive hypothetical test demands. Explain each selected lens and why it matters HERE; do not invent broad requirements.\nUser context (data):\n{guidance}\nReturn a JSON object with summary:string, related_modules:[string \"path:line — consumer relationship\"], risks:[string], lenses:[{{name,reason,instructions}}]. Cite inspected file:line evidence in summary/modules/reasons. No findings yet. No quality score.")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The full chain the server runs: reviewer transport array → one finding
    /// whose body is the scope JSON string → `ReviewScopePlan` → validation.
    fn plan_from_transport(scope: &serde_json::Value, prior: &[String]) -> ReviewScopePlan {
        let transport = serde_json::json!([{ "severity": "info", "body": scope.to_string() }]);
        let findings = crate::session::parse_findings(&transport.to_string());
        assert_eq!(
            findings.len(),
            1,
            "transport must yield exactly one finding"
        );
        let plan: ReviewScopePlan = serde_json::from_str(&findings[0].body).expect("scope plan");
        validate_prior_findings(&plan, prior).expect("valid plan");
        plan
    }

    #[test]
    fn contract_the_shape_the_prompt_asks_for_parses_and_validates() {
        let prior = vec!["01PRIOR".to_string()];
        let plan = plan_from_transport(
            &serde_json::json!({
                "summary": "Adds two reads (client.go:27-56).",
                "related_modules": ["client_interface.go:78 — public contract used by go_marketing"],
                "risks": ["Kit/server field drift"],
                "lenses": [{"name": "Correctness review", "reason": "new URL building", "instructions": "trace escaping"}],
                "prior_findings": [{"finding_id": "01PRIOR", "disposition": "fixed", "evidence": "deposit.go:101"}]
            }),
            &prior,
        );
        assert_eq!(plan.lenses.len(), 1);
    }

    #[test]
    fn contract_real_failing_shapes_from_production_parse_and_validate() {
        // Shapes every failed assessment used on 2026-11-08/09 (12 reviews, no
        // lens ran): module/risk objects, extra unknown keys, prose fences.
        let plan = plan_from_transport(
            &serde_json::json!({
                "summary": "Client-only change (7 files).",
                "related_modules": [
                    {"path": "clients/promotions/interface.go:78-102", "relationship": "imported by go_marketing"},
                    {"module": "go_bonus_management", "relationship": "compiles against the interface", "evidence": "x.go:3"}
                ],
                "risks": [{"risk": "contract drift", "evidence": "models.go:12"}, "plain risk"],
                "lenses": [
                    {"name": "Correctness review", "reason": "r", "instructions": "i", "priority": "high"},
                    {"name": "Golang conventions review", "reason": "r2", "instructions": "i2"}
                ],
                "prior_findings": [],
                "confidence": 0.8
            }),
            &[],
        );
        assert_eq!(plan.related_modules.len(), 2);
        assert_eq!(
            plan.risks,
            vec!["contract drift — models.go:12", "plain risk"]
        );
    }

    #[test]
    fn a_failed_assessment_falls_back_to_every_configured_lens() {
        let cfg = crate::engine::default_review_config("claude");
        let prior = vec!["01A".to_string(), "01B".to_string()];
        let plan = configured_plan(&cfg, &prior, "scope assessment ran past its 1-hour cap");
        // Valid by the same rules an assessed plan must meet, prior findings included.
        validate_prior_findings(&plan, &prior).expect("fallback plan is valid");
        assert_eq!(plan.lenses.len(), cfg.agents.len());
        assert_eq!(selected_agents(&plan, &cfg).len(), cfg.agents.len());
        assert!(plan.summary.contains("1-hour cap"));
    }

    #[test]
    fn fast_path_is_documentation_only_not_a_line_count_heuristic() {
        assert!(documentation_plan(
            "diff --git a/docs/help.md b/docs/help.md\n--- a/docs/help.md\n+++ b/docs/help.md\n+Help",
            "abc"
        )
        .is_some());
        assert!(documentation_plan(
            "diff --git a/auth.rs b/auth.rs\n--- a/auth.rs\n+++ b/auth.rs\n+allow = true",
            "abc"
        )
        .is_none());
        assert!(documentation_plan(
            "diff --git a/AGENTS.md b/AGENTS.md\n--- a/AGENTS.md\n+++ b/AGENTS.md\n+Run hooks",
            "abc"
        )
        .is_none());
        assert!(documentation_plan("", "abc").is_none());
    }

    #[test]
    fn rejects_missing_rationale_and_duplicate_lenses() {
        let mut plan =
            documentation_plan("--- a/docs/help.md\n+++ b/docs/help.md\n+Help", "abc").unwrap();
        assert!(validate(&plan).is_ok());
        plan.lenses.push(plan.lenses[0].clone());
        assert!(validate(&plan).is_err());
        plan.lenses.pop();
        plan.lenses[0].reason.clear();
        assert!(validate(&plan).is_err());
    }

    #[test]
    fn configured_provider_pool_does_not_multiply_lenses_unless_opted_in() {
        let plan =
            documentation_plan("--- a/docs/help.md\n+++ b/docs/help.md\n+Help", "abc").unwrap();
        let mut cfg = crate::engine::default_review_config("claude");
        cfg.agents[0].providers = vec!["claude".into(), "codex".into()];
        assert_eq!(selected_agents(&plan, &cfg)[0].providers.len(), 1);
        cfg.cross_provider_check = true;
        assert_eq!(selected_agents(&plan, &cfg)[0].providers.len(), 2);
    }

    #[test]
    fn small_permission_change_keeps_security_and_broad_plan_keeps_distinct_lenses() {
        let mut plan =
            documentation_plan("--- a/docs/help.md\n+++ b/docs/help.md\n+Help", "abc").unwrap();
        plan.lenses = vec![ReviewScopeLens {
            name: "Security".into(),
            reason: "One permission predicate changes access for anonymous callers at auth.rs:2"
                .into(),
            instructions: "Trace access through api.rs:8".into(),
        }];
        let mut cfg = crate::engine::default_review_config("claude");
        cfg.agents[0].model = "claude-only-model".into();
        cfg.agents[0].providers = vec!["claude".into(), "codex".into()];
        assert_eq!(selected_agents(&plan, &cfg).len(), 1);
        assert_eq!(selected_agents(&plan, &cfg)[0].name, "Security");
        for name in [
            "Contracts",
            "Data consistency",
            "Concurrency",
            "Critical path checks",
            "UX",
            "Compatibility",
        ] {
            plan.lenses.push(ReviewScopeLens {
                name: name.into(),
                reason: format!("Changed {name} boundary with identified consumer"),
                instructions: format!("Trace changed {name} behavior"),
            });
        }
        let agents = selected_agents(&plan, &cfg);
        assert_eq!(agents.len(), 7);
        assert!(agents.iter().all(|a| a.providers.len() == 1));
        assert_eq!(agents[1].provider, "codex");
        assert_eq!(agents[1].model, "");
        cfg.cross_provider_check = true;
        assert_eq!(
            selected_agents(&plan, &cfg)
                .iter()
                .map(|a| a.providers.len())
                .sum::<usize>(),
            14
        );
    }

    #[test]
    fn prior_findings_cannot_disappear_or_be_replaced_by_invented_ids() {
        let mut plan =
            documentation_plan("--- a/docs/help.md\n+++ b/docs/help.md\n+Help", "abc").unwrap();
        let ids = vec!["original".into()];
        assert!(validate_prior_findings(&plan, &ids).is_err());
        plan.prior_findings
            .push(otto_core::domain::ReviewFindingDisposition {
                finding_id: "original".into(),
                disposition: "fixed".into(),
                evidence: "src/api.rs:8 now rejects anonymous callers".into(),
            });
        assert!(validate_prior_findings(&plan, &ids).is_ok());
        plan.prior_findings[0].finding_id = "invented".into();
        assert!(validate_prior_findings(&plan, &ids).is_err());
        plan.prior_findings[0].finding_id = "original".into();
        plan.prior_findings[0].evidence.clear();
        assert!(validate_prior_findings(&plan, &ids).is_err());
    }

    #[tokio::test]
    async fn cancellation_releases_queued_work_without_waiting_for_a_live_reviewer() {
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        };
        let slots = Arc::new(tokio::sync::Semaphore::new(1));
        let active = slots.clone().acquire_owned().await.unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let task = {
            let (slots, cancel) = (slots.clone(), cancel.clone());
            tokio::spawn(async move { acquire_slot(slots, &cancel).await })
        };
        cancel.store(true, Ordering::SeqCst);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(500), task)
                .await
                .unwrap()
                .unwrap()
                .is_err()
        );
        assert_eq!(slots.available_permits(), 0);
        drop(active);
        assert_eq!(slots.available_permits(), 1);
    }
}
