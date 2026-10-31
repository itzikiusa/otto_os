//! The eval-lab scoring pipeline — turns one iteration's produced code into a
//! multi-signal [`EvalScore`].
//!
//! Computes the iteration's diff / tests / lint / review / human signals,
//! derives the authoritative verification status + readiness score, and blends
//! the signals into a composite via the pure math in [`otto_core::eval_score`].
//! No agent is involved, so it runs in both `generate` mode (after validation)
//! and `score_only` mode (no agent at all).

use otto_core::domain::{
    DiffScore, EvalIteration, EvalScore, GoldenTask, ScoreWeights, SignalScore, SkillEval,
};
use otto_core::eval_score::{
    compute_composite, diff_score, human_score, signal_from_cmd, signal_score,
};
use otto_core::Result;

use crate::state::ServerCtx;

/// Wall-clock cap per test/lint command during iteration scoring.
const CMD_TIMEOUT_SECS: u64 = 600;

/// Serialize rating writes, retry admission and final score publication for one
/// evaluation. The lock is never held while agents run. Weak entries are pruned
/// so the registry retains only active publishers/waiters.
pub(crate) async fn update_guard(id: &str) -> tokio::sync::OwnedMutexGuard<()> {
    static LOCKS: otto_core::keyed_lock::KeyedLocks<String> =
        otto_core::keyed_lock::KeyedLocks::new();
    LOCKS.lock(id).await
}

#[cfg(test)]
type RetryCheckpoint = (
    tokio::sync::oneshot::Sender<()>,
    tokio::sync::oneshot::Receiver<()>,
);

#[cfg(test)]
pub(crate) fn retry_checkpoints(
) -> &'static std::sync::Mutex<std::collections::HashMap<String, RetryCheckpoint>> {
    static POINTS: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, RetryCheckpoint>>,
    > = std::sync::OnceLock::new();
    POINTS.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// Publish a retried validator score under the same lock as human ratings. The
/// fresh iteration read and both iteration/headline writes form one update.
pub(crate) async fn publish_validation_retry(
    ctx: &ServerCtx,
    eval_id: &otto_core::Id,
    iter_id: &otto_core::Id,
    previous: Option<EvalScore>,
) -> Result<()> {
    let _guard = update_guard(eval_id).await;
    let updated = ctx.skill_evals_store.get_iteration(iter_id).await?;
    #[cfg(test)]
    {
        let cp = retry_checkpoints().lock().unwrap().remove(iter_id.as_str());
        if let Some((entered, release)) = cp {
            let _ = entered.send(());
            let _ = release.await;
        }
    }
    // The legacy iteration badge uses the same validator denominator as an
    // initial run. Incomplete validators contribute zero, never disappear.
    let mean = (!updated.agents.is_empty()).then(|| {
        updated
            .agents
            .iter()
            .map(|agent| {
                if agent.status == "done" {
                    agent.score
                } else {
                    0.0
                }
            })
            .sum::<f64>()
            / updated.agents.len() as f64
    });
    let scoring = rescore_validation(&updated, previous);
    ctx.skill_evals_store
        .publish_iter_scoring(
            eval_id,
            iter_id,
            &scoring,
            mean,
            "Validation retry complete",
        )
        .await?;
    ctx.skill_evals_store
        .set_iter_status(iter_id, "done", "validation retry complete")
        .await
}

/// Run the full scoring pipeline for one iteration and return its [`EvalScore`].
/// Publishes the score under the evaluation update lock using the latest rating.
/// Commands run outside the lock.
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
) -> Result<EvalScore> {
    cancellable(
        cancel,
        score_iteration_inner(
            ctx, eval, iter, _golden, weights, diff_base, test_cmd, lint_cmd, cancel,
        ),
    )
    .await
}

/// Dropping a command future terminates its owned process group.
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
) -> Result<EvalScore> {
    let Some(worktree) = iter.worktree_path.clone() else {
        // Nothing on disk to score — return an empty score.
        return Ok(EvalScore {
            weights: weights.clone(),
            ..Default::default()
        });
    };

    // 1. Diff-quality signal over the iteration's changes.
    let diff = score_worktree_diff(&worktree, diff_base).await;

    // 2. Tests + lint command signals.
    let tests = cancellable(cancel, run_cmd_signal(&worktree, test_cmd, cancel)).await?;
    let lint = cancellable(cancel, run_cmd_signal(&worktree, lint_cmd, cancel)).await?;

    // Commands can take minutes. Re-read user edits only after they finish, and
    // serialize score publication with ratings and validation retries.
    let _guard = update_guard(&eval.id).await;
    let iter = ctx.skill_evals_store.get_iteration(&iter.id).await?;

    // 3. Review signal from the iteration's validator findings.
    let (review, review_passed) = review_signal(&iter);

    // 4. Human rating signal (when present).
    let human = human_score(iter.human_rating, &iter.human_note, &iter.human_rater);

    let mut score = EvalScore {
        tests,
        lint,
        diff,
        review,
        human,
        weights: weights.clone(),
        composite: 0.0,
        proof_status: String::new(),
        done_score: 0,
    };
    finalize_eval_score(&mut score, review_passed);

    ctx.skill_evals_store
        .publish_iter_scoring(
            &eval.id,
            &iter.id,
            &score,
            (eval.mode == "score_only").then_some(score.composite),
            "Scoring complete",
        )
        .await?;
    Ok(score)
}

/// Re-derive an iteration's score after a human rating change WITHOUT re-running
/// commands: re-reads the persisted signals, updates the human signal,
/// recomputes the verification status, and recomputes the composite.
/// Deterministic and cheap — a star rating never re-runs tests.
pub fn rescore_with_human(iter: &EvalIteration, rating: u8, note: &str, rater: &str) -> EvalScore {
    // A recorded zero weight is intentional (e.g. lint-only scoring). Only an
    // absent score needs defaults; EvalScore::default already supplies them.
    let mut score = iter.scoring.clone().unwrap_or_default();
    let (_, review_passed) = review_signal(iter);
    score.human = human_score(Some(rating), note, rater);
    finalize_eval_score(&mut score, review_passed);
    score
}

/// Refresh only validator evidence after retry; original test/lint commands
/// are not re-run. Admission cleared the published score until this succeeds.
pub(crate) fn rescore_validation(iter: &EvalIteration, previous: Option<EvalScore>) -> EvalScore {
    let mut score = previous.unwrap_or_default();
    let (review, review_passed) = review_signal(iter);
    score.review = review;
    score.human = human_score(iter.human_rating, &iter.human_note, &iter.human_rater);
    finalize_eval_score(&mut score, review_passed);
    score
}

fn finalize_eval_score(score: &mut EvalScore, review_passed: bool) {
    score.composite = compute_composite(score);
    let any_failed = (score.tests.ran && score.tests.score < 100.0)
        || (score.lint.ran && score.lint.score < 100.0)
        || (score.review.ran && !review_passed);
    let status = if any_failed {
        "failed"
    } else if score.diff.ran
        && score.tests.ran
        && score.tests.score >= 100.0
        && looks_like_test_command(&score.tests.detail)
    {
        "passed"
    } else if score.tests.ran
        || score.lint.ran
        || score.diff.ran
        || score.review.ran
        || score.human.rating.is_some()
    {
        "partial"
    } else {
        "missing"
    };
    score.proof_status = status.to_string();
    score.done_score = score.composite.round().clamp(0.0, 100.0) as u8;
}

/// True if `cmd` looks like a recognized test/quality runner (not a trivial `true`).
fn looks_like_test_command(cmd: &str) -> bool {
    let c = cmd.to_lowercase();
    const NEEDLES: &[&str] = &[
        "cargo test",
        "cargo nextest",
        "go test",
        "npm test",
        "npm run test",
        "npm run check",
        "yarn test",
        "pnpm test",
        "jest",
        "vitest",
        "playwright test",
        "pytest",
        "python -m pytest",
        "go vet",
        "cargo clippy",
        "svelte-check",
        "ctest",
        "gradle test",
        "mvn test",
        "rspec",
        "phpunit",
    ];
    NEEDLES.iter().any(|n| c.contains(n))
}

fn is_risky_file(path: &str) -> bool {
    let p = path.to_lowercase();
    if p.contains("migrations/")
        || p.ends_with(".sql")
        || p.starts_with(".github/workflows/")
        || p == "cargo.toml"
        || p.ends_with("/cargo.toml")
        || p == "package.json"
        || p.ends_with("/package.json")
    {
        return true;
    }
    let base = p.rsplit('/').next().unwrap_or(&p);
    const RISKY_WORDS: &[&str] = &[
        "auth",
        "rbac",
        "policy",
        "secret",
        "secrets",
        "keychain",
        "sandbox",
        "netguard",
        "permission",
        "permissions",
        "password",
        "passwords",
        "crypto",
        "token",
        "tokens",
        "credential",
        "credentials",
    ];
    base.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .any(|w| RISKY_WORDS.contains(&w))
}

async fn score_worktree_diff(cwd: &str, base: Option<&str>) -> DiffScore {
    let git = otto_git::LocalGit::new(cwd);
    if git.rev_parse("HEAD").await.is_err() {
        return DiffScore::default();
    }
    let target = match base {
        Some(b) => otto_git::DiffTarget::Range(b.to_string(), "HEAD".to_string()),
        None => otto_git::DiffTarget::Working,
    };
    let opts = otto_git::DiffOpts {
        summary: true,
        ..otto_git::DiffOpts::default()
    };
    let Ok(resp) = git.diff_with(&target, &opts).await else {
        return DiffScore::default();
    };
    if resp.files.is_empty() {
        return DiffScore::default();
    }
    let files = resp.files.len() as u32;
    let additions: u32 = resp.files.iter().filter_map(|f| f.added).sum();
    let deletions: u32 = resp.files.iter().filter_map(|f| f.deleted).sum();
    let risky_files: Vec<&str> = resp
        .files
        .iter()
        .map(|f| f.path.as_str())
        .filter(|p| is_risky_file(p))
        .collect();
    let migration_touched = risky_files
        .iter()
        .any(|s| s.contains("migrations/") || s.ends_with(".sql"));
    let loc = (additions + deletions) as usize;
    // Diff-alone risk (excludes test/review failure penalties so signals are not double-counted).
    let mut risk: i32 = ((loc / 20) as i32).min(40) + ((risky_files.len() as i32) * 8).min(32) + 10;
    if migration_touched {
        risk += 10;
    }
    diff_score(
        files,
        additions,
        deletions,
        risky_files.len() as u32,
        risk.clamp(0, 100) as u8,
    )
}

/// Run a test/lint command (if configured) and map it to a 0/100 gate signal.
async fn run_cmd_signal(
    cwd: &str,
    cmd: Option<&str>,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<SignalScore> {
    match cmd {
        Some(c) if !c.trim().is_empty() => {
            let run =
                otto_automation::goal_loop_commands::run(cwd, c, CMD_TIMEOUT_SECS, cancel).await;
            let ok = run.success;
            Ok(signal_from_cmd(
                true,
                ok,
                format!("`{c}` → {}", if ok { "passed" } else { "failed" }),
            ))
        }
        _ => Ok(SignalScore::default()),
    }
}

/// Aggregate the iteration's validator findings into `(SignalScore, passed)`.
/// Returns `(default, true)` when the iteration has no validators.
pub(crate) fn review_signal(iter: &EvalIteration) -> (SignalScore, bool) {
    if iter.agents.is_empty() {
        return (SignalScore::default(), true);
    }
    let mut findings = Vec::new();
    for a in &iter.agents {
        findings.extend(a.findings.clone());
    }
    let (findings_passed, findings_score) = crate::skill_eval::score_findings(&findings);
    let complete = iter.agents.iter().all(|agent| agent.status == "done");
    let passed = complete && findings_passed;
    let score = if complete { findings_score } else { 0.0 };
    (
        signal_score(true, score, format!("{} finding(s)", findings.len())),
        passed,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
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
    async fn validation_retry_keeps_the_latest_persisted_human_rating() {
        let (_dir, ctx, _eval, iter) = fixture().await;
        let previous = EvalScore {
            human: human_score(Some(5), "before retry", "editor"),
            ..Default::default()
        };
        ctx.skill_evals_store
            .set_iter_scoring(&iter.id, &previous)
            .await
            .unwrap();
        // The human rating is changed while the validators are still running.
        ctx.skill_evals_store
            .set_iter_human(&iter.id, 1, "latest rating", "editor")
            .await
            .unwrap();
        let fresh = ctx.skill_evals_store.get_iteration(&iter.id).await.unwrap();
        let score = rescore_validation(&fresh, Some(previous));
        ctx.skill_evals_store
            .set_iter_scoring(&iter.id, &score)
            .await
            .unwrap();
        let stored = ctx.skill_evals_store.get_iteration(&iter.id).await.unwrap();
        assert_eq!(stored.human_rating, Some(1));
        let score = stored.scoring.unwrap();
        assert_eq!(score.human.rating, Some(1));
        assert_eq!(score.human.note, "latest rating");
        assert_eq!(score.composite, 20.0);
    }

    #[tokio::test]
    async fn errored_or_failed_retry_replaces_previously_passed_review_evidence() {
        let (_dir, _ctx, _eval, mut iter) = fixture().await;
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
        let (sig, passed) = review_signal(&iter);
        assert_eq!(sig.score, 100.0);
        assert!(passed);
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
            let score = rescore_validation(&iter, Some(previous));
            assert!(score.review.score < 100.0);
            assert_eq!(score.proof_status, "failed");
        }
    }
}
