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
use otto_core::eval_score::{compute_composite, diff_score, human_score, signal_from_cmd, signal_score};
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
) -> Result<(EvalScore, String)> {
    let ws = &eval.workspace_id;
    let Some(worktree) = iter.worktree_path.clone() else {
        // Nothing on disk to score — return an empty score, no pack.
        return Ok((EvalScore { weights: weights.clone(), ..Default::default() }, String::new()));
    };

    // 1. Ensure the proof pack (idempotent) + link it to the resolved repo.
    let pack = proof::gate(ctx, WorkItemKind::Task, &iter.id, ws, &pack_title(eval), "otto").await?;
    if let Some(repo_id) = proof::resolve_repo_for_cwd(ctx, ws, &worktree).await {
        let _ = ctx.proof_repo.set_repo_link(&pack.id, Some(&repo_id), None).await;
    }

    // 2. Diff artifact → diff-quality signal.
    let _ = proof::assemble_diff(ctx, &pack, &worktree, diff_base).await;
    let arts = ctx.proof_repo.list_artifacts(&pack.id).await.unwrap_or_default();
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
    let tests = run_cmd_signal(ctx, &pack, &worktree, test_cmd, "test").await?;
    let lint = run_cmd_signal(ctx, &pack, &worktree, lint_cmd, "lint").await?;

    // 4. Review signal from the iteration's validator findings.
    let review = review_signal(ctx, &pack, iter).await?;

    // 5. Human rating signal (Approval artifact when present).
    let human = human_score(iter.human_rating, &iter.human_note, &iter.human_rater);
    if let Some(r) = iter.human_rating {
        let body = format!("rating: {r}/5\n{}", iter.human_note);
        let by = if iter.human_rater.is_empty() { "otto" } else { &iter.human_rater };
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
            Ok(signal_from_cmd(true, ok, format!("`{c}` → {}", st.as_str())))
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
    let (passed, score) = crate::skill_eval::score_findings(&findings);
    let status = if passed {
        ProofArtifactStatus::Passed
    } else {
        ProofArtifactStatus::Failed
    };
    let mut body = format!("{} finding(s)\n", findings.len());
    for f in &findings {
        body.push_str(&format!(
            "- [{}] {}{}\n",
            f.severity,
            f.issue,
            f.location.as_deref().map(|l| format!(" ({l})")).unwrap_or_default()
        ));
    }
    let _ = proof::upsert_content_artifact(
        ctx,
        pack,
        ProofArtifactKind::Review,
        "Validator findings",
        &body,
        status,
        json!({ "findings": findings.len(), "passed": passed }),
        "otto",
    )
    .await;
    Ok(signal_score(true, score, format!("{} finding(s)", findings.len())))
}
