//! Execute loop operations through durable before/after records. A successful
//! record is authoritative even when the daemon died before the parent advanced.

use otto_core::workflows::{NodeStatus, RetryPolicy, WorkflowCheckpoint};
use otto_core::{Error, Id, Result};
use otto_state::WorkflowsRepo;
use serde_json::Value;
use std::future::Future;

pub(crate) async fn execute<F, Fut>(
    repo: &WorkflowsRepo,
    run_id: &Id,
    mut checkpoint: WorkflowCheckpoint,
    policy: &RetryPolicy,
    restart_safe: bool,
    mut action: F,
) -> Result<(Value, Vec<String>)>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<(Value, Vec<String>)>>,
{
    if let Some(previous) = repo.checkpoint(run_id, &checkpoint.node_id).await? {
        checkpoint = previous;
        if checkpoint.status == NodeStatus::Success {
            return Ok((
                checkpoint.output.unwrap_or(Value::Null),
                vec![format!(
                    "✓ adopted checkpoint: {} (attempt {})",
                    checkpoint.name, checkpoint.attempts
                )],
            ));
        }
        if checkpoint.status == NodeStatus::Running && !restart_safe {
            return Err(Error::Conflict(format!("{} was interrupted with an unknown outcome; inspect it and explicitly retry the failed step", checkpoint.name)));
        }
        if checkpoint.status == NodeStatus::Error
            && crate::workflow_engine::retry_backoff(
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
    loop {
        checkpoint.status = NodeStatus::Running;
        checkpoint.attempts += 1;
        checkpoint.updated_at = chrono::Utc::now();
        repo.save_checkpoint(run_id, &checkpoint).await?;
        match action().await {
            Ok((output, mut logs)) => {
                checkpoint.status = NodeStatus::Success;
                checkpoint.output = Some(output.clone());
                checkpoint.error = None;
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
                return Ok((output, logs));
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
                let backoff = (policy.backoff_ms as f64
                    * policy
                        .factor
                        .powi(checkpoint.attempts.saturating_sub(1) as i32))
                .min(60_000.0) as u64;
                let Some((delay, _, reason)) = crate::workflow_engine::retry_backoff(
                    &error.to_string(),
                    &policy,
                    checkpoint.attempts,
                    backoff,
                    0,
                ) else {
                    return Err(error);
                };
                checkpoint
                    .logs
                    .push(format!("Retry in {}ms: {reason}", delay));
                repo.save_checkpoint(run_id, &checkpoint).await?;
                tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
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
}
