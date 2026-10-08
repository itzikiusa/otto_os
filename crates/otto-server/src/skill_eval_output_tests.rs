use super::*;

#[tokio::test]
async fn completion_order_preserves_validator_dimension_in_improver_prompt() {
    let mut tasks = tokio::task::JoinSet::new();
    let (ready, wait) = tokio::sync::oneshot::channel();
    tasks.spawn(async move {
        wait.await.unwrap();
        (
            0,
            75.0,
            vec![EvalFinding {
                severity: "fail".into(),
                issue: "missing permission check".into(),
                suggestion: "check ownership".into(),
                location: None,
            }],
        )
    });
    tasks.spawn(async move {
        (
            1,
            92.0,
            vec![EvalFinding {
                severity: "warn".into(),
                issue: "quadratic scan".into(),
                suggestion: "index the rows".into(),
                location: None,
            }],
        )
    });
    // Keep security blocked while the production collector consumes performance.
    let names = ["security".into(), "performance".into()];
    let collection = collect_validation_results(tasks, &names);
    tokio::pin!(collection);
    assert!(
        tokio::time::timeout(Duration::from_millis(10), &mut collection)
            .await
            .is_err()
    );
    ready.send(()).unwrap();
    let (_, findings) = collection.await;
    let prompt = build_improver_prompt("task", &[], &findings, Path::new("/tmp/unused"));
    assert!(
        prompt.contains("[performance] (warn) quadratic scan"),
        "{prompt}"
    );
    assert!(
        prompt.contains("[security] (fail) missing permission check"),
        "{prompt}"
    );
}

#[test]
fn partial_validator_file_is_preserved_until_a_complete_verdict() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("result.json");
    for partial in ["", "[", "[{\"issue\":\"partial"] {
        std::fs::write(&path, partial).unwrap();
        assert!(read_capture_output(&path, true).is_none());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), partial);
    }
    std::fs::write(&path, "[]").unwrap();
    assert_eq!(read_capture_output(&path, true).as_deref(), Some("[]"));
    assert!(!path.exists());
}

#[test]
fn valid_clean_transcript_is_a_verdict_and_invalid_passes_do_not_score() {
    for text in ["[]", "```json\n[]\n```", "{\"findings\":[]}"] {
        assert!(capture_output_ready(text, true));
        let mut scores = Vec::new();
        let mut findings = Vec::new();
        assert!(record_validation_pass(
            &AgentOutcome {
                session_id: None,
                text: text.into(),
                errored: false
            },
            &mut scores,
            &mut findings
        ));
        assert_eq!(scores, vec![100.0]);
        assert!(findings.is_empty());
    }
    for text in [
        "",
        "not json",
        "[{}]",
        "[null]",
        "{\"error\":\"failed\",\"details\":[]}",
    ] {
        assert!(!capture_output_ready(text, true));
        let mut scores = Vec::new();
        let mut findings = Vec::new();
        assert!(!record_validation_pass(
            &AgentOutcome {
                session_id: None,
                text: text.into(),
                errored: false
            },
            &mut scores,
            &mut findings
        ));
        assert!(scores.is_empty());
    }
}

#[tokio::test]
async fn malformed_validation_cannot_publish_passed_review_evidence() {
    use otto_core::proof::{ProofArtifactKind, ProofArtifactStatus, WorkItemKind};
    let pool = otto_state::db::test_pool().await;
    sqlx::query("INSERT INTO users(id,username,password_hash,is_root,created_at) VALUES('editor','editor','unused',0,'2026-10-08T00:00:00Z')")
        .execute(&pool).await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let ctx = ServerCtx::for_tests(&pool, dir.path()).await;
    let ws = ctx
        .workspaces
        .create("fixture", dir.path().to_str().unwrap(), &"editor".into())
        .await
        .unwrap();
    let eval = ctx
        .skill_evals_store
        .create_eval(
            &ws.id,
            "skill",
            "task",
            "fixture",
            1,
            &serde_json::json!({}),
        )
        .await
        .unwrap();
    let mut iter = ctx
        .skill_evals_store
        .add_iteration(&eval.id, 1, None, "skill", "body", "fixture", &[])
        .await
        .unwrap();
    let pack = ctx
        .proof_repo
        .create_pack(
            &ws.id,
            WorkItemKind::Task,
            &iter.id,
            "fixture",
            "editor",
            None,
        )
        .await
        .unwrap();
    iter.proof_pack_id = Some(pack.id.clone());
    for (texts, expected_status, expected_score) in [
        (vec!["[]"], "done", 100.0),
        (vec!["not json"], "error", 0.0),
        (vec!["[{\"issue\":"], "error", 0.0),
        (vec!["[{}]"], "error", 0.0),
        (vec!["[]", "not json"], "error", 0.0),
        (vec!["not json", "[]"], "error", 0.0),
        (vec!["[]", "[]"], "done", 100.0),
    ] {
        let mut scores = Vec::new();
        let mut findings = Vec::new();
        for text in &texts {
            record_validation_pass(
                &AgentOutcome {
                    session_id: None,
                    text: (*text).into(),
                    errored: false,
                },
                &mut scores,
                &mut findings,
            );
        }
        let base = EvalValidationState {
            validation: "fixture".into(),
            name: "fixture".into(),
            provider: "fixture".into(),
            model: String::new(),
            status: "running".into(),
            note: String::new(),
            passed: false,
            score: 0.0,
            session_id: None,
            findings: vec![],
        };
        let state = finish_validation(base, None, scores, findings, texts.len() as u32);
        assert_eq!(state.status, expected_status);
        assert_eq!(state.score, expected_score);
        let mut clean = state.clone();
        clean.status = "done".into();
        clean.score = 100.0;
        clean.passed = true;
        ctx.skill_evals_store
            .set_iter_agents(&iter.id, &[state, clean])
            .await
            .unwrap();
        let mut stored = ctx.skill_evals_store.get_iteration(&iter.id).await.unwrap();
        stored.proof_pack_id = Some(pack.id.clone());
        let (score, _) = crate::eval_score::rescore_validation(&ctx, &eval, &stored, None)
            .await
            .unwrap();
        assert_eq!(score.review.score, expected_score);
        let artifacts = ctx.proof_repo.list_artifacts_meta(&pack.id).await.unwrap();
        let review = artifacts
            .iter()
            .find(|a| a.kind == ProofArtifactKind::Review)
            .unwrap();
        assert_eq!(
            review.status,
            if expected_status == "done" {
                ProofArtifactStatus::Passed
            } else {
                ProofArtifactStatus::Failed
            }
        );
        ctx.skill_evals_store
            .set_iter_scoring(&iter.id, &score, Some(&pack.id))
            .await
            .unwrap();
        ctx.skill_evals_store
            .set_iter_score(&iter.id, 100.0)
            .await
            .unwrap();
        crate::eval_score::publish_validation_retry(&ctx, &eval.id, &iter.id, Some(score))
            .await
            .unwrap();
        let published = ctx.skill_evals_store.get_iteration(&iter.id).await.unwrap();
        assert_eq!(
            published.score,
            (100.0 + expected_score) / 2.0,
            "iteration badge must include the failed validator in its denominator"
        );
        assert_eq!(published.scoring.unwrap().review.score, expected_score);
    }
}

#[tokio::test]
async fn rating_waits_for_retry_publication_and_keeps_the_final_score_consistent() {
    use otto_core::proof::{ProofArtifactKind, WorkItemKind};
    let pool = otto_state::db::test_pool().await;
    sqlx::query("INSERT INTO users(id,username,password_hash,is_root,created_at) VALUES('editor','editor','unused',0,'2026-10-08T00:00:00Z')")
        .execute(&pool).await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let ctx = ServerCtx::for_tests(&pool, dir.path()).await;
    let ws = ctx
        .workspaces
        .create("fixture", dir.path().to_str().unwrap(), &"editor".into())
        .await
        .unwrap();
    let eval = ctx
        .skill_evals_store
        .create_eval(
            &ws.id,
            "skill",
            "task",
            "fixture",
            1,
            &serde_json::json!({}),
        )
        .await
        .unwrap();
    let agent = EvalValidationState {
        validation: "fixture".into(),
        name: "fixture".into(),
        provider: "fixture".into(),
        model: String::new(),
        status: "done".into(),
        note: String::new(),
        passed: true,
        score: 100.0,
        session_id: None,
        findings: vec![],
    };
    let iter = ctx
        .skill_evals_store
        .add_iteration(&eval.id, 1, None, "skill", "body", "fixture", &[agent])
        .await
        .unwrap();
    let pack = ctx
        .proof_repo
        .create_pack(
            &ws.id,
            WorkItemKind::Task,
            &iter.id,
            "fixture",
            "editor",
            None,
        )
        .await
        .unwrap();
    let previous = EvalScore {
        human: otto_core::eval_score::human_score(Some(5), "old", "editor"),
        ..Default::default()
    };
    ctx.skill_evals_store
        .set_iter_scoring(&iter.id, &previous, Some(&pack.id))
        .await
        .unwrap();
    ctx.skill_evals_store
        .set_iter_human(&iter.id, 5, "old", "editor")
        .await
        .unwrap();

    // Pause the actual retry between its fresh snapshot and score publication.
    let proof_lock = Arc::new(tokio::sync::Mutex::new(()));
    ctx.proof_locks
        .lock()
        .unwrap()
        .insert(pack.id.clone(), proof_lock.clone());
    let blocked_proof = proof_lock.lock().await;
    let retry_ctx = ctx.clone();
    let eval_id = eval.id.clone();
    let iter_id = iter.id.clone();
    let retry = tokio::spawn(async move {
        crate::eval_score::publish_validation_retry(&retry_ctx, &eval_id, &iter_id, Some(previous))
            .await
    });
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if ctx
                .proof_repo
                .list_artifacts_meta(&pack.id)
                .await
                .unwrap()
                .iter()
                .any(|a| a.kind == ProofArtifactKind::Review)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let user = otto_state::UsersRepo::new(pool)
        .get(&"editor".into())
        .await
        .unwrap();
    let rating = rate_iteration(
        AxPath((eval.id.clone(), iter.id.clone())),
        State(ctx.clone()),
        CurrentUser(user),
        Json(RateIterationReq {
            rating: 1,
            note: "new".into(),
        }),
    );
    tokio::pin!(rating);
    assert!(tokio::time::timeout(Duration::from_millis(20), &mut rating)
        .await
        .is_err());
    assert_eq!(
        ctx.skill_evals_store
            .get_iteration(&iter.id)
            .await
            .unwrap()
            .human_rating,
        Some(5),
        "rating must wait before persisting while retry owns publication"
    );
    drop(blocked_proof);
    retry.await.unwrap().unwrap();
    let _ = rating.await.unwrap();
    let stored = ctx.skill_evals_store.get_iteration(&iter.id).await.unwrap();
    assert_eq!(stored.human_rating, Some(1));
    let scoring = stored.scoring.unwrap();
    assert_eq!(scoring.human.rating, Some(1));
    assert_eq!(scoring.human.note, "new");
    assert!((scoring.composite - 70.0).abs() < 1e-6);
    assert_eq!(
        ctx.skill_evals_store
            .get_eval(&eval.id)
            .await
            .unwrap()
            .composite_score,
        Some(scoring.composite)
    );
}

#[tokio::test]
async fn initial_scoring_publishes_rating_saved_while_commands_run() {
    let pool = otto_state::db::test_pool().await;
    sqlx::query("INSERT INTO users(id,username,password_hash,is_root,created_at) VALUES('editor','editor','unused',0,'2026-10-08T00:00:00Z')")
        .execute(&pool).await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let ctx = ServerCtx::for_tests(&pool, dir.path()).await;
    let ws = ctx
        .workspaces
        .create("fixture", dir.path().to_str().unwrap(), &"editor".into())
        .await
        .unwrap();
    let eval = ctx
        .skill_evals_store
        .create_eval(
            &ws.id,
            "skill",
            "task",
            "fixture",
            1,
            &serde_json::json!({}),
        )
        .await
        .unwrap();
    let req: StartSkillEvalReq = serde_json::from_value(serde_json::json!({
        "source":{"kind":"library","reference":"fixture"},
        "task":"fixture", "impl_cli":"fixture", "validations":[], "iterations":1,
        "mode":"score_only", "test_cmd":"printf started > started; while [ ! -f release ]; do sleep 0.02; done"
    })).unwrap();
    let run_ctx = ctx.clone();
    let eval_id = eval.id.clone();
    let run = tokio::spawn(async move {
        run_score_only_core(
            &run_ctx,
            &eval_id,
            &ws,
            &req,
            &Arc::new(AtomicBool::new(false)),
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(10), async {
        while !dir.path().join("started").exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let iter = ctx
        .skill_evals_store
        .get_eval(&eval.id)
        .await
        .unwrap()
        .iterations
        .remove(0);
    let user = otto_state::UsersRepo::new(pool)
        .get(&"editor".into())
        .await
        .unwrap();
    let _ = rate_iteration(
        AxPath((eval.id.clone(), iter.id.clone())),
        State(ctx.clone()),
        CurrentUser(user),
        Json(RateIterationReq {
            rating: 1,
            note: "during command".into(),
        }),
    )
    .await
    .unwrap();
    std::fs::write(dir.path().join("release"), "go").unwrap();
    run.await.unwrap().unwrap();
    let stored = ctx.skill_evals_store.get_eval(&eval.id).await.unwrap();
    let iter = &stored.iterations[0];
    assert_eq!(iter.human_rating, Some(1));
    let score = iter.scoring.as_ref().unwrap();
    assert_eq!(
        score.human.rating,
        Some(1),
        "initial scoring overwrote the concurrent rating signal"
    );
    assert_eq!(score.human.note, "during command");
    assert_eq!(stored.composite_score, Some(score.composite));
    assert_eq!(stored.best_score, Some(score.composite));
    let artifacts = ctx
        .proof_repo
        .list_artifacts_meta(iter.proof_pack_id.as_ref().unwrap())
        .await
        .unwrap();
    let approval = artifacts
        .iter()
        .find(|a| a.kind == otto_core::proof::ProofArtifactKind::Approval)
        .unwrap();
    assert_eq!(approval.metadata["rating"], serde_json::json!(1));
}

#[tokio::test]
async fn rating_reselects_best_iteration_from_current_scores() {
    let pool = otto_state::db::test_pool().await;
    sqlx::query("INSERT INTO users(id,username,password_hash,is_root,created_at) VALUES('editor','editor','unused',0,'2026-10-08T00:00:00Z')")
        .execute(&pool).await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let ctx = ServerCtx::for_tests(&pool, dir.path()).await;
    let ws = ctx
        .workspaces
        .create("fixture", dir.path().to_str().unwrap(), &"editor".into())
        .await
        .unwrap();
    let eval = ctx
        .skill_evals_store
        .create_eval(
            &ws.id,
            "skill",
            "task",
            "fixture",
            2,
            &serde_json::json!({}),
        )
        .await
        .unwrap();
    let mut iterations = vec![];
    for (n, rating) in [(1, 5), (2, 4)] {
        let it = ctx
            .skill_evals_store
            .add_iteration(&eval.id, n, None, "skill", "body", "fixture", &[])
            .await
            .unwrap();
        let user = otto_state::UsersRepo::new(pool.clone())
            .get(&"editor".into())
            .await
            .unwrap();
        let _ = rate_iteration(
            AxPath((eval.id.clone(), it.id.clone())),
            State(ctx.clone()),
            CurrentUser(user),
            Json(RateIterationReq {
                rating,
                note: "initial".into(),
            }),
        )
        .await
        .unwrap();
        iterations.push(it);
    }
    let initial = ctx.skill_evals_store.get_eval(&eval.id).await.unwrap();
    assert_eq!(initial.best_iteration, Some(1));
    assert_eq!(initial.best_score, Some(100.0));
    let user = otto_state::UsersRepo::new(pool)
        .get(&"editor".into())
        .await
        .unwrap();
    let _ = rate_iteration(
        AxPath((eval.id.clone(), iterations[0].id.clone())),
        State(ctx.clone()),
        CurrentUser(user),
        Json(RateIterationReq {
            rating: 1,
            note: "corrected".into(),
        }),
    )
    .await
    .unwrap();
    let stored = ctx.skill_evals_store.get_eval(&eval.id).await.unwrap();
    assert_eq!(stored.best_iteration, Some(2));
    assert_eq!(stored.best_score, Some(80.0));
    assert_eq!(stored.composite_score, Some(80.0));
}

#[tokio::test]
async fn rating_storage_failure_does_not_keep_a_stale_publishable_score() {
    let pool = otto_state::db::test_pool().await;
    sqlx::query("INSERT INTO users(id,username,password_hash,is_root,created_at) VALUES('editor','editor','unused',0,'2026-10-08T00:00:00Z')").execute(&pool).await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let ctx = ServerCtx::for_tests(&pool, dir.path()).await;
    let ws = ctx
        .workspaces
        .create("fixture", dir.path().to_str().unwrap(), &"editor".into())
        .await
        .unwrap();
    let eval = ctx
        .skill_evals_store
        .create_eval(
            &ws.id,
            "skill",
            "task",
            "fixture",
            1,
            &serde_json::json!({}),
        )
        .await
        .unwrap();
    let iter = ctx
        .skill_evals_store
        .add_iteration(&eval.id, 1, None, "skill", "body", "fixture", &[])
        .await
        .unwrap();
    let user = otto_state::UsersRepo::new(pool.clone())
        .get(&"editor".into())
        .await
        .unwrap();
    let _ = rate_iteration(
        AxPath((eval.id.clone(), iter.id.clone())),
        State(ctx.clone()),
        CurrentUser(user.clone()),
        Json(RateIterationReq {
            rating: 5,
            note: "old".into(),
        }),
    )
    .await
    .unwrap();
    // Retain a real command signal across a failed rating and a later retry.
    let mut previous = ctx
        .skill_evals_store
        .get_iteration(&iter.id)
        .await
        .unwrap()
        .scoring
        .unwrap();
    previous.tests = otto_core::eval_score::signal_from_cmd(true, true, "already ran");
    ctx.skill_evals_store
        .set_iter_scoring(&iter.id, &previous, None)
        .await
        .unwrap();
    ctx.skill_evals_store
        .set_status(&eval.id, SkillEvalStatus::Done, None)
        .await
        .unwrap();
    ctx.skill_evals_store
        .set_iter_status(&iter.id, "done", "complete")
        .await
        .unwrap();
    let mut config = default_skill_eval_config("fixture");
    config.require_proof_pass = false;
    config.promote_min_score = 0.0;
    otto_state::SettingsRepo::new(pool.clone())
        .put("skill_eval", &serde_json::to_value(config).unwrap())
        .await
        .unwrap();
    for trigger in [
        "CREATE TRIGGER reject_publication BEFORE UPDATE OF scoring_json ON skill_eval_iterations WHEN json_extract(NEW.scoring_json, '$.proof_status') != 'pending' BEGIN SELECT RAISE(ABORT, 'injected scoring failure'); END",
        "CREATE TRIGGER reject_publication BEFORE UPDATE OF composite_score ON skill_evals WHEN NEW.composite_score IS NOT NULL BEGIN SELECT RAISE(ABORT, 'injected headline failure'); END",
        "CREATE TRIGGER reject_publication BEFORE UPDATE ON proof_artifacts WHEN NEW.kind = 'approval' BEGIN SELECT RAISE(ABORT, 'injected approval failure'); END",
        "CREATE TRIGGER reject_publication BEFORE UPDATE ON proof_packs BEGIN SELECT RAISE(ABORT, 'injected proof status failure'); END",
    ] {
        sqlx::query(trigger).execute(&pool).await.unwrap();
        let result = rate_iteration(
            AxPath((eval.id.clone(), iter.id.clone())), State(ctx.clone()), CurrentUser(user.clone()),
            Json(RateIterationReq { rating: 1, note: "new".into() }),
        ).await;
        assert!(result.is_err(), "rating falsely reported success after storage failure: {trigger}");
        let stored = ctx.skill_evals_store.get_eval(&eval.id).await.unwrap();
        assert_eq!(stored.iterations[0].human_rating, Some(1));
        let pending = stored.iterations[0].scoring.as_ref().unwrap();
        assert_eq!(pending.proof_status, "pending");
        assert_eq!(pending.human.rating, Some(1));
        assert_eq!(serde_json::to_value(&pending.tests).unwrap(), serde_json::to_value(&previous.tests).unwrap());
        assert!(stored.best_score.is_none());
        assert!(stored.best_iteration.is_none());
        assert!(stored.composite_score.is_none());
        let gate = iteration_gate(&ctx, &stored, &stored.iterations[0]).await;
        assert!(!gate.allowed, "pending evidence must block even without proof required");
        assert!(!gate.require_proof);
        sqlx::query("DROP TRIGGER reject_publication").execute(&pool).await.unwrap();
        let _ = rate_iteration(
            AxPath((eval.id.clone(), iter.id.clone())), State(ctx.clone()), CurrentUser(user.clone()),
            Json(RateIterationReq { rating: 1, note: "recovered".into() }),
        ).await.unwrap();
        let recovered = ctx.skill_evals_store.get_eval(&eval.id).await.unwrap();
        let score = recovered.iterations[0].scoring.as_ref().unwrap();
        assert_ne!(score.proof_status, "pending");
        assert_eq!(serde_json::to_value(&score.tests).unwrap(), serde_json::to_value(&previous.tests).unwrap());
        assert_eq!(score.human.note, "recovered");
        assert_eq!(recovered.best_score, Some(score.composite));
        assert_eq!(recovered.composite_score, Some(score.composite));
        assert!(iteration_gate(&ctx, &recovered, &recovered.iterations[0]).await.allowed);
    }
}

#[test]
fn skill_eval_attempt_paths_are_unique() {
    assert_ne!(
        output_path(&"eval".into(), 1, "validator-retry"),
        output_path(&"eval".into(), 1, "validator-retry")
    );
}

#[test]
fn failed_implementation_cannot_produce_a_validation_summary() {
    let failed = AgentOutcome {
        session_id: None,
        text: String::new(),
        errored: true,
    };
    assert!(implementation_summary(&failed).is_err());
}
