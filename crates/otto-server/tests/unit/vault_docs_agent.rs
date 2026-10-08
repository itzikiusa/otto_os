#![allow(clippy::disallowed_methods)] // isolated filesystem/process fixtures
use super::*;

const OKF_SKILL: &str = "# OKF — Open Knowledge Format authoring\nfrontmatter rules…";

#[test]
fn multi_writer_prompt_confines_to_its_draft_dir() {
    let p = build_writer_prompt(
        "document the auth flow",
        7,
        2,
        3,
        "abc12345",
        "docs",
        false,
        None,
        None,
    );
    assert!(p.contains("OTTO_TASK: vault_docs_write"));
    assert!(p.contains("WRITER AGENT 2 of 3"));
    assert!(p.contains("`_drafts/docs-run-abc12345/agent-2/`"));
    assert!(p.contains("otto_vault_write"));
    assert!(
        p.contains("vault_id 7") || p.contains("vault \n7") || p.contains("{7}") || p.contains("7")
    );
    // Multi-writer drafts: no results file, no target-dir instruction.
    assert!(!p.contains("results file"));
    assert!(!p.contains("`docs/`"));
    // Not an OKF vault → no OKF block.
    assert!(!p.contains("OKF"));
    assert!(p.contains("document the auth flow"));
}

#[test]
fn single_writer_prompt_targets_finals_and_results_file() {
    let p = build_writer_prompt(
        "document the auth flow",
        7,
        1,
        1,
        "abc12345",
        "docs/auth",
        true,
        None,
        Some("/tmp/otto-vaultdocs-r1.json"),
    );
    assert!(p.contains("FINAL documentation"));
    assert!(p.contains("`docs/auth/`"));
    assert!(p.contains("/tmp/otto-vaultdocs-r1.json"));
    assert!(p.contains("\"written\""));
    assert!(!p.contains("_drafts/")); // single writer never drafts
                                      // OKF vault: block present; claude-style (no inline text) mentions the skill.
    assert!(p.contains("OKF — this vault is OKF-conformant"));
    assert!(p.contains("okf-authoring"));
    assert!(p.contains("index.md"));
}

#[test]
fn okf_block_inlines_skill_for_non_claude() {
    let p = build_writer_prompt("x", 1, 1, 2, "run8run8", "", true, Some(OKF_SKILL), None);
    assert!(p.contains("AUTHORING SKILLS"));
    assert!(p.contains(OKF_SKILL));
    // And the claude variant carries the skill NAME, not the body.
    let c = build_writer_prompt("x", 1, 1, 2, "run8run8", "", true, None, None);
    assert!(c.contains("okf-authoring"));
    assert!(!c.contains(OKF_SKILL));
}

#[test]
fn skills_block_renders_on_non_okf_vaults_too() {
    // Prepared-prompt skills must reach the prompt even when okf=false.
    let p = build_writer_prompt(
        "x",
        1,
        1,
        2,
        "run8run8",
        "",
        false,
        Some("SKILL BODY"),
        None,
    );
    assert!(p.contains("AUTHORING SKILLS"));
    assert!(p.contains("SKILL BODY"));
    assert!(!p.contains("OKF — this vault is OKF-conformant"));
    // No skills, no OKF → no block at all.
    let n = build_writer_prompt("x", 1, 1, 2, "run8run8", "", false, None, None);
    assert!(!n.contains("AUTHORING SKILLS"));
}

#[test]
fn staged_package_guidance_is_provider_specific() {
    let mut files = std::collections::HashMap::new();
    files.insert(
        "okf-authoring".to_string(),
        vec!["SKILL.md".to_string(), "references/spec.md".to_string()],
    );
    let staged = crate::modules::StagedSkillPackages {
        root: "/tmp/staged-skills".to_string(),
        files,
    };
    let names = vec!["okf-authoring".to_string()];
    let fallback =
        std::collections::HashMap::from([("okf-authoring".to_string(), "method".to_string())]);

    let codex = skill_package_guidance("codex", Some(&staged), &names, &fallback).unwrap();
    assert!(codex.contains("okf-authoring/SKILL.md"));
    assert!(codex.contains("references/spec.md"));
    assert!(
        !codex.contains("method"),
        "package bodies must not be inlined"
    );

    let claude = skill_package_guidance("claude", Some(&staged), &names, &fallback).unwrap();
    assert!(claude.contains("invoke"));
    assert!(claude.contains("okf-authoring"));
    assert!(!claude.contains("references/spec.md"));
}

#[test]
fn partial_staging_inlines_only_failed_packages() {
    let staged = crate::modules::StagedSkillPackages {
        root: "/tmp/staged-skills".to_string(),
        files: std::collections::HashMap::from([(
            "okf-authoring".to_string(),
            vec!["SKILL.md".to_string()],
        )]),
    };
    let names = vec!["okf-authoring".to_string(), "jira-story-writer".to_string()];
    let fallback = std::collections::HashMap::from([
        ("okf-authoring".to_string(), "OKF BODY".to_string()),
        ("jira-story-writer".to_string(), "JIRA BODY".to_string()),
    ]);

    for provider in ["codex", "claude"] {
        let guidance = skill_package_guidance(provider, Some(&staged), &names, &fallback).unwrap();
        assert!(guidance.contains("JIRA BODY"));
        assert!(!guidance.contains("OKF BODY"));
    }
}

#[test]
fn summarizer_prompt_inlines_drafts_under_the_cap() {
    let drafts = vec![
        (
            "_drafts/docs-run-r/agent-1/a.md".to_string(),
            "short draft A".to_string(),
        ),
        (
            "_drafts/docs-run-r/agent-2/b.md".to_string(),
            "b".repeat(DRAFT_INLINE_CAP),
        ),
        (
            "_drafts/docs-run-r/agent-2/c.md".to_string(),
            "unreachable".to_string(),
        ),
    ];
    let p = build_summarizer_prompt(
        "req",
        3,
        2,
        "docs",
        &drafts,
        true,
        Some(OKF_SKILL),
        "/tmp/r.json",
    );
    assert!(p.contains("OTTO_TASK: vault_docs_summarize"));
    assert!(p.contains("### _drafts/docs-run-r/agent-1/a.md"));
    assert!(p.contains("short draft A"));
    // The huge second draft exhausts the budget → truncated marker…
    assert!(p.contains("[truncated]"));
    // …and the third is listed by path only.
    assert!(p.contains("### _drafts/docs-run-r/agent-2/c.md"));
    assert!(p.contains("[not inlined — read via otto_vault_read]"));
    assert!(!p.contains("unreachable"));
    // OKF: validate + index refresh + skill guidance; results file; target dir.
    assert!(p.contains("otto_vault_okf_validate"));
    assert!(p.contains(OKF_SKILL));
    assert!(p.contains("/tmp/r.json"));
    assert!(p.contains("`docs/`"));
    // The stated cap matches the enforced one.
    assert!(p.contains(&DRAFT_INLINE_CAP.to_string()));
}

#[test]
fn refine_prompt_first_turn_inlines_capped_content() {
    let long = "x".repeat(REFINE_INLINE_CAP + 10);
    let p = build_refine_prompt(
        "tighten the intro",
        5,
        "docs/a.md",
        "hash123",
        Some(&long),
        true,
        None,
    );
    assert!(p.contains("OTTO_TASK: vault_docs_refine"));
    assert!(p.contains("if_hash `hash123`"));
    assert!(p.contains("TRUNCATED"));
    assert!(p.contains("AUGMENT, don't rewrite"));
    // Enforced: inlined content really is capped (prompt shorter than raw note).
    assert!(p.len() < long.len() + 3_000);
    // Follow-up turn: no content, just instruction + fresh hash.
    let f = build_refine_prompt("more", 5, "docs/a.md", "hash456", None, true, None);
    assert!(f.contains("hash456"));
    assert!(f.contains("SAME note"));
    assert!(!f.contains("CURRENT CONTENT"));
}

#[test]
fn cap_chars_is_char_boundary_safe() {
    let (s, t) = cap_chars("héllo wörld", 4);
    assert_eq!(s, "héll");
    assert!(t);
    let (s, t) = cap_chars("short", 100);
    assert_eq!(s, "short");
    assert!(!t);
}

#[test]
fn parse_results_is_tolerant() {
    // Canonical object.
    assert_eq!(
        parse_results(r#"{"written": ["a.md", "b/c.md"]}"#).unwrap(),
        vec!["a.md", "b/c.md"]
    );
    // Bare array.
    assert_eq!(parse_results(r#"["a.md"]"#).unwrap(), vec!["a.md"]);
    // Buried in prose + fenced.
    let prose = "Done!\n```json\n{\"written\": [\"./x.md\", \"/y.md\", \"\", \"x.md\"]}\n```";
    // Normalized (./ and / stripped), empties dropped, deduped in order.
    assert_eq!(parse_results(prose).unwrap(), vec!["x.md", "y.md"]);
    // Garbage / wrong shapes → None.
    assert!(parse_results("no json here").is_none());
    assert!(parse_results(r#"{"other": 1}"#).is_none());
    assert!(parse_results("").is_none());
}

#[test]
fn written_fallback_diffs_under_target_dir_only() {
    let before: HashSet<String> = ["docs/old.md".to_string()].into();
    let after: HashSet<String> = [
        "docs/old.md".to_string(),
        "docs/new-b.md".to_string(),
        "docs/new-a.md".to_string(),
        "elsewhere/x.md".to_string(),
        "_drafts/docs-run-r/agent-1/d.md".to_string(),
    ]
    .into();
    // Scoped to target_dir, sorted, drafts never counted.
    assert_eq!(
        written_fallback(&before, &after, "docs"),
        vec!["docs/new-a.md", "docs/new-b.md"]
    );
    // Root target: everything new except drafts.
    assert_eq!(
        written_fallback(&before, &after, ""),
        vec!["docs/new-a.md", "docs/new-b.md", "elsewhere/x.md"]
    );
}

#[test]
fn target_dir_normalizes_and_rejects_traversal() {
    assert_eq!(normalize_target_dir("").unwrap(), "");
    assert_eq!(normalize_target_dir(" /docs/api/ ").unwrap(), "docs/api");
    assert!(normalize_target_dir("../up").is_err());
    assert!(normalize_target_dir("a/../b").is_err());
    assert!(normalize_target_dir(".trash/x").is_err());
    assert!(normalize_target_dir("a\\b").is_err());
}

#[test]
fn review_request_defaults_method_and_iterations() {
    let req: RunReq = serde_json::from_value(serde_json::json!({
        "prompt": "document everything",
        "agents": [{"provider": "codex"}],
        "review": {"reviewers": [{"provider": "claude"}]}
    }))
    .unwrap();
    let review = req.review.as_ref().unwrap();
    assert_eq!(review.max_iterations, 3);
    assert_eq!(review.reviewers[0].skill, "vault-docs-review");
    assert!(validate_review_request(review).is_ok());
}

#[test]
fn review_request_validation_is_actionable() {
    let review = |reviewers: Vec<ReviewerReq>, max_iterations| ReviewReq {
        reviewers,
        max_iterations,
    };
    let reviewer = |skill: &str| ReviewerReq {
        provider: "claude".into(),
        model: None,
        skill: skill.into(),
        focus: None,
    };

    let err = validate_review_request(&review(vec![], 3)).unwrap_err();
    assert!(err.contains("reviewers must be 1..=4"));
    let err = validate_review_request(&review(
        (0..5).map(|_| reviewer("vault-docs-review")).collect(),
        3,
    ))
    .unwrap_err();
    assert!(err.contains("reviewers must be 1..=4"));
    let err = validate_review_request(&review(vec![reviewer("vault-docs-review")], 0)).unwrap_err();
    assert!(err.contains("max_iterations must be 1..=10"));
    let err =
        validate_review_request(&review(vec![reviewer("vault-docs-review")], 11)).unwrap_err();
    assert!(err.contains("max_iterations must be 1..=10"));
    let err = validate_review_request(&review(vec![reviewer("unknown-review")], 3)).unwrap_err();
    assert!(err.contains("unknown reviewer skill 'unknown-review'"));
    assert!(err.contains("vault-docs-review"));
}

fn sample_reviewer(state: &str, findings: Vec<VaultDocsFinding>) -> VaultDocsReviewer {
    VaultDocsReviewer {
        index: 0,
        provider: "claude".into(),
        model: None,
        skill: "vault-docs-review".into(),
        focus: None,
        state: state.into(),
        session_id: None,
        findings,
        error: None,
    }
}

#[test]
fn review_findings_parser_accepts_empty_and_source_backed_arrays() {
    assert!(parse_review_findings("[]").unwrap().is_empty());
    let findings = parse_review_findings(
        r#"[{
            "severity":"blocking",
            "category":"api",
            "summary":"POST /widgets omits its 422 response body",
            "evidence":[
                {"repo_path":"src/routes/widgets.rs","line":91},
                {"doc_path":"widgets/api.md","section":"POST /widgets"}
            ],
            "missed_item":"ValidationError response schema and example",
            "required_fix":"Document the 422 schema/example and add it to OpenAPI"
        }]"#,
    )
    .unwrap();
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].severity, "blocking");
    assert_eq!(findings[0].evidence[0].line, Some(91));
    assert_eq!(
        findings[0].evidence[1].doc_path.as_deref(),
        Some("widgets/api.md")
    );
}

#[test]
fn review_findings_parser_rejects_malformed_or_unproven_output() {
    for malformed in [
        "not json",
        r#"{"findings": []}"#,
        r#"[{"severity":"blocking"}]"#,
        r#"[{"severity":"urgent","category":"api","summary":"x","evidence":[{"repo_path":"a.rs","line":1}],"missed_item":"x","required_fix":"x"}]"#,
        r#"[{"severity":"major","category":"banana","summary":"x","evidence":[{"repo_path":"a.rs","line":1}],"missed_item":"x","required_fix":"x"}]"#,
        r#"[{"severity":"major","category":"api","summary":"x","evidence":[],"missed_item":"x","required_fix":"x"}]"#,
        r#"[{"severity":"major","category":"api","summary":"x","evidence":[{}],"missed_item":"x","required_fix":"x"}]"#,
    ] {
        assert!(
            parse_review_findings(malformed).is_err(),
            "must reject: {malformed}"
        );
    }
}

#[test]
fn review_round_decision_requires_same_round_clean_and_honors_cap() {
    let clean = sample_reviewer("done", vec![]);
    let finding = VaultDocsFinding {
        severity: "major".into(),
        category: "data".into(),
        summary: "Missing write impact".into(),
        evidence: vec![VaultDocsFindingEvidence {
            repo_path: Some("src/dao.rs".into()),
            line: Some(42),
            doc_path: Some("data/orders.md".into()),
            section: Some("Writes".into()),
        }],
        missed_item: "transaction boundary".into(),
        required_fix: "Document the transaction boundary".into(),
    };
    let dirty = sample_reviewer("done", vec![finding]);
    let still_running = sample_reviewer("running", vec![]);

    assert!(all_reviewers_clean(&[clean.clone(), clean.clone()]));
    assert!(!all_reviewers_clean(&[]));
    assert!(!all_reviewers_clean(&[clean.clone(), dirty.clone()]));
    assert!(!all_reviewers_clean(&[clean.clone(), still_running]));
    assert_eq!(next_review_action(&[clean], 1, 3), ReviewAction::Clean);
    assert_eq!(
        next_review_action(std::slice::from_ref(&dirty), 1, 3),
        ReviewAction::Revise
    );
    assert_eq!(next_review_action(&[dirty], 3, 3), ReviewAction::Exhausted);

    for state in ["pending", "running"] {
        let incomplete = sample_reviewer(state, vec![]);
        assert_eq!(
            next_review_action(std::slice::from_ref(&incomplete), 1, 3),
            ReviewAction::Pending
        );
        assert_eq!(
            next_review_action(&[incomplete], 3, 3),
            ReviewAction::Pending
        );
    }
    for state in ["error", "cancelled", "interrupted"] {
        let failed = sample_reviewer(state, vec![]);
        assert_eq!(
            next_review_action(std::slice::from_ref(&failed), 1, 3),
            ReviewAction::Error
        );
        assert_eq!(next_review_action(&[failed], 3, 3), ReviewAction::Error);
    }
    assert_eq!(next_review_action(&[], 1, 3), ReviewAction::Error);
}

#[test]
fn review_round_driver_fans_out_fresh_reviewers_and_persists_visible_states() {
    let configured = vec![
        sample_reviewer("done", vec![]),
        VaultDocsReviewer {
            index: 1,
            skill: "vault-api-review".into(),
            focus: Some("request bodies".into()),
            ..sample_reviewer("error", vec![])
        },
    ];
    let round = fresh_review_round(2, &configured);
    assert_eq!(round.iteration, 2);
    assert_eq!(round.state, "reviewing");
    assert_eq!(round.reviewers.len(), 2);
    assert!(round.reviewers.iter().all(|reviewer| {
        reviewer.state == "pending"
            && reviewer.session_id.is_none()
            && reviewer.findings.is_empty()
            && reviewer.error.is_none()
    }));
    assert_eq!(round.reviewers[1].skill, "vault-api-review");
    assert_eq!(round.reviewers[1].focus.as_deref(), Some("request bodies"));
    assert_eq!(round.revision.state, "skipped");
}

#[test]
fn review_round_driver_maps_clean_revision_exhaustion_and_partial_failure() {
    let clean = vec![sample_reviewer("done", vec![])];
    assert_eq!(
        review_round_outcome(&clean, 1, 3),
        ReviewRoundOutcome::Clean
    );

    let finding = VaultDocsFinding {
        severity: "major".into(),
        category: "api".into(),
        summary: "Missing body".into(),
        evidence: vec![VaultDocsFindingEvidence {
            repo_path: Some("src/api.rs".into()),
            line: Some(9),
            doc_path: None,
            section: None,
        }],
        missed_item: "request schema".into(),
        required_fix: "document it".into(),
    };
    let dirty = vec![sample_reviewer("done", vec![finding])];
    assert_eq!(
        review_round_outcome(&dirty, 1, 3),
        ReviewRoundOutcome::Revise
    );
    assert_eq!(
        review_round_outcome(&dirty, 3, 3),
        ReviewRoundOutcome::Exhausted
    );

    let partial = vec![
        sample_reviewer("done", vec![]),
        sample_reviewer("error", vec![]),
    ];
    assert_eq!(
        review_round_outcome(&partial, 1, 3),
        ReviewRoundOutcome::Error
    );
}

#[test]
fn retry_terminates_an_inflight_turn_before_signalling_but_keeps_failed_session_live() {
    assert!(retry_needs_termination("running"));
    assert!(retry_needs_termination("pending"));
    assert!(!retry_needs_termination("error"));
}

#[test]
fn errored_writer_waits_only_while_a_peer_is_still_moving() {
    let reg = new_run_registry();
    let mut run = sample_run("running");
    // agents: 0=done, 1=running, 2=pending (from sample_run). Slot 0 errored
    // → the stage is open for it while 1/2 move.
    run.agents[0].state = "error".into();
    reg.lock().unwrap().insert(
        run.id.clone(),
        RunEntry {
            run,
            cancel: Arc::new(AtomicBool::new(false)),
            retries: Arc::new(Mutex::new(HashSet::new())),
            persist_tx: None,
        },
    );
    assert!(writers_stage_open(&reg, "run-12345678", 0));
    // A slot never counts ITSELF as an open peer.
    assert!(writers_stage_open(&reg, "run-12345678", 1)); // peer 2 pending
    with_run(&reg, "run-12345678", |r| {
        r.agents[1].state = "done".into();
        r.agents[2].state = "error".into();
    });
    // Every peer terminal → the waiter must give up (barrier can't hang).
    assert!(!writers_stage_open(&reg, "run-12345678", 0));
    assert!(!writers_stage_open(&reg, "run-12345678", 2));
    // Unknown run → closed.
    assert!(!writers_stage_open(&reg, "nope", 0));
}

#[test]
fn targeted_failed_slot_retry_preserves_completed_peers_and_round_findings() {
    let finding = VaultDocsFinding {
        severity: "major".into(),
        category: "api".into(),
        summary: "Missing response".into(),
        evidence: vec![VaultDocsFindingEvidence {
            repo_path: Some("src/api.rs".into()),
            line: Some(7),
            doc_path: None,
            section: None,
        }],
        missed_item: "response body".into(),
        required_fix: "document response".into(),
    };
    let completed = sample_reviewer("done", vec![finding.clone()]);
    let mut failed = sample_reviewer("error", vec![]);
    failed.index = 1;
    failed.error = Some("malformed JSON".into());
    let mut run = sample_run("reviewing");
    run.review = VaultDocsReview {
        state: "reviewing".into(),
        max_iterations: 3,
        current_iteration: 1,
        outcome: None,
        reviewers: vec![completed.clone(), failed.clone()],
        rounds: vec![VaultDocsReviewRound {
            iteration: 1,
            state: "reviewing".into(),
            reviewers: vec![completed, failed],
            revision: VaultDocsRevision {
                state: "error".into(),
                session_id: Some("author-1".into()),
                changed_paths: vec!["old.md".into()],
                error: Some("audit failed".into()),
            },
        }],
    };

    reset_reviewer_for_retry(&mut run, 1, 1);
    assert_eq!(run.review.reviewers[0].state, "done");
    assert_eq!(run.review.reviewers[0].findings.len(), 1);
    assert_eq!(run.review.reviewers[0].findings[0].summary, finding.summary);
    assert_eq!(run.review.rounds[0].reviewers[0].findings.len(), 1);
    // Top-level reviewers are immutable resolved configuration snapshots;
    // only the active round carries mutable live state.
    assert_eq!(run.review.reviewers[1].state, "error");
    assert_eq!(
        run.review.reviewers[1].error.as_deref(),
        Some("malformed JSON")
    );
    assert_eq!(run.review.rounds[0].reviewers[1].state, "pending");

    reset_revision_for_retry(&mut run, 1);
    assert_eq!(run.review.rounds[0].revision.state, "pending");
    assert!(run.review.rounds[0].revision.error.is_none());
    assert!(run.review.rounds[0].revision.changed_paths.is_empty());
    assert_eq!(run.review.reviewers[0].findings.len(), 1);
}

#[test]
fn retry_handler_state_composition_flags_only_the_requested_error_slot() {
    let finding = VaultDocsFinding {
        severity: "major".into(),
        category: "api".into(),
        summary: "Missing response".into(),
        evidence: vec![VaultDocsFindingEvidence {
            repo_path: Some("src/api.rs".into()),
            line: Some(7),
            doc_path: None,
            section: None,
        }],
        missed_item: "response body".into(),
        required_fix: "document response".into(),
    };
    let completed = sample_reviewer("done", vec![finding.clone()]);
    let mut failed = sample_reviewer("error", vec![]);
    failed.index = 1;
    failed.session_id = Some("failed-session".into());
    failed.error = Some("malformed JSON".into());
    let mut run = sample_run("reviewing");
    run.review = VaultDocsReview {
        state: "reviewing".into(),
        max_iterations: 3,
        current_iteration: 1,
        outcome: None,
        reviewers: vec![completed.clone(), failed.clone()],
        rounds: vec![VaultDocsReviewRound {
            iteration: 1,
            state: "reviewing".into(),
            reviewers: vec![completed, failed],
            revision: VaultDocsRevision::default(),
        }],
    };
    let retries = Arc::new(Mutex::new(HashSet::new()));
    let mut entry = RunEntry {
        run,
        cancel: Arc::new(AtomicBool::new(false)),
        retries: Arc::clone(&retries),
        persist_tx: None,
    };

    let (sid, key) = activate_review_retry(&mut entry, 1, Some(1)).unwrap();
    assert_eq!(sid.as_deref(), Some("failed-session"));
    assert_eq!(key, reviewer_retry_key(1, 1));
    assert!(retries.lock().unwrap().contains(&key));
    assert_eq!(entry.run.review.rounds[0].reviewers[0].state, "done");
    assert_eq!(
        entry.run.review.rounds[0].reviewers[0].findings[0].summary,
        finding.summary
    );
    assert_eq!(entry.run.review.rounds[0].reviewers[1].state, "pending");
    assert!(entry.run.review.rounds[0].reviewers[1].error.is_none());

    let error = activate_review_retry(&mut entry, 2, Some(1)).unwrap_err();
    assert!(error.0.to_string().contains("not active"));
}

#[test]
fn review_prompt_carries_method_focus_evidence_and_result_contract() {
    let prompt = build_reviewer_prompt(
        "Document the widget service",
        9,
        "widgets",
        2,
        "vault-api-review",
        Some("Prioritize externally consumed contracts"),
        "/tmp/staged/skills/vault-api-review/SKILL.md",
        "/tmp/review-round-2-agent-1.json",
    );
    assert!(prompt.contains("OTTO_TASK: vault_docs_review"));
    assert!(prompt.contains("round 2"));
    assert!(prompt.contains("vault-api-review"));
    assert!(prompt.contains("/tmp/staged/skills/vault-api-review/SKILL.md"));
    assert!(prompt.contains("Prioritize externally consumed contracts"));
    assert!(prompt.contains("real repository path and line"));
    assert!(prompt.contains("speculative"));
    assert!(prompt.contains("read-only"));
    assert!(prompt.contains("missed_item"));
    assert!(prompt.contains("required_fix"));
    assert!(prompt.contains("coverage|api|data|runtime|evidence|quality"));
    assert!(prompt.contains("/tmp/review-round-2-agent-1.json"));
    assert!(prompt.contains("Document the widget service"));
}

#[test]
fn review_revision_prompt_carries_findings_and_changed_path_results_contract() {
    let finding = VaultDocsFinding {
        severity: "major".into(),
        category: "api".into(),
        summary: "Missing response body".into(),
        evidence: vec![VaultDocsFindingEvidence {
            repo_path: Some("src/routes.rs".into()),
            line: Some(8),
            doc_path: Some("api/widgets.md".into()),
            section: Some("POST /widgets".into()),
        }],
        missed_item: "422 schema".into(),
        required_fix: "Add schema and example".into(),
    };
    let prompt = build_revision_prompt(
        "Document the widget service",
        9,
        "widgets",
        2,
        &[finding],
        "/tmp/revision-round-2.json",
    );
    assert!(prompt.contains("OTTO_TASK: vault_docs_revise"));
    assert!(prompt.contains("Missing response body"));
    assert!(prompt.contains("Add schema and example"));
    assert!(prompt.contains("coverage.md"));
    assert!(prompt.contains("index.md"));
    assert!(prompt.contains("otto_vault_okf_validate"));
    assert!(prompt.contains(r#"{"written": ["path/to/changed.md", ...]}"#));
    assert!(prompt.contains("/tmp/revision-round-2.json"));
}

pub(super) fn sample_run(state: &str) -> VaultDocsRun {
    VaultDocsRun {
        id: "run-12345678".into(),
        ws_id: "w1".into(),
        vault_id: 1,
        kind: "docs".into(),
        prompt: "p".into(),
        target_dir: String::new(),
        note_path: String::new(),
        state: state.into(),
        agents: vec![
            VaultDocsAgent {
                index: 0,
                name: "writer-1 · claude".into(),
                provider: "claude".into(),
                model: None,
                state: "done".into(),
                session_id: Some("s1".into()),
                error: None,
                drafts: vec![],
            },
            VaultDocsAgent {
                index: 1,
                name: "writer-2 · claude".into(),
                provider: "claude".into(),
                model: None,
                state: "running".into(),
                session_id: Some("s2".into()),
                error: None,
                drafts: vec![],
            },
            VaultDocsAgent {
                index: 2,
                name: "writer-3 · claude".into(),
                provider: "claude".into(),
                model: None,
                state: "pending".into(),
                session_id: None,
                error: None,
                drafts: vec![],
            },
        ],
        summarizer: VaultDocsSummarizer {
            provider: "claude".into(),
            model: None,
            state: "pending".into(),
            session_id: None,
            error: None,
        },
        review: VaultDocsReview::default(),
        written: vec![],
        error: None,
        started_at: "2026-07-12T10:00:00Z".into(),
        finished_at: None,
    }
}

#[derive(Clone, Default)]
#[allow(clippy::type_complexity)]
struct ScriptedReviewTurnRunner {
    outputs:
        Arc<Mutex<HashMap<ReviewTurnKind, std::collections::VecDeque<Result<String, String>>>>>,
    seen: Arc<Mutex<Vec<ReviewTurnKind>>>,
    scans: Arc<std::sync::atomic::AtomicUsize>,
}

impl ScriptedReviewTurnRunner {
    fn with_outputs(
        outputs: impl IntoIterator<Item = (ReviewTurnKind, Vec<Result<&'static str, &'static str>>)>,
    ) -> Self {
        let outputs = outputs
            .into_iter()
            .map(|(kind, values)| {
                (
                    kind,
                    values
                        .into_iter()
                        .map(|value| value.map(str::to_string).map_err(str::to_string))
                        .collect(),
                )
            })
            .collect();
        Self {
            outputs: Arc::new(Mutex::new(outputs)),
            ..Self::default()
        }
    }

    fn attempts(&self, kind: &ReviewTurnKind) -> usize {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|seen| *seen == kind)
            .count()
    }
}

#[async_trait::async_trait]
impl ReviewTurnRunner for ScriptedReviewTurnRunner {
    async fn run_turn(
        &self,
        request: ReviewTurnRequest,
        on_ready: Box<dyn FnOnce(Id) + Send>,
    ) -> Result<(String, Id), String> {
        self.seen.lock().unwrap().push(request.kind.clone());
        let sid = Id::from(format!("test-{:?}", request.kind));
        on_ready(sid.clone());
        let output = self
            .outputs
            .lock()
            .unwrap()
            .get_mut(&request.kind)
            .and_then(std::collections::VecDeque::pop_front)
            .unwrap_or_else(|| Err(format!("no scripted output for {:?}", request.kind)))?;
        std::fs::write(&request.result_path, output)
            .map_err(|error| format!("write scripted output: {error}"))?;
        Ok(("scripted".into(), sid))
    }

    async fn scan(&self, _vault_id: i64) {
        self.scans.fetch_add(1, Ordering::Relaxed);
    }
}

#[allow(clippy::type_complexity)]
fn review_loop_fixture(
    max_iterations: u8,
    reviewer_count: usize,
) -> (
    RunRegistry,
    Arc<AtomicBool>,
    Arc<Mutex<HashSet<usize>>>,
    crate::modules::StagedSkillPackages,
    String,
) {
    let reviewers = (0..reviewer_count)
        .map(|index| VaultDocsReviewer {
            index,
            ..sample_reviewer("pending", vec![])
        })
        .collect::<Vec<_>>();
    let mut run = sample_run("running");
    run.id = format!("test-review-{}", otto_core::new_id());
    let run_id = run.id.clone();
    run.review = VaultDocsReview {
        state: "pending".into(),
        max_iterations,
        current_iteration: 0,
        outcome: None,
        reviewers,
        rounds: vec![],
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let retries = Arc::new(Mutex::new(HashSet::new()));
    let reg = new_run_registry();
    reg.lock().unwrap().insert(
        run.id.clone(),
        RunEntry {
            run,
            cancel: Arc::clone(&cancel),
            retries: Arc::clone(&retries),
            persist_tx: None,
        },
    );
    let packages = crate::modules::StagedSkillPackages {
        root: "/tmp/test-vault-review-skills".into(),
        files: HashMap::from([("vault-docs-review".into(), vec!["SKILL.md".into()])]),
    };
    (reg, cancel, retries, packages, run_id)
}

const REVIEW_FINDING_JSON: &str = r#"[{
    "severity":"major","category":"api","summary":"Missing response body",
    "evidence":[{"repo_path":"src/api.rs","line":7}],
    "missed_item":"response body","required_fix":"document response"
}]"#;

#[tokio::test]
async fn review_loop_runs_dirty_revision_then_all_reviewers_clean() {
    let runner = ScriptedReviewTurnRunner::with_outputs([
        (
            ReviewTurnKind::Reviewer {
                iteration: 1,
                index: 0,
            },
            vec![Ok("[]")],
        ),
        (
            ReviewTurnKind::Reviewer {
                iteration: 1,
                index: 1,
            },
            vec![Ok(REVIEW_FINDING_JSON)],
        ),
        (
            ReviewTurnKind::Revision { iteration: 1 },
            vec![Ok(r#"{"written":["docs/api.md","coverage.md"]}"#)],
        ),
        (
            ReviewTurnKind::Reviewer {
                iteration: 2,
                index: 0,
            },
            vec![Ok("[]")],
        ),
        (
            ReviewTurnKind::Reviewer {
                iteration: 2,
                index: 1,
            },
            vec![Ok("[]")],
        ),
    ]);
    let (reg, cancel, retries, packages, run_id) = review_loop_fixture(3, 2);
    let result = run_review_loop_with_runner(
        Arc::new(runner.clone()),
        1,
        "/tmp",
        &reg,
        &run_id,
        "document everything",
        "docs",
        &WriterSpec {
            provider: "claude".into(),
            model: None,
        },
        "author-1",
        Some(&packages),
        cancel,
        retries,
    )
    .await;
    assert!(matches!(result, ReviewLoopResult::Clean));
    let run = reg.lock().unwrap().get(&run_id).unwrap().run.clone();
    assert_eq!(run.review.state, "clean");
    assert_eq!(run.review.current_iteration, 2);
    assert_eq!(run.review.rounds.len(), 2);
    assert_eq!(run.review.rounds[0].state, "revised");
    assert_eq!(run.review.rounds[0].revision.state, "done");
    assert_eq!(
        run.review.rounds[0].revision.changed_paths,
        vec!["docs/api.md", "coverage.md"]
    );
    assert_eq!(run.review.rounds[1].state, "clean");
    assert!(run.review.rounds[1]
        .reviewers
        .iter()
        .all(|reviewer| reviewer.findings.is_empty()));
    assert_eq!(runner.scans.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn review_loop_exhausts_exactly_at_user_iteration_limit() {
    let runner = ScriptedReviewTurnRunner::with_outputs([
        (
            ReviewTurnKind::Reviewer {
                iteration: 1,
                index: 0,
            },
            vec![Ok(REVIEW_FINDING_JSON)],
        ),
        (
            ReviewTurnKind::Revision { iteration: 1 },
            vec![Ok(r#"{"written":["docs/api.md"]}"#)],
        ),
        (
            ReviewTurnKind::Reviewer {
                iteration: 2,
                index: 0,
            },
            vec![Ok(REVIEW_FINDING_JSON)],
        ),
    ]);
    let (reg, cancel, retries, packages, run_id) = review_loop_fixture(2, 1);
    let result = run_review_loop_with_runner(
        Arc::new(runner),
        1,
        "/tmp",
        &reg,
        &run_id,
        "document everything",
        "docs",
        &WriterSpec {
            provider: "claude".into(),
            model: None,
        },
        "author-1",
        Some(&packages),
        cancel,
        retries,
    )
    .await;
    assert!(matches!(result, ReviewLoopResult::Exhausted));
    let run = reg.lock().unwrap().get(&run_id).unwrap().run.clone();
    assert_eq!(run.review.state, "exhausted");
    assert_eq!(run.review.rounds.len(), 2);
    assert_eq!(run.review.rounds[1].state, "exhausted");
    assert_eq!(run.review.rounds[1].revision.state, "skipped");
}

#[tokio::test]
async fn malformed_reviewer_output_is_not_clean_and_consumes_targeted_retry() {
    let kind = ReviewTurnKind::Reviewer {
        iteration: 1,
        index: 0,
    };
    let runner =
        ScriptedReviewTurnRunner::with_outputs([(kind.clone(), vec![Ok("not-json"), Ok("[]")])]);
    let (reg, cancel, retries, packages, run_id) = review_loop_fixture(1, 1);
    retries.lock().unwrap().insert(reviewer_retry_key(1, 0));
    let result = run_review_loop_with_runner(
        Arc::new(runner.clone()),
        1,
        "/tmp",
        &reg,
        &run_id,
        "document everything",
        "docs",
        &WriterSpec {
            provider: "claude".into(),
            model: None,
        },
        "author-1",
        Some(&packages),
        cancel,
        retries,
    )
    .await;
    assert!(matches!(result, ReviewLoopResult::Clean));
    assert_eq!(runner.attempts(&kind), 2);
    let run = reg.lock().unwrap().get(&run_id).unwrap().run.clone();
    assert_eq!(run.review.rounds[0].reviewers[0].state, "done");
    assert!(run.review.rounds[0].reviewers[0].findings.is_empty());
}

#[test]
fn interrupted_sweep_flips_only_non_terminal_states() {
    let mut run = sample_run("running");
    let reviewer = sample_reviewer("running", vec![]);
    run.review = VaultDocsReview {
        state: "reviewing".into(),
        max_iterations: 3,
        current_iteration: 1,
        outcome: None,
        reviewers: vec![reviewer.clone()],
        rounds: vec![VaultDocsReviewRound {
            iteration: 1,
            state: "revising".into(),
            reviewers: vec![reviewer],
            revision: VaultDocsRevision {
                state: "pending".into(),
                ..VaultDocsRevision::default()
            },
        }],
    };
    assert!(mark_run_interrupted(&mut run));
    assert_eq!(run.state, "interrupted");
    assert!(run.finished_at.is_some());
    assert!(run.error.as_deref().unwrap().contains("restart"));
    // done stays done; running/pending flip.
    assert_eq!(run.agents[0].state, "done");
    assert_eq!(run.agents[1].state, "interrupted");
    assert_eq!(run.agents[2].state, "interrupted");
    assert_eq!(run.summarizer.state, "interrupted");
    assert_eq!(run.review.state, "interrupted");
    assert_eq!(run.review.reviewers[0].state, "running");
    assert_eq!(run.review.rounds[0].state, "interrupted");
    assert_eq!(run.review.rounds[0].reviewers[0].state, "interrupted");
    assert_eq!(run.review.rounds[0].revision.state, "interrupted");

    // Terminal runs are untouched (idempotent across sweeps).
    let mut done = sample_run("done");
    assert!(!mark_run_interrupted(&mut done));
    assert_eq!(done.state, "done");
    let mut exhausted = sample_run("done_with_findings");
    assert!(!mark_run_interrupted(&mut exhausted));
    assert_eq!(exhausted.state, "done_with_findings");
    let mut twice = sample_run("running");
    mark_run_interrupted(&mut twice);
    assert!(!mark_run_interrupted(&mut twice));

    // A skipped summarizer (single-writer / all-failed) stays skipped.
    let mut single = sample_run("running");
    single.summarizer.state = "skipped".into();
    mark_run_interrupted(&mut single);
    assert_eq!(single.summarizer.state, "skipped");
}

#[tokio::test]
async fn interrupted_recovery_persists_flat_and_nested_review_states() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            sqlx::sqlite::SqliteConnectOptions::new()
                .in_memory(true)
                .foreign_keys(false),
        )
        .await
        .unwrap();
    sqlx::migrate!("../otto-state/migrations")
        .run(&pool)
        .await
        .unwrap();
    let repo = otto_state::VaultDocsRunsRepo::new(pool);
    let reviewer = sample_reviewer("running", vec![]);
    let mut run = sample_run("reviewing");
    run.review = VaultDocsReview {
        state: "reviewing".into(),
        max_iterations: 3,
        current_iteration: 1,
        outcome: None,
        reviewers: vec![reviewer.clone()],
        rounds: vec![VaultDocsReviewRound {
            iteration: 1,
            state: "revising".into(),
            reviewers: vec![reviewer],
            revision: VaultDocsRevision {
                state: "running".into(),
                session_id: Some("author-1".into()),
                ..VaultDocsRevision::default()
            },
        }],
    };
    let mut row = run_row(&run);
    repo.upsert(&row).await.unwrap();

    let recovered = persist_interrupted_row(&repo, &mut row)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(recovered.state, "interrupted");
    assert_eq!(recovered.review.state, "interrupted");
    assert_eq!(recovered.review.rounds[0].state, "interrupted");
    assert_eq!(recovered.review.rounds[0].reviewers[0].state, "interrupted");
    assert_eq!(recovered.review.rounds[0].revision.state, "interrupted");

    let durable = repo.get(&run.id).await.unwrap().unwrap();
    assert_eq!(durable.state, "interrupted");
    assert!(durable.finished_at.is_some());
    let payload: VaultDocsRun = serde_json::from_str(&durable.payload).unwrap();
    assert_eq!(payload.review.rounds[0].revision.state, "interrupted");
}

#[test]
fn payload_without_kind_or_note_path_still_deserializes_as_docs() {
    // Forward-compat: rows written before the fields existed.
    let legacy = r#"{
        "id": "r1", "ws_id": "w1", "vault_id": 1, "prompt": "p",
        "target_dir": "", "state": "running", "agents": [],
        "summarizer": {"provider": "claude", "model": null, "state": "pending",
                       "session_id": null, "error": null},
        "written": [], "error": null,
        "started_at": "t", "finished_at": null
    }"#;
    let run: VaultDocsRun = serde_json::from_str(legacy).unwrap();
    assert_eq!(run.kind, "docs");
    assert_eq!(run.note_path, "");
    assert_eq!(run.review.state, "skipped");
    assert_eq!(run.review.max_iterations, 3);
    assert!(run.review.rounds.is_empty());
    // And a full round-trip preserves the new fields.
    let mut run = sample_run("running");
    run.kind = "refine".into();
    run.note_path = "docs/a.md".into();
    let back: VaultDocsRun = serde_json::from_str(&serde_json::to_string(&run).unwrap()).unwrap();
    assert_eq!(back.kind, "refine");
    assert_eq!(back.note_path, "docs/a.md");
}

#[test]
fn orphan_drafts_move_to_trash_with_collision_suffix() {
    let tmp = std::env::temp_dir().join(format!("otto-vdr-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let drafts = tmp.join("_drafts/docs-run-run-1234/agent-1");
    std::fs::create_dir_all(&drafts).unwrap();
    std::fs::write(drafts.join("a.md"), "draft").unwrap();
    let root = tmp.to_string_lossy().to_string();

    // "run-1234" are the first 8 chars of the run id.
    assert!(trash_orphan_drafts(&root, "run-12345678"));
    assert!(!tmp.join("_drafts/docs-run-run-1234").exists());
    assert!(tmp.join(".trash/docs-run-run-1234/agent-1/a.md").exists());

    // Missing dir → no-op.
    assert!(!trash_orphan_drafts(&root, "run-12345678"));

    // Collision (same run trashed before) → `-interrupted` suffix.
    std::fs::create_dir_all(tmp.join("_drafts/docs-run-run-1234")).unwrap();
    assert!(trash_orphan_drafts(&root, "run-12345678"));
    assert!(tmp.join(".trash/docs-run-run-1234-interrupted").exists());

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn run_state_transitions_respect_terminal_states() {
    let reg = new_run_registry();
    let run = VaultDocsRun {
        id: "r1".into(),
        ws_id: "w1".into(),
        vault_id: 1,
        kind: "docs".into(),
        prompt: "p".into(),
        target_dir: String::new(),
        note_path: String::new(),
        state: "running".into(),
        agents: vec![],
        summarizer: VaultDocsSummarizer {
            provider: "claude".into(),
            model: None,
            state: "pending".into(),
            session_id: None,
            error: None,
        },
        review: VaultDocsReview::default(),
        written: vec![],
        error: None,
        started_at: "t".into(),
        finished_at: None,
    };
    reg.lock().unwrap().insert(
        "r1".into(),
        RunEntry {
            run,
            cancel: Arc::new(AtomicBool::new(false)),
            retries: Arc::new(Mutex::new(HashSet::new())),
            persist_tx: None,
        },
    );
    // Cancel lands first…
    finish_run(&reg, "r1", "cancelled", None);
    // …and a late "done" from the orchestrator cannot overwrite it.
    finish_run(&reg, "r1", "done", None);
    let snap = reg.lock().unwrap().get("r1").unwrap().run.clone();
    assert_eq!(snap.state, "cancelled");
    assert!(snap.finished_at.is_some());
}
