//! Exercise cancellation while retry preparation is waiting on local Git.
//! The pause is test-only and keyed to one owned temporary worktree.
use super::*;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use tokio::sync::oneshot;

type Pause = (oneshot::Sender<()>, oneshot::Receiver<()>);

fn pauses() -> &'static Mutex<HashMap<String, Pause>> {
    static PAUSES: OnceLock<Mutex<HashMap<String, Pause>>> = OnceLock::new();
    PAUSES.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(super) async fn before_diff(path: &str) {
    let pause = pauses().lock().unwrap().remove(path);
    if let Some((entered, resume)) = pause {
        let _ = entered.send(());
        let _ = resume.await;
    }
}

#[tokio::test]
async fn cancellation_does_not_wait_for_retry_diff_and_retry_rechecks_status() {
    let dir = tempfile::tempdir().unwrap();
    let pool = otto_state::db::test_pool().await;
    sqlx::query("INSERT INTO users(id,username,password_hash,is_root,created_at) VALUES('editor','editor','unused',0,'2026-10-08T00:00:00Z')").execute(&pool).await.unwrap();
    let ctx = ServerCtx::for_tests(&pool, dir.path()).await;
    let ws = ctx
        .workspaces
        .create("fixture", dir.path().to_str().unwrap(), &"editor".into())
        .await
        .unwrap();
    let req: StartSkillEvalReq = serde_json::from_value(serde_json::json!({
        "source":{"kind":"library","reference":"fixture"},
        "task":"fixture", "impl_cli":"fixture", "iterations":1,
        "validations":[{"name":"performance","providers":["fixture"],"criteria":"fixture"}]
    }))
    .unwrap();
    let eval = ctx
        .skill_evals_store
        .create_eval(
            &ws.id,
            "fixture",
            "fixture",
            "fixture",
            1,
            &serde_json::to_value(req).unwrap(),
        )
        .await
        .unwrap();
    let agent = EvalValidationState {
        validation: "performance".into(),
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
        .add_iteration(&eval.id, 1, None, "fixture", "body", "fixture", &[agent])
        .await
        .unwrap();
    ctx.skill_evals_store
        .set_iter_impl(&iter.id, None, "fixture", dir.path().to_str())
        .await
        .unwrap();
    ctx.skill_evals_store
        .set_status(&eval.id, SkillEvalStatus::Done, None)
        .await
        .unwrap();
    let user = otto_state::UsersRepo::new(pool.clone())
        .get(&"editor".into())
        .await
        .unwrap();
    let (entered_tx, entered_rx) = oneshot::channel();
    let (resume_tx, resume_rx) = oneshot::channel();
    pauses().lock().unwrap().insert(
        dir.path().to_string_lossy().into_owned(),
        (entered_tx, resume_rx),
    );
    let task_ctx = ctx.clone();
    let eval_id = eval.id.clone();
    let task_iter = iter.id.clone();
    let retry = tokio::spawn(async move {
        retry_validation(
            AxPath((eval_id, task_iter, 0)),
            State(task_ctx),
            CurrentUser(user),
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(2), entered_rx)
        .await
        .unwrap()
        .unwrap();
    // The diff cannot finish until we explicitly resume it. This is a lock
    // ordering assertion, not a disk-speed or production latency benchmark.
    let cancelled = tokio::time::timeout(Duration::from_secs(1), cancel_run(&ctx, &eval.id)).await;
    if cancelled.is_err() {
        retry.abort();
        drop(resume_tx);
        let _ = retry.await;
        panic!("cancellation waited for retry diff while holding the publication lock");
    }
    cancelled.unwrap().unwrap();
    resume_tx.send(()).unwrap();
    let result = tokio::time::timeout(Duration::from_secs(5), retry)
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(result, Err(ApiError(Error::Conflict(_)))),
        "retry must recheck cancellation after diff preparation"
    );
    let saved = ctx.skill_evals_store.get_eval(&eval.id).await.unwrap();
    assert_eq!(saved.status, SkillEvalStatus::Cancelled);
    assert_eq!(saved.iterations[0].agents[0].status, "done");
    assert!(
        ctx.skill_eval_cancels.lock().unwrap().is_empty(),
        "rejected retry must release its lease"
    );
}
