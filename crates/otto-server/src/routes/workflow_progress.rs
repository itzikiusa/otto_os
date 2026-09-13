//! Summary polls and explicitly requested workflow bodies.
use crate::{
    auth::{require_ws_role, CurrentUser},
    error::{ApiError, ApiResult},
    state::ServerCtx,
};
use axum::{
    extract::{Path, Query, State},
    Json,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use otto_core::{
    domain::{User, WorkspaceRole},
    Error, Id,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

async fn gate(ctx: &ServerCtx, user: &User, id: &str) -> ApiResult<()> {
    let wid: Option<String> =
        sqlx::query_scalar("SELECT workspace_id FROM workflow_runs WHERE id=?")
            .bind(id)
            .fetch_optional(&ctx.pool)
            .await
            .map_err(|e| ApiError(Error::Internal(format!("workflow access: {e}"))))?;
    let wid = wid.ok_or_else(|| ApiError(Error::NotFound("workflow run not found".into())))?;
    require_ws_role(ctx, user, &wid, WorkspaceRole::Viewer).await
}
#[derive(Deserialize)]
pub struct ProgressQuery {
    after_rev: Option<i64>,
}
pub async fn progress(
    Path(id): Path<Id>,
    Query(q): Query<ProgressQuery>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Value>> {
    gate(&ctx, &user, &id).await?;
    let mut result = otto_state::workflow_progress::progress(&ctx.pool, &id, q.after_rev)
        .await
        .map_err(ApiError)?;
    if result["changed"] == true {
        if let Some(dir) =
            otto_core::paths::confine_join(&ctx.data_dir.join("workflow-context"), &id)
        {
            if tokio::fs::metadata(&dir).await.is_ok_and(|m| m.is_dir()) {
                result["run"]["context_dir"] = Value::String(dir.to_string_lossy().into_owned());
            }
        }
    }
    Ok(Json(result))
}
#[derive(Deserialize)]
pub struct CheckpointQuery {
    cursor: Option<String>,
    limit: Option<usize>,
}
#[derive(Serialize, Deserialize)]
struct Cursor {
    run: String,
    generation: i64,
    after: i64,
}
fn decode_cursor(cursor: Option<&str>, id: &str) -> ApiResult<Option<Cursor>> {
    let Some(raw) = cursor else { return Ok(None) };
    let invalid = || {
        ApiError(Error::Invalid(
            "invalid checkpoint cursor; reload checkpoints".into(),
        ))
    };
    if raw.len() > 4096 {
        return Err(invalid());
    }
    let cursor: Cursor =
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(raw).map_err(|_| invalid())?)
            .map_err(|_| invalid())?;
    if cursor.run != id || cursor.after < 0 || cursor.generation < 0 {
        return Err(invalid());
    }
    Ok(Some(cursor))
}
pub async fn checkpoints(
    Path(id): Path<Id>,
    Query(q): Query<CheckpointQuery>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Value>> {
    gate(&ctx, &user, &id).await?;
    let cursor = decode_cursor(q.cursor.as_deref(), &id)?;
    let mut page = otto_state::workflow_progress::checkpoint_page(
        &ctx.pool,
        &id,
        cursor.as_ref().map(|c| c.generation),
        cursor.as_ref().map_or(0, |c| c.after),
        q.limit.unwrap_or(100),
    )
    .await
    .map_err(ApiError)?;
    let next = page["next_after"].as_i64().map(|after| Cursor {
        run: id,
        generation: page["generation"].as_i64().unwrap_or_default(),
        after,
    });
    page["next_cursor"] = match next {
        Some(next) => Value::String(URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&next).map_err(|e| ApiError(Error::Internal(e.to_string())))?,
        )),
        None => Value::Null,
    };
    page.as_object_mut().unwrap().remove("next_after");
    Ok(Json(page))
}
pub async fn checkpoint_detail(
    Path((id, node)): Path<(Id, String)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Value>> {
    gate(&ctx, &user, &id).await?;
    Ok(Json(
        otto_state::workflow_progress::detail(&ctx.pool, &id, &node, true)
            .await
            .map_err(ApiError)?,
    ))
}
pub async fn node_detail(
    Path((id, node)): Path<(Id, String)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Value>> {
    gate(&ctx, &user, &id).await?;
    Ok(Json(
        otto_state::workflow_progress::detail(&ctx.pool, &id, &node, false)
            .await
            .map_err(ApiError)?,
    ))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn checkpoint_cursor_is_bound_to_run_and_valid_offset() {
        let cursor = URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&Cursor {
                run: "a".into(),
                generation: 3,
                after: 9,
            })
            .unwrap(),
        );
        assert_eq!(decode_cursor(Some(&cursor), "a").unwrap().unwrap().after, 9);
        assert!(decode_cursor(Some(&cursor), "b").is_err());
        assert!(decode_cursor(Some(&"x".repeat(5000)), "a").is_err());
    }
}
