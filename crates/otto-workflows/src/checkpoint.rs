//! Execute loop operations through durable before/after records. A successful
//! record is authoritative even when the daemon died before the parent advanced.

use otto_core::cancel_signal::CancelSignal;
use otto_core::workflows::{NodeStatus, RetryPolicy, WorkflowCheckpoint};
use otto_core::{Error, Id, Result};
use otto_state::WorkflowsRepo;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::future::Future;
use std::time::Duration;

/// Upper bound (inclusive) for per-retry jitter passed to `retry_backoff` (ms).
const MAX_RETRY_JITTER_MS: u64 = 5_000;
/// Backstop DB re-check interval while sleeping in a retry backoff.
const CANCEL_POLL_SLICE: Duration = Duration::from_secs(2);

/// Bounded, strictly non-zero jitter in `1..=MAX_RETRY_JITTER_MS` ms derived
/// from the run/node/attempt tuple plus clock nanos so simultaneous retries
/// spread out instead of stampeding at the exact same millisecond.
pub fn retry_jitter_ms(run_id: &str, node_id: &str, attempt: u32) -> u64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let mut h = Sha256::new();
    h.update(run_id.as_bytes());
    h.update(b":");
    h.update(node_id.as_bytes());
    h.update(b":");
    h.update(attempt.to_le_bytes());
    h.update(nanos.to_le_bytes());
    let digest = h.finalize();
    let sample = u64::from_le_bytes(digest[..8].try_into().unwrap_or([0; 8]));
    1 + (sample % MAX_RETRY_JITTER_MS)
}

async fn is_run_canceled(repo: &WorkflowsRepo, run_id: &Id, cancel: Option<&CancelSignal>) -> bool {
    cancel.is_some_and(CancelSignal::is_cancelled) || repo.is_canceled(run_id).await
}

/// Sleep `delay` unless canceled via `cancel` or the run's cancel broadcast /
/// DB status. Returns `true` when canceled so the caller aborts immediately.
async fn sleep_or_canceled(
    repo: &WorkflowsRepo,
    run_id: &Id,
    delay: Duration,
    cancel: Option<&CancelSignal>,
) -> bool {
    // Subscribe BEFORE the DB check so a cancel broadcast racing right after
    // `is_canceled` is never missed.
    let mut cancels = otto_state::workflows::subscribe_cancels();
    let deadline = tokio::time::Instant::now() + delay;
    loop {
        if is_run_canceled(repo, run_id, cancel).await {
            return true;
        }
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            return false;
        }
        let slice = left.min(CANCEL_POLL_SLICE);
        let cancel_fut = async {
            match cancel {
                Some(sig) => sig.cancelled().await,
                None => std::future::pending::<()>().await,
            }
        };
        tokio::select! {
            biased;
            () = cancel_fut => return true,
            msg = cancels.recv() => {
                match msg {
                    Ok(id) if id == *run_id => return true,
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {}
                }
            }
            () = tokio::time::sleep(slice) => {}
        }
    }
}

/// Durable execution result; adoption must not rewrite an earlier handoff.
#[derive(Debug)]
pub struct CheckpointOutcome {
    pub output: Value,
    pub logs: Vec<String>,
    pub adopted: bool,
}

pub async fn execute<F, Fut>(
    repo: &WorkflowsRepo,
    run_id: &Id,
    checkpoint: WorkflowCheckpoint,
    policy: &RetryPolicy,
    restart_safe: bool,
    action: F,
) -> Result<(Value, Vec<String>)>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<(Value, Vec<String>)>>,
{
    execute_observed(
        repo,
        run_id,
        checkpoint,
        policy,
        restart_safe,
        action,
        |_| {},
    )
    .await
    .map(|outcome| (outcome.output, outcome.logs))
}

/// As `execute`, publishing each attempt and retry before starting more work.
pub async fn execute_observed<F, Fut, O>(
    repo: &WorkflowsRepo,
    run_id: &Id,
    checkpoint: WorkflowCheckpoint,
    policy: &RetryPolicy,
    restart_safe: bool,
    action: F,
    observe: O,
) -> Result<CheckpointOutcome>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<(Value, Vec<String>)>>,
    O: FnMut(String),
{
    execute_with_cancel(
        repo,
        run_id,
        checkpoint,
        policy,
        restart_safe,
        action,
        observe,
        None,
    )
    .await
}

/// As [`execute_observed`], also waking promptly on an explicit [`CancelSignal`]
/// (in addition to `repo.is_canceled(run_id)` / `subscribe_cancels`).
#[allow(clippy::too_many_arguments)]
pub async fn execute_with_cancel<F, Fut, O>(
    repo: &WorkflowsRepo,
    run_id: &Id,
    mut checkpoint: WorkflowCheckpoint,
    policy: &RetryPolicy,
    restart_safe: bool,
    mut action: F,
    mut observe: O,
    cancel: Option<&CancelSignal>,
) -> Result<CheckpointOutcome>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<(Value, Vec<String>)>>,
    O: FnMut(String),
{
    if let Some(previous) = repo.checkpoint(run_id, &checkpoint.node_id).await? {
        checkpoint = previous;
        if checkpoint.status == NodeStatus::Success
            || (checkpoint.status == NodeStatus::Error
                && checkpoint
                    .output
                    .as_ref()
                    .and_then(crate::retry::outcome_error)
                    .is_some())
        {
            return Ok(CheckpointOutcome {
                output: checkpoint.output.unwrap_or(Value::Null),
                logs: vec![format!(
                    "✓ adopted checkpoint: {} (attempt {})",
                    checkpoint.name, checkpoint.attempts
                )],
                adopted: true,
            });
        }
        if checkpoint.status == NodeStatus::Running && !restart_safe {
            return Err(Error::Conflict(format!("{} was interrupted with an unknown outcome; inspect it and explicitly retry the failed step", checkpoint.name)));
        }
        if checkpoint.status == NodeStatus::Error
            && crate::retry::retry_backoff(
                checkpoint.error.as_deref().unwrap_or("step failed"),
                policy,
                checkpoint.attempts,
                policy.backoff_ms,
                0,
            )
            .is_none()
        {
            return Err(Error::Upstream(
                checkpoint
                    .error
                    .unwrap_or_else(|| "loop retry budget exhausted".into()),
            ));
        }
    }
    let policy = policy.clamped();
    let mut attempt_limit = policy.max_attempts + 1;
    if checkpoint
        .error
        .as_deref()
        .and_then(crate::retry::retry_class)
        .is_some()
        && policy.max_attempts > 0
    {
        attempt_limit = attempt_limit.max(5);
    }
    loop {
        if is_run_canceled(repo, run_id, cancel).await {
            return Err(Error::Internal("run canceled".into()));
        }
        checkpoint.status = NodeStatus::Running;
        checkpoint.output = None;
        checkpoint.error = None;
        checkpoint.attempts += 1;
        checkpoint.updated_at = chrono::Utc::now();
        repo.save_checkpoint(run_id, &checkpoint).await?;
        observe(format!(
            "{} — attempt {}/{} (iteration {})",
            checkpoint.name, checkpoint.attempts, attempt_limit, checkpoint.iteration
        ));
        match action().await {
            Ok((output, mut logs)) => {
                checkpoint.error = crate::retry::outcome_error(&output).map(str::to_owned);
                checkpoint.status = if checkpoint.error.is_some() {
                    NodeStatus::Error
                } else {
                    NodeStatus::Success
                };
                checkpoint.output = Some(output.clone());
                logs.push(format!(
                    "✓ checkpoint saved after attempt {}",
                    checkpoint.attempts
                ));
                checkpoint.logs.extend(logs);
                logs = checkpoint.logs.clone();
                checkpoint.updated_at = chrono::Utc::now();
                // Failure to record success is an unknown outcome. Do not
                // execute the operation again in this process.
                repo.save_checkpoint(run_id, &checkpoint).await?;
                return Ok(CheckpointOutcome {
                    output,
                    logs,
                    adopted: false,
                });
            }
            Err(error) => {
                checkpoint.status = NodeStatus::Error;
                checkpoint.error = Some(error.to_string());
                checkpoint
                    .logs
                    .push(format!("attempt {}: {error}", checkpoint.attempts));
                checkpoint.updated_at = chrono::Utc::now();
                repo.save_checkpoint(run_id, &checkpoint).await?;
                if matches!(
                    checkpoint.kind.as_str(),
                    "human_approval" | "manual_trigger"
                ) {
                    return Err(error);
                }
                if is_run_canceled(repo, run_id, cancel).await {
                    return Err(Error::Internal("run canceled".into()));
                }
                let backoff = (policy.backoff_ms as f64
                    * policy
                        .factor
                        .powi(checkpoint.attempts.saturating_sub(1) as i32))
                .min(60_000.0) as u64;
                let jitter = retry_jitter_ms(run_id, &checkpoint.node_id, checkpoint.attempts);
                let Some((delay, max_eff, reason)) = crate::retry::retry_backoff(
                    &error.to_string(),
                    &policy,
                    checkpoint.attempts,
                    backoff,
                    jitter,
                ) else {
                    return Err(error);
                };
                attempt_limit = max_eff + 1;
                let retry = format!(
                    "{} — ↻ retry {}/{} in {}ms (iteration {}; {reason})",
                    checkpoint.name,
                    checkpoint.attempts + 1,
                    max_eff + 1,
                    delay,
                    checkpoint.iteration
                );
                checkpoint.logs.push(retry.clone());
                repo.save_checkpoint(run_id, &checkpoint).await?;
                observe(retry);
                if sleep_or_canceled(repo, run_id, Duration::from_millis(delay), cancel).await {
                    return Err(Error::Internal("run canceled".into()));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    async fn fixture() -> (tempfile::TempDir, WorkflowsRepo, Id, WorkflowCheckpoint) {
        let dir = tempfile::tempdir().unwrap();
        // Use the daemon's bootstrap so checkpoint tests include every current
        // migration, projection trigger and foreign-key dependency.
        let pool = otto_state::open(&dir.path().join("state.sqlite"))
            .await
            .unwrap();
        let user = otto_state::UsersRepo::new(pool.clone())
            .create("checkpoint-fixture", "", "Checkpoint fixture", false)
            .await
            .unwrap();
        let workspace = otto_state::WorkspacesRepo::new(pool.clone())
            .create("Checkpoint fixture", dir.path().to_str().unwrap(), &user.id)
            .await
            .unwrap();
        let repo = WorkflowsRepo::new(pool);
        let workflow = repo
            .create(
                &workspace.id,
                "Checkpoint fixture",
                "",
                "",
                &Default::default(),
                &user.id,
            )
            .await
            .unwrap();
        let run = repo
            .create_run(&workflow.id, &workspace.id, &json!({}), Some(&user.id))
            .await
            .unwrap();
        // Explicit retry is only admitted for a settled run, as in production.
        repo.update_run(
            &run.id,
            otto_core::workflows::RunStatus::Error,
            &[],
            None,
            true,
        )
        .await
        .unwrap();
        let cp = WorkflowCheckpoint {
            node_id: "loop#1.0".into(),
            loop_id: "loop".into(),
            iteration: 1,
            step_index: 0,
            kind: "http_request".into(),
            name: "record mutation".into(),
            status: NodeStatus::Pending,
            attempts: 0,
            input: json!({}),
            output: None,
            error: None,
            logs: vec![],
            updated_at: chrono::Utc::now(),
        };
        (dir, repo, run.id, cp)
    }

    #[tokio::test]
    async fn semantic_gate_failure_is_durable_and_never_reexecuted() {
        let (_dir, repo, run_id, mut cp) = fixture().await;
        cp.kind = "review_run".into();
        let calls = AtomicUsize::new(0);
        let policy = RetryPolicy {
            max_attempts: 3,
            backoff_ms: 1,
            factor: 1.0,
        };
        for _ in 0..2 {
            let (out, _) = execute(&repo, &run_id, cp.clone(), &policy, true, || async {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok((
                    json!({"review_id":"finished-review", "passed":false,
                    "_workflow_error":"review did not meet required coverage"}),
                    vec![],
                ))
            })
            .await
            .unwrap();
            assert_eq!(out["review_id"], "finished-review");
        }
        let saved = repo
            .checkpoint(&run_id, &cp.node_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved.status, NodeStatus::Error);
        assert_eq!(saved.attempts, 1);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            saved.error.as_deref(),
            Some("review did not meet required coverage")
        );
    }

    #[tokio::test]
    async fn completed_external_step_is_not_executed_after_reentry() {
        let (_dir, repo, run_id, cp) = fixture().await;
        let calls = AtomicUsize::new(0);
        for _ in 0..2 {
            let (out, _) = execute(
                &repo,
                &run_id,
                cp.clone(),
                &RetryPolicy::default(),
                false,
                || async {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok((json!({"id":42}), vec![]))
                },
            )
            .await
            .unwrap();
            assert_eq!(out["id"], 42);
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn unknown_external_outcome_stops_until_operator_retry() {
        let (_dir, repo, run_id, mut cp) = fixture().await;
        cp.status = NodeStatus::Running;
        cp.attempts = 1;
        repo.save_checkpoint(&run_id, &cp).await.unwrap();
        let result = execute(
            &repo,
            &run_id,
            cp.clone(),
            &RetryPolicy::default(),
            false,
            || async { panic!("must not repeat external action") },
        )
        .await;
        assert!(matches!(result, Err(Error::Conflict(_))));
        repo.prepare_retry(&run_id, &["loop".into()], false, &Default::default())
            .await
            .unwrap();
        execute(
            &repo,
            &run_id,
            cp,
            &RetryPolicy::default(),
            false,
            || async { Ok((json!("checked and retried"), vec![])) },
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn inner_retry_budget_and_attempts_are_durable() {
        let (_dir, repo, run_id, cp) = fixture().await;
        let calls = AtomicUsize::new(0);
        let policy = RetryPolicy {
            max_attempts: 1,
            ..Default::default()
        };
        execute(&repo, &run_id, cp, &policy, true, || async {
            if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                Err(Error::Upstream("transient".into()))
            } else {
                Ok((json!("done"), vec![]))
            }
        })
        .await
        .unwrap();
        let saved = repo.checkpoints(&run_id).await.unwrap();
        assert_eq!(saved[0].attempts, 2);
        assert_eq!(saved[0].status, NodeStatus::Success);
    }

    #[test]
    fn retry_jitter_is_nonzero_and_bounded() {
        let mut seen = std::collections::HashSet::new();
        for attempt in 1..=20 {
            let j = retry_jitter_ms("run-1", "loop#1.0", attempt);
            assert!(
                (1..=MAX_RETRY_JITTER_MS).contains(&j),
                "jitter out of bounds: {j}"
            );
            seen.insert(j);
        }
        assert!(seen.len() > 1, "jitter should vary across attempts");
    }

    #[tokio::test]
    async fn retry_backoff_applies_nonzero_jitter_and_aborts_promptly_on_cancel_signal() {
        let (_dir, repo, run_id, cp) = fixture().await;
        let calls = AtomicUsize::new(0);
        let cancel = CancelSignal::new();
        let cancel_trigger = cancel.clone();
        let observed = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let obs_clone = observed.clone();
        let policy = RetryPolicy {
            max_attempts: 2,
            backoff_ms: 1_000,
            factor: 2.0,
        };
        let started = std::time::Instant::now();
        let result = execute_with_cancel(
            &repo,
            &run_id,
            cp,
            &policy,
            true,
            || {
                let calls = &calls;
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Err(Error::Upstream("HTTP 529 overloaded".into()))
                }
            },
            move |line| {
                obs_clone.lock().unwrap().push(line.clone());
                if line.contains("↻ retry") {
                    // Cancel immediately once the retry sleep is about to begin.
                    let c = cancel_trigger.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(Duration::from_millis(25)).await;
                        c.cancel();
                    });
                }
            },
            Some(&cancel),
        )
        .await;
        let elapsed = started.elapsed();
        assert!(
            matches!(result, Err(Error::Internal(ref m)) if m.contains("canceled")),
            "expected cancel error, got {result:?}"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(
            elapsed < Duration::from_secs(2),
            "cancel should abort the 20s+ backoff promptly, took {elapsed:?}"
        );
        let retry_line = observed
            .lock()
            .unwrap()
            .iter()
            .find(|l| l.contains("↻ retry"))
            .cloned()
            .expect("retry log line emitted");
        // Parse `in <delay>ms` and verify non-zero jitter pushed it strictly above 20_000ms.
        let delay_ms: u64 = retry_line
            .split(" in ")
            .nth(1)
            .and_then(|s| s.split("ms").next())
            .and_then(|s| s.parse().ok())
            .expect("delay ms in retry line");
        assert!(
            delay_ms > 20_000 && delay_ms <= 20_000 + MAX_RETRY_JITTER_MS,
            "expected 20_000 < delay <= {}, got {delay_ms} ({retry_line})",
            20_000 + MAX_RETRY_JITTER_MS
        );
    }

    #[tokio::test]
    async fn retry_backoff_aborts_promptly_on_repo_request_cancel() {
        let (_dir, repo, run_id, cp) = fixture().await;
        repo.reopen_run(&run_id).await.unwrap();
        let calls = AtomicUsize::new(0);
        let policy = RetryPolicy {
            max_attempts: 2,
            backoff_ms: 15_000,
            factor: 1.0,
        };
        let repo_cancel = repo.clone();
        let run_cancel = run_id.clone();
        let started = std::time::Instant::now();
        let result = execute_observed(
            &repo,
            &run_id,
            cp,
            &policy,
            true,
            || {
                let calls = &calls;
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Err(Error::Upstream("transient failure".into()))
                }
            },
            move |line| {
                if line.contains("↻ retry") {
                    let r = repo_cancel.clone();
                    let id = run_cancel.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(Duration::from_millis(25)).await;
                        r.request_cancel(&id).await.unwrap();
                    });
                }
            },
        )
        .await;
        assert!(
            matches!(result, Err(Error::Internal(ref m)) if m.contains("canceled")),
            "expected cancel error, got {result:?}"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "repo cancel should wake backoff immediately, took {:?}",
            started.elapsed()
        );
    }
}
