use super::*;
use crate::context::RunContextFiles;
use serde_json::json;

struct Host;

#[async_trait::async_trait]
impl ScriptHost for Host {
    async fn repo(&self, _: &str) -> Option<(String, String)> {
        None
    }
    async fn sandbox(&self, _: &str) -> Option<otto_sandbox::SandboxPolicy> {
        None
    }
    fn login_path(&self) -> Option<String> {
        Some("/usr/bin:/bin".into())
    }
}

fn crit(id: &str, verify: &str, extra: Value) -> Criterion {
    let mut v = json!({"id": id, "text": format!("criterion {id}"), "verify": verify});
    v.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    serde_json::from_value(v).unwrap()
}

/// GL1.2: a command criterion is ground truth — exit status first, the
/// output tail after it, every byte in the criterion's log.
#[tokio::test]
async fn commands_are_ground_truth_and_rules_cite_the_step() {
    let dir = tempfile::tempdir().unwrap();
    let files = RunContextFiles::create(dir.path(), "run-1");
    let cwd = dir.path().to_str().unwrap().to_string();
    let at = ScriptRun {
        files: &files,
        run_id: "run-1",
        run_cwd: &cwd,
        step_base: "step2-check-criteria-iter1",
    };
    let criteria = vec![
        crit(
            "ok",
            "command",
            json!({"command": "echo fine; test -n \"$OTTO_RUN_ID\""}),
        ),
        crit(
            "bad",
            "command",
            json!({"command": "echo broken >&2; exit 3"}),
        ),
        crit(
            "rule",
            "expr",
            json!({"expr": "steps.review.passed == true"}),
        ),
        crit("judge", "agent", json!({})),
    ];
    let ictx = json!({"iteration": 1, "steps": {"review": {"passed": false, "summary": "review 72/100, 2 blocking findings"}}});
    let d = decide_commands_and_rules(&Host, &at, &criteria, &ictx)
        .await
        .unwrap();
    assert_eq!(d.len(), 3, "agent criteria are not decided here");
    assert!(d[0].met);
    assert!(d[0].evidence.starts_with("verify command exited 0"));
    assert!(!d[1].met);
    assert!(
        d[1].evidence.starts_with("verify command failed (exit 3)"),
        "{}",
        d[1].evidence
    );
    assert!(d[1].evidence.contains("broken"));
    assert_eq!(d[1].verified_by.as_deref(), Some("command"));
    assert!(!d[2].met);
    assert_eq!(
        d[2].evidence,
        "steps.review.passed == true is false — review 72/100, 2 blocking findings"
    );
    let log = dir
        .path()
        .join("workflow-context/run-1/step2-check-criteria-iter1.bad.log");
    let log = std::fs::read_to_string(&log).unwrap_or_else(|e| panic!("{}: {e}", log.display()));
    assert!(log.contains("broken"));
}

#[test]
fn params_need_criteria_and_read_the_evaluator() {
    assert!(CheckParams::parse(&json!({})).is_err());
    assert!(CheckParams::parse(&json!({"criteria": []})).is_err());
    let p = CheckParams::parse(&json!({
        "criteria": [{"id": "a", "text": "t", "verify": "agent"}],
        "evaluator": {"provider": "codex", "model": " ", "effort": "low"},
        "final_review": true,
    }))
    .unwrap();
    assert!(p.needs_agent() && p.final_review);
    assert_eq!(p.provider.as_deref(), Some("codex"));
    assert_eq!(p.model, None);
    assert_eq!(p.effort.as_deref(), Some("low"));
}

#[test]
fn the_previous_iterations_check_and_the_evaluators_answer() {
    assert_eq!(
        previous_iteration_id("goal#3.1").as_deref(),
        Some("goal#2.1")
    );
    assert_eq!(previous_iteration_id("goal#1.1"), None);
    assert_eq!(previous_iteration_id("check"), None);
    let json = r#"{"criteria":[{"id":"a","met":true,"evidence":"saw it"}],"verdict":"achieved"}"#;
    assert!(evaluation(Some(json), "").is_some());
    assert!(evaluation(None, &format!("done:\n```json\n{json}\n```")).is_some());
    assert!(evaluation(Some("not json"), "nothing").is_none());
}

/// DR-107: the record goes to the check parked on that criterion only, is
/// bound to the criterion's revision and the iteration, and a second record
/// (or one after the wait ended) is refused.
#[tokio::test]
async fn a_person_records_only_the_criterion_the_check_waits_for() {
    let c = crit("look", "human", json!({}));
    let (a, mut rx, guard) = park("r1", "goal", &c, Some(2));
    assert_eq!(a.criterion_id.as_deref(), Some("look"));
    assert!(deliver("r1", "goal", "other", true, "x", "Ann").is_err());
    assert!(deliver("r1", "goal", "look", true, "  ", "Ann").is_err());
    deliver("r1", "goal", "look", true, "Saw it", "Ann").unwrap();
    let e = rx.recv().await.unwrap();
    assert_eq!(
        (e.met, e.iteration, e.by.as_str()),
        (Some(true), Some(2), "Ann")
    );
    assert_eq!(e.revision.as_deref(), Some(criteria::revision(&c).as_str()));
    assert!(deliver("r1", "goal", "look", true, "again", "Ann").is_err());
    let (_a, _rx, guard2) = park("r1", "goal", &c, Some(2));
    drop(guard2);
    assert!(
        deliver("r1", "goal", "look", true, "late", "Ann").is_err(),
        "the wait ended"
    );
    drop(guard);
}

#[tokio::test]
async fn rule_over_review_and_command_record_structured_progress() {
    let dir = tempfile::tempdir().unwrap();
    let files = RunContextFiles::create(dir.path(), "run-prog");
    let cwd = dir.path().to_str().unwrap().to_string();
    let at = ScriptRun {
        files: &files,
        run_id: "run-prog",
        run_cwd: &cwd,
        step_base: "step2-check-iter3",
    };
    let criteria = vec![
        crit(
            "review-passes",
            "expr",
            json!({"expr": "steps.review.passed == true"}),
        ),
        crit(
            "coverage",
            "expr",
            json!({"expr": "steps.bench.coverage >= 80"}),
        ),
        crit(
            "tests",
            "command",
            json!({"command": "printf 'running tests\\n---\\nFAIL: TestCart\\n\\n' && exit 1"}),
        ),
    ];
    let ictx = json!({
        "iteration": 3,
        "steps": {
            "review": {
                "passed": false,
                "score": 72,
                "threshold": 95,
                "blocking": 3,
                "advisory": 2,
                "severity": {"bug": 3, "warn": 2, "info": 1},
                "complete": true,
                "outcome": "failed",
                "reason": "3 must-fix finding(s) remain open"
            },
            "bench": {
                "coverage": 64
            }
        }
    });
    let d = decide_commands_and_rules(&Host, &at, &criteria, &ictx)
        .await
        .unwrap();
    assert_eq!(d.len(), 3);

    // 1. Rule over a review step captures score, threshold, must-fix, should-fix, and blocker label.
    let rp = d[0].progress.as_ref().expect("review progress");
    assert_eq!((rp.value, rp.target), (Some(72.0), Some(95.0)));
    assert_eq!(
        rp.label.as_deref(),
        Some("3 must-fix, 2 should-fix findings left")
    );
    assert_eq!(rp.values.get("steps.review.passed"), Some(&json!(false)));
    assert_eq!(rp.values.get("score"), Some(&json!(72)));
    assert_eq!(rp.values.get("threshold"), Some(&json!(95)));
    assert_eq!(rp.values.get("must_fix"), Some(&json!(3)));
    assert_eq!(rp.values.get("should_fix"), Some(&json!(2)));
    assert!(
        d[0].evidence.contains("72 / 95"),
        "evidence carries score/target: {}",
        d[0].evidence
    );

    // 2. Generic numeric rule captures value and target without review-specific fields.
    let bp = d[1].progress.as_ref().expect("bench progress");
    assert_eq!((bp.value, bp.target), (Some(64.0), Some(80.0)));
    assert_eq!(bp.values.get("steps.bench.coverage"), Some(&json!(64)));

    // 3. Command criterion captures exit code and last meaningful line (skipping blank/decor lines).
    let cp = d[2].progress.as_ref().expect("command progress");
    assert_eq!(cp.values.get("exit_code"), Some(&json!(1)));
    assert_eq!(cp.values.get("last_line"), Some(&json!("FAIL: TestCart")));
    assert_eq!(cp.label.as_deref(), Some("exit 1 — FAIL: TestCart"));
}
