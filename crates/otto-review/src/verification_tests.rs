//! A review that did not finish must never read as a verdict, and the
//! reviewers' finished work must never be thrown away. The fixtures are the
//! 2026-11-12 "PR Reviewer" runs: every lens scored 7–10 with 10–16 findings,
//! the verifier never started, and the step reported "score 0 — FAILED".

use super::*;
use crate::engine::{draft_comments_from_json, parse_draft_comments, DraftComment};
use serde_json::json;

fn finding(path: &str, line: u32, severity: &str, body: &str) -> ReviewFinding {
    ReviewFinding {
        path: Some(path.into()),
        line: Some(line),
        line_end: None,
        severity: severity.into(),
        body: body.into(),
        lens: None,
    }
}

/// Five lenses, three findings each (distinct), like last night's batches.
fn batches() -> Vec<Vec<ReviewFinding>> {
    (0..5)
        .map(|b| {
            (0..3)
                .map(|j| {
                    finding(
                        &format!("src/l{b}.ts"),
                        10 + 20 * j,
                        "warn",
                        &format!("lens {b} claim {j}"),
                    )
                })
                .collect()
        })
        .collect()
}

fn verdict(ids: &[&str], disposition: &str, evidence: Option<&str>) -> DraftComment {
    serde_json::from_value(json!({
        "path": "src/l0.ts", "line": 10, "severity": "warn", "body": "verified defect",
        "disposition": disposition, "evidence": evidence, "source_ids": ids,
    }))
    .unwrap()
}

fn agent(name: &str, status: &str) -> ReviewAgentState {
    serde_json::from_value(json!({"name": name, "provider": "claude", "model": "",
        "status": status, "note": "", "comment_count": 0}))
    .unwrap()
}

fn review(status: &str, error: Option<&str>, agents: Value, comments: Value) -> Review {
    serde_json::from_value(json!({
        "id": "r", "repo_id": "repo", "pr_number": 7, "status": status, "error": error,
        "comments": comments, "agents": agents, "created_at": "2026-11-12T00:00:00Z",
        "scope_plan": {"summary": "s", "lenses": [], "verification_notes": [
            "not_relevant: x — y", format!("{COVERAGE_NOTE_PREFIX}31 of 34 reviewer claims verified")]},
    }))
    .unwrap()
}

use serde_json::Value;

fn comment(body: &str) -> Value {
    json!({"id": body, "review_id": "r", "path": "a.ts", "line": 1, "severity": "warn",
        "body": body, "state": "draft", "posted": false, "created_at": "2026-11-12T00:00:00Z"})
}

#[test]
fn one_disposition_list_offers_unverified() {
    assert_eq!(UNVERIFIED, "unverified");
    assert!(NOTE_DISPOSITIONS.contains(&UNVERIFIED));
}

/// Item 1: the summarizer failed → the deterministic floor's findings are
/// STORED, every one marked unverified (they were all suppressed before, so
/// 0 comments → "score 0").
#[test]
fn a_failed_verifier_keeps_every_reviewer_finding_marked_unverified() {
    let b = batches();
    let floor = crate::prompts::deterministic_comments(&"r".into(), &b);
    assert_eq!(floor.len(), 15);
    let s = settle(
        floor,
        &b,
        Some("submit: agent TUI never drew in session s1"),
    );
    assert!(s.incomplete);
    assert_eq!((s.accounted, s.total, s.unverified), (0, 15, 15));
    assert_eq!(s.comments.len(), 15);
    for c in &s.comments {
        assert!(is_unverified(&c.body), "{}", c.body);
        assert_eq!(c.disposition.as_deref(), Some(UNVERIFIED));
        assert!(c.reasoning.as_deref().unwrap().starts_with("unverified: "));
    }
    let reviewers: Vec<_> = (0..5).map(|i| agent(&format!("lens{i}"), "done")).collect();
    let why = incomplete_reason(&s, Some("agent TUI never drew"), None, &reviewers).unwrap();
    assert!(why.starts_with(INCOMPLETE_PREFIX), "{why}");
    assert!(
        why.contains("final verifier agent failed (agent TUI never drew)"),
        "{why}"
    );
    assert!(
        why.contains("15 reviewer findings are stored unverified"),
        "{why}"
    );
    // The stored error carries the error kind first; the cause is still found.
    let stored = format!("invalid: {why}");
    assert!(incomplete_cause(&stored)
        .unwrap()
        .starts_with("the final verifier"));
    // Marking is idempotent.
    let again = settle(s.comments, &b, Some("x"));
    assert!(again
        .comments
        .iter()
        .all(|c| !c.body[UNVERIFIED_MARK.len()..].starts_with(UNVERIFIED_MARK)));
}

/// Item 3: one claim the verifier did not account for no longer errors the
/// whole review — the verified comments stand, the claim is carried forward
/// unverified (and still counts), and coverage is reported.
#[test]
fn one_uncovered_claim_is_carried_forward_not_fatal() {
    let b = vec![vec![
        finding("a.ts", 1, "bug", "first"),
        finding("a.ts", 40, "warn", "second"),
        finding("b.ts", 9, "warn", "third"),
    ]];
    let parsed = vec![
        verdict(
            &["batch:1:claim:1"],
            "verified",
            Some("a.ts:1 traced to api.ts:3"),
        ),
        verdict(
            &["batch:1:claim:2"],
            "not_relevant",
            Some("guarded at a.ts:38"),
        ),
    ];
    let s = settle(parsed, &b, None);
    assert!(!s.incomplete);
    assert_eq!((s.accounted, s.total, s.unverified), (2, 3, 1));
    assert_eq!(s.comments.len(), 2, "one verified + one carried");
    assert!(!is_unverified(&s.comments[0].body));
    assert_eq!(s.comments[1].body, format!("{UNVERIFIED_MARK}third"));
    assert_eq!(s.comments[1].severity, "warn", "severity survives");
    assert!(s
        .notes
        .iter()
        .any(|n| n.starts_with("not_relevant: verified defect")));
    assert!(s.notes.contains(&format!(
        "{COVERAGE_NOTE_PREFIX}2 of 3 reviewer claims verified; 1 carried forward unverified"
    )));
    let done = vec![agent("correctness", "done")];
    assert_eq!(incomplete_reason(&s, None, None, &done), None);
}

/// Anti-laundering: an unknown id is not coverage, a repeated id counts
/// once, and a verdict without evidence covers nothing.
#[test]
fn unknown_duplicate_and_unevidenced_ids_never_count_as_coverage() {
    let b = vec![vec![
        finding("a.ts", 1, "bug", "one"),
        finding("a.ts", 30, "bug", "two"),
    ]];
    let parsed = vec![
        verdict(
            &["batch:1:claim:1", "batch:1:claim:1", "invented"],
            "verified",
            Some("a.ts:1"),
        ),
        verdict(&["batch:1:claim:1"], "verified", Some("a.ts:1 again")),
        verdict(&["batch:1:claim:2"], "verified", None),
    ];
    let s = settle(parsed, &b, None);
    assert_eq!((s.accounted, s.total), (1, 2));
    assert_eq!(s.unverified, 1);
    assert!(s
        .comments
        .iter()
        .any(|c| c.body == format!("{UNVERIFIED_MARK}two")));
}

/// Only when the verifier accounted for NOTHING does the review fall to the
/// incomplete state.
#[test]
fn a_verifier_that_accounts_for_nothing_is_incomplete() {
    let b = batches();
    let s = settle(vec![verdict(&[], "verified", Some("x.ts:1"))], &b, None);
    assert!(s.incomplete);
    assert_eq!(s.unverified, 15);
    let why = incomplete_reason(&s, None, None, &[]).unwrap();
    assert!(why.contains("accounted for none of the 15"), "{why}");
    // No claims at all and a clean verifier: complete.
    let clean = settle(vec![], &[vec![], vec![]], None);
    assert!(!clean.incomplete && clean.notes.is_empty());
}

/// Item 6: a reviewer that failed after its retries is named; the others'
/// verified findings stand. A summarizer-only retry is a full verification;
/// a partial diff is not.
#[test]
fn a_missing_lens_or_a_partial_diff_is_named_and_a_summarizer_retry_is_not() {
    let b = vec![vec![finding("a.ts", 1, "bug", "one")]];
    let s = settle(
        vec![verdict(&["batch:1:claim:1"], "verified", Some("a.ts:1"))],
        &b,
        None,
    );
    let rows = vec![
        agent("correctness", "done"),
        agent("security", "error"),
        agent("tests", "skipped"),
    ];
    let why = incomplete_reason(&s, None, None, &rows).unwrap();
    assert!(
        why.contains("reviewer(s) did not finish: security"),
        "{why}"
    );
    let ok = vec![agent("correctness", "done")];
    assert_eq!(
        incomplete_reason(&s, None, Some(SUMMARIZER_RETRY_BLOCKER), &ok),
        None
    );
    let why = incomplete_reason(&s, None, Some("partial diff (files omitted)"), &ok).unwrap();
    assert!(why.contains("partial diff"), "{why}");
}

/// Item 5: start/delivery/provider failures are retried; cancellation, the
/// deadline and a real (empty) answer are not.
#[test]
fn only_start_and_provider_failures_retry_the_summarizer() {
    for e in [
        "submit: agent TUI never drew in session s1",
        "submit: could not paste the prompt into session s1",
        "submit: run ended before the paste into session s1",
        "agent session exited before accepting the prompt",
        "API Error: 529 overloaded",
    ] {
        assert!(summarizer_retryable(e), "{e}");
    }
    for e in [
        "review cancelled",
        "summarizer absolute deadline exceeded",
        "returned no comments for 4 findings",
        "agent session exited before replying",
    ] {
        assert!(!summarizer_retryable(e), "{e}");
    }
    const { assert!(SUMMARIZER_ATTEMPTS >= 2) };
}

/// Item 2: the workflow step's view of last night's review — not a verdict,
/// no score, the unverified findings counted, and a chat message that says
/// so (never "Findings: none 🎉").
#[test]
fn last_nights_review_reports_incomplete_with_no_score() {
    let err = format!(
        "invalid: {INCOMPLETE_PREFIX}the final verifier agent failed (agent TUI never drew); 2 reviewer findings are stored unverified. Retry the summarizer to verify them."
    );
    let r = review(
        "error",
        Some(&err),
        json!([{"name":"correctness","provider":"claude","model":"","status":"done","note":"","comment_count":0,"score":8},
               {"name":"summary","provider":"claude","model":"","status":"done","note":"","comment_count":2,"fallback":true}]),
        json!([
            comment(&format!("{UNVERIFIED_MARK}a")),
            comment(&format!("{UNVERIFIED_MARK}b"))
        ]),
    );
    let rep = report(Some(&r), "error");
    assert_eq!(rep.verification, "incomplete");
    assert_eq!(rep.unverified, 2);
    assert!(rep.verifier_failed && !rep.partial);
    assert!(rep.cause.starts_with("the final verifier agent failed"));
    assert_eq!(
        rep.coverage.as_deref(),
        Some("31 of 34 reviewer claims verified")
    );
    assert_eq!(rep.gate_status("error", false), "error");
    assert_eq!(
        rep.gate_status("error", true),
        "done",
        "allow_summary_fallback opt-in"
    );

    let s = rep.step(false, false, false, "error", (40, 40), "ignored".into());
    assert_eq!(s.outcome, "incomplete");
    assert!(s.score.is_null() && s.review_score.is_null());
    assert_eq!(s.score_text, "incomplete");
    assert_eq!(s.partial_score, None);
    assert!(s
        .reason
        .starts_with("review incomplete — not a verdict: the final verifier"));

    let out = json!({"outcome": s.outcome, "score": s.score, "no_changes": false,
        "findings": [], "lens_scores": [{"lens":"correctness","score":8,"reason":""}]});
    let msg = step_chat("🔍 *Review*", out.as_object().unwrap(), &rep);
    assert!(
        msg.starts_with("🔍 *Review* — ⚠️ Review incomplete — not a verdict"),
        "{msg}"
    );
    assert!(
        msg.contains("Why: the final verifier agent failed"),
        "{msg}"
    );
    assert!(
        msg.contains("2 reviewer finding(s) stored unverified"),
        "{msg}"
    );
    assert!(msg.contains("correctness: *8/10*"), "{msg}");
    assert!(msg.contains("retry the summarizer"), "{msg}");
    assert!(!msg.contains("none 🎉") && !msg.contains("/100"), "{msg}");
}

/// Item 6: one lens missing — the score over what was verified is shown as
/// partial, the step is still not a pass.
#[test]
fn a_missing_lens_gives_a_partial_score_and_names_the_lens() {
    let err = format!("invalid: {INCOMPLETE_PREFIX}reviewer(s) did not finish: security; …");
    let r = review(
        "error",
        Some(&err),
        json!([{"name":"correctness","provider":"claude","model":"","status":"done","note":"","comment_count":0},
               {"name":"security","provider":"codex","model":"","status":"error","note":"timeout","comment_count":0},
               {"name":"summary","provider":"claude","model":"","status":"done","note":"","comment_count":1}]),
        json!([comment("verified one")]),
    );
    let rep = report(Some(&r), "error");
    assert_eq!(rep.missing_lenses, vec!["security".to_string()]);
    assert!(rep.partial && !rep.verifier_failed);
    let s = rep.step(false, false, false, "error", (72, 72), String::new());
    assert_eq!((s.outcome, s.partial_score), ("incomplete", Some(72)));
    assert_eq!(s.score_text, "partial — 72/100");
    let out = json!({"outcome": "incomplete", "partial_score": 72, "no_changes": false});
    let msg = step_chat("H", out.as_object().unwrap(), &rep);
    assert!(
        msg.contains("Lenses that did not finish: security"),
        "{msg}"
    );
    assert!(msg.contains("Partial score: 72/100"), "{msg}");
}

#[test]
fn completed_reviews_keep_their_verdict_message() {
    let r = review("done", None, json!([]), json!([]));
    let rep = report(Some(&r), "done");
    assert_eq!(rep.verification, "verified");
    let s = rep.step(
        false,
        true,
        false,
        "done",
        (60, 60),
        "1 must-fix finding(s) remain open".into(),
    );
    assert_eq!((s.outcome, s.score.clone()), ("failed", json!(60)));
    let out = json!({"outcome": "failed", "score": 60, "threshold": 80, "reason": s.reason,
        "no_changes": false, "open": 1, "findings": ["[bug] a.ts:1 — x"], "lens_scores": []});
    let msg = step_chat("H", out.as_object().unwrap(), &rep);
    assert!(
        msg.starts_with("H done — score *60/100* (pass ≥ 80) — ⚠️ 1 must-fix"),
        "{msg}"
    );
    assert!(
        msg.contains("Findings (1 open):\n • [bug] a.ts:1 — x"),
        "{msg}"
    );
    let pass = json!({"outcome": "passed", "score": 95, "threshold": 80, "no_changes": false, "findings": []});
    let msg = step_chat("H", pass.as_object().unwrap(), &rep);
    assert!(
        msg.contains("✅ passed") && msg.ends_with("Findings: none 🎉"),
        "{msg}"
    );
    let empty = json!({"outcome": "failed", "no_changes": true, "base": "main"});
    let msg = step_chat("H", empty.as_object().unwrap(), &rep);
    assert!(
        msg.contains("empty diff vs `main`") && msg.contains("No reviewers ran"),
        "{msg}"
    );
    // Not awaited: pending, not failed, no score.
    let rep = report(
        Some(&review("running", None, json!([]), json!([]))),
        "running",
    );
    let s = rep.step(false, false, false, "running", (100, 100), String::new());
    assert_eq!((s.outcome, s.score.is_null()), ("pending", true));
}

// --- Item 4: per-element lenient parsing of the summarizer handoff ---

/// Contract: the shape the verifier prompt asks for parses — a verified
/// comment, a note for each rejection disposition the prompt offers, and the
/// overall score element.
#[test]
fn the_prompts_own_shape_parses() {
    let cfg = crate::engine::default_review_config("claude").summarizer;
    let prompt = crate::prompts::summarizer_prompt("ctx", &cfg, &batches());
    assert!(prompt.contains("source_ids") && prompt.contains("disposition=verified"));
    let mut items = vec![
        json!({"path":"src/l0.ts","line":10,"severity":"warn","body":"x",
        "disposition":"verified","evidence":"src/l0.ts:10 traced","source_ids":["batch:1:claim:1"]}),
    ];
    for d in NOTE_DISPOSITIONS {
        items.push(
            json!({"body":"claim","disposition":d,"evidence":"e","source_ids":["batch:2:claim:1"]}),
        );
    }
    items.push(json!({"lens_score": 7, "score_reason": "should-fix only"}));
    let text = Value::Array(items).to_string();
    assert!(crate::summarizer::valid_result(&text));
    assert_eq!(draft_comments_from_json(&text).unwrap().len(), 4);
}

/// Each malformed-element shape is coerced or dropped on its own; it no
/// longer rejects the file (which ran the verifier to its hard deadline).
#[test]
fn malformed_elements_are_coerced_or_dropped_one_by_one() {
    let text = json!([
        {"body": "null severity", "severity": null},
        {"body": "string line", "line": "12", "line_end": "L14"},
        {"body": "range", "line": "42-45"},
        {"title": "no body at all"},
        {"body": "one id as a string", "source_ids": "batch:1:claim:2"},
        {"body": "mixed ids", "source_ids": ["batch:1:claim:3", 7, null]},
        {"body": "object evidence", "evidence": {"file": "a.ts"}, "path": ["a.ts"]},
        {"body": 42, "severity": 3},
        "a bare string",
        {"lens_score": 9, "score_reason": "ok"}
    ])
    .to_string();
    assert!(crate::summarizer::valid_result(&text));
    let c = draft_comments_from_json(&text).unwrap();
    assert_eq!(
        c.len(),
        7,
        "the bodiless element and the bare string are dropped"
    );
    assert_eq!(c[0].severity, "info");
    assert_eq!((c[1].line, c[1].line_end), (Some(12), Some(14)));
    assert_eq!((c[2].line, c[2].line_end), (Some(42), Some(45)));
    assert_eq!(c[3].source_ids, vec!["batch:1:claim:2".to_string()]);
    assert_eq!(c[4].source_ids, vec!["batch:1:claim:3".to_string()]);
    assert!(c[5].evidence.is_none() && c[5].path.is_none());
    assert_eq!((c[6].body.as_str(), c[6].severity.as_str()), ("42", "3"));
    // An empty array is a valid (empty) result; an array of nothing but
    // non-comments is not — the agent misread the format.
    assert!(crate::summarizer::valid_result("[]"));
    assert!(!crate::summarizer::valid_result(
        r#"[{"title":"x"},{"lens_score":5}]"#
    ));
    assert!(!crate::summarizer::valid_result(
        r#"{"body":"not an array"}"#
    ));
}

/// A real-shaped payload with ONE broken element keeps every other verdict.
#[test]
fn a_real_payload_with_one_broken_element_keeps_the_rest() {
    let v = |ids: &[&str], d: &str| {
        json!({"path":"a.go","line":1,"severity":"warn","body":"b",
            "evidence":"a.go:1 traced","disposition":d,"source_ids":ids})
    };
    let payload = json!([
        v(&["batch:1:claim:1", "batch:4:claim:1"], "verified"),
        v(&["batch:5:claim:1"], "verified"),
        {"path":"a.go","line":"7","severity":null,"evidence":"a.go:7","disposition":"verified","source_ids":["batch:5:claim:2"]},
        v(&["batch:7:claim:1"], "unverified"),
        v(&["batch:8:claim:1"], "not_relevant"),
        {"lens_score":7,"score_reason":"should-fix only"}
    ])
    .to_string();
    let parsed = parse_draft_comments(&"r".into(), &payload);
    // The bodiless verdict is not a comment; it rides along as an unreadable
    // stand-in so the notes can count it.
    assert_eq!(parsed.iter().filter(|c| !c.unreadable()).count(), 4);
    assert_eq!(parsed.iter().filter(|c| c.unreadable()).count(), 1);
    assert_eq!(parsed.iter().filter(|c| !c.suppressed()).count(), 2);
    let b: Vec<Vec<ReviewFinding>> = [1, 0, 0, 1, 2, 0, 1, 1]
        .iter()
        .map(|n| {
            (0..*n)
                .map(|j| finding("a.go", j + 1, "warn", "c"))
                .collect()
        })
        .collect();
    let s = settle(parsed, &b, None);
    assert!(s.comments.iter().all(|c| !c.unreadable()), "never stored");
    let note = s
        .notes
        .iter()
        .find(|n| n.starts_with(UNREADABLE_NOTE_PREFIX))
        .expect("unreadable note");
    assert!(
        note.starts_with("Verifier output: 1 of 5 items could not be read"),
        "{note}"
    );
    assert!(note.contains("item 3: no text `body`"), "{note}");
    // batch:5:claim:2 was in the unreadable item: carried forward, not lost.
    assert_eq!((s.accounted, s.total, s.unverified), (5, 6, 1));
}
