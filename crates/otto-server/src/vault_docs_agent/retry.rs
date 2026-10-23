//! The docs-run retry endpoints: a user retry of one writer, the summarizer,
//! a reviewer or a revision. Each validates the target, kills a still-running
//! session and requests the retry on the run's `fanout::RetryQueue`; the
//! slot loop (`otto_agent_run::fanout::attempts`) re-runs only that slot.
//! Routed from [`super::routes`].

use super::*;

/// `POST /vault/docs-agents/runs/{run_id}/agents/{index}/retry` — LIVE-run
/// retry for one stuck/failed writer, exactly like a PR-review agent retry:
/// flag the slot, kill its current session (Ctrl+C grace, then kill) so the
/// in-flight turn errors out, and the orchestrator's turn loop re-spawns a
/// FRESH session with the same prompt. 409 once the run is terminal (history
/// runs re-run via a new run) or when the slot isn't still moving.
pub(super) async fn retry_agent(
    Path((run_id, index)): Path<(String, usize)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<axum::http::StatusCode> {
    retry_target(ctx, user, run_id, index).await
}

/// `POST /vault/docs-agents/runs/{run_id}/summarizer/retry` — same, for the
/// consolidation stage.
pub(super) async fn retry_summarizer(
    Path(run_id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<axum::http::StatusCode> {
    retry_target(
        ctx,
        user,
        run_id,
        otto_agent_run::fanout::SUMMARIZER_RETRY_IDX,
    )
    .await
}

pub(super) async fn retry_reviewer(
    Path((run_id, iteration, index)): Path<(String, u8, usize)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<axum::http::StatusCode> {
    retry_review_target(ctx, user, run_id, iteration, Some(index)).await
}

pub(super) async fn retry_revision(
    Path((run_id, iteration)): Path<(String, u8)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<axum::http::StatusCode> {
    retry_review_target(ctx, user, run_id, iteration, None).await
}

/// Validate one reviewer/revision retry without mutating the registry. The
/// handler uses this snapshot to finish any in-flight teardown before the
/// retry key can become visible to the orchestration loop.
pub(super) fn review_retry_target(
    entry: &RunEntry,
    iteration: u8,
    reviewer_index: Option<usize>,
) -> Result<(Option<String>, usize, String), ApiError> {
    if is_terminal(&entry.run.state) {
        return Err(ApiError(Error::Conflict(
            "run already finished — start a new run instead".into(),
        )));
    }
    if entry.run.review.current_iteration != iteration {
        return Err(ApiError(Error::Conflict(format!(
            "review round {iteration} is not active"
        ))));
    }
    let round = entry
        .run
        .review
        .rounds
        .last()
        .filter(|round| round.iteration == iteration)
        .ok_or_else(|| ApiError(Error::NotFound(format!("review round {iteration}"))))?;
    let (sid, key, state) = match reviewer_index {
        Some(index) => {
            if entry.run.state != "reviewing" {
                return Err(ApiError(Error::Conflict(
                    "reviewers are not running".into(),
                )));
            }
            let reviewer = round
                .reviewers
                .get(index)
                .ok_or_else(|| ApiError(Error::NotFound(format!("reviewer {index}"))))?;
            if !matches!(reviewer.state.as_str(), "pending" | "running" | "error") {
                return Err(ApiError(Error::Conflict(
                    "reviewer is not active or retryable".into(),
                )));
            }
            (
                reviewer.session_id.clone(),
                reviewer_retry_key(iteration, index),
                reviewer.state.clone(),
            )
        }
        None => {
            if entry.run.state != "revising"
                || !matches!(
                    round.revision.state.as_str(),
                    "pending" | "running" | "error"
                )
            {
                return Err(ApiError(Error::Conflict(
                    "revision is not active or retryable".into(),
                )));
            }
            (
                round.revision.session_id.clone(),
                revision_retry_key(iteration),
                round.revision.state.clone(),
            )
        }
    };
    if entry.retries.lock().unwrap().is_closed(key) {
        return Err(retry_limit_reached());
    }
    Ok((sid, key, state))
}

pub(super) fn retry_limit_reached() -> ApiError {
    ApiError(Error::Conflict(
        "retry limit reached — start a new run".into(),
    ))
}

pub(super) fn activate_review_retry(
    entry: &mut RunEntry,
    iteration: u8,
    reviewer_index: Option<usize>,
) -> Result<(Option<String>, usize), ApiError> {
    let (sid, key, _state) = review_retry_target(entry, iteration, reviewer_index)?;
    entry
        .retries
        .lock()
        .unwrap()
        .request(key)
        .map_err(|_| retry_limit_reached())?;
    if let Some(index) = reviewer_index {
        reset_reviewer_for_retry(&mut entry.run, iteration, index);
    } else {
        reset_revision_for_retry(&mut entry.run, iteration);
    }
    Ok((sid, key))
}

pub(super) async fn retry_review_target(
    ctx: ServerCtx,
    user: User,
    run_id: String,
    iteration: u8,
    reviewer_index: Option<usize>,
) -> ApiResult<axum::http::StatusCode> {
    let ws_id = {
        let reg = ctx.vault_docs_runs.lock().unwrap();
        let entry = reg
            .get(&run_id)
            .ok_or_else(|| ApiError(Error::NotFound(format!("docs run {run_id}"))))?;
        entry.run.ws_id.clone()
    };
    crate::auth::require_ws_role(&ctx, &user, &Id::from(ws_id), WorkspaceRole::Editor).await?;
    // Snapshot the current target before changing any visible state. A running
    // turn must be completely torn down before its retry key becomes visible;
    // otherwise the orchestrator can resume the same final-author id while a
    // detached grace-kill is still pending and that old kill murders the new
    // attempt. A failed turn is already idle, so preserve its conversation and
    // submit the corrective prompt without killing it.
    let (sid, _retry_key, state) = {
        let reg = ctx.vault_docs_runs.lock().unwrap();
        let entry = reg
            .get(&run_id)
            .ok_or_else(|| ApiError(Error::NotFound(format!("docs run {run_id}"))))?;
        review_retry_target(entry, iteration, reviewer_index)?
    };
    if retry_needs_termination(&state) {
        if let Some(sid) = sid {
            terminate_session(ctx.manager.clone(), Id::from(sid)).await;
        }
    }
    let (_sid, _retry_key) = {
        let mut reg = ctx.vault_docs_runs.lock().unwrap();
        let entry = reg
            .get_mut(&run_id)
            .ok_or_else(|| ApiError(Error::NotFound(format!("docs run {run_id}"))))?;
        activate_review_retry(entry, iteration, reviewer_index)?
    };
    persist(&ctx.vault_docs_runs, &run_id);
    Ok(axum::http::StatusCode::ACCEPTED)
}

pub(super) fn retry_needs_termination(state: &str) -> bool {
    matches!(state, "pending" | "running")
}

pub(super) async fn terminate_session(manager: Arc<otto_sessions::SessionManager>, sid: Id) {
    let _ = manager.input(&sid, b"\x03").await;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let _ = manager.input(&sid, b"\x03").await;
    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
    let _ = manager.kill_session(&sid).await;
}

/// True while some OTHER writer slot is still moving — the window in which an
/// errored slot keeps listening for a user retry. Once every peer is terminal
/// the writers barrier must not hang on a slot nobody can un-stick.
pub(super) fn writers_stage_open(reg: &RunRegistry, run_id: &str, index: usize) -> bool {
    reg.lock().unwrap().get(run_id).is_some_and(|e| {
        e.run
            .agents
            .iter()
            .any(|a| a.index != index && matches!(a.state.as_str(), "running" | "pending"))
    })
}

pub(super) fn author_retry_target(
    entry: &RunEntry,
    index: usize,
) -> Result<(Option<String>, bool), ApiError> {
    if entry.retries.lock().unwrap().is_closed(index) {
        return Err(retry_limit_reached());
    }
    if is_terminal(&entry.run.state) {
        return Err(ApiError(Error::Conflict(
            "run already finished — start a new run instead".into(),
        )));
    }
    let (sid, needs_kill) = if index == otto_agent_run::fanout::SUMMARIZER_RETRY_IDX {
        if entry.run.state != "summarizing"
            || !matches!(entry.run.summarizer.state.as_str(), "running" | "pending")
        {
            return Err(ApiError(Error::Conflict(
                "summarizer is not running".into(),
            )));
        }
        (entry.run.summarizer.session_id.clone(), true)
    } else {
        let a = entry
            .run
            .agents
            .get(index)
            .ok_or_else(|| ApiError(Error::NotFound(format!("writer {index}"))))?;
        // An ERRORED writer stays retryable while the writers stage is
        // still open (its turn loop keeps listening for the flag as long
        // as any peer is moving). Its session is already dead — no kill.
        match a.state.as_str() {
            "running" | "pending" => (a.session_id.clone(), true),
            "error" if entry.run.state == "running" => (None, false),
            _ => {
                return Err(ApiError(Error::Conflict(
                    "writer is not retryable — cancel and start a new run".into(),
                )));
            }
        }
    };
    Ok((sid, needs_kill))
}

/// Validate a writer/summarizer retry, request it on the run's queue and
/// reset the slot's row to "pending". Returns the session to kill, if any.
pub(super) fn activate_author_retry(
    entry: &mut RunEntry,
    index: usize,
) -> Result<(Option<String>, bool), ApiError> {
    let target = author_retry_target(entry, index)?;
    entry
        .retries
        .lock()
        .unwrap()
        .request(index)
        .map_err(|_| retry_limit_reached())?;
    if index == otto_agent_run::fanout::SUMMARIZER_RETRY_IDX {
        entry.run.summarizer.state = "pending".into();
        entry.run.summarizer.error = None;
    } else if let Some(a) = entry.run.agents.get_mut(index) {
        a.state = "pending".into();
        a.error = None;
    }
    Ok(target)
}

pub(super) async fn retry_target(
    ctx: ServerCtx,
    user: User,
    run_id: String,
    index: usize,
) -> ApiResult<axum::http::StatusCode> {
    let ws_id = {
        let reg = ctx.vault_docs_runs.lock().unwrap();
        let e = reg
            .get(&run_id)
            .ok_or_else(|| ApiError(Error::NotFound(format!("docs run {run_id}"))))?;
        e.run.ws_id.clone()
    };
    crate::auth::require_ws_role(&ctx, &user, &Id::from(ws_id), WorkspaceRole::Editor).await?;
    let (sid, needs_kill) = {
        let mut reg = ctx.vault_docs_runs.lock().unwrap();
        let e = reg
            .get_mut(&run_id)
            .ok_or_else(|| ApiError(Error::NotFound(format!("docs run {run_id}"))))?;
        activate_author_retry(e, index)?
    };
    persist(&ctx.vault_docs_runs, &run_id);
    // Kill the current session so the in-flight turn errors and the loop
    // consumes the retry flag. Same interrupt-then-kill dance as cancel.
    // (Skipped for an errored slot — its session already died.)
    if let Some(sid) = sid.filter(|_| needs_kill) {
        tokio::spawn(terminate_session(ctx.manager.clone(), Id::from(sid)));
    }
    Ok(axum::http::StatusCode::ACCEPTED)
}
