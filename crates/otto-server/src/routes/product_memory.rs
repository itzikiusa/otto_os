//! Product → memory ingest. Extracts a story's structured artifacts (answered
//! questions, learnings, latest analysis, latest version) into the memory layer
//! so they become recallable. Editor-gated; the heavy lifting lives in
//! `otto_product::ProductMemory`.

use axum::extract::{Path, State};
use axum::Json;
use otto_core::domain::WorkspaceRole;
use otto_core::Id;
use serde::Serialize;

use crate::auth::CurrentUser;
use crate::error::ApiResult;
use crate::state::ServerCtx;

#[derive(Serialize)]
pub struct IngestResp {
    pub ingested: usize,
}

/// `POST /api/v1/workspaces/{ws}/product/stories/{sid}/memory/ingest`
pub async fn ingest(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Path((ws, sid)): Path<(Id, Id)>,
) -> ApiResult<Json<IngestResp>> {
    ctx.roles.check(&user, &ws, WorkspaceRole::Editor).await?;
    let pm = otto_product::ProductMemory::new(ctx.memory.clone());
    let ingested = pm.ingest_story(&ctx.product_repo, &ws, &sid, &user.id).await?;
    Ok(Json(IngestResp { ingested }))
}
