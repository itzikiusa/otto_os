//! `GET /sessions/{id}/slash-commands` — thin HTTP handler over
//! [`otto_sessions::slash_commands`].

use std::path::PathBuf;

use axum::extract::{Path as AxPath, State};
use axum::Json;
use otto_core::domain::WorkspaceRole;
use otto_core::Id;
pub use otto_sessions::slash_commands::{codex_home, dirs_home, list, SlashCommand};

use crate::auth::CurrentUser;
use crate::error::ApiResult;
use crate::state::ServerCtx;

/// `GET /sessions/{id}/slash-commands` — Viewer + owner-or-admin (same gate
/// as the transcript; the list reveals the user's command/skill names).
pub async fn slash_commands(
    AxPath(id): AxPath<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<SlashCommand>>> {
    let session = super::transcript::session_gate(&ctx, &user, &id, WorkspaceRole::Viewer).await?;
    let provider = super::transcript::effective_provider(&session);
    let cwd = session
        .meta
        .get("nested_cwd")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| session.cwd.clone());
    let cwd = PathBuf::from(cwd);
    let list =
        tokio::task::spawn_blocking(move || list(&provider, &cwd, &dirs_home(), &codex_home()))
            .await
            .unwrap_or_default();
    Ok(Json(list))
}
