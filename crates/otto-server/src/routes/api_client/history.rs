//! API-client history routes (split out of `routes/api_client.rs`).

use super::*;

#[derive(Debug, Deserialize)]
pub struct HistoryFilter {
    pub limit: Option<i64>,
    pub q: Option<String>,
    pub status: Option<i64>,
    pub request_id: Option<String>,
    pub source: Option<String>,
}

pub(super) fn history_query(filter: HistoryFilter) -> ApiHistoryQuery {
    ApiHistoryQuery {
        limit: filter
            .limit
            .unwrap_or(HISTORY_DEFAULT)
            .clamp(1, HISTORY_MAX),
        q: filter.q,
        status: filter.status,
        request_id: filter.request_id,
        source: filter.source,
    }
}

/// Additive metadata projection; retained replay bodies remain in detail/full history.
pub async fn list_history_summaries(
    Path(wid): Path<Id>,
    Query(filter): Query<HistoryFilter>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<ApiHistorySummary>>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Viewer).await?;
    Ok(Json(
        repo(&ctx)
            .list_history_summaries(&wid, &history_query(filter))
            .await?,
    ))
}

/// `GET /workspaces/{wid}/api-client/history` (optional filters)
pub async fn list_history(
    Path(wid): Path<Id>,
    Query(filter): Query<HistoryFilter>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<ApiHistoryEntry>>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Viewer).await?;
    Ok(Json(
        repo(&ctx)
            .list_history_filtered(&wid, &history_query(filter))
            .await?,
    ))
}

/// `GET /workspaces/{wid}/api-client/history/{id}`
pub async fn get_history(
    Path((wid, id)): Path<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<ApiHistoryEntry>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Viewer).await?;
    let entry = repo(&ctx).get_history(&id).await?;
    ensure_in_workspace(&entry.workspace_id, &wid)?;
    Ok(Json(entry))
}

/// `DELETE /workspaces/{wid}/api-client/history`
pub async fn clear_history(
    Path(wid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<StatusCode> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    repo(&ctx).clear_history(&wid).await?;
    invalidate_storage_gauge(&wid);
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// backend-correctness-2-01: a huge "keep N days" is clamped (it used to
    /// reach `now - days(n)` and panic every prune); negatives fall back to
    /// the default.
    #[test]
    fn retention_days_are_clamped() {
        let s = json!({"api_client": {"history_max_rows": 10, "history_max_days": 99_999_999}});
        assert_eq!(
            retention_from_settings(Some(&s)),
            (10, HISTORY_KEEP_DAYS_MAX)
        );
        assert_eq!(
            retention_from_settings(None),
            (HISTORY_KEEP_ROWS_DEFAULT, HISTORY_KEEP_DAYS_DEFAULT)
        );
        let neg = json!({"api_client": {"history_max_days": -1}});
        assert_eq!(
            retention_from_settings(Some(&neg)).1,
            HISTORY_KEEP_DAYS_DEFAULT
        );
    }
}
