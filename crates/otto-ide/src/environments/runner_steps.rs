//! The steps a run takes before its program starts — `before_launch` and the
//! Go / Java compile — as stoppable, tracked helpers.
//!
//! Each step races the run's Stop signal: Stop drops the step, and dropping it
//! kills the step's whole process group ([`crate::proc_group::HelperGroup`]),
//! so a `go build`, a hung `mvn install` or a `docker compose up` never keeps
//! running behind a console that already says "Stopped". A local `before_launch`
//! streams its output into the run console (bounded, lossy lines) instead of
//! buffering it. When a hook's shell exits, anything it left running in its
//! group is killed, the same as for the program itself: a pgid held after its
//! leader is gone could be reused by an unrelated process before the run ends.

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;

use super::runner::{append_run_line, ActiveRunHandle};
use super::runner_proc::spawn_local_before_launch;
use crate::line_reader::{spawn_line_drain, MAX_OUTPUT_LINE_BYTES};
use crate::proc_group::{HelperGroup, BUILD_STEP_TIMEOUT};

/// Backstop for a local `before_launch` hook that never exits.
pub(super) const BEFORE_LAUNCH_TIMEOUT: Duration = BUILD_STEP_TIMEOUT;

pub(super) enum StepOutcome<T> {
    Done(T),
    Failed(String),
    Stopped,
}

/// Resolves once the run has been asked to stop (never, if the sender is gone).
pub(super) async fn stop_requested(stop_rx: &mut watch::Receiver<bool>) {
    if stop_rx.wait_for(|stopped| *stopped).await.is_err() {
        std::future::pending::<()>().await;
    }
}

/// Run `step` unless Stop arrives first; on Stop the step future is dropped,
/// which kills any helper group it owns.
pub(super) async fn unless_stopped<T>(
    stop_rx: &mut watch::Receiver<bool>,
    step: impl Future<Output = T>,
) -> Option<T> {
    if *stop_rx.borrow() {
        return None;
    }
    tokio::select! {
        v = step => Some(v),
        _ = stop_requested(stop_rx) => None,
    }
}

/// Run a local `before_launch` hook with its output streamed into the console.
pub(super) async fn run_local_before_launch(
    handle: &Arc<ActiveRunHandle>,
    script: &str,
    cwd: &str,
    env_map: &BTreeMap<String, String>,
    stop_rx: &mut watch::Receiver<bool>,
    limit: Duration,
) -> StepOutcome<()> {
    let mut child = match spawn_local_before_launch(script, cwd, env_map) {
        Ok(c) => c,
        Err(e) => return StepOutcome::Failed(e),
    };
    let mut group = HelperGroup::track(child.id());
    let mut readers = Vec::with_capacity(2);
    if let Some(out) = child.stdout.take() {
        let h = Arc::clone(handle);
        readers.push(spawn_line_drain(out, MAX_OUTPUT_LINE_BYTES, move |l| {
            append_run_line(&h, l)
        }));
    }
    if let Some(err) = child.stderr.take() {
        let h = Arc::clone(handle);
        readers.push(spawn_line_drain(err, MAX_OUTPUT_LINE_BYTES, move |l| {
            append_run_line(&h, l)
        }));
    }
    let waited = tokio::select! {
        r = tokio::time::timeout(limit, child.wait()) => r,
        _ = stop_requested(stop_rx) => {
            group.kill();
            let _ = child.start_kill();
            return StepOutcome::Stopped;
        }
    };
    // Stragglers die with the leader so the pipes close and the readers finish;
    // their last lines reach the console before the next step's header.
    group.kill();
    for r in readers {
        let _ = tokio::time::timeout(Duration::from_millis(250), r).await;
    }
    match waited {
        Err(_) => {
            let _ = child.start_kill();
            StepOutcome::Failed(format!(
                "before_launch timed out after {}s",
                limit.as_secs()
            ))
        }
        Ok(Err(e)) => StepOutcome::Failed(format!("before_launch: {e}")),
        Ok(Ok(status)) if !status.success() => StepOutcome::Failed(match status.code() {
            Some(code) => format!("before_launch failed (exit {code})"),
            None => "before_launch failed (killed by a signal)".to_string(),
        }),
        Ok(Ok(_)) => StepOutcome::Done(()),
    }
}
