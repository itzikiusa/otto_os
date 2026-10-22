//! API-client environment routes (split out of `routes/api_client.rs`).

use super::*;

/// `GET /workspaces/{wid}/api-client/environments`
pub async fn list_environments(
    Path(wid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<ApiEnvironment>>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Viewer).await?;
    Ok(Json(repo(&ctx).list_environments(&wid).await?))
}

/// `POST /workspaces/{wid}/api-client/environments`
pub async fn create_environment(
    Path(wid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<UpsertApiEnvironmentReq>,
) -> ApiResult<Json<ApiEnvironment>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    if req.name.trim().is_empty() {
        return Err(Error::Invalid("environment name must not be empty".into()).into());
    }
    // The row never holds a value for a key marked secret; those arrive only
    // via the write-only `secret_values` and go straight to the Keychain.
    let secret_keys = req.secret_keys.clone().unwrap_or_default();
    let variables =
        api_secrets::strip_secret_variables(&normalize_json_object(req.variables), &secret_keys);
    let env = repo(&ctx)
        .create_environment(NewApiEnvironment {
            workspace_id: wid,
            name: req.name.trim().to_string(),
            variables,
            secret_keys: secret_keys.clone(),
        })
        .await?;
    let blob: BTreeMap<String, String> = req
        .secret_values
        .into_iter()
        .filter(|(k, _)| secret_keys.contains(k))
        .collect();
    api_secrets::store_blob_async(&ctx.secrets, &api_secrets::env_ref(&env.id), &blob).await?;
    notify_changed(&ctx, &env.workspace_id, "environment", Some(&env.id), false);
    Ok(Json(env))
}

/// `PATCH /workspaces/{wid}/api-client/environments/{id}`
pub async fn update_environment(
    Path((wid, id)): Path<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<UpsertApiEnvironmentReq>,
) -> ApiResult<Json<ApiEnvironment>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    let repo = repo(&ctx);
    let existing = repo.get_environment(&id).await?;
    ensure_in_workspace(&existing.workspace_id, &wid)?;
    // Omitted `secret_keys` keeps the stored set (a PATCH that leaves it out
    // must not silently delete every Keychain secret).
    let secret_keys: Vec<String> = req
        .secret_keys
        .clone()
        .unwrap_or_else(|| existing.secret_keys.clone());
    let vars =
        api_secrets::strip_secret_variables(&normalize_json_object(req.variables), &secret_keys);
    let sref = api_secrets::env_ref(&id);
    let mut blob = api_secrets::load_blob_async(&ctx.secrets, &sref).await;
    apply_secret_changes(
        &mut blob,
        &secret_keys,
        &req.secret_renames,
        req.secret_values,
    );
    api_secrets::store_blob_async(&ctx.secrets, &sref, &blob).await?;
    let env = repo
        .update_environment(&id, Some(req.name.trim()), Some(&vars), Some(&secret_keys))
        .await?;
    notify_changed(&ctx, &wid, "environment", Some(&id), false);
    Ok(Json(env))
}

/// Update an environment's Keychain blob in place: renamed secrets carry
/// their stored value to the new name first (`renames` = old → new, only onto
/// a key that is still secret), then values for keys no longer marked secret
/// are dropped, then the write-only new/changed values are overlaid.
pub(crate) fn apply_secret_changes(
    blob: &mut BTreeMap<String, String>,
    secret_keys: &[String],
    renames: &BTreeMap<String, String>,
    new_values: BTreeMap<String, String>,
) {
    for (old, new) in renames {
        if old == new || !secret_keys.contains(new) {
            continue;
        }
        if let Some(value) = blob.remove(old) {
            blob.entry(new.clone()).or_insert(value);
        }
    }
    blob.retain(|k, _| secret_keys.contains(k));
    for (k, v) in new_values {
        if secret_keys.contains(&k) {
            blob.insert(k, v);
        }
    }
}

/// `DELETE /workspaces/{wid}/api-client/environments/{id}`
pub async fn delete_environment(
    Path((wid, id)): Path<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<StatusCode> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    let repo = repo(&ctx);
    ensure_in_workspace(&repo.get_environment(&id).await?.workspace_id, &wid)?;
    repo.delete_environment(&id).await?;
    // Best-effort: drop the environment's Keychain blob with it.
    let _ = otto_core::secrets::delete_async(&ctx.secrets, &api_secrets::env_ref(&id)).await;
    notify_changed(&ctx, &wid, "environment", Some(&id), true);
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /workspaces/{wid}/api-client/environments/{id}/activate`
pub async fn activate_environment(
    Path((wid, id)): Path<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<ApiEnvironment>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    let repo = repo(&ctx);
    ensure_in_workspace(&repo.get_environment(&id).await?.workspace_id, &wid)?;
    let env = repo.set_active(&wid, &id).await?;
    notify_changed(&ctx, &wid, "environment", Some(&id), false);
    Ok(Json(env))
}
