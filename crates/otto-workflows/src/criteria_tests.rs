use super::*;
use chrono::Utc;
use serde_json::json;

fn crit(id: &str, verify: VerifyKind) -> Criterion {
    Criterion {
        id: id.into(),
        text: format!("criterion {id}"),
        verify,
        how: String::new(),
        command: None,
        expr: None,
        timeout_s: None,
    }
}

fn eval(verdict: Verdict, criteria: Value) -> Evaluation {
    serde_json::from_value(json!({
        "verdict": verdict, "criteria": criteria,
        "feedback": "polish the docs", "rationale": "more could be done"
    }))
    .unwrap()
}

fn record(c: &Criterion, met: bool, iteration: Option<u32>) -> HumanEntry {
    HumanEntry {
        kind: "verify".into(),
        ask: Some(c.id.clone()),
        text: "Saw it in the browser".into(),
        met: Some(met),
        revision: Some(revision(c)),
        iteration,
        by: "Dana".into(),
        at: Utc::now(),
    }
}

#[test]
fn model_cannot_satisfy_human_criterion() {
    let h = crit("h", VerifyKind::Human);
    let raw = eval(
        Verdict::Achieved,
        json!([{"id":"h","met":true,"evidence":"model claims done"}]),
    );
    let d = decide(Some(&raw), std::slice::from_ref(&h), &[], &[], Some(1));
    assert!(!d.eval.criteria[0].met);
    assert_eq!(d.eval.criteria[0].evidence, AWAITING_HUMAN);
    assert_eq!(
        d.eval.verdict,
        Verdict::Continue,
        "never blocked by a missing record"
    );
    assert_eq!(d.park_on.as_deref(), Some("h"));
}

#[test]
fn revised_criteria_invalidate_human_approval() {
    let mut h = crit("h", VerifyKind::Human);
    let records = vec![record(&h, true, Some(1))];
    let d = decide(None, std::slice::from_ref(&h), &[], &records, Some(1));
    assert!(d.eval.criteria[0].met);
    assert_eq!(
        d.eval.criteria[0].evidence,
        "Verified by Dana: Saw it in the browser"
    );
    assert_eq!(d.eval.criteria[0].verified_by.as_deref(), Some("Dana"));
    h.text = "Different check".into();
    let d = decide(None, std::slice::from_ref(&h), &[], &records, Some(1));
    assert!(!d.eval.criteria[0].met);
    assert_eq!(d.park_on.as_deref(), Some("h"));
}

#[test]
fn later_work_invalidates_a_human_record() {
    let h = crit("h", VerifyKind::Human);
    let records = vec![record(&h, true, Some(2))];
    let d = decide(None, std::slice::from_ref(&h), &[], &records, Some(3));
    assert!(
        !d.eval.criteria[0].met,
        "an iteration-2 record does not count in iteration 3"
    );
}

#[test]
fn a_not_met_record_continues_with_the_persons_note_never_blocked() {
    let h = crit("h", VerifyKind::Human);
    let records = vec![record(&h, false, Some(1))];
    let raw = eval(Verdict::Continue, json!([]));
    let d = decide(Some(&raw), std::slice::from_ref(&h), &[], &records, Some(1));
    assert!(!d.eval.criteria[0].met);
    assert_eq!(
        d.eval.criteria[0].evidence,
        "Not met, says Dana: Saw it in the browser"
    );
    assert_eq!(d.eval.verdict, Verdict::Continue);
    assert_eq!(
        d.park_on, None,
        "the person answered; they are not asked again"
    );
}

#[test]
fn a_record_arriving_after_a_park_is_never_stuck_behind_the_earlier_pass() {
    let a = crit("a", VerifyKind::Agent);
    let h = crit("h", VerifyKind::Human);
    let criteria = vec![a, h.clone()];
    let raw = eval(
        Verdict::Continue,
        json!([{"id":"a","met":true,"evidence":"read the handler"}]),
    );
    let first = decide(Some(&raw), &criteria, &[], &[], Some(1));
    assert_eq!(
        (first.eval.verdict, first.park_on.as_deref()),
        (Verdict::Continue, Some("h"))
    );
    // The wake re-runs from the same raw answer: now everything is met.
    let second = decide(
        Some(&raw),
        &criteria,
        &[],
        &[record(&h, true, Some(1))],
        Some(1),
    );
    assert_eq!(second.eval.verdict, Verdict::Achieved);
    assert_eq!(second.park_on, None);
}

#[test]
fn a_person_is_not_asked_while_automated_criteria_are_unmet() {
    let c = crit("c", VerifyKind::Command);
    let h = crit("h", VerifyKind::Human);
    let decided = vec![Assessed {
        id: "c".into(),
        met: false,
        evidence: "verify command failed (exit 1)".into(),
        verified_by: Some("command".into()),
    }];
    let d = decide(None, &[c, h], &decided, &[], Some(1));
    assert_eq!(d.park_on, None);
}

#[test]
fn a_command_criterion_is_ground_truth_and_the_checker_cannot_change_it() {
    let c = crit("c", VerifyKind::Command);
    let (met, evidence) = command_evidence(1, "assertion failed");
    let decided = vec![Assessed {
        id: "c".into(),
        met,
        evidence,
        verified_by: Some("command".into()),
    }];
    let raw = eval(
        Verdict::Achieved,
        json!([{"id":"c","met":true,"evidence":"looks fine to me"}]),
    );
    let d = decide(Some(&raw), std::slice::from_ref(&c), &decided, &[], None);
    assert!(!d.eval.criteria[0].met);
    assert!(d.eval.criteria[0]
        .evidence
        .starts_with("verify command failed (exit 1)"));
    assert_eq!(d.eval.verdict, Verdict::Continue);
    // Exit 0 while the checker says not met → met.
    let (met, evidence) = command_evidence(0, "");
    let decided = vec![Assessed {
        id: "c".into(),
        met,
        evidence,
        verified_by: None,
    }];
    let raw = eval(
        Verdict::Continue,
        json!([{"id":"c","met":false,"evidence":"nope"}]),
    );
    let d = decide(Some(&raw), &[c], &decided, &[], None);
    assert!(d.eval.criteria[0].met);
    assert_eq!(d.eval.verdict, Verdict::Achieved);
}

#[test]
fn no_valid_verdict_leaves_every_agent_criterion_unmet_and_continues() {
    let criteria = vec![crit("a", VerifyKind::Agent), crit("u", VerifyKind::Unknown)];
    assert!(parse_evaluation("I could not finish.").is_none());
    let d = decide(None, &criteria, &[], &[], Some(1));
    assert_eq!(d.eval.criteria[0].evidence, NO_VERDICT);
    assert_eq!(d.eval.criteria[1].evidence, UNKNOWN_CHECK);
    assert_eq!(d.eval.verdict, Verdict::Continue);
}

#[test]
fn a_met_claim_without_evidence_counts_as_not_met() {
    let mut e = eval(
        Verdict::Achieved,
        json!([
            {"id":"a","met":true,"evidence":"  "},
            {"id":"b","met":true},
            {"id":"c","met":true,"evidence":"read src/lib.rs: handler returns 409"}
        ]),
    );
    require_evidence(&mut e);
    assert!(!e.criteria[0].met);
    assert_eq!(e.criteria[0].evidence, NO_EVIDENCE);
    assert!(!e.criteria[1].met, "a missing evidence field is blank too");
    assert!(e.criteria[2].met, "cited evidence is kept");
    let criteria: Vec<_> = ["a", "b", "c"]
        .iter()
        .map(|id| crit(id, VerifyKind::Agent))
        .collect();
    reconcile(&criteria, &[], &[], None, &mut e);
    assert_eq!(e.verdict, Verdict::Continue);
    let out = CheckOutput::new(&criteria, &e, (String::new(), 0), None);
    assert_eq!(
        (out.met, out.total, out.progress_pct, out.all_met),
        (1, 3, 33, false)
    );
}

#[test]
fn all_criteria_met_with_evidence_completes_despite_continue() {
    let mut e = eval(
        Verdict::Continue,
        json!([
            {"id":"a","met":true,"evidence":"verify command exited 0"},
            {"id":"b","met":true,"evidence":"checked the error message"}
        ]),
    );
    assert!(complete_when_all_met(&mut e));
    assert_eq!(e.verdict, Verdict::Achieved);
    assert!(e.rationale.contains("polish the docs"), "{}", e.rationale);
}

#[test]
fn completion_override_needs_every_criterion_and_respects_blocked() {
    let unmet = json!([
        {"id":"a","met":true,"evidence":"ok"},
        {"id":"b","met":false,"evidence":"verify command failed (exit 1)"}
    ]);
    let mut e = eval(Verdict::Continue, unmet);
    assert!(!complete_when_all_met(&mut e));
    assert_eq!(e.verdict, Verdict::Continue);
    let met = json!([{"id":"a","met":true,"evidence":"ok"}]);
    let mut e = eval(Verdict::Blocked, met);
    assert!(!complete_when_all_met(&mut e), "blocked asks a person");
    assert_eq!(e.verdict, Verdict::Blocked);
    let mut e = eval(Verdict::Continue, json!([]));
    assert!(!complete_when_all_met(&mut e), "no criteria is not done");
    let mut e = eval(
        Verdict::Continue,
        json!([{"id":"a","met":true,"evidence":""}]),
    );
    assert!(!complete_when_all_met(&mut e), "met without evidence");
}

#[test]
fn verify_output_keeps_a_bounded_tail_after_the_headline() {
    assert_eq!(
        with_output("verify command exited 0", " \n"),
        "verify command exited 0"
    );
    let short = with_output("verify command failed (exit 1)", "assertion failed\n");
    assert_eq!(evidence_headline(&short), "verify command failed (exit 1)");
    assert!(short.ends_with("assertion failed"));
    let long = format!("{}TAIL", "é".repeat(VERIFY_OUTPUT_TAIL_BYTES));
    let kept = with_output("verify command failed (exit 2)", &long);
    let (_, out) = kept.split_once(VERIFY_OUTPUT_MARKER).unwrap();
    assert!(out.starts_with("… earlier output omitted"));
    assert!(out.ends_with("TAIL"));
    assert!(out.len() <= VERIFY_OUTPUT_TAIL_BYTES + 64, "{}", out.len());
    assert_eq!(evidence_headline("plain evidence"), "plain evidence");
}

#[test]
fn repeated_identical_failure_counts_but_new_evidence_resets() {
    let mut e = eval(
        Verdict::Continue,
        json!([{"id":"h","met":false,"evidence":"x"}]),
    );
    let first = record_progress(None, &e);
    assert_eq!(first.1, 1);
    let second = record_progress(Some((&first.0, first.1)), &e);
    assert_eq!(second.1, 2);
    e.criteria[0].evidence = "different result".into();
    assert_eq!(record_progress(Some((&second.0, second.1)), &e).1, 1);
    e.criteria[0].met = true;
    assert_eq!(
        record_progress(Some((&second.0, 2)), &e),
        (String::new(), 0)
    );
}

#[test]
fn repeated_failure_ignores_a_changing_output_tail() {
    let mut e = eval(Verdict::Continue, json!([{"id":"h","met":false}]));
    e.criteria[0].evidence = with_output("verify command failed (exit 1)", "took 1.2s");
    let first = record_progress(None, &e);
    e.criteria[0].evidence = with_output("verify command failed (exit 1)", "took 3.4s");
    assert_eq!(record_progress(Some((&first.0, first.1)), &e).1, 2);
}

#[test]
fn the_evaluator_answer_is_found_in_fences_and_prose() {
    let raw = "```json\n{\"note\":\"example\"}\n```\nResult: {\"verdict\":\"continue\",\"criteria\":[{\"id\":\"a\",\"met\":true,\"evidence\":\"tests pass\"}],\"feedback\":\"do x\"}";
    let e = parse_evaluation(raw).expect("the later object");
    assert_eq!(e.verdict, Verdict::Continue);
    assert_eq!(e.criteria[0].id, "a");
    assert!(parse_evaluation("{\"verdict\":\"maybe\",\"criteria\":[]}").is_none());
}

#[test]
fn the_completion_review_passes_only_approved_with_no_findings() {
    assert!(review_passes("{\"approved\":true,\"findings\":[]}").0);
    let (ok, review) =
        review_passes("```json\n{\"approved\":true,\"findings\":[\"test not run\"]}\n```");
    assert!(!ok);
    let mut e = eval(Verdict::Achieved, json!([]));
    apply_review(&mut e, ok, &review);
    assert_eq!(e.verdict, Verdict::Blocked);
    assert_eq!(
        e.feedback,
        "The completion review has unresolved findings: test not run"
    );
    let (ok, _) = review_passes("approved!");
    assert!(!ok, "no valid JSON blocks");
}

#[test]
fn the_evaluator_prompt_lists_agent_criteria_and_marks_decided_ones() {
    let mut a = crit("a", VerifyKind::Agent);
    a.how = "open the page".into();
    let decided = vec![Assessed {
        id: "c".into(),
        met: true,
        evidence: with_output("verify command exited 0", "ok"),
        verified_by: None,
    }];
    let p = evaluator_prompt(
        &[a, crit("c", VerifyKind::Command)],
        &decided,
        Some("Be brief."),
        "/ctx/step4.output.json",
    );
    assert!(
        p.contains("- a: criterion a (inspect: open the page)"),
        "{p}"
    );
    assert!(
        !p.contains("- c: criterion c"),
        "commands are not judged: {p}"
    );
    assert!(p.contains("decided, do not re-judge"));
    assert!(
        p.contains("- c: met — verify command exited 0\n"),
        "no output tail: {p}"
    );
    assert!(p.contains("Be brief.") && p.contains("Write JSON to /ctx/step4.output.json"));
}

#[test]
fn check_output_renders_a_table_and_caps_the_feedback() {
    let criteria = vec![crit("a", VerifyKind::Agent), crit("c", VerifyKind::Command)];
    let mut e = eval(Verdict::Continue, json!([]));
    e.feedback = "x".repeat(10_000);
    e.criteria = vec![
        Assessed {
            id: "a".into(),
            met: true,
            evidence: "seen | ok".into(),
            verified_by: Some("agent".into()),
        },
        Assessed {
            id: "c".into(),
            met: false,
            evidence: with_output("verify command failed (exit 2)", "boom"),
            verified_by: Some("command".into()),
        },
    ];
    let out = CheckOutput::new(&criteria, &e, ("sig".into(), 1), None);
    assert!(out.feedback_md.len() <= FEEDBACK_MD_MAX);
    assert!(out
        .feedback_md
        .starts_with("- Not met: criterion c — verify command failed (exit 2)\n"));
    let md = out.step_file();
    assert!(
        md.starts_with("**1 of 2 criteria met** · verdict continue"),
        "{md}"
    );
    assert!(
        md.contains("| criterion a | agent | Met | seen \\| ok |"),
        "{md}"
    );
    let v = serde_json::to_value(&out).unwrap();
    assert_eq!(v["criteria"][1]["verify"], "command");
    assert_eq!(v["verdict"], "continue");
    assert!(v.get("review").is_none());
}

fn check(verdict: Verdict, repeated: u32, feedback: &str) -> CheckOutput {
    let mut e = eval(verdict, json!([]));
    e.feedback = feedback.into();
    CheckOutput::new(&[], &e, ("s".into(), repeated), None)
}

#[test]
fn the_loop_budget_reads_params_and_raised_limits() {
    let b = LoopBudget::new(&json!({}), None);
    assert_eq!((b.max_iterations, b.max_active_s), (3, None));
    assert_eq!(
        LoopBudget::new(&json!({"max_iterations": 40}), None).max_iterations,
        10
    );
    let b = LoopBudget::new(&json!({"max_iterations": 40, "max_active_s": 60}), None);
    assert_eq!(
        (b.max_iterations, b.max_active_s),
        (40, Some(300)),
        "a time budget lifts the cap to 50"
    );
    let raised = LoopLimits {
        max_iterations: Some(5),
        max_active_s: Some(7_200),
        unblocked_at: Some(3),
    };
    let b = LoopBudget::new(
        &json!({"max_iterations": 3, "hold_when_unmet": true}),
        Some(&raised),
    );
    assert_eq!(
        (
            b.max_iterations,
            b.max_active_s,
            b.unblocked_at,
            b.hold_when_unmet
        ),
        (5, Some(7_200), Some(3), true)
    );
}

#[test]
fn out_of_iterations_or_time_exhausts_and_holds_only_when_asked() {
    let hold = LoopBudget::new(
        &json!({"max_iterations": 6, "max_active_s": 7_200, "hold_when_unmet": true}),
        None,
    );
    assert_eq!(
        loop_stop(&hold, StopPoint::BeforeIteration { used: 5 }),
        None
    );
    assert_eq!(
        loop_stop(&hold, StopPoint::BeforeIteration { used: 6 }),
        Some(Stop::Hold {
            kind: HoldKind::Exhausted,
            reason: "Out of iterations: 6 of 6 used".into()
        })
    );
    assert_eq!(
        loop_stop(
            &hold,
            StopPoint::BeforeStep {
                active_ms: 7_199_000
            }
        ),
        None
    );
    assert_eq!(
        loop_stop(
            &hold,
            StopPoint::BeforeStep {
                active_ms: 7_230_000
            }
        ),
        Some(Stop::Hold {
            kind: HoldKind::Exhausted,
            reason: "Out of time: 2h 0m of 2h active".into()
        })
    );
    let today = LoopBudget::new(&json!({"max_iterations": 2}), None);
    assert_eq!(
        loop_stop(&today, StopPoint::BeforeIteration { used: 2 }),
        Some(Stop::Unmet("Out of iterations: 2 of 2 used".into()))
    );
    assert_eq!(
        loop_stop(
            &today,
            StopPoint::BeforeStep {
                active_ms: u64::MAX / 2
            }
        ),
        None,
        "no time budget"
    );
}

#[test]
fn a_check_ends_satisfied_blocked_or_after_two_identical_failures() {
    let b = LoopBudget::new(
        &json!({"hold_when_unmet": true, "stop_on_repeat_failure": true}),
        None,
    );
    let at = |c: &CheckOutput| {
        loop_stop(
            &b,
            StopPoint::AfterCheck {
                iteration: 2,
                check: c,
            },
        )
    };
    assert_eq!(at(&check(Verdict::Achieved, 0, "")), Some(Stop::Satisfied));
    assert_eq!(
        at(&check(
            Verdict::Blocked,
            1,
            "Needs the staging password. Ask ops."
        )),
        Some(Stop::Hold {
            kind: HoldKind::Blocked,
            reason: "Needs the staging password.".into()
        })
    );
    assert_eq!(at(&check(Verdict::Continue, 1, "")), None);
    assert_eq!(
        at(&check(Verdict::Continue, 2, "")),
        Some(Stop::Hold {
            kind: HoldKind::Blocked,
            reason: REPEATED_FAILURE.into()
        })
    );
    let no_repeat = LoopBudget::new(&json!({}), None);
    assert_eq!(
        loop_stop(
            &no_repeat,
            StopPoint::AfterCheck {
                iteration: 2,
                check: &check(Verdict::Continue, 5, "")
            }
        ),
        None
    );
    assert_eq!(
        loop_stop(
            &no_repeat,
            StopPoint::AfterCheck {
                iteration: 2,
                check: &check(Verdict::Blocked, 1, "")
            }
        ),
        Some(Stop::Unmet("The check needs a person".into()))
    );
}

#[test]
fn after_resume_a_stored_blocked_verdict_does_not_block_again() {
    let raised = LoopLimits {
        unblocked_at: Some(3),
        ..LoopLimits::default()
    };
    let b = LoopBudget::new(
        &json!({"hold_when_unmet": true, "stop_on_repeat_failure": true}),
        Some(&raised),
    );
    let blocked = check(Verdict::Blocked, 2, "stuck");
    assert_eq!(
        loop_stop(
            &b,
            StopPoint::AfterCheck {
                iteration: 3,
                check: &blocked
            }
        ),
        None
    );
    assert!(loop_stop(
        &b,
        StopPoint::AfterCheck {
            iteration: 4,
            check: &blocked
        }
    )
    .is_some());
    let achieved = check(Verdict::Achieved, 0, "");
    assert_eq!(
        loop_stop(
            &b,
            StopPoint::AfterCheck {
                iteration: 1,
                check: &achieved
            }
        ),
        Some(Stop::Satisfied)
    );
}

#[test]
fn a_failed_inner_step_holds_blocked_only_with_hold_when_unmet() {
    let hold = LoopBudget::new(&json!({"hold_when_unmet": true}), None);
    assert_eq!(
        loop_stop(
            &hold,
            StopPoint::StepFailed {
                iteration: 2,
                step: "Review",
                error: "review r1 is incomplete. 2 of 3 lenses ran"
            }
        ),
        Some(Stop::Hold {
            kind: HoldKind::Blocked,
            reason: "Review failed at iteration 2: review r1 is incomplete.".into()
        })
    );
    let today = LoopBudget::new(&json!({}), None);
    assert_eq!(
        loop_stop(
            &today,
            StopPoint::StepFailed {
                iteration: 2,
                step: "Review",
                error: "x"
            }
        ),
        None
    );
}

#[test]
fn command_timeouts_default_and_clamp() {
    let mut c = crit("c", VerifyKind::Command);
    assert_eq!(command_timeout_s(&c), 600);
    c.timeout_s = Some(1);
    assert_eq!(command_timeout_s(&c), 10);
    c.timeout_s = Some(99_999);
    assert_eq!(command_timeout_s(&c), 3_600);
}

fn hold(kind: HoldKind, iteration: u32) -> otto_core::workflows::RunHold {
    otto_core::workflows::RunHold {
        kind,
        reason: String::new(),
        since: Utc::now(),
        by: None,
        node_ids: vec!["goal".into()],
        iteration: Some(iteration),
    }
}

/// GL7.1: Resume after Exhausted needs a raised limit and names it; Extend
/// writes the new totals; a Blocked hold records where it resumed.
#[test]
fn resume_needs_room_above_what_is_used() {
    let params = json!({"max_iterations": 3, "hold_when_unmet": true});
    let out_of_iters = hold(HoldKind::Exhausted, 3);
    assert_eq!(
        resume_limits(&params, None, &out_of_iters, (3, 0), (None, None)).unwrap_err(),
        "Raise the iteration limit to resume: 3 of 3 iterations are used."
    );
    let l = resume_limits(&params, None, &out_of_iters, (3, 0), (Some(5), None)).unwrap();
    assert_eq!(l.max_iterations, Some(5));
    let timed = json!({"max_iterations": 9, "max_active_s": 3600});
    let out_of_time = hold(HoldKind::Exhausted, 4);
    let err =
        resume_limits(&timed, None, &out_of_time, (4, 3_600_000), (Some(12), None)).unwrap_err();
    assert_eq!(
        err,
        "Raise the time limit to resume: 1h of 1h active time is used."
    );
    let l = resume_limits(
        &timed,
        None,
        &out_of_time,
        (4, 3_600_000),
        (None, Some(5400)),
    )
    .unwrap();
    assert_eq!((l.max_iterations, l.max_active_s), (None, Some(5400)));
    let blocked = resume_limits(
        &params,
        Some(&l),
        &hold(HoldKind::Blocked, 2),
        (2, 0),
        (None, None),
    )
    .unwrap();
    assert_eq!(blocked.unblocked_at, Some(2));
    assert_eq!(
        blocked.max_active_s,
        Some(5400),
        "earlier extensions are kept"
    );
}
