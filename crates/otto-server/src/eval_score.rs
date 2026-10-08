//! The eval-lab scoring pipeline — turns one iteration's produced code into a
//! multi-signal [`EvalScore`] backed by a Proof Pack.
//!
//! It is the wiring between the eval engine and the proof engine: it assembles a
//! Proof Pack (`WorkItemKind::Task` ⇒ `CodeChange`) from the iteration's
//! tests / lint / diff / review / human signals, lets the proof engine derive the
//! authoritative status + done-score, and blends the signals into a composite via
//! the pure math in [`otto_core::eval_score`]. No agent is involved, so it runs in
//! both `generate` mode (after validation) and `score_only` mode (no agent at all).

use otto_core::domain::{
    DiffScore, EvalIteration, EvalScore, GoldenTask, ScoreWeights, SignalScore, SkillEval,
};
use otto_core::eval_score::{
    compute_composite, diff_score, human_score, signal_from_cmd, signal_score,
};
use otto_core::proof::{
    compute_risk, ProofArtifact, ProofArtifactKind, ProofArtifactStatus, WorkItemKind,
};
use otto_core::Result;
use serde_json::json;

use crate::proof;
use crate::state::ServerCtx;

/// A short, human title for an eval iteration's proof pack.
fn pack_title(eval: &SkillEval) -> String {
    let task: String = eval.task.chars().take(60).collect();
    format!("eval: {} · {}", eval.source_skill, task.trim())
}

fn meta_u32(a: &ProofArtifact, key: &str) -> u32 {
    a.metadata.get(key).and_then(|v| v.as_u64()).unwrap_or(0) as u32
}

/// Run the full scoring pipeline for one iteration and return its [`EvalScore`].
/// Assembles (and persists) the iteration's Proof Pack along the way; the caller
/// persists the returned score via `set_iter_scoring`.
///
/// - `diff_base` is the git ref the diff is measured against: `None` ⇒ working tree
///   vs HEAD (uncommitted impl-agent changes / dirty target), `Some(ref)` ⇒
///   `ref..HEAD` (a committed branch target).
/// - `test_cmd` / `lint_cmd` are the resolved commands (golden → request → config).
#[allow(clippy::too_many_arguments)]
pub async fn score_iteration(
    ctx: &ServerCtx,
    eval: &SkillEval,
    iter: &EvalIteration,
    _golden: Option<&GoldenTask>,
    weights: &ScoreWeights,
    diff_base: Option<&str>,
    test_cmd: Option<&str>,
    lint_cmd: Option<&str>,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<(EvalScore, String)> {
    cancellable(
        cancel,
        score_iteration_inner(
            ctx, eval, iter, _golden, weights, diff_base, test_cmd, lint_cmd, cancel,
        ),
    )
    .await
}

/// Dropping a proof command future terminates its owned process group.
pub(crate) async fn cancellable<T>(
    cancel: &std::sync::atomic::AtomicBool,
    future: impl std::future::Future<Output = Result<T>>,
) -> Result<T> {
    tokio::select! {
        biased;
        _ = async {
            while !cancel.load(std::sync::atomic::Ordering::SeqCst) {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        } => Err(otto_core::Error::Conflict("evaluation cancelled".into())),
        result = future => result,
    }
}

#[allow(clippy::too_many_arguments)]
async fn score_iteration_inner(
    ctx: &ServerCtx,
    eval: &SkillEval,
    iter: &EvalIteration,
    _golden: Option<&GoldenTask>,
    weights: &ScoreWeights,
    diff_base: Option<&str>,
    test_cmd: Option<&str>,
    lint_cmd: Option<&str>,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<(EvalScore, String)> {
    let ws = &eval.workspace_id;
    let Some(worktree) = iter.worktree_path.clone() else {
        // Nothing on disk to score — return an empty score, no pack.
        return Ok((
            EvalScore {
                weights: weights.clone(),
                ..Default::default()
            },
            String::new(),
        ));
    };

    // 1. Ensure the proof pack (idempotent) + link it to the resolved repo.
    let pack = proof::gate(
        ctx,
        WorkItemKind::Task,
        &iter.id,
        ws,
        &pack_title(eval),
        "otto",
    )
    .await?;
    if let Some(repo_id) = proof::resolve_repo_for_cwd(ctx, ws, &worktree).await {
        let _ = ctx
            .proof_repo
            .set_repo_link(&pack.id, Some(&repo_id), None)
            .await;
    }

    // 2. Diff artifact → diff-quality signal.
    let _ = proof::assemble_diff(ctx, &pack, &worktree, diff_base).await;
    let arts = ctx
        .proof_repo
        .list_artifacts(&pack.id)
        .await
        .unwrap_or_default();
    let diff: DiffScore = match arts.iter().find(|a| a.kind == ProofArtifactKind::Diff) {
        Some(d) => {
            let files = meta_u32(d, "files_changed");
            let add = meta_u32(d, "additions");
            let del = meta_u32(d, "deletions");
            let risky = d
                .metadata
                .get("risky_files")
                .and_then(|v| v.as_array())
                .map(|a| a.len() as u32)
                .unwrap_or(0);
            // Risk over the diff artifact ALONE so the failing-test / review
            // penalties inside `compute_risk` don't double-count those signals.
            let risk = compute_risk(std::slice::from_ref(d));
            diff_score(files, add, del, risky, risk)
        }
        None => DiffScore::default(),
    };

    // 3. Tests + lint command signals (recorded as proof `command` artifacts).
    let tests = cancellable(
        cancel,
        run_cmd_signal(ctx, &pack, &worktree, test_cmd, "test"),
    )
    .await?;
    let lint = cancellable(
        cancel,
        run_cmd_signal(ctx, &pack, &worktree, lint_cmd, "lint"),
    )
    .await?;

    // 4. Review signal from the iteration's validator findings.
    let review = review_signal(ctx, &pack, iter).await?;

    // 5. Human rating signal (Approval artifact when present).
    let human = human_score(iter.human_rating, &iter.human_note, &iter.human_rater);
    if let Some(r) = iter.human_rating {
        let body = format!("rating: {r}/5\n{}", iter.human_note);
        let by = if iter.human_rater.is_empty() {
            "otto"
        } else {
            &iter.human_rater
        };
        let _ = proof::upsert_content_artifact(
            ctx,
            &pack,
            ProofArtifactKind::Approval,
            "Human rating",
            &body,
            ProofArtifactStatus::Passed,
            json!({ "rating": r }),
            by,
        )
        .await;
    }

    // 6. Let the proof engine derive the authoritative status + done score.
    let refreshed = proof::recompute_and_emit(ctx, &pack.id).await?;

    let mut score = EvalScore {
        tests,
        lint,
        diff,
        review,
        human,
        weights: weights.clone(),
        composite: 0.0,
        proof_status: refreshed.status.as_str().to_string(),
        done_score: refreshed.done_score,
    };
    score.composite = compute_composite(&score);
    Ok((score, pack.id))
}

/// Re-derive an iteration's score after a human rating change WITHOUT re-running
/// commands: re-reads the persisted signals, upserts the Approval artifact,
/// recomputes the proof status, and recomputes the composite. Deterministic and
/// cheap — a star rating never re-runs tests.
pub async fn rescore_with_human(
    ctx: &ServerCtx,
    eval: &SkillEval,
    iter: &EvalIteration,
    rating: u8,
    note: &str,
    rater: &str,
) -> Result<(EvalScore, String)> {
    let pack = proof::gate(
        ctx,
        WorkItemKind::Task,
        &iter.id,
        &eval.workspace_id,
        &pack_title(eval),
        "otto",
    )
    .await?;
    let body = format!("rating: {rating}/5\n{note}");
    let by = if rater.is_empty() { "otto" } else { rater };
    let _ = proof::upsert_content_artifact(
        ctx,
        &pack,
        ProofArtifactKind::Approval,
        "Human rating",
        &body,
        ProofArtifactStatus::Passed,
        json!({ "rating": rating }),
        by,
    )
    .await;
    let refreshed = proof::recompute_and_emit(ctx, &pack.id).await?;

    let mut score = iter.scoring.clone().unwrap_or_default();
    if score.weights.tests == 0.0 && score.weights.review == 0.0 {
        score.weights = ScoreWeights::default();
    }
    score.human = human_score(Some(rating), note, rater);
    score.proof_status = refreshed.status.as_str().to_string();
    score.done_score = refreshed.done_score;
    score.composite = compute_composite(&score);
    Ok((score, pack.id))
}

/// Refresh only validator evidence after retry; original test/lint commands
/// are not re-run. Admission cleared the published score until this succeeds.
pub(crate) async fn rescore_validation(
    ctx: &ServerCtx,
    eval: &SkillEval,
    iter: &EvalIteration,
    previous: Option<EvalScore>,
) -> Result<(EvalScore, String)> {
    let id = iter.proof_pack_id.as_deref().ok_or_else(|| {
        otto_core::Error::Conflict("iteration has no proof pack; run a new evaluation".into())
    })?;
    let pack = ctx.proof_repo.get_pack(id).await?;
    if pack.workspace_id != eval.workspace_id
        || pack.work_item_id != iter.id
        || pack.work_item_kind != WorkItemKind::Task
    {
        return Err(otto_core::Error::Conflict(
            "proof pack belongs to another iteration".into(),
        ));
    }
    let mut score = previous.unwrap_or_default();
    score.review = review_signal(ctx, &pack, iter).await?;
    let fresh = proof::recompute_and_emit(ctx, id).await?;
    score.proof_status = fresh.status.as_str().to_string();
    score.done_score = fresh.done_score;
    score.composite = compute_composite(&score);
    Ok((score, id.to_string()))
}

/// Run a test/lint command (if configured) as a proof `command` artifact and map
/// it to a 0/100 gate signal.
async fn run_cmd_signal(
    ctx: &ServerCtx,
    pack: &otto_core::proof::ProofPack,
    cwd: &str,
    cmd: Option<&str>,
    kind_hint: &str,
) -> Result<SignalScore> {
    match cmd {
        Some(c) if !c.trim().is_empty() => {
            let st = proof::run_command_artifact(ctx, pack, cwd, c, Some(kind_hint)).await?;
            let ok = st == ProofArtifactStatus::Passed;
            Ok(signal_from_cmd(
                true,
                ok,
                format!("`{c}` → {}", st.as_str()),
            ))
        }
        _ => Ok(SignalScore::default()),
    }
}

/// Aggregate the iteration's validator findings into a review signal + a Review
/// proof artifact. No-op (signal not run) when the iteration has no validators.
async fn review_signal(
    ctx: &ServerCtx,
    pack: &otto_core::proof::ProofPack,
    iter: &EvalIteration,
) -> Result<SignalScore> {
    if iter.agents.is_empty() {
        return Ok(SignalScore::default());
    }
    let mut findings = Vec::new();
    for a in &iter.agents {
        findings.extend(a.findings.clone());
    }
    let (findings_passed, findings_score) = crate::skill_eval::score_findings(&findings);
    let complete = iter.agents.iter().all(|agent| agent.status == "done");
    let passed = complete && findings_passed;
    let score = if complete { findings_score } else { 0.0 };
    let status = if passed {
        ProofArtifactStatus::Passed
    } else {
        ProofArtifactStatus::Failed
    };
    let mut body = format!(
        "{} finding(s); all validators completed: {complete}\n",
        findings.len()
    );
    for f in &findings {
        body.push_str(&format!(
            "- [{}] {}{}\n",
            f.severity,
            f.issue,
            f.location
                .as_deref()
                .map(|l| format!(" ({l})"))
                .unwrap_or_default()
        ));
    }
    proof::upsert_content_artifact(
        ctx,
        pack,
        ProofArtifactKind::Review,
        "Validator findings",
        &body,
        status,
        json!({ "findings": findings.len(), "passed": passed }),
        "otto",
    )
    .await?;
    Ok(signal_score(
        true,
        score,
        format!("{} finding(s)", findings.len()),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };

    async fn fixture() -> (tempfile::TempDir, ServerCtx, SkillEval, EvalIteration) {
        let dir = tempfile::tempdir().unwrap();
        let pool = otto_state::db::test_pool().await;
        sqlx::query("INSERT INTO users(id,username,password_hash,is_root,created_at) VALUES('editor','editor','unused',0,'2026-10-08T00:00:00Z')").execute(&pool).await.unwrap();
        let ctx = ServerCtx::for_tests(&pool, dir.path()).await;
        let ws = ctx
            .workspaces
            .create("fixture", dir.path().to_str().unwrap(), &"editor".into())
            .await
            .unwrap();
        let eval = ctx
            .skill_evals_store
            .create_eval(&ws.id, "skill", "task", "fixture", 1, &json!({}))
            .await
            .unwrap();
        let mut iter = ctx
            .skill_evals_store
            .add_iteration(&eval.id, 1, None, "skill", "body", "fixture", &[])
            .await
            .unwrap();
        iter.worktree_path = Some(dir.path().to_string_lossy().into_owned());
        (dir, ctx, eval, iter)
    }

    #[tokio::test]
    async fn cancelled_scoring_kills_command_and_never_starts_lint() {
        let (dir, ctx, eval, iter) = fixture().await;
        let cancel = Arc::new(AtomicBool::new(false));
        let child_cancel = cancel.clone();
        let task = tokio::spawn(async move {
            score_iteration(
                &ctx,
                &eval,
                &iter,
                None,
                &ScoreWeights::default(),
                None,
                Some("printf started > started; sleep 1; printf late > late"),
                Some("printf lint > lint"),
                &child_cancel,
            )
            .await
        });
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while !dir.path().join("started").exists() {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        cancel.store(true, Ordering::SeqCst);
        assert!(task.await.unwrap().is_err());
        tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
        assert!(!dir.path().join("late").exists());
        assert!(!dir.path().join("lint").exists());
    }

    #[tokio::test]
    async fn errored_or_failed_retry_replaces_previously_passed_review_evidence() {
        let (_dir, ctx, eval, mut iter) = fixture().await;
        let pack = ctx
            .proof_repo
            .create_pack(
                &eval.workspace_id,
                WorkItemKind::Task,
                &iter.id,
                "fixture",
                "editor",
                None,
            )
            .await
            .unwrap();
        iter.proof_pack_id = Some(pack.id.clone());
        let agent = |status: &str, findings: Vec<otto_core::domain::EvalFinding>| {
            otto_core::domain::EvalValidationState {
                validation: "fixture".into(),
                name: "fixture".into(),
                provider: "fixture".into(),
                model: String::new(),
                status: status.into(),
                note: String::new(),
                passed: findings.is_empty(),
                score: 100.0,
                session_id: None,
                findings,
            }
        };
        iter.agents = vec![agent("done", vec![])];
        assert_eq!(
            review_signal(&ctx, &pack, &iter).await.unwrap().score,
            100.0
        );
        for failed in [
            agent("error", vec![]),
            agent(
                "done",
                vec![otto_core::domain::EvalFinding {
                    severity: "fail".into(),
                    issue: "regression".into(),
                    suggestion: String::new(),
                    location: None,
                }],
            ),
        ] {
            iter.agents = vec![failed];
            let previous = EvalScore {
                composite: 100.0,
                proof_status: "passed".into(),
                ..Default::default()
            };
            let (score, _) = rescore_validation(&ctx, &eval, &iter, Some(previous))
                .await
                .unwrap();
            assert!(score.review.score < 100.0);
            let artifacts = ctx.proof_repo.list_artifacts_meta(&pack.id).await.unwrap();
            assert!(artifacts
                .iter()
                .any(|a| a.kind == ProofArtifactKind::Review
                    && a.status == ProofArtifactStatus::Failed));
            assert_ne!(score.proof_status, "passed");
        }
    }
}
