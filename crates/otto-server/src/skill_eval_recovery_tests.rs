//! Deterministic interruption tests. The checkpoint is compiled only in tests,
//! is keyed by a fixture iteration ID, and cannot pause unrelated evaluations.
use super::*;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

fn checkpoints() -> &'static Mutex<HashMap<Id, tokio::sync::oneshot::Sender<()>>> {
    static POINTS: OnceLock<Mutex<HashMap<Id, tokio::sync::oneshot::Sender<()>>>> = OnceLock::new();
    POINTS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(super) async fn after_rating_pending(iter_id: &Id) {
    let checkpoint = checkpoints().lock().unwrap().remove(iter_id);
    if let Some(ready) = checkpoint {
        let _ = ready.send(());
        std::future::pending::<()>().await;
    }
}

async fn fixture(
    pool: &otto_state::DbPool,
    dir: &Path,
) -> (ServerCtx, SkillEval, otto_core::domain::EvalIteration, User) {
    sqlx::query("INSERT INTO users(id,username,password_hash,is_root,created_at) VALUES('editor','editor','unused',0,'2026-10-08T00:00:00Z')").execute(pool).await.unwrap();
    let ctx = ServerCtx::for_tests(pool, dir).await;
    let ws = ctx
        .workspaces
        .create("fixture", dir.to_str().unwrap(), &"editor".into())
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
        validation: "correctness".into(),
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
            note: "original".into(),
        }),
    )
    .await
    .unwrap();
    let mut score = ctx
        .skill_evals_store
        .get_iteration(&iter.id)
        .await
        .unwrap()
        .scoring
        .unwrap();
    score.tests = otto_core::eval_score::signal_from_cmd(true, false, "original failed tests");
    score.lint = otto_core::eval_score::signal_from_cmd(true, true, "original passing lint");
    score.composite = otto_core::eval_score::compute_composite(&score);
    let pack = ctx
        .skill_evals_store
        .get_iteration(&iter.id)
        .await
        .unwrap()
        .proof_pack_id
        .unwrap();
    ctx.skill_evals_store
        .publish_iter_scoring(&eval.id, &iter.id, &score, &pack, None, "fixture")
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
    (ctx, eval, iter, user)
}

fn assert_commands(score: &EvalScore) {
    assert!(score.tests.ran);
    assert_eq!(score.tests.score, 0.0);
    assert_eq!(score.tests.detail, "original failed tests");
    assert!(score.lint.ran);
    assert_eq!(score.lint.score, 100.0);
    assert_eq!(score.lint.detail, "original passing lint");
}

#[tokio::test]
async fn dropped_rating_handler_recovers_pending_evidence_and_approval() {
    let dir = tempfile::tempdir().unwrap();
    let pool = otto_state::db::test_pool().await;
    let (ctx, eval, iter, user) = fixture(&pool, dir.path()).await;
    let (ready, pending) = tokio::sync::oneshot::channel();
    checkpoints().lock().unwrap().insert(iter.id.clone(), ready);
    let rating = tokio::spawn(rate_iteration(
        AxPath((eval.id.clone(), iter.id.clone())),
        State(ctx.clone()),
        CurrentUser(user.clone()),
        Json(RateIterationReq {
            rating: 1,
            note: "interrupted".into(),
        }),
    ));
    tokio::time::timeout(Duration::from_secs(10), pending)
        .await
        .unwrap()
        .unwrap();
    rating.abort();
    assert!(rating.await.unwrap_err().is_cancelled());
    let saved = ctx.skill_evals_store.get_eval(&eval.id).await.unwrap();
    let score = saved.iterations[0].scoring.as_ref().unwrap();
    assert_commands(score);
    assert_eq!(score.proof_status, "pending");
    assert_eq!(score.human.rating, Some(1));
    assert!(saved.composite_score.is_none());
    assert!(
        !iteration_gate(&ctx, &saved, &saved.iterations[0])
            .await
            .allowed
    );
    let _ = rate_iteration(
        AxPath((eval.id.clone(), iter.id.clone())),
        State(ctx.clone()),
        CurrentUser(user),
        Json(RateIterationReq {
            rating: 1,
            note: "recovered".into(),
        }),
    )
    .await
    .unwrap();
    let recovered = ctx.skill_evals_store.get_eval(&eval.id).await.unwrap();
    let score = recovered.iterations[0].scoring.as_ref().unwrap();
    assert_commands(score);
    assert_ne!(score.proof_status, "pending");
    assert_eq!(score.human.rating, Some(1));
    assert_eq!(recovered.composite_score, Some(score.composite));
    assert_eq!(recovered.best_score, Some(score.composite));
    let arts = ctx
        .proof_repo
        .list_artifacts(recovered.iterations[0].proof_pack_id.as_ref().unwrap())
        .await
        .unwrap();
    let approval = arts
        .iter()
        .find(|a| a.kind == otto_core::proof::ProofArtifactKind::Approval)
        .unwrap();
    assert_eq!(approval.metadata["rating"], 1);
}

#[tokio::test]
async fn cancellation_storage_failure_is_reported_without_signalling_worker() {
    let dir = tempfile::tempdir().unwrap();
    let pool = otto_state::db::test_pool().await;
    let (ctx, eval, _iter, user) = fixture(&pool, dir.path()).await;
    ctx.skill_evals_store
        .set_status(&eval.id, SkillEvalStatus::Running, None)
        .await
        .unwrap();
    let lease = RetryLease::claim(&ctx.skill_eval_cancels, &eval.id, "validation").unwrap();
    sqlx::query("CREATE TRIGGER reject_cancel BEFORE UPDATE OF status ON skill_evals WHEN NEW.status = 'cancelled' BEGIN SELECT RAISE(ABORT, 'cancel unavailable'); END").execute(&pool).await.unwrap();
    let result = cancel_eval(
        AxPath(eval.id.clone()),
        State(ctx.clone()),
        CurrentUser(user.clone()),
    )
    .await;
    assert!(
        result.is_err(),
        "cancel returned success although durable cancellation failed"
    );
    assert!(
        !is_cancelled(&lease.flag),
        "worker must not exit leaving a running row"
    );
    assert_eq!(
        ctx.skill_evals_store
            .get_eval(&eval.id)
            .await
            .unwrap()
            .status,
        SkillEvalStatus::Running
    );
    sqlx::query("DROP TRIGGER reject_cancel")
        .execute(&pool)
        .await
        .unwrap();
    let _ = cancel_eval(
        AxPath(eval.id.clone()),
        State(ctx.clone()),
        CurrentUser(user),
    )
    .await
    .unwrap();
    assert!(is_cancelled(&lease.flag));
    assert_eq!(
        ctx.skill_evals_store
            .get_eval(&eval.id)
            .await
            .unwrap()
            .status,
        SkillEvalStatus::Cancelled
    );
}

// Invoked only by the parent test below in a separate OS process. The child
// retains its SQLite pools until SIGKILL, so this exercises WAL crash recovery.
#[tokio::test]
async fn retry_admission_child_process() {
    let Ok(root) = std::env::var("OTTO_TEST_RETRY_PROCESS_ROOT") else {
        return;
    };
    let root = Path::new(&root);
    let ids: (Id, Id) =
        serde_json::from_slice(&std::fs::read(root.join("ids.json")).unwrap()).unwrap();
    let pool = otto_state::db::open(&root.join("state.db")).await.unwrap();
    let repo = SkillEvalsRepo::new(pool);
    let iter = repo.get_iteration(&ids.1).await.unwrap();
    let mut pending = iter.agents[0].clone();
    pending.status = "pending".into();
    assert!(repo
        .begin_validation_retry(&ids.0, &ids.1, 0, &pending)
        .await
        .unwrap());
    std::fs::write(root.join("admitted"), std::process::id().to_string()).unwrap();
    std::future::pending::<()>().await;
}

#[tokio::test]
async fn killed_retry_process_preserves_commands_across_disk_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("state.db");
    let pool = otto_state::db::open(&db).await.unwrap();
    let (ctx, eval, iter, _user) = fixture(&pool, dir.path()).await;
    std::fs::write(
        dir.path().join("ids.json"),
        serde_json::to_vec(&(eval.id.clone(), iter.id.clone())).unwrap(),
    )
    .unwrap();
    drop(ctx);
    pool.close().await;
    let mut child = tokio::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "skill_eval::recovery_tests::retry_admission_child_process",
            "--nocapture",
        ])
        .env("OTTO_TEST_RETRY_PROCESS_ROOT", dir.path())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(20), async {
        while !dir.path().join("admitted").exists() {
            assert!(
                child.try_wait().unwrap().is_none(),
                "child exited before admission"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let child_pid = child.id().unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.path().join("admitted")).unwrap(),
        child_pid.to_string()
    );
    assert_ne!(child_pid, std::process::id());
    child.kill().await.unwrap();
    assert!(!child.wait().await.unwrap().success());
    let pool = otto_state::db::open(&db).await.unwrap();
    let ctx = ServerCtx::for_tests(&pool, dir.path()).await;
    assert_eq!(
        ctx.skill_evals_store
            .fail_running("daemon restarted")
            .await
            .unwrap(),
        1
    );
    let saved = ctx.skill_evals_store.get_eval(&eval.id).await.unwrap();
    assert_eq!(saved.status, SkillEvalStatus::Error);
    assert_commands(saved.iterations[0].scoring.as_ref().unwrap());
    assert_eq!(
        saved.iterations[0].scoring.as_ref().unwrap().proof_status,
        "pending"
    );
    assert!(
        !iteration_gate(&ctx, &saved, &saved.iterations[0])
            .await
            .allowed
    );
    let mut pending = saved.iterations[0].agents[0].clone();
    pending.status = "pending".into();
    assert!(ctx
        .skill_evals_store
        .begin_validation_retry(&eval.id, &iter.id, 0, &pending)
        .await
        .unwrap());
    // Deterministic local validator output passes through the actual parser and
    // final-state builder; no external provider or in-memory prior score survives.
    let mut scores = vec![];
    let mut findings = vec![];
    assert!(record_validation_pass(
        &AgentOutcome {
            session_id: None,
            text: "[]".into(),
            errored: false
        },
        &mut scores,
        &mut findings
    ));
    let done = finish_validation(pending, None, scores, findings, 1);
    ctx.skill_evals_store
        .set_iter_agent_at(&iter.id, 0, &done)
        .await
        .unwrap();
    let previous = ctx
        .skill_evals_store
        .get_iteration(&iter.id)
        .await
        .unwrap()
        .scoring;
    crate::eval_score::publish_validation_retry(&ctx, &eval.id, &iter.id, previous)
        .await
        .unwrap();
    assert!(ctx
        .skill_evals_store
        .finish_running(&eval.id, SkillEvalStatus::Done, None)
        .await
        .unwrap());
    let recovered = ctx.skill_evals_store.get_eval(&eval.id).await.unwrap();
    let score = recovered.iterations[0].scoring.as_ref().unwrap();
    assert_commands(score);
    assert_ne!(score.proof_status, "pending");
    assert_eq!(score.review.score, 100.0);
    assert_eq!(recovered.composite_score, Some(score.composite));
    assert_eq!(recovered.best_score, Some(score.composite));
    assert!(
        iteration_gate(&ctx, &recovered, &recovered.iterations[0])
            .await
            .allowed
    );
    pool.close().await;
}

#[tokio::test]
async fn promotion_waits_for_publication_and_rechecks_pending_score() {
    let dir = tempfile::tempdir().unwrap();
    let pool = otto_state::db::test_pool().await;
    let (ctx, eval, iter, mut user) = fixture(&pool, dir.path()).await;
    user.is_root = true;
    let auth = crate::auth::CurrentAuthContext(otto_core::auth::AuthContext {
        real_user: user.clone(),
        effective_user: user.clone(),
        scope: None,
        mcp_only: false,
        mcp_scope: None,
        mcp_internal: false,
        mcp_session_id: None,
        managed_session_id: None,
    });
    let guard = crate::eval_score::update_guard(&eval.id).await;
    let promote = promote_skill(
        AxPath(eval.id.clone()),
        State(ctx.clone()),
        auth,
        CurrentUser(user),
        Json(PromoteSkillReq {
            iteration_id: iter.id.clone(),
            source: "tested".into(),
            name: "interrupted-promotion-fixture".into(),
            force: false,
        }),
    );
    tokio::pin!(promote);
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut promote)
            .await
            .is_err(),
        "promotion escaped the in-flight score publication lock"
    );
    assert!(ctx
        .context_library
        .get_skill("interrupted-promotion-fixture")
        .is_none());
    let mut pending = ctx
        .skill_evals_store
        .get_iteration(&iter.id)
        .await
        .unwrap()
        .scoring
        .unwrap();
    pending.human = otto_core::eval_score::human_score(Some(1), "pending change", "editor");
    pending.proof_status = "pending".into();
    pending.done_score = 0;
    ctx.skill_evals_store
        .begin_human_rating(&eval.id, &iter.id, &pending)
        .await
        .unwrap();
    drop(guard);
    let result = tokio::time::timeout(Duration::from_secs(10), &mut promote)
        .await
        .unwrap();
    assert!(matches!(result, Err(ApiError(Error::Conflict(_)))));
    assert!(ctx
        .context_library
        .get_skill("interrupted-promotion-fixture")
        .is_none());
}
