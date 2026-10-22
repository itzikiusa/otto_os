//! API-client saved-request routes (split out of `routes/api_client.rs`).

use super::*;

#[derive(Debug, Deserialize)]
pub struct RequestsFilter {
    pub collection_id: Option<Id>,
}

#[derive(Debug, Deserialize)]
pub struct RequestShape {
    pub shape: Option<String>,
}

/// `GET /workspaces/{wid}/api-client/requests` (?collection_id)
pub async fn list_requests(
    Path(wid): Path<Id>,
    Query(filter): Query<RequestsFilter>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<ApiRequest>>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Viewer).await?;
    Ok(Json(
        repo(&ctx)
            .list_requests(&wid, filter.collection_id.as_ref())
            .await?,
    ))
}

/// `GET /workspaces/{wid}/api-client/requests/summaries` (?collection_id) —
/// the scalar projection of `/requests` ([`ApiOverviewRequest`] rows: no
/// body, headers, auth values, scripts or docs).
pub async fn list_request_summaries(
    Path(wid): Path<Id>,
    Query(filter): Query<RequestsFilter>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<ApiOverviewRequest>>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Viewer).await?;
    Ok(Json(
        repo(&ctx)
            .list_request_summaries(&wid, filter.collection_id.as_ref())
            .await?,
    ))
}

/// `GET /workspaces/{wid}/api-client/requests/{id}`
pub async fn get_request(
    Path((wid, id)): Path<(Id, Id)>,
    Query(shape): Query<RequestShape>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<ApiRequest>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Viewer).await?;
    let req = repo(&ctx).get_request(&id).await?;
    ensure_in_workspace(&req.workspace_id, &wid)?;
    Ok(Json(if shape.shape.as_deref() == Some("agent") {
        shape_request_agent(req)
    } else {
        req
    }))
}

/// Apply the bounded, credential-masked request shape used by agents.
/// `ApiRequest` has no `truncated` flag, so a capped body says so in-band —
/// otherwise the model reads a silently clipped payload as the whole thing.
pub(crate) fn shape_request_agent(mut request: ApiRequest) -> ApiRequest {
    request.auth = api_secrets::redact_auth(&request.auth);
    request.headers = api_secrets::mask_kv_rows(&request.headers);
    request.query = api_secrets::mask_kv_rows(&request.query);
    if request.body.len() > AGENT_BODY_MAX {
        truncate_string(&mut request.body, AGENT_BODY_MAX);
        request.body.push_str(AGENT_BODY_TRUNCATED);
    }
    request
}

/// `POST /workspaces/{wid}/api-client/requests`
pub async fn create_request(
    Path(wid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    headers: HeaderMap,
    Json(req): Json<UpsertApiRequestReq>,
) -> ApiResult<Json<ApiRequest>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    if req.name.trim().is_empty() {
        return Err(Error::Invalid("request name must not be empty".into()).into());
    }
    let mut extras = validate_extras(req.extras.clone())?;
    let (source, session_id) = caller_source(&headers);
    if source["kind"] == "agent" {
        extras = Some(stamp_agent(extras, session_id.as_deref()));
    }
    ensure_collection_in_workspace(&repo(&ctx), req.collection_id.as_deref(), &wid).await?;
    let position = repo(&ctx)
        .count_requests(&wid, req.collection_id.as_ref())
        .await?;

    // Pre-generate the id so secret members can move to the Keychain BEFORE the
    // row exists (lazy migration on save; plaintext never touches SQLite).
    let id = otto_core::new_id();
    let own_ref = api_secrets::request_ref(&id);
    let (auth_row, blob) = api_secrets::split_auth_secrets(
        &normalize_json_object(req.auth.clone()),
        &own_ref,
        &BTreeMap::new(),
    )
    .map_err(|m| ApiError(Error::Invalid(m)))?;
    // A brand-new id has no Keychain item: an empty blob would only issue a
    // synchronous Keychain delete per created request (3k on an import).
    if !blob.is_empty() {
        api_secrets::store_blob_async(&ctx.secrets, &own_ref, &blob).await?;
    }

    let mut new = req_to_new(&wid, req, position, extras);
    new.id = Some(id);
    new.auth = auth_row;
    let created = repo(&ctx).create_request(new).await?;
    notify_changed(&ctx, &wid, "request", Some(&created.id), false);
    Ok(Json(created))
}

/// `PATCH /workspaces/{wid}/api-client/requests/{id}`
pub async fn update_request(
    Path((wid, id)): Path<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    headers: HeaderMap,
    Json(mut req): Json<UpsertApiRequestReq>,
) -> ApiResult<Json<ApiRequest>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    let _credential_guard = api_secrets::request_guard(&id).await;
    let repo = repo(&ctx);
    let existing = repo.get_request(&id).await?;
    ensure_in_workspace(&existing.workspace_id, &wid)?;
    // Only a CHANGED collection is validated, so an unrelated edit of a
    // legacy row still saves.
    if req.collection_id != existing.collection_id {
        ensure_collection_in_workspace(&repo, req.collection_id.as_deref(), &wid).await?;
    }
    let incoming_extras = validate_extras(req.extras.clone())?;
    let mut extras = if req.extras.is_none() {
        existing.extras.clone()
    } else {
        incoming_extras
    };
    let (source, session_id) = caller_source(&headers);
    if source["kind"] == "agent" {
        extras = Some(stamp_agent(extras, session_id.as_deref()));
    }

    // Lazy secret migration: plaintext secret members move to the Keychain;
    // markers sent back unchanged keep their stored values.
    let previous_blob =
        api_secrets::load_blob_checked_async(&ctx.secrets, &api_secrets::request_ref(&id)).await?;
    let auth_row = if let Some(incoming) = merged_auth_for_update(&req.auth) {
        let own_ref = api_secrets::request_ref(&id);
        let existing_blob = api_secrets::load_blob_checked_async(&ctx.secrets, &own_ref).await?;
        let (auth_row, blob) = api_secrets::split_auth_secrets(
            &normalize_json_object(incoming),
            &own_ref,
            &existing_blob,
        )
        .map_err(|m| ApiError(Error::Invalid(m)))?;
        api_secrets::store_blob_async(&ctx.secrets, &own_ref, &blob).await?;
        auth_row
    } else {
        existing.auth.clone()
    };

    req.auth = auth_row.clone();
    let mut new = req_to_new(&wid, req, existing.position, extras);
    new.auth = auth_row;
    let updated = match repo.update_request(&id, new).await {
        Ok(updated) => updated,
        Err(error) => {
            api_secrets::store_blob_async(
                &ctx.secrets,
                &api_secrets::request_ref(&id),
                &previous_blob,
            )
            .await?;
            return Err(error.into());
        }
    };
    notify_changed(&ctx, &wid, "request", Some(&id), false);
    Ok(Json(updated))
}

/// `None` means an absent/null PATCH auth keeps the stored row and Keychain blob.
pub(crate) fn merged_auth_for_update(incoming: &Value) -> Option<Value> {
    (!incoming.is_null()).then(|| incoming.clone())
}

/// `DELETE /workspaces/{wid}/api-client/requests/{id}`
pub async fn delete_request(
    Path((wid, id)): Path<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<StatusCode> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    let _credential_guard = api_secrets::request_guard(&id).await;
    let repo = repo(&ctx);
    ensure_in_workspace(&repo.get_request(&id).await?.workspace_id, &wid)?;
    repo.delete_request(&id).await?;
    // Best-effort: drop the request's Keychain blob with it.
    let _ = otto_core::secrets::delete_async(&ctx.secrets, &api_secrets::request_ref(&id)).await;
    notify_changed(&ctx, &wid, "request", Some(&id), true);
    Ok(StatusCode::NO_CONTENT)
}

/// Enforce the `extras` contract: JSON object, ≤ 256 KiB serialized. The UI
/// owns the inner shape (like `auth`).
pub(super) fn validate_extras(extras: Option<Value>) -> Result<Option<Value>, ApiError> {
    match extras {
        None | Some(Value::Null) => Ok(None),
        Some(v) if !v.is_object() => Err(ApiError(Error::Invalid(
            "extras must be a JSON object".into(),
        ))),
        Some(v) => {
            if v.to_string().len() > EXTRAS_MAX_BYTES {
                return Err(ApiError(Error::Invalid(format!(
                    "extras exceeds the {} KiB cap",
                    EXTRAS_MAX_BYTES / 1024
                ))));
            }
            Ok(Some(v))
        }
    }
}

pub(super) fn req_to_new(
    wid: &Id,
    req: UpsertApiRequestReq,
    position: i64,
    extras: Option<Value>,
) -> NewApiRequest {
    NewApiRequest {
        id: None,
        workspace_id: wid.clone(),
        collection_id: req.collection_id,
        name: req.name.trim().to_string(),
        method: req.method.to_uppercase(),
        url: req.url,
        headers: normalize_json_array(req.headers),
        query: normalize_json_array(req.query),
        body_mode: req.body_mode,
        body: req.body,
        auth: normalize_json_object(req.auth),
        ssh_connection_id: req.ssh_connection_id,
        extras,
        position,
    }
}
