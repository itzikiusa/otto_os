//! API-client collection routes (split out of `routes/api_client.rs`).

use super::*;

/// `GET /workspaces/{wid}/api-client/collections`
pub async fn list_collections(
    Path(wid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<ApiCollection>>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Viewer).await?;
    Ok(Json(repo(&ctx).list_collections(&wid).await?))
}

/// Tell the workspace's open API clients a saved object changed (perf N4) —
/// a person's edit, an agent's `api_upsert_request` (it goes through these
/// same routes) or an import. Ids only; the UI drops its 60 s list cache.
pub(crate) fn notify_changed(
    ctx: &ServerCtx,
    wid: &Id,
    kind: &str,
    id: Option<&Id>,
    deleted: bool,
) {
    let _ = ctx.events.send(Event::ApiClientChanged {
        workspace_id: wid.clone(),
        kind: kind.to_string(),
        id: id.cloned(),
        deleted,
    });
}

/// `POST /workspaces/{wid}/api-client/collections`
pub async fn create_collection(
    Path(wid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<UpsertApiCollectionReq>,
) -> ApiResult<Json<ApiCollection>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    if req.name.trim().is_empty() {
        return Err(Error::Invalid("collection name must not be empty".into()).into());
    }
    ensure_collection_in_workspace(&repo(&ctx), req.parent_id.as_deref(), &wid).await?;
    let position = repo(&ctx).count_collections(&wid).await?;
    let col = repo(&ctx)
        .create_collection(NewApiCollection {
            workspace_id: wid,
            name: req.name.trim().to_string(),
            parent_id: req.parent_id,
            position,
        })
        .await?;
    notify_changed(&ctx, &col.workspace_id, "collection", Some(&col.id), false);
    Ok(Json(col))
}

/// `PATCH /workspaces/{wid}/api-client/collections/{id}`
pub async fn update_collection(
    Path((wid, id)): Path<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<UpsertApiCollectionReq>,
) -> ApiResult<Json<ApiCollection>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    let repo = repo(&ctx);
    let existing = repo.get_collection(&id).await?;
    ensure_in_workspace(&existing.workspace_id, &wid)?;
    // Only a CHANGED parent is validated, so a legacy row can still be renamed.
    if req.parent_id != existing.parent_id {
        if let Some(parent) = req.parent_id.as_deref() {
            ensure_collection_in_workspace(&repo, Some(parent), &wid).await?;
            if repo.collection_parent_would_cycle(&id, parent).await? {
                return Err(ApiError(Error::Invalid(
                    "a collection can't be moved under itself or one of its sub-collections".into(),
                )));
            }
        }
    }
    let col = repo
        .update_collection(&id, Some(req.name.trim()), Some(req.parent_id.as_deref()))
        .await?;
    notify_changed(&ctx, &wid, "collection", Some(&id), false);
    Ok(Json(col))
}

/// `DELETE /workspaces/{wid}/api-client/collections/{id}`
pub async fn delete_collection(
    Path((wid, id)): Path<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<StatusCode> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    let repo = repo(&ctx);
    ensure_in_workspace(&repo.get_collection(&id).await?.workspace_id, &wid)?;
    repo.delete_collection(&id).await?;
    notify_changed(&ctx, &wid, "collection", Some(&id), true);
    Ok(StatusCode::NO_CONTENT)
}

/// 400 unless `collection_id` (when given) is a collection of `wid`
/// (data-integrity-2-05). The `parent_id` / `collection_id` FKs only prove the
/// id exists in SOME workspace: a row filed under another workspace's
/// collection is never reached by that workspace's tree (so it vanishes from
/// the UI), and the other workspace's delete would cascade into this one.
/// Agents reach these routes through `otto_api_upsert_request`, so a
/// model-chosen id is enough to trigger it.
pub(crate) async fn ensure_collection_in_workspace(
    repo: &ApiClientRepo,
    collection_id: Option<&str>,
    wid: &Id,
) -> Result<(), ApiError> {
    let Some(cid) = collection_id else {
        return Ok(());
    };
    match repo.collection_workspace(cid).await? {
        Some(owner) if &owner == wid => Ok(()),
        _ => Err(ApiError(Error::Invalid(format!(
            "collection {cid} is not in this workspace"
        )))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn two_workspaces() -> (ApiClientRepo, Id, Id) {
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(true);
        let pool = otto_state::DbPool::from(
            sqlx::sqlite::SqlitePoolOptions::new()
                .max_connections(1)
                .connect_with(opts)
                .await
                .unwrap(),
        );
        sqlx::migrate!("../otto-state/migrations")
            .run(&pool)
            .await
            .unwrap();
        let now = chrono::Utc::now().to_rfc3339();
        let (a, b) = (otto_core::new_id(), otto_core::new_id());
        for ws in [&a, &b] {
            sqlx::query(
                "INSERT INTO workspaces (id, name, root_path, created_at) VALUES (?, 'ws', '/tmp', ?)",
            )
            .bind(ws)
            .bind(&now)
            .execute(&pool)
            .await
            .unwrap();
        }
        (ApiClientRepo::new(pool), a, b)
    }

    /// data-integrity-2-05: a parent / collection id from another workspace
    /// (or one that doesn't exist) is rejected; one from this workspace and
    /// "none" pass.
    #[tokio::test]
    async fn collection_refs_must_belong_to_the_workspace() {
        let (repo, a, b) = two_workspaces().await;
        let foreign = repo
            .create_collection(NewApiCollection {
                workspace_id: b.clone(),
                name: "theirs".into(),
                parent_id: None,
                position: 0,
            })
            .await
            .unwrap();
        let own = repo
            .create_collection(NewApiCollection {
                workspace_id: a.clone(),
                name: "ours".into(),
                parent_id: None,
                position: 0,
            })
            .await
            .unwrap();
        assert!(ensure_collection_in_workspace(&repo, None, &a)
            .await
            .is_ok());
        assert!(ensure_collection_in_workspace(&repo, Some(&own.id), &a)
            .await
            .is_ok());
        let err = ensure_collection_in_workspace(&repo, Some(&foreign.id), &a)
            .await
            .unwrap_err();
        assert!(matches!(err.0, Error::Invalid(_)), "{:?}", err.0);
        assert!(ensure_collection_in_workspace(&repo, Some("missing"), &a)
            .await
            .is_err());
    }
}
