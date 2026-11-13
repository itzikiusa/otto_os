//! Prompt ⇄ reader contract tests. Each canonical example is the text the
//! prompt embeds AND the input the production reader gets here; each list a
//! prompt offers is checked against what the checker accepts. The fixtures
//! are the three payload shapes that ended real reviews in "error, score 0".

use super::*;
use crate::engine::{
    default_review_config, draft_comments_from_json, parse_draft_comments, DraftComment,
    NOTE_DISPOSITIONS,
};
use crate::session::{parse_findings, parse_findings_array, parse_lens_score, read_findings};
use crate::verification::{
    incomplete_reason, report, settle, INCOMPLETE_PREFIX, NO_REVIEWER_FINISHED,
};
use otto_core::domain::{CommentSeverity, Review, ReviewAgentState, ReviewFinding};
use serde_json::json;

fn finding(path: &str, line: u32, body: &str) -> ReviewFinding {
    ReviewFinding {
        path: Some(path.into()),
        line: Some(line),
        line_end: None,
        severity: "warn".into(),
        body: body.into(),
        lens: None,
    }
}

/// `sizes[b]` distinct claims in batch `b`.
fn batches(sizes: &[u32]) -> Vec<Vec<ReviewFinding>> {
    sizes
        .iter()
        .enumerate()
        .map(|(b, n)| {
            (0..*n)
                .map(|j| finding(&format!("src/b{b}.rs"), 10 + j, &format!("claim {b}/{j}")))
                .collect()
        })
        .collect()
}

fn row(name: &str, status: &str) -> ReviewAgentState {
    serde_json::from_value(json!({"name": name, "provider": "claude", "model": "",
        "status": status, "note": "", "comment_count": 0}))
    .unwrap()
}

fn review(status: &str, error: Option<&str>, agents: serde_json::Value) -> Review {
    serde_json::from_value(json!({
        "id": "r", "repo_id": "repo", "pr_number": 7, "status": status, "error": error,
        "comments": [], "agents": agents, "created_at": "2026-11-13T00:00:00Z",
        "scope_plan": {"summary": "s", "lenses": []},
    }))
    .unwrap()
}

fn run_of(raw: &str) -> crate::session::AgentRunResult {
    crate::session::AgentRunResult {
        findings: parse_findings(raw),
        errored: false,
        reason: None,
    }
}

// --- Reviewer findings ---------------------------------------------------

#[test]
fn contract_reviewer_prompt_embeds_its_example_and_the_reader_accepts_it() {
    let prompt = crate::prompts::reviewer_prompt("", 1, 2, "LENS", "", "", "/tmp/r.diff");
    assert!(prompt.contains(REVIEWER_OUTPUT_EXAMPLE));
    assert!(prompt.contains(&format!("\"{}\"", crate::session::LENS_SCORE_KEY)));
    assert!(prompt.contains(&severity_choices()));
    let (findings, unreadable) = read_findings(REVIEWER_OUTPUT_EXAMPLE).expect("an array");
    assert_eq!((findings.len(), unreadable), (1, 0));
    let f = &findings[0];
    assert_eq!(
        (f.path.as_deref(), f.line, f.severity.as_str()),
        (Some("src/auth.rs"), Some(42), "bug")
    );
    assert!(f.body.starts_with("allow() now returns true"));
    assert_eq!(
        parse_lens_score(REVIEWER_OUTPUT_EXAMPLE),
        Some((4, "one must-fix access-control defect".to_string()))
    );
}

/// Every severity a prompt offers is kept as is, every prompt renders the
/// same list, and whatever an agent writes is STORED as one of them.
#[test]
fn contract_every_listed_severity_is_kept_and_only_listed_ones_are_stored() {
    let listed: Vec<&str> = FINDING_SEVERITIES.iter().map(|(s, _)| *s).collect();
    for s in &listed {
        let raw = json!([{"path": "a.rs", "line": 1, "severity": s, "body": "b"}]).to_string();
        assert_eq!(parse_findings(&raw)[0].severity, *s);
        assert_eq!(CommentSeverity::normalize(s).as_str(), *s);
    }
    for off_list in ["Critical", "HIGH", "nit", "minor", "something else", ""] {
        let stored = CommentSeverity::normalize(off_list);
        assert!(listed.contains(&stored.as_str()), "{off_list}");
    }
    let cfg = default_review_config("claude");
    for a in &cfg.agents {
        assert!(a.prompt.contains(&finding_schema()), "{}", a.name);
    }
    assert!(cfg.summarizer.prompt.contains(&severity_choices()));
    assert!(crate::engine::workflow_review_lens_prompt().contains(&finding_schema()));
    let plan = crate::scope::configured_plan(&cfg, &[], "x");
    for a in crate::scope::selected_agents(&plan, &cfg) {
        assert!(a.prompt.contains(&severity_legend()), "{}", a.prompt);
    }
    assert_eq!(
        severity_legend(),
        "bug=must fix, warn=should fix, info=optional"
    );
}

#[test]
fn contract_orchestrator_prompt_embeds_the_lens_file_example_and_merge_keeps_the_lens() {
    let lenses = vec![crate::engine::OrchestratorLens {
        name: "Security".into(),
        slug: "security-review".into(),
        skill_text: "METHOD".into(),
        instructions: String::new(),
        read_only: true,
    }];
    let dir = std::path::PathBuf::from("/tmp");
    let prompt = crate::engine::compose_orchestrator_prompt(
        &lenses,
        "claude",
        |slug| crate::session::lens_findings_path_in(&dir, "R", 0, slug),
        &crate::session::findings_path_in(&dir, "R", 0),
    );
    assert!(prompt.contains(LENS_FILE_EXAMPLE));
    let f = parse_findings_array(LENS_FILE_EXAMPLE).expect("an array");
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].lens.as_deref(), Some("security-review"));
}

/// An element no reader can use is skipped AND counted; the rest stand.
#[test]
fn lenient_reviewer_elements_are_skipped_and_counted() {
    let raw = r#"Here are my findings:
```json
[{"path":"a.rs","line":"12","severity":null,"body":"kept","extra":{"x":1}},
 "a bare string", 7,
 {"file":"b.rs","line":3,"summary":"kept via summary"},
 {"lens_score":"8/10","score_reason":"fine"}]
```"#;
    let (findings, unreadable) = read_findings(raw).expect("an array");
    assert_eq!((findings.len(), unreadable), (2, 2));
    assert_eq!(
        (findings[0].line, findings[0].severity.as_str()),
        (Some(12), "info")
    );
    assert_eq!(findings[1].path.as_deref(), Some("b.rs"));
    assert_eq!(parse_lens_score(raw).map(|s| s.0), Some(8));
    assert_eq!(
        crate::session::with_unreadable("2 findings".into(), unreadable),
        "2 findings · 2 items could not be read"
    );
    assert_eq!(
        crate::session::with_unreadable("1 finding".into(), 0),
        "1 finding"
    );
}

// --- Scope plan ------------------------------------------------------------

#[test]
fn contract_scope_prompt_embeds_its_example_and_the_reader_accepts_it() {
    let cfg = default_review_config("claude");
    let prompt = crate::scope::scope_prompt("/tmp/r.diff", "", "", &cfg);
    assert!(prompt.contains(SCOPE_PLAN_EXAMPLE));
    assert!(prompt.contains(&scope_transport_example()));
    assert!(prompt.contains(&format!(
        "disposition:{}",
        PRIOR_SCOPE_DISPOSITIONS.join("|")
    )));
    let prior = [SCOPE_EXAMPLE_PRIOR_ID.to_string()];
    let plan = crate::scope::plan_from_run(&run_of(&scope_transport_example()), "", &prior)
        .expect("the prompt's own answer parses");
    assert_eq!(plan.lenses.len(), 1);
    assert_eq!(plan.lenses[0].name, "Security");
    assert!(plan.lenses[0].reason.contains("src/auth.rs:42"));
    assert_eq!(plan.related_modules.len(), 1);
    assert_eq!(plan.prior_findings[0].disposition, "still_open");
    assert!(plan.verification_notes.is_empty(), "nothing skipped");
}

/// Common deviations from the transport: the plan object alone, the plan
/// object as the array element, a fence and a sentence before it.
#[test]
fn lenient_scope_plan_without_its_transport_still_parses() {
    let prior = [SCOPE_EXAMPLE_PRIOR_ID.to_string()];
    let element = format!("[{SCOPE_PLAN_EXAMPLE}]");
    let fenced = format!("Here is the plan:\n```json\n{SCOPE_PLAN_EXAMPLE}\n```");
    for raw in [SCOPE_PLAN_EXAMPLE, element.as_str(), fenced.as_str()] {
        let plan = crate::scope::plan_from_run(&run_of(raw), "", &prior)
            .unwrap_or_else(|e| panic!("{e}: {raw}"));
        assert_eq!(plan.lenses[0].name, "Security", "{raw}");
    }
    // A reviewer's findings array is never mistaken for a plan.
    assert_eq!(parse_findings(REVIEWER_OUTPUT_EXAMPLE)[0].line, Some(42));
}

#[test]
fn contract_every_prior_scope_disposition_is_kept_and_nothing_else() {
    let ids = vec!["01P".to_string()];
    let plan_with = |d: &str| {
        json!({"summary": "s", "lenses": ["Security"],
            "prior_findings": [{"finding_id": "01P", "disposition": d, "evidence": "a.rs:1"}]})
        .to_string()
    };
    for d in PRIOR_SCOPE_DISPOSITIONS {
        let plan = crate::scope::parse_plan(&plan_with(d), &ids).unwrap();
        assert_eq!(plan.prior_findings[0].disposition, d);
        crate::scope::validate_prior_findings(&plan, &ids).unwrap();
    }
    for off_list in ["resolved", "open", "unverified", "verified"] {
        let plan = crate::scope::parse_plan(&plan_with(off_list), &ids).unwrap();
        assert_eq!(
            (
                plan.prior_findings[0].disposition.as_str(),
                plan.prior_findings[0].evidence.as_str()
            ),
            ("still_open", crate::scope::NOT_ASSESSED),
            "{off_list} must not be taken as a verdict"
        );
    }
}

#[test]
fn lenient_scope_lenses_are_skipped_and_counted() {
    let mut lenses: Vec<serde_json::Value> = (0..12).map(|i| json!(format!("Lens {i}"))).collect();
    lenses.push(json!({"reason": "no name"}));
    lenses.push(json!("lens 0"));
    let body = json!({"summary": "s", "lenses": lenses,
        "prior_findings": [{"disposition": "fixed", "evidence": "no id"}]})
    .to_string();
    let plan = crate::scope::parse_plan(&body, &[]).unwrap();
    assert_eq!(plan.lenses.len(), MAX_SCOPE_LENSES);
    assert_eq!(
        plan.verification_notes,
        ["Scope assessment: 4 lens entries skipped (1 without a name, 1 repeated, 2 over the lens cap); 1 previous-finding entry without a finding_id skipped."]
    );
    // Whole-output garbage still fails, naming what was expected.
    let err = crate::scope::parse_plan("I could not finish the assessment.", &[]).unwrap_err();
    assert!(
        err.contains("expected a JSON object with summary and lenses"),
        "{err}"
    );
}

// --- Verifier / summarizer -------------------------------------------------

#[test]
fn contract_verifier_prompt_embeds_its_example_and_the_reader_settles_it() {
    let cfg = default_review_config("claude").summarizer;
    let prompt = crate::prompts::summarizer_prompt("ctx", &cfg, &batches(&[2, 1]));
    assert!(prompt.contains(VERIFIER_OUTPUT_EXAMPLE));
    assert!(crate::summarizer::valid_result(VERIFIER_OUTPUT_EXAMPLE));
    let parsed = parse_draft_comments(&"r".into(), VERIFIER_OUTPUT_EXAMPLE);
    assert_eq!(parsed.len(), 2);
    assert!(!parsed[0].suppressed());
    assert_eq!(parsed[0].source_ids.len(), 2);
    assert_eq!(parsed[0].line, Some(42));
    assert!(parsed[1].suppressed() && parsed[1].verification_complete());
    assert_eq!(
        parse_lens_score(VERIFIER_OUTPUT_EXAMPLE).map(|s| s.0),
        Some(5)
    );
    assert!(crate::engine::verifier_covers_claims(&parsed, &[2, 1]));
    let s = settle(parsed, &batches(&[2, 1]), None);
    assert_eq!((s.accounted, s.total, s.unverified), (3, 3, 0));
    assert!(!s.incomplete);
    assert_eq!(s.comments.len(), 1, "the note is not a comment");
    assert_eq!(
        incomplete_reason(&s, None, None, &[row("lens", "done")]),
        None
    );
}

/// The dispositions the verifier prompt offers, read out of the prompt, are
/// exactly the ones the checker accepts — no more, no fewer.
#[test]
fn contract_the_checker_accepts_exactly_the_dispositions_the_prompt_offers() {
    let cfg = default_review_config("claude").summarizer;
    let prompt = crate::prompts::summarizer_prompt("ctx", &cfg, &batches(&[1]));
    let mut offered: Vec<&str> = prompt
        .split("disposition=")
        .skip(1)
        .flat_map(|rest| {
            let end = rest
                .find(|c: char| !(c.is_ascii_lowercase() || c == '_' || c == '|'))
                .unwrap_or(rest.len());
            rest[..end].split('|')
        })
        .collect();
    offered.sort_unstable();
    offered.dedup();
    let mut accepted = verifier_dispositions();
    accepted.sort_unstable();
    assert_eq!(offered, accepted);
    let claim = |d: &str| -> DraftComment {
        serde_json::from_value(json!({"body": "c", "disposition": d, "evidence": "a.rs:1"}))
            .unwrap()
    };
    for d in &accepted {
        assert!(claim(d).verification_complete(), "{d}");
    }
    for off_list in [
        "approved",
        "rejected",
        "fixed",
        "still_open",
        "false_positive",
        UNREADABLE,
        "",
    ] {
        assert!(!claim(off_list).verification_complete(), "{off_list}");
        assert!(claim(off_list).suppressed(), "{off_list}");
    }
    assert!(!NOTE_DISPOSITIONS.contains(&UNREADABLE));
}

/// A listed word in another spelling is the listed word; an unknown word is
/// still rejected.
#[test]
fn lenient_disposition_spellings_are_normalized() {
    let text = json!([
        {"body": "a", "disposition": "Verified", "evidence": "a.rs:1"},
        {"body": "b", "disposition": " not-relevant ", "evidence": "b.rs:2"},
        {"body": "c", "disposition": "NOT RELEVANT", "evidence": "c.rs:3"},
        {"body": "d", "disposition": "Approved", "evidence": "d.rs:4"}
    ])
    .to_string();
    let c = draft_comments_from_json(&text).unwrap();
    let d: Vec<_> = c
        .iter()
        .map(|c| c.disposition.as_deref().unwrap())
        .collect();
    assert_eq!(d, ["verified", "not_relevant", "not_relevant", "approved"]);
    assert!(!c[0].suppressed());
    assert!(c[1].verification_complete() && c[2].verification_complete());
    assert!(!c[3].verification_complete());
}

#[test]
fn lenient_verifier_items_are_skipped_counted_and_never_stored() {
    let text = json!([
        {"body": "kept", "disposition": "verified", "evidence": "a.rs:1", "source_ids": ["batch:1:claim:1"]},
        {"title": "no body", "disposition": "verified", "source_ids": ["batch:1:claim:2"]},
        ["not", "an", "object"],
        {"lens_score": 6, "score_reason": "x"}
    ])
    .to_string();
    let parsed = parse_draft_comments(&"r".into(), &text);
    assert_eq!(parsed.iter().filter(|c| c.unreadable()).count(), 2);
    let s = settle(parsed, &batches(&[2]), None);
    assert_eq!(s.comments.len(), 2, "one verified, one carried unverified");
    assert!(s.comments.iter().all(|c| !c.unreadable()));
    let note = s
        .notes
        .iter()
        .find(|n| n.starts_with(crate::verification::UNREADABLE_NOTE_PREFIX))
        .unwrap();
    assert!(
        note.contains("2 of 3 items could not be read")
            && note.contains("item 2: no text `body`")
            && note.contains("item 3: not a JSON object"),
        "{note}"
    );
    // Garbage as a whole is rejected with what was expected.
    let err = draft_comments_from_json("not json")
        .err()
        .expect("rejected");
    assert!(
        err.to_string()
            .contains("expected a JSON array of comments"),
        "{err}"
    );
}

/// A settled summary keeps the scope step's notes and replaces the rest.
#[test]
fn contract_scope_notes_survive_a_settled_summary() {
    let cfg = default_review_config("claude");
    let mut plan = crate::scope::configured_plan(&cfg, &[], "scope session failed: stuck");
    plan.verification_notes
        .push(format!("{} (1 of 2).", crate::restart::RESUME_NOTE));
    plan.verification_notes
        .push("Verification coverage: old".into());
    crate::verification::set_notes(&mut plan, &["Verification coverage: new".to_string()]);
    assert_eq!(
        plan.verification_notes,
        [
            "Scope assessment fell back to the configured lenses: scope session failed: stuck",
            "Verification coverage: new"
        ]
    );
}

// --- Historical payloads ---------------------------------------------------

/// #150/#154: related_modules / risks as objects ("invalid type: map,
/// expected a string") aborted 12 of 14 reviews before any lens ran.
#[test]
fn fixture_object_shaped_scope_plan_is_accepted() {
    let raw = include_str!("fixtures/scope_object_modules.json");
    let plan = crate::scope::plan_from_run(&run_of(raw), "", &[]).expect("accepted");
    // Object entries read subject first, whatever order the model used.
    assert_eq!(
        plan.related_modules[0],
        "clients/promotions/interface.go:78-102 — imported by go_marketing"
    );
    let module = &plan.related_modules[1];
    assert!(module.starts_with("go_bonus_management — "), "{module}");
    assert!(module.contains("bonus/client.go:3") && module.contains("compiles against"));
    assert_eq!(plan.risks[0], "kit/server field drift — models.go:12");
    assert_eq!(plan.lenses.len(), 2);
}

/// #153: an unparseable plan (or a scope session that failed) widens the
/// review to every configured lens; it never stops it.
#[test]
fn fixture_unparseable_scope_plan_falls_back_to_the_configured_lenses() {
    let cfg = default_review_config("claude");
    let prior = vec!["01A".to_string()];
    for raw in [
        "I ran out of time.",
        r#"[{"severity":"info","body":"{\"summary\": \"cut off"}]"#,
        r#"[{"severity":"info","body":"{}"},{"severity":"info","body":"{}"}]"#,
        "[]",
    ] {
        let why = crate::scope::plan_from_run(&run_of(raw), "", &prior).expect_err("not a plan");
        let plan = crate::scope::configured_plan(&cfg, &prior, &why);
        crate::scope::validate_prior_findings(&plan, &prior).expect("fallback is valid");
        assert_eq!(
            crate::scope::selected_agents(&plan, &cfg).len(),
            cfg.agents.len(),
            "{raw}"
        );
        assert!(plan.verification_notes[0].contains(&why), "{raw}");
    }
}

/// #149 → b7b5d1edd: the verifier's overall `{"lens_score"}` element made
/// every handoff invalid; the review fell back and ended "error, score 0".
#[test]
fn fixture_summarizer_handoff_with_its_score_element_is_a_verdict() {
    let raw = include_str!("fixtures/summarizer_score_element.json");
    assert!(crate::summarizer::valid_result(raw));
    let parsed = parse_draft_comments(&"r".into(), raw);
    assert_eq!(parsed.len(), 3);
    assert_eq!(parse_lens_score(raw).map(|s| s.0), Some(6));
    let s = settle(parsed, &batches(&[2, 2]), None);
    assert_eq!((s.accounted, s.total, s.unverified), (4, 4, 0));
    assert_eq!(s.comments.len(), 2);
    let rows = [row("correctness", "done"), row("security", "done")];
    assert_eq!(incomplete_reason(&s, None, None, &rows), None);
}

/// #153 (go_marketing #249): `unverified` offered by the prompt, rejected by
/// the check — 7 verified comments, yet "incomplete" and a workflow score 0.
#[test]
fn fixture_verifier_unverified_disposition_is_a_verdict() {
    let raw = include_str!("fixtures/verifier_unverified_go_marketing.json");
    assert!(crate::summarizer::valid_result(raw));
    let parsed = parse_draft_comments(&"r".into(), raw);
    let sizes = [1, 0, 0, 1, 3, 1, 4, 1];
    assert!(crate::engine::verifier_covers_claims(
        &parsed,
        &sizes.map(|n| n as usize)
    ));
    let s = settle(parsed, &batches(&sizes), None);
    assert_eq!((s.accounted, s.total, s.unverified), (11, 11, 0));
    assert_eq!(s.comments.len(), 7);
    assert!(s
        .notes
        .iter()
        .any(|n| n.starts_with("unverified: Claim batch:7:claim:1")));
    let rows: Vec<_> = (0..8).map(|i| row(&format!("lens{i}"), "done")).collect();
    assert_eq!(incomplete_reason(&s, None, None, &rows), None);
}

// --- An unfinished review never reads as a score ---------------------------

/// Every lens failed: the verifier had nothing to verify (and may still have
/// rated it 10/10). No score of any kind — not even a "partial" one.
#[test]
fn incomplete_when_every_lens_failed_has_no_score() {
    let s = settle(vec![], &batches(&[0, 0]), None);
    let rows = [row("correctness", "error"), row("security", "error")];
    let why = incomplete_reason(&s, None, None, &rows).expect("not a verdict");
    assert!(
        why.starts_with(&format!(
            "{INCOMPLETE_PREFIX}{NO_REVIEWER_FINISHED}: correctness, security"
        )),
        "{why}"
    );
    let r = review(
        "error",
        Some(&format!("invalid: {why}")),
        json!([
            {"name":"correctness","provider":"claude","model":"","status":"error","note":"timed out","comment_count":0},
            {"name":"security","provider":"codex","model":"","status":"error","note":"stuck","comment_count":0},
            {"name":"Summarizer","provider":"claude","model":"","status":"done","note":"0 final comments","comment_count":0,"score":10,"score_reason":"clean"}
        ]),
    );
    let rep = report(Some(&r), "error");
    assert_eq!(rep.verification, "incomplete");
    assert!(!rep.partial && !rep.verifier_failed);
    let step = rep.step(false, false, false, "error", (100, 100), String::new());
    assert_eq!((step.outcome, step.partial_score), ("incomplete", None));
    assert!(step.score.is_null() && step.review_score.is_null());
    assert_eq!(step.score_text, "incomplete");
    let scores = crate::prompts::lens_scores_json(Some(&r));
    assert!(scores.is_empty(), "the verifier rated nothing: {scores:?}");
    let out = json!({"outcome": step.outcome, "no_changes": false, "lens_scores": scores});
    let msg = crate::verification::step_chat("H", out.as_object().unwrap(), &rep);
    assert!(!msg.contains("/100") && !msg.contains("/10"), "{msg}");
    assert!(msg.contains("no reviewer finished"), "{msg}");
    // A re-run of the summarizer cannot help: the step launches a new review.
    assert_eq!(
        crate::restart::adoption(&r),
        crate::restart::Adoption::Launch
    );
}

/// The summarizer failed on both attempts (or answered nothing): every
/// reviewer finding is kept, unverified, and no score is reported.
#[test]
fn incomplete_when_the_summarizer_failed_twice_keeps_the_findings() {
    let b = batches(&[2, 1]);
    let floor = crate::prompts::deterministic_comments(&"r".into(), &b);
    let cause = "returned no comments for 3 findings";
    let s = settle(floor, &b, Some(cause));
    assert_eq!((s.unverified, s.comments.len()), (3, 3));
    let rows = [row("a", "done"), row("b", "done")];
    let why = incomplete_reason(&s, Some(cause), None, &rows).unwrap();
    let r = review(
        "error",
        Some(&format!("invalid: {why}")),
        json!([
            {"name":"a","provider":"claude","model":"","status":"done","note":"","comment_count":2,"score":7,
             "findings":[{"path":"src/b0.rs","line":10,"severity":"warn","body":"claim 0/0"}]},
            {"name":"b","provider":"claude","model":"","status":"done","note":"","comment_count":1},
            {"name":"Summarizer","provider":"claude","model":"","status":"done","note":"","comment_count":3,"fallback":true}
        ]),
    );
    let rep = report(Some(&r), "error");
    assert!(rep.verifier_failed && rep.verification == "incomplete");
    let step = rep.step(false, false, false, "error", (0, 0), String::new());
    assert!(step.score.is_null() && step.partial_score.is_none());
    assert_eq!(
        crate::restart::adoption(&r),
        crate::restart::Adoption::RerunSummarizer
    );
}

/// A restart that interrupted the review (or its summarizer, past the resume
/// limit) settles it with no score.
#[test]
fn incomplete_after_a_restart_has_no_score() {
    for error in [
        crate::restart::INTERRUPTED,
        crate::restart::INTERRUPTED_SUMMARIZING,
        crate::restart::RESUMES_EXHAUSTED,
    ] {
        let r = review(
            "error",
            Some(error),
            json!([
                {"name":"a","provider":"claude","model":"","status":"done","note":"","comment_count":1},
                {"name":"Summarizer","provider":"claude","model":"","status":"error","note":"","comment_count":0}
            ]),
        );
        let rep = report(Some(&r), "error");
        assert_eq!(rep.cause, error);
        let step = rep.step(false, false, false, "error", (0, 0), String::new());
        assert_eq!(
            (step.outcome, step.partial_score),
            ("incomplete", None),
            "{error}"
        );
        assert!(step.score.is_null(), "{error}");
    }
}
