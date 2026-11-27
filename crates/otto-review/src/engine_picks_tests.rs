//! Model and effort per reviewer and provider, drop-only scope, the node's
//! review-wide keys and the spawn order (runtime.md §7.3, §7.6, §7.7).
use super::*;
use crate::scope::{configured_plan, keep_configured, selected_agents};
use otto_core::api::ScopeAssessment;
use otto_core::domain::{ModelPick, ReviewMode, ReviewScopeLens};
use serde_json::json;

fn pick(model: &str, effort: &str) -> ModelPick {
    ModelPick {
        model: model.into(),
        effort: effort.into(),
    }
}

fn lens(name: &str, providers: &[&str], model: &str, effort: &str) -> ReviewAgentCfg {
    ReviewAgentCfg {
        name: name.into(),
        provider: providers[0].into(),
        providers: providers.iter().map(|p| p.to_string()).collect(),
        model: model.into(),
        effort: effort.into(),
        prompt: format!("{name} instructions"),
        ..Default::default()
    }
}

fn cfg(agents: Vec<ReviewAgentCfg>) -> ReviewConfig {
    let mut c = default_review_config("claude");
    c.agents = agents;
    c
}

fn library() -> (tempfile::TempDir, otto_context::Library) {
    let tmp = tempfile::tempdir().unwrap();
    let lib = otto_context::Library::new(tmp.path().join("library"));
    (tmp, lib)
}

fn runs(c: &ReviewConfig, mode: ReviewMode) -> Vec<(String, String, String)> {
    let (_t, lib) = library();
    expand_agent_runs(c, mode, &lib)
        .into_iter()
        .map(|r| (r.provider, r.model, r.effort))
        .collect()
}

#[test]
fn a_pick_resolves_lens_then_own_then_review_wide_then_default() {
    let mut a = lens("Correctness", &["claude", "codex", "agy"], "opus", "high");
    a.picks.insert("codex".into(), pick("gpt-5.5", "low"));
    let mut c = cfg(vec![a.clone()]);
    c.picks.insert("claude".into(), pick("sonnet", "medium"));
    c.picks.insert("agy".into(), pick("", "high"));
    assert_eq!(resolve_pick(&a, &c, "codex"), pick("gpt-5.5", "low"));
    assert_eq!(resolve_pick(&a, &c, "claude"), pick("opus", "high"));
    assert_eq!(resolve_pick(&a, &c, "agy"), pick("", "high"));
    // Today's rule: a lens's own model never leaks onto a secondary provider.
    let plain = lens("Plain", &["claude", "codex"], "opus", "");
    assert_eq!(
        resolve_pick(&plain, &cfg(vec![]), "codex"),
        ModelPick::default()
    );
    // An empty own pick falls through to the review-wide one.
    let bare = lens("Bare", &["claude"], "", "");
    assert_eq!(resolve_pick(&bare, &c, "claude"), pick("sonnet", "medium"));
}

#[test]
fn fan_out_rows_carry_the_model_and_effort_of_their_provider() {
    let mut a = lens("Correctness", &["claude", "codex"], "opus", "high");
    a.picks.insert("codex".into(), pick("gpt-5.5", "xhigh"));
    let b = lens("Security", &["claude", "codex"], "", "");
    let mut c = cfg(vec![a, b]);
    c.picks.insert("codex".into(), pick("gpt-5.4", "low"));
    let s = |p: &str, m: &str, e: &str| (p.to_string(), m.to_string(), e.to_string());
    assert_eq!(
        runs(&c, ReviewMode::FanOut),
        vec![
            s("claude", "opus", "high"),
            s("codex", "gpt-5.5", "xhigh"),
            s("claude", "", ""),
            s("codex", "gpt-5.4", "low"),
        ]
    );
    // The seeded rows carry the resolved effort (the thread reads it there).
    let (_t, lib) = library();
    let rows = crate::prompts::seed_states(
        &expand_agent_runs(&c, ReviewMode::FanOut, &lib),
        &c.summarizer,
    );
    assert_eq!(rows[1].effort, "xhigh");
    assert_eq!(rows[1].model, "gpt-5.5");
}

#[test]
fn an_old_config_reads_and_runs_as_before() {
    // No new key anywhere: models resolve exactly as before the picks.
    let c: ReviewConfig = serde_json::from_value(json!({
        "agents": [{"name": "A", "provider": "claude", "providers": ["claude", "codex"],
                    "model": "opus", "prompt": "p"}],
        "summarizer": {"name": "S", "provider": "claude", "model": "", "prompt": "s"}
    }))
    .unwrap();
    let s = |p: &str, m: &str| (p.to_string(), m.to_string(), String::new());
    assert_eq!(
        runs(&c, ReviewMode::FanOut),
        vec![s("claude", "opus"), s("codex", "")]
    );
    // CR1-011: the primary's model never reaches the Codex orchestrator.
    assert_eq!(
        runs(&c, ReviewMode::Orchestrator),
        vec![s("claude", "opus"), s("codex", "")]
    );
    // …and writes back without the new keys (a rollback binary reads it).
    let back = serde_json::to_value(&c).unwrap();
    for k in [
        "picks",
        "reviewer_time_box_secs",
        "scope_assessment",
        "runner",
        "thread_meta",
    ] {
        assert!(back.get(k).is_none(), "{k} written: {back}");
    }
    assert!(back["agents"][0].get("effort").is_none() && back["agents"][0].get("picks").is_none());
}

#[test]
fn an_orchestrator_takes_a_lens_pick_for_its_provider_first() {
    let mut a = lens("A", &["claude", "codex"], "opus", "high");
    a.picks.insert("codex".into(), pick("gpt-5.5", "medium"));
    let mut c = cfg(vec![a, lens("B", &["claude"], "", "")]);
    c.picks.insert("agy".into(), pick("gemini-3", ""));
    c.agents[1].providers.push("agy".into());
    let s = |p: &str, m: &str, e: &str| (p.to_string(), m.to_string(), e.to_string());
    assert_eq!(
        runs(&c, ReviewMode::Orchestrator),
        vec![
            s("claude", "opus", "high"),
            s("codex", "gpt-5.5", "medium"),
            s("agy", "gemini-3", "")
        ]
    );
}

#[test]
fn scope_selected_lenses_keep_their_picks_and_invented_ones_take_the_review_wide_pick() {
    let mut a = lens("Correctness", &["claude"], "opus", "high");
    a.picks.insert("codex".into(), pick("gpt-5.5", "low"));
    let mut c = cfg(vec![a]);
    c.picks.insert("claude".into(), pick("sonnet", "low"));
    let mut plan = configured_plan(&c, &[], "test");
    plan.lenses.push(ReviewScopeLens {
        name: "Invented".into(),
        reason: "r".into(),
        instructions: "i".into(),
    });
    let selected = selected_agents(&plan, &c);
    assert_eq!(
        (selected[0].model.as_str(), selected[0].effort.as_str()),
        ("opus", "high")
    );
    assert_eq!(
        selected[0].picks.get("codex"),
        Some(&pick("gpt-5.5", "low"))
    );
    let mut c2 = c.clone();
    c2.agents = selected;
    let r = runs(&c2, ReviewMode::FanOut);
    assert_eq!(r[1], ("claude".into(), "sonnet".into(), "low".into()));
}

fn plan_with(c: &ReviewConfig, names: &[&str]) -> otto_core::domain::ReviewScopePlan {
    let mut plan = configured_plan(c, &[], "test");
    plan.lenses = names
        .iter()
        .map(|n| ReviewScopeLens {
            name: n.to_string(),
            reason: "r".into(),
            instructions: "i".into(),
        })
        .collect();
    plan
}

#[test]
fn configured_scope_may_drop_a_declared_lens_never_add_one() {
    let mut c = cfg(vec![
        lens("Correctness", &["claude"], "", ""),
        lens("Security", &["claude"], "", ""),
    ]);
    let names = |p: &otto_core::domain::ReviewScopePlan| {
        p.lenses.iter().map(|l| l.name.clone()).collect::<Vec<_>>()
    };
    // Default (auto): the assessment's plan stands, additions included.
    let mut auto = plan_with(&c, &["security", "Performance"]);
    keep_configured(&mut auto, &c, &[]);
    assert_eq!(names(&auto), ["security", "Performance"]);
    c.scope_assessment = ScopeAssessment::Configured;
    let mut added = plan_with(&c, &["security", "Performance", "Correctness"]);
    keep_configured(&mut added, &c, &[]);
    assert_eq!(names(&added), ["security", "Correctness"]);
    let mut dropped = plan_with(&c, &["Security"]);
    keep_configured(&mut dropped, &c, &[]);
    assert_eq!(names(&dropped), ["Security"]);
    let mut none = plan_with(&c, &["Performance"]);
    keep_configured(&mut none, &c, &["f1".into()]);
    assert_eq!(names(&none), ["Correctness", "Security"]);
    assert_eq!(none.prior_findings.len(), 1);
}

#[test]
fn the_review_run_node_sets_picks_scope_and_time_box() {
    let p = json!({"picks": {"codex": {"model": "gpt-5.5", "effort": "low"}},
                   "scope_assessment": "configured", "reviewer_time_box_secs": 480,
                   "cross_provider_check": true});
    assert!(has_node_review_params(&p));
    assert!(!has_node_review_params(&json!({"reviewers": []})));
    let mut c = cfg(vec![]);
    apply_node_review_params(&mut c, &p);
    assert_eq!(c.picks.get("codex"), Some(&pick("gpt-5.5", "low")));
    assert_eq!(c.scope_assessment, ScopeAssessment::Configured);
    assert_eq!(c.reviewer_time_box_secs, Some(480));
    assert!(c.cross_provider_check);
    let mut d = cfg(vec![]);
    apply_node_review_params(&mut d, &json!({"scope_assessment": "auto"}));
    assert_eq!(d.scope_assessment, ScopeAssessment::Auto);
}

#[test]
fn workflow_reviewers_carry_effort_and_picks_and_the_summarizer_its_effort() {
    let reviewers = vec![
        json!({"lens": "grill", "providers": ["claude", "codex"], "model": "opus",
        "effort": "high", "picks": {"codex": {"model": "gpt-5.5", "effort": "xhigh"}}}),
    ];
    let s = json!({"provider": "codex", "model": "gpt-5.5", "effort": "medium"});
    let c = workflow_review_config_from_json("claude", &reviewers, Some(&s));
    assert_eq!(c.agents[0].effort, "high");
    assert_eq!(
        c.agents[0].picks.get("codex"),
        Some(&pick("gpt-5.5", "xhigh"))
    );
    assert_eq!(
        (
            c.summarizer.provider.as_str(),
            c.summarizer.model.as_str(),
            c.summarizer.effort.as_str()
        ),
        ("codex", "gpt-5.5", "medium")
    );
    assert_eq!(crate::scope::scope_row(&c.summarizer).effort, "medium");
}

#[test]
fn workflow_review_configs_never_introduce_claude_when_step_names_other_providers() {
    let flat = workflow_review_config(
        "claude",
        &["agy".into(), "codex".into()],
        &["correctness".into()],
    );
    assert_eq!(
        flat.summarizer.provider, "agy",
        "flat workflow providers must not leave summarizer on hardcoded claude"
    );

    let reviewers = vec![json!({"lens": "grill", "providers": ["agy", "codex"]})];
    let rich_no_sum = workflow_review_config_from_json("claude", &reviewers, None);
    assert_eq!(
        rich_no_sum.summarizer.provider, "agy",
        "rich workflow reviewers without explicit summarizer must not fall back to claude"
    );

    let rich_empty_sum = workflow_review_config_from_json(
        "claude",
        &reviewers,
        Some(&json!({"instructions": "scope"})),
    );
    assert_eq!(
        rich_empty_sum.summarizer.provider, "agy",
        "summarizer block without provider must stay on a step-named provider, not claude"
    );

    let checks = vec![("unit".into(), "cargo test".into())];
    let params = json!({"summarizer": {"provider": "agy"}});
    let mut with_checks = workflow_review_base_config(
        "claude",
        &params,
        &[],
        &["codex".into()],
        &["correctness".into()],
    )
    .unwrap();
    apply_workflow_review_overlays(&mut with_checks, "claude", &params, &checks, false);
    assert_eq!(with_checks.summarizer.provider, "agy");
    assert!(
        with_checks.agents.iter().all(|a| a.provider == "codex"),
        "checks reviewer must use step-named provider, not claude: {:?}",
        with_checks
            .agents
            .iter()
            .map(|a| (&a.name, &a.provider))
            .collect::<Vec<_>>()
    );
}

#[test]
fn summarizer_only_and_checks_only_do_not_replace_repo_reviewers_or_force_claude() {
    // BUG-13: setting only summarizer instructions/provider with no explicit reviewers
    // must NOT build a generic override that wipes the repo's configured reviewers.
    let sum_only = workflow_review_base_config(
        "codex",
        &json!({"summarizer": {"provider": "agy", "instructions": "scope gate"}}),
        &[],
        &[],
        &[],
    );
    assert!(
        sum_only.is_none(),
        "summarizer-only params must leave base override None so repo reviewers are inherited: {:?}",
        sum_only.map(|c| c.agents.len())
    );

    // BUG-13: applying summarizer-only overlay onto an inherited repo config keeps
    // the repo's custom lenses/providers intact while updating the summarizer.
    let mut repo_cfg = cfg(vec![lens("CustomRepoLens", &["agy"], "gemini-2.5", "high")]);
    repo_cfg.summarizer.provider = "codex".into();
    apply_workflow_review_overlays(
        &mut repo_cfg,
        "codex",
        &json!({"summarizer": {"provider": "agy", "instructions": "scope gate"}}),
        &[],
        false,
    );
    assert_eq!(repo_cfg.agents.len(), 1);
    assert_eq!(repo_cfg.agents[0].name, "CustomRepoLens");
    assert_eq!(repo_cfg.summarizer.provider, "agy");
    assert!(repo_cfg.summarizer.prompt.contains("scope gate"));

    // BUG-14: a Codex-default workflow with required checks + goals (no explicit
    // providers/summarizer) must never synthesize a claude summarizer or move goals to claude.
    let checks_only = workflow_review_config("codex", &[], &[]);
    assert_eq!(
        checks_only.summarizer.provider, "codex",
        "workflow_review_config with codex default must not keep hardcoded claude summarizer"
    );
    let checks = vec![("unit".to_string(), "cargo test".to_string())];
    let p_checks = json!({"goals": ["tests pass"]});
    let base = workflow_review_base_config("codex", &p_checks, &[], &[], &[]);
    assert!(base.is_none(), "checks-only leaves repo config implicit");
    assert_eq!(
        goals_provider(&p_checks, base.as_ref(), &[], "codex"),
        "codex",
        "checks + goals on codex default must grade goals on codex, not claude"
    );
    let mut target_cfg = workflow_review_config("codex", &[], &[]);
    apply_workflow_review_overlays(&mut target_cfg, "codex", &p_checks, &checks, false);
    assert_eq!(target_cfg.summarizer.provider, "codex");
    assert_eq!(target_cfg.agents.last().unwrap().provider, "codex");
}

#[test]
fn codex_reviewers_start_first_and_rows_keep_their_order() {
    let c = cfg(vec![
        lens("A", &["claude", "codex"], "", ""),
        lens("B", &["agy", "codex"], "", ""),
    ]);
    let (_t, lib) = library();
    let order: Vec<(usize, String)> =
        in_spawn_order(expand_agent_runs(&c, ReviewMode::FanOut, &lib))
            .into_iter()
            .map(|(i, r)| (i, r.provider))
            .collect();
    let s = |i: usize, p: &str| (i, p.to_string());
    assert_eq!(
        order,
        vec![s(1, "codex"), s(3, "codex"), s(0, "claude"), s(2, "agy")]
    );
}

/// CR1-011: an orchestrator takes a lens's own effort for its own provider
/// even without a model, and never a model of another provider.
#[test]
fn an_orchestrator_takes_only_its_own_providers_model_and_effort() {
    let c = cfg(vec![
        lens("A", &["claude", "codex"], "claude-opus-4-6", "high"),
        lens("B", &["codex"], "", "low"),
    ]);
    let s = |p: &str, m: &str, e: &str| (p.to_string(), m.to_string(), e.to_string());
    assert_eq!(
        runs(&c, ReviewMode::Orchestrator),
        vec![
            s("claude", "claude-opus-4-6", "high"),
            s("codex", "", "low")
        ]
    );
}

/// CR1-070: a lens led by another provider (cross-provider check) keeps its
/// own model and effort on its own provider.
#[test]
fn a_cross_checked_lens_keeps_its_own_pick_on_its_provider() {
    let mut a = lens("Correctness", &["claude", "codex"], "gpt-5", "high");
    a.provider = "codex".into();
    let mut c = cfg(vec![a]);
    c.cross_provider_check = true;
    let plan = plan_with(&c, &["Correctness"]);
    let mut c2 = c.clone();
    c2.agents = selected_agents(&plan, &c);
    let r = runs(&c2, ReviewMode::FanOut);
    assert_eq!(
        r[1],
        ("codex".into(), "gpt-5".into(), "high".into()),
        "{r:?}"
    );
}
