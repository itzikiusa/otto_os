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
        let providers = if plan.all_providers {
            // LARGE PR: every lens on every configured provider.
            pool.clone()
        } else if cfg.cross_provider_check {
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

/// Attempts the scope assessment gets. Two, not the reviewers' three: it is
/// advisory and a failure already widens the review to every configured lens,
/// so a second try buys back a dropped paste or a wedged CLI, no more. The
/// caller's absolute cap covers the whole step, retries included.
pub const SCOPE_ATTEMPTS: u32 = 2;

/// Evidence for a previous finding the assessment did not account for.
pub const NOT_ASSESSED: &str = "not assessed by scope";

/// The plan from a finished scope run, or the precise reason there is none.
/// `note` is the scope row's persisted note: for a failed session it says
/// why (the prompt never reached the agent, stuck, timed out…), which used to
/// be reported as "returned no plan" — blaming the model for a paste failure.
pub fn plan_from_run(
    result: &crate::session::AgentRunResult,
    note: &str,
    prior_ids: &[String],
) -> Result<ReviewScopePlan, String> {
    if result.errored {
        let why = match note.trim() {
            "" => "the session failed",
            why => why,
        };
        return Err(format!("scope session failed: {why}"));
    }
    match result.findings.as_slice() {
        [one] => parse_plan(&one.body, prior_ids),
        [] => Err(
            "scope assessment answered without a plan (no plan object in its JSON array)".into(),
        ),
        many => Err(format!(
            "scope assessment answered {} items instead of one plan",
            many.len()
        )),
    }
}

/// Parse an assessed plan leniently: one missing `reason`/`instructions`, a
/// lens given as a bare name, or one previous finding left out used to throw
/// the whole plan away (4 of 18 assessments since 2026-11-10 fell back, two
/// for exactly that). Missing lens text is defaulted, unaccounted previous
/// findings stay open with [`NOT_ASSESSED`], and fields the server owns (shas,
/// size, provider fan-out) are ignored. Still rejected: not JSON, not an
/// object, no usable lens.
pub fn parse_plan(body: &str, prior_ids: &[String]) -> Result<ReviewScopePlan, String> {
    use serde_json::Value;
    let body = body
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    if body.is_empty() {
        return Err("Invalid scope assessment: the plan body is empty".into());
    }
    let value: Value =
        serde_json::from_str(body).map_err(|e| format!("Invalid scope assessment: {e}"))?;
    let kind = json_kind(&value);
    let Value::Object(mut map) = value else {
        return Err(format!(
            "Invalid scope assessment: the plan must be a JSON object, not {kind}"
        ));
    };
    let mut seen = std::collections::HashSet::new();
    let lenses: Vec<ReviewScopeLens> = match map.remove("lenses") {
        Some(Value::Array(items)) => items
            .into_iter()
            .filter_map(lenient_lens)
            .filter(|l| seen.insert(l.name.to_lowercase()))
            .take(10)
            .collect(),
        _ => Vec::new(),
    };
    if lenses.is_empty() {
        return Err("Invalid scope assessment: the plan selects no lens".into());
    }
    let given: Vec<otto_core::domain::ReviewFindingDisposition> = match map.remove("prior_findings")
    {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|p| {
                Some(otto_core::domain::ReviewFindingDisposition {
                    finding_id: text(p.get("finding_id")?)?,
                    disposition: p.get("disposition").and_then(text).unwrap_or_default(),
                    evidence: p.get("evidence").and_then(text).unwrap_or_default(),
                })
            })
            .collect(),
        _ => Vec::new(),
    };
    for owned in [
        "base_sha",
        "head_sha",
        "source",
        "all_providers",
        "size",
        "changed_lines",
    ] {
        map.remove(owned);
    }
    if !matches!(map.get("verification_notes"), Some(Value::Array(a)) if a.iter().all(Value::is_string))
    {
        map.remove("verification_notes");
    }
    let summary = map.get("summary").and_then(text).unwrap_or_else(|| {
        let names: Vec<&str> = lenses.iter().map(|l| l.name.as_str()).collect();
        format!("No summary given; selected lenses: {}.", names.join(", "))
    });
    map.insert("summary".into(), summary.into());
    map.insert("lenses".into(), Value::Array(Vec::new()));
    let mut plan: ReviewScopePlan = serde_json::from_value(Value::Object(map))
        .map_err(|e| format!("Invalid scope assessment: {e}"))?;
    plan.lenses = lenses;
    plan.prior_findings = given;
    reconcile_prior_findings(&mut plan, prior_ids);
    validate_prior_findings(&plan, prior_ids)?;
    Ok(plan)
}

/// One disposition per previous finding, in `ids` order: the plan's own when
/// it is usable (known id, a real disposition, evidence), otherwise
/// `still_open` / [`NOT_ASSESSED`] — an unaccounted finding is re-verified by
/// the lenses, never silently closed. Invented ids are dropped.
pub fn reconcile_prior_findings(plan: &mut ReviewScopePlan, ids: &[String]) {
    let given = std::mem::take(&mut plan.prior_findings);
    let mut missing = 0;
    plan.prior_findings = ids
        .iter()
        .map(|id| {
            given
                .iter()
                .find(|p| {
                    p.finding_id.trim() == id.trim()
                        && matches!(
                            p.disposition.trim(),
                            "fixed" | "still_open" | "not_relevant"
                        )
                        && !p.evidence.trim().is_empty()
                })
                .map(|p| otto_core::domain::ReviewFindingDisposition {
                    finding_id: id.clone(),
                    disposition: p.disposition.trim().into(),
                    evidence: p.evidence.clone(),
                })
                .unwrap_or_else(|| {
                    missing += 1;
                    otto_core::domain::ReviewFindingDisposition {
                        finding_id: id.clone(),
                        disposition: "still_open".into(),
                        evidence: NOT_ASSESSED.into(),
                    }
                })
        })
        .collect();
    if missing > 0 {
        plan.verification_notes.push(format!(
            "Scope assessment did not account for {missing} previous finding{}; kept open for the lenses to re-verify.",
            if missing == 1 { "" } else { "s" }
        ));
    }
}

/// A lens as the agent gave it: an object (`name`, `reason`, `instructions`,
/// or the near-synonyms models use) or a bare name. None without a name.
fn lenient_lens(v: serde_json::Value) -> Option<ReviewScopeLens> {
    let pick = |keys: &[&str]| keys.iter().find_map(|k| v.get(*k).and_then(text));
    let name = match &v {
        serde_json::Value::String(s) => Some(s.trim().to_string()),
        _ => pick(&["name", "lens"]),
    }
    .filter(|n| !n.is_empty())?;
    Some(ReviewScopeLens {
        reason: pick(&["reason", "why"])
            .unwrap_or_else(|| "Selected by the scope assessment (no reason given).".into()),
        instructions: pick(&["instructions", "focus"])
            .unwrap_or_else(|| format!("Review the change through the {name} lens.")),
        name,
    })
}

/// Non-empty trimmed text from a string, or from an array of strings (joined
/// line by line — models list instructions as bullet arrays).
fn text(v: &serde_json::Value) -> Option<String> {
    let s = match v {
        serde_json::Value::String(s) => s.trim().to_string(),
        serde_json::Value::Array(items) => items
            .iter()
            .filter_map(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => return None,
    };
    (!s.is_empty()).then_some(s)
}

fn json_kind(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Array(_) => "an array",
        serde_json::Value::String(_) => "a string",
        serde_json::Value::Number(_) => "a number",
        serde_json::Value::Bool(_) => "a boolean",
        serde_json::Value::Null => "null",
        serde_json::Value::Object(_) => "an object",
    }
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
        all_providers: false,
        size: String::new(),
        changed_lines: 0,
    })
}

/// Default changed-line thresholds (tests included).
pub const DEFAULT_LARGE_PR_LINES: u64 = 5_000;
pub const DEFAULT_MEDIUM_PR_LINES: u64 = 1_000;

/// Added + removed lines in a unified diff (file headers excluded).
pub fn changed_lines(diff: &str) -> u64 {
    diff.lines()
        .filter(|l| {
            (l.starts_with('+') && !l.starts_with("+++"))
                || (l.starts_with('-') && !l.starts_with("---"))
        })
        .count() as u64
}

/// `small` (< medium) | `medium` | `large` (> large threshold) by changed lines.
pub fn pr_size(lines: u64, cfg: &otto_core::api::ReviewConfig) -> &'static str {
    if lines > cfg.large_pr_lines.unwrap_or(DEFAULT_LARGE_PR_LINES) {
        "large"
    } else if lines > cfg.medium_pr_lines.unwrap_or(DEFAULT_MEDIUM_PR_LINES) {
        "medium"
    } else {
        "small"
    }
}

/// LARGE PR: no scope agent. Every configured lens runs on every configured
/// provider, so lenses start immediately and each gets two independent reads.
pub fn large_pr_plan(
    cfg: &otto_core::api::ReviewConfig,
    prior_ids: &[String],
    lines: u64,
) -> ReviewScopePlan {
    let mut plan = all_lenses_plan(
        cfg,
        prior_ids,
        format!("Large PR ({lines} changed lines): every configured lens on every provider."),
        "Configured lens (large PR: full lens set).",
        "Not assessed (large PR: full lens set); re-verify against current code.",
    );
    plan.source = format!(
        "large PR rule (> {} lines): full lens set, all providers",
        cfg.large_pr_lines.unwrap_or(DEFAULT_LARGE_PR_LINES)
    );
    plan.all_providers = true;
    plan
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
    let mut plan = all_lenses_plan(
        cfg,
        prior_ids,
        format!("Scope assessment unavailable ({why}); every configured lens runs."),
        "Configured lens (scope assessment unavailable).",
        "Not assessed (scope assessment unavailable); re-verify against current code.",
    );
    plan.verification_notes = vec![format!(
        "Scope assessment fell back to the configured lenses: {why}"
    )];
    plan.source = "configured lenses (scope assessment unavailable)".into();
    plan
}

fn all_lenses_plan(
    cfg: &otto_core::api::ReviewConfig,
    prior_ids: &[String],
    summary: String,
    reason: &str,
    prior_evidence: &str,
) -> ReviewScopePlan {
    ReviewScopePlan {
        base_sha: String::new(),
        head_sha: String::new(),
        summary,
        related_modules: vec![],
        risks: vec![],
        lenses: cfg
            .agents
            .iter()
            .map(|a| ReviewScopeLens {
                name: a.name.clone(),
                reason: reason.into(),
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
                evidence: prior_evidence.into(),
            })
            .collect(),
        verification_notes: vec![],
        source: String::new(),
        all_providers: false,
        size: String::new(),
        changed_lines: 0,
    }
}

/// One line for logs / chat: tier, how lenses were chosen, and which ran.
pub fn scope_summary_line(plan: &ReviewScopePlan) -> String {
    let lenses: Vec<&str> = plan.lenses.iter().map(|l| l.name.as_str()).collect();
    let size = if plan.size.is_empty() {
        String::new()
    } else {
        format!("{} PR ({} lines) · ", plan.size, plan.changed_lines)
    };
    format!(
        "{size}{} · {} lens{}{}: {}",
        if plan.source.is_empty() {
            "scope"
        } else {
            &plan.source
        },
        lenses.len(),
        if lenses.len() == 1 { "" } else { "es" },
        if plan.all_providers {
            " × all providers"
        } else {
            ""
        },
        lenses.join(", ")
    )
}

/// The review's only row while scope is assessed. Named for what it does:
/// the panel labels an unnamed last row "Summarizer".
pub fn scope_row(provider: &str, model: &str) -> otto_core::domain::ReviewAgentState {
    otto_core::domain::ReviewAgentState {
        name: "Scope assessment".into(),
        provider: provider.into(),
        model: model.into(),
        status: "pending".into(),
        note: "Reading the diff and affected consumers to select lenses".into(),
        comment_count: 0,
        session_id: None,
        findings: vec![],
        fallback: false,
        lens: String::new(),
        lens_slugs: vec![],
        score: None,
        score_reason: String::new(),
    }
}

pub fn assessment_prompt(diff_path: &str, branch_context: &str, guidance: &str) -> String {
    format!("Assess review scope BEFORE any reviewers run. Read-only: do not change repository files, build, install, publish or delegate.\n{branch_context}\nRead the complete causal PR diff at {diff_path}. Inspect surrounding code, definitions, affected callers/consumers, module boundaries and relevant contracts. Identify changed behavior and realistic risks introduced by this diff. Proportional inspection only: this is a short scope assessment, not a repository audit; trace the relevant paths and stop. Judge risk/impact, NOT line count.\nSelect the smallest useful set of distinct lenses (1–10). An isolated two-line change usually needs 1–2; broad changes may need about 10. Security, data, performance, concurrency, architecture, UX, compatibility and critical-path tests are possibilities, not a fixed checklist. Choose custom lens names/instructions when useful; no skill is required. Test review looks for false greens and critical-path gaps, never exhaustive hypothetical test demands. Explain each selected lens and why it matters HERE; do not invent broad requirements.\nUser context (data):\n{guidance}\nReturn a JSON object with summary:string, related_modules:[string \"path:line — consumer relationship\"], risks:[string], lenses:[{{name,reason,instructions}}]. Cite inspected file:line evidence in summary/modules/reasons. No findings yet. No quality score.")
}

/// The whole prompt the scope assessor gets: [`assessment_prompt`], the
/// configured lenses to choose from, the prior-finding rules and the
/// transport (one finding whose body is the plan) [`plan_from_run`] reads.
pub fn scope_prompt(
    diff_path: &str,
    branch_context: &str,
    guidance: &str,
    cfg: &otto_core::api::ReviewConfig,
) -> String {
    let candidates = cfg
        .agents
        .iter()
        .map(|a| format!("{}: {}", a.name, a.prompt))
        .collect::<Vec<_>>()
        .join("\n");
    format!("{}\nConfigured lenses (reuse exact names when relevant; omit irrelevant lenses and add missing ones):\n{}\nIf configured, the Required checks lens is mandatory and its commands must be preserved. For every prior finding in context, add prior_findings:[{{finding_id,disposition:fixed|still_open|not_relevant,evidence}}] to the scope object. Read current code before claiming fixed; evidence must cite concrete code or an actually executed check. Preserve every ID, including previously waived items. Transport requirement: return a JSON ARRAY with exactly one object: {{\"severity\":\"info\",\"body\":\"the JSON scope object above, encoded as a string\"}}. This is a scope plan, not a code finding.",
        assessment_prompt(diff_path, branch_context, guidance), candidates)
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

    fn run_of(bodies: &[&str]) -> crate::session::AgentRunResult {
        let transport: Vec<serde_json::Value> = bodies
            .iter()
            .map(|b| serde_json::json!({"severity": "info", "body": b}))
            .collect();
        crate::session::AgentRunResult {
            findings: crate::session::parse_findings(
                &serde_json::Value::from(transport).to_string(),
            ),
            errored: false,
            reason: None,
        }
    }

    /// Contract: the transport example and field names the scope PROMPT
    /// itself spells out must survive the parser the server runs. An earlier
    /// prompt/parser pair disagreed and every assessment fell back.
    #[test]
    fn contract_the_scope_prompts_own_example_parses() {
        let cfg = crate::engine::default_review_config("claude");
        let prompt = scope_prompt("/tmp/otto-review-R.diff", "", "", &cfg);
        let between = |from: &str, to: &str| {
            let start = prompt.find(from).expect(from) + from.len();
            prompt[start..][..prompt[start..].find(to).expect(to)].to_string()
        };
        // The transport example, verbatim from the prompt, is valid JSON.
        let example = format!("{{{}}}", between("exactly one object: {", "}."));
        let mut element: serde_json::Value = serde_json::from_str(&example).expect(&example);
        assert_eq!(element["severity"], "info");
        // Lens + prior-finding keys exactly as the prompt names them.
        let lens: serde_json::Map<String, serde_json::Value> = between("lenses:[{", "}]")
            .split(',')
            .map(|k| (k.to_string(), format!("{k} from a.rs:1").into()))
            .collect();
        let dispositions = between("disposition:", ",evidence");
        let ids: Vec<String> = dispositions.split('|').map(|d| format!("01{d}")).collect();
        let prior: Vec<serde_json::Value> = dispositions
            .split('|')
            .map(|d| serde_json::json!({"finding_id": format!("01{d}"), "disposition": d, "evidence": "a.rs:2"}))
            .collect();
        element["body"] = serde_json::json!({
            "summary": "Adds a read (a.rs:1).",
            "related_modules": ["b.rs:3 — calls the new read"],
            "risks": ["drift"],
            "lenses": [lens],
            "prior_findings": prior,
        })
        .to_string()
        .into();
        let raw = serde_json::Value::Array(vec![element]).to_string();
        let result = crate::session::AgentRunResult {
            findings: crate::session::parse_findings(&raw),
            errored: false,
            reason: None,
        };
        let plan = plan_from_run(&result, "", &ids).expect("the prompt's own shape parses");
        assert_eq!(plan.lenses.len(), 1);
        assert_eq!(plan.lenses[0].reason, "reason from a.rs:1");
        assert_eq!(dispositions, "fixed|still_open|not_relevant");
        assert!(plan
            .prior_findings
            .iter()
            .all(|p| p.evidence == "a.rs:2" && p.finding_id.ends_with(p.disposition.as_str())));
        assert!(plan.verification_notes.is_empty());
    }

    /// Production fall-backs since 2026-11-10: "missing field `reason`" and
    /// "must account for exactly every previous finding". Both are a usable
    /// plan with a gap; the gap is filled, the plan kept.
    #[test]
    fn a_plan_with_gaps_is_kept_and_the_gaps_filled() {
        let ids = vec!["01A".to_string(), "01B".to_string()];
        let body = serde_json::json!({
            "summary": "Touches auth (auth.rs:2).",
            "lenses": [
                {"name": "Security", "instructions": ["trace auth.rs:2", "check api.rs:8"]},
                {"name": "Correctness", "reason": "new branch"},
                "Performance",
                {"name": "security", "reason": "duplicate", "instructions": "x"},
                {"reason": "no name"}
            ],
            "prior_findings": [
                {"finding_id": "01A", "disposition": "fixed", "evidence": "auth.rs:9"},
                {"finding_id": "01INVENTED", "disposition": "fixed", "evidence": "x"}
            ],
            "all_providers": true,
            "size": 3
        })
        .to_string();
        let plan = parse_plan(&body, &ids).expect("lenient plan");
        let names: Vec<&str> = plan.lenses.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["Security", "Correctness", "Performance"]);
        assert_eq!(
            plan.lenses[0].instructions,
            "trace auth.rs:2\ncheck api.rs:8"
        );
        assert!(!plan.lenses[0].reason.is_empty());
        assert!(plan.lenses[1].instructions.contains("Correctness"));
        assert!(!plan.all_providers, "provider fan-out is the server's call");
        assert_eq!(plan.prior_findings.len(), 2);
        assert_eq!(plan.prior_findings[0].disposition, "fixed");
        assert_eq!(
            (
                plan.prior_findings[1].disposition.as_str(),
                plan.prior_findings[1].evidence.as_str()
            ),
            ("still_open", NOT_ASSESSED)
        );
        assert_eq!(plan.verification_notes.len(), 1);
        validate_prior_findings(&plan, &ids).expect("valid after filling");
        // A missing summary is synthesised too.
        let bare = parse_plan(r#"{"lenses":["Security"]}"#, &[]).unwrap();
        assert!(bare.summary.contains("Security"));
    }

    #[test]
    fn unusable_answers_are_rejected_with_the_precise_reason() {
        let err = |r: Result<ReviewScopePlan, String>| r.unwrap_err();
        assert!(err(parse_plan("not json", &[])).starts_with("Invalid scope assessment"));
        assert!(err(parse_plan(r#"["a.go"]"#, &[])).contains("must be a JSON object, not an array"));
        assert!(err(parse_plan(r#"{"summary":"s","lenses":[]}"#, &[])).contains("no lens"));
        assert!(err(parse_plan(r#"{"summary":"s"}"#, &[])).contains("no lens"));
        assert!(err(parse_plan("  ", &[])).contains("empty"));
        // At the run level: an empty array / a list of paths is no plan, two
        // objects are not one plan, and a failed session reports its own note.
        let no_plan = err(plan_from_run(&run_of(&[]), "", &[]));
        assert!(no_plan.contains("without a plan"), "{no_plan}");
        assert!(err(plan_from_run(&run_of(&["{}", "{}"]), "", &[])).contains("2 items"));
        let failed = crate::session::AgentRunResult {
            findings: vec![],
            errored: true,
            reason: Some(crate::session::PROMPT_REFUSED_NOTE.into()),
        };
        let why = err(plan_from_run(
            &failed,
            crate::session::PROMPT_REFUSED_NOTE,
            &[],
        ));
        assert!(why.contains("never reached the agent"), "{why}");
        assert!(!why.contains("no plan"));
        assert!(err(plan_from_run(&failed, "", &[])).contains("session failed"));
        // The real reason reaches the fallback plan's summary.
        let cfg = crate::engine::default_review_config("claude");
        assert!(configured_plan(&cfg, &[], &why)
            .summary
            .contains("never reached the agent"));
    }

    #[test]
    fn size_tiers_follow_changed_lines_with_tests_included() {
        let cfg = crate::engine::default_review_config("claude");
        let diff = "diff --git a/x b/x\n--- a/x\n+++ b/x\n+a\n-b\n c\n+d\n";
        assert_eq!(changed_lines(diff), 3);
        assert_eq!(pr_size(999, &cfg), "small");
        assert_eq!(pr_size(1_000, &cfg), "small");
        assert_eq!(pr_size(1_001, &cfg), "medium");
        assert_eq!(pr_size(5_000, &cfg), "medium");
        assert_eq!(pr_size(5_001, &cfg), "large");
        let mut tuned = cfg.clone();
        tuned.large_pr_lines = Some(100);
        assert_eq!(pr_size(101, &tuned), "large");
    }

    #[test]
    fn a_large_pr_runs_every_configured_lens_on_every_provider() {
        let mut cfg = crate::engine::default_review_config("claude");
        cfg.agents[0].provider = "claude".into();
        cfg.agents[0].providers = vec!["claude".into()];
        if cfg.agents.len() > 1 {
            cfg.agents[1].provider = "codex".into();
            cfg.agents[1].providers = vec!["codex".into()];
        }
        let prior = vec!["01P".to_string()];
        let plan = large_pr_plan(&cfg, &prior, 12_000);
        validate_prior_findings(&plan, &prior).expect("large plan is valid");
        let agents = selected_agents(&plan, &cfg);
        assert_eq!(agents.len(), cfg.agents.len(), "one row per lens");
        let pool_len = if cfg.agents.len() > 1 { 2 } else { 1 };
        assert!(
            agents.iter().all(|a| a.providers.len() == pool_len),
            "every provider per lens"
        );
        assert!(scope_summary_line(&plan).contains("all providers"));
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
