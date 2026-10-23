//! Run ONE agent turn against a managed Otto **session**, creating it on first
//! use and RESUMING it across turns. The runner itself lives in
//! [`otto_agent_run::session_turn`] (architecture-5-02): it needs only the
//! [`otto_sessions::SessionManager`], so engines outside the server can drive
//! turns too. This module keeps the `&ServerCtx` entry points every server
//! caller uses, mapping the runner's `otto_core::Error` to [`ApiError`], plus
//! the resume-ownership check that needs the state DB.

use std::sync::Arc;
use std::time::Duration;

use otto_core::domain::{User, Workspace};
use otto_core::{Error, Id};
use serde_json::Value;

use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;

pub use otto_agent_run::session_turn::{submit_confirm_for, TurnOpts, STUCK_IDLE};

/// Shared artifact visibility does not grant authority to resume its owner's
/// agent. SessionManager's live lookup also reads this authoritative DB row.
/// Only R12's studio adapters opt into this check; missing old sessions retain
/// the runner's existing fresh-session fallback.
pub(crate) async fn require_owned_resume(
    pool: &otto_state::DbPool,
    workspace: &str,
    user: &str,
    existing: Option<&Id>,
) -> otto_core::Result<()> {
    let Some(id) = existing else {
        return Ok(());
    };
    match otto_state::SessionsRepo::new(pool.clone()).get(id).await {
        Ok(session) if session.workspace_id == workspace && session.created_by == user => Ok(()),
        Ok(_) => Err(Error::Forbidden(
            "cannot resume an assistant session owned by another user or workspace".into(),
        )),
        Err(Error::NotFound(_)) => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
#[path = "../tests/unit/assist_resume_ownership.rs"]
mod resume_ownership_tests;

/// Run one turn. Returns `(reply_text, session_id)`. Persist the returned
/// `session_id` so the next turn resumes the SAME session.
/// See [`otto_agent_run::session_turn::run_session_turn`].
#[allow(clippy::too_many_arguments)]
pub async fn run_session_turn(
    ctx: &ServerCtx,
    ws: &Workspace,
    user: &User,
    existing: Option<&Id>,
    title: &str,
    cwd: &str,
    provider: &str,
    meta: Value,
    prompt: &str,
    stuck_after: Duration,
    on_ready: impl FnOnce(&Id),
) -> ApiResult<(String, Id)> {
    otto_agent_run::session_turn::run_session_turn(
        &ctx.manager,
        ws,
        user,
        existing,
        title,
        cwd,
        provider,
        meta,
        prompt,
        stuck_after,
        on_ready,
    )
    .await
    .map_err(ApiError)
}

/// The plain-chat reply protocol with caller-owned cancellation.
/// See [`otto_agent_run::session_turn::run_session_turn_cancellable`].
#[allow(clippy::too_many_arguments)]
pub async fn run_session_turn_cancellable(
    ctx: &ServerCtx,
    ws: &Workspace,
    user: &User,
    existing: Option<&Id>,
    title: &str,
    cwd: &str,
    provider: &str,
    meta: Value,
    prompt: &str,
    stuck_after: Duration,
    cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    on_ready: impl FnOnce(&Id),
) -> ApiResult<(String, Id)> {
    otto_agent_run::session_turn::run_session_turn_cancellable(
        &ctx.manager,
        ws,
        user,
        existing,
        title,
        cwd,
        provider,
        meta,
        prompt,
        stuck_after,
        cancel,
        on_ready,
    )
    .await
    .map_err(ApiError)
}

/// [`run_session_turn`] with extra completion channels (see [`TurnOpts`]).
#[allow(clippy::too_many_arguments)]
pub async fn run_session_turn_with(
    ctx: &ServerCtx,
    ws: &Workspace,
    user: &User,
    existing: Option<&Id>,
    title: &str,
    cwd: &str,
    provider: &str,
    meta: Value,
    prompt: &str,
    stuck_after: Duration,
    opts: TurnOpts,
    on_ready: impl FnOnce(&Id),
) -> ApiResult<(String, Id)> {
    otto_agent_run::session_turn::run_session_turn_with(
        &ctx.manager,
        ws,
        user,
        existing,
        title,
        cwd,
        provider,
        meta,
        prompt,
        stuck_after,
        opts,
        on_ready,
    )
    .await
    .map_err(ApiError)
}

#[cfg(test)]
mod tests {
    use super::*;
    use otto_agent_run::session_turn::SESSION_LEASES;
    use otto_core::api::CreateSessionReq;

    async fn live_shell(
        ctx: &crate::state::ServerCtx,
        ws: &Workspace,
        user: &Id,
        dir: &std::path::Path,
    ) -> Id {
        let req = CreateSessionReq {
            kind: otto_core::domain::SessionKind::Agent,
            provider: Some("shell".into()),
            title: Some("thread".into()),
            cwd: None,
            connection_id: None,
            model: None,
            meta: None,
        };
        let spec = otto_pty::CommandSpec {
            program: "/bin/sh".into(),
            args: vec![],
            cwd: Some(dir.to_string_lossy().into()),
            env: vec![],
        };
        ctx.manager
            .create(ws, user, req, Some(spec))
            .await
            .unwrap()
            .id
    }

    #[tokio::test]
    async fn product_studio_stop_and_story_delete_drain_the_owned_session() {
        use otto_product::ProductStudioHost;
        for delete_story in [false, true] {
            let tmp = tempfile::tempdir().unwrap();
            let pool = crate::test_support::mem_pool().await;
            let ctx = crate::state::ServerCtx::for_tests(&pool, tmp.path().join("data")).await;
            let user = otto_state::UsersRepo::new(pool.clone())
                .create("studio-stop", "x", "U", false)
                .await
                .unwrap();
            let ws = ctx
                .workspaces
                .create("w", tmp.path().to_str().unwrap(), &user.id)
                .await
                .unwrap();
            let id = live_shell(&ctx, &ws, &user.id, tmp.path()).await;
            let story = otto_core::new_id();
            let ownership = otto_product::story_lifecycle::claim(&story).await.unwrap();
            let cancel = otto_core::cancel_signal::CancelSignal::new();
            let running_ctx = ctx.clone();
            let running_id = id.clone();
            let running_story = story.clone();
            let running_cancel = cancel.clone();
            let cwd = tmp.path().to_str().unwrap().to_owned();
            let running = tokio::spawn(async move {
                running_ctx
                    .run_session_turn(
                        &ws,
                        &user,
                        Some(&running_id),
                        "studio stop",
                        &cwd,
                        "shell",
                        serde_json::json!({"story_id": running_story}),
                        "sleep 30",
                        Duration::from_secs(30),
                        running_cancel,
                    )
                    .await
            });
            // Observe real ownership, not a delay that assumes startup finished.
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    match tokio::time::timeout(Duration::from_millis(10), SESSION_LEASES.lock(&id))
                        .await
                    {
                        Err(_) => break,
                        Ok(lease) => drop(lease),
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            let deletion = if delete_story {
                Some(tokio::spawn(async move {
                    otto_product::story_lifecycle::begin_delete(&story).await
                }))
            } else {
                cancel.cancel();
                None
            };
            assert!(tokio::time::timeout(Duration::from_secs(5), running)
                .await
                .unwrap()
                .unwrap()
                .is_err());
            assert!(!ctx.manager.is_live(&id), "delete_story={delete_story}");
            assert!(
                tokio::time::timeout(Duration::from_secs(1), SESSION_LEASES.lock(&id))
                    .await
                    .is_ok()
            );
            drop(ownership);
            if let Some(deletion) = deletion {
                drop(deletion.await.unwrap());
            }
        }
    }
}
