//! API client ("Postman" section) endpoints, workspace-scoped under
//! `/api/v1/workspaces/{wid}/api-client/...` (plus the workspace-agnostic
//! `POST /api-client/import-curl`). Editor role is required for every
//! workspace-scoped route; mutations and reads alike (the API client is a
//! collaborative authoring surface).
//!
//! Execution runs through the daemon via a shared `reqwest` client (mirroring
//! the browser proxy), so requests dodge webview CORS/CSP and get real timing.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use otto_core::api::{
    ApiResponse, ApiRunResult, ApiRunStepResult, ExecuteApiReq, ImportCurlReq, ParsedCurl,
    UpsertApiAutomationReq, UpsertApiCollectionReq, UpsertApiEnvironmentReq, UpsertApiRequestReq,
};
use otto_core::domain::{
    ApiAutomation, ApiCollection, ApiEnvironment, ApiHistoryEntry, ApiRequest, WorkspaceRole,
};
use otto_core::{Error, Id};
use otto_state::{
    ApiClientRepo, NewApiAutomation, NewApiCollection, NewApiEnvironment, NewApiHistory,
    NewApiRequest,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::api_helpers::{collection_to_openapi, eval_assertion, json_path, parse_curl};
use crate::auth::{require_ws_role, CurrentUser};
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;

/// Hard cap for `GET .../history?limit=`.
const HISTORY_MAX: i64 = 500;
const HISTORY_DEFAULT: i64 = 100;
/// Execution timeout for outbound requests.
const EXECUTE_TIMEOUT: Duration = Duration::from_secs(60);

fn repo(ctx: &ServerCtx) -> ApiClientRepo {
    ApiClientRepo::new(ctx.pool.clone())
}

/// Shared outbound HTTP client (built once). Follows redirects, generous body
/// timeout enforced per-request via `.timeout()`.
fn http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent("Otto-ApiClient/1.0")
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()
            .unwrap_or_default()
    })
}

// ===========================================================================
// Collections
// ===========================================================================

/// `GET /workspaces/{wid}/api-client/collections`
pub async fn list_collections(
    Path(wid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<ApiCollection>>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Viewer).await?;
    Ok(Json(repo(&ctx).list_collections(&wid).await?))
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
    let position = repo(&ctx).list_collections(&wid).await?.len() as i64;
    let col = repo(&ctx)
        .create_collection(NewApiCollection {
            workspace_id: wid,
            name: req.name.trim().to_string(),
            parent_id: req.parent_id,
            position,
        })
        .await?;
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
    ensure_in_workspace(&repo.get_collection(&id).await?.workspace_id, &wid)?;
    let col = repo
        .update_collection(&id, Some(req.name.trim()), Some(req.parent_id.as_deref()))
        .await?;
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
    Ok(StatusCode::NO_CONTENT)
}

// ===========================================================================
// Requests
// ===========================================================================

#[derive(Debug, Deserialize)]
pub struct RequestsFilter {
    pub collection_id: Option<Id>,
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

/// `GET /workspaces/{wid}/api-client/requests/{id}`
pub async fn get_request(
    Path((wid, id)): Path<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<ApiRequest>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Viewer).await?;
    let req = repo(&ctx).get_request(&id).await?;
    ensure_in_workspace(&req.workspace_id, &wid)?;
    Ok(Json(req))
}

/// `POST /workspaces/{wid}/api-client/requests`
pub async fn create_request(
    Path(wid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<UpsertApiRequestReq>,
) -> ApiResult<Json<ApiRequest>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    if req.name.trim().is_empty() {
        return Err(Error::Invalid("request name must not be empty".into()).into());
    }
    let position = repo(&ctx)
        .list_requests(&wid, req.collection_id.as_ref())
        .await?
        .len() as i64;
    let created = repo(&ctx)
        .create_request(req_to_new(&wid, req, position))
        .await?;
    Ok(Json(created))
}

/// `PATCH /workspaces/{wid}/api-client/requests/{id}`
pub async fn update_request(
    Path((wid, id)): Path<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<UpsertApiRequestReq>,
) -> ApiResult<Json<ApiRequest>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    let repo = repo(&ctx);
    let existing = repo.get_request(&id).await?;
    ensure_in_workspace(&existing.workspace_id, &wid)?;
    let updated = repo
        .update_request(&id, req_to_new(&wid, req, existing.position))
        .await?;
    Ok(Json(updated))
}

/// `DELETE /workspaces/{wid}/api-client/requests/{id}`
pub async fn delete_request(
    Path((wid, id)): Path<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<StatusCode> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    let repo = repo(&ctx);
    ensure_in_workspace(&repo.get_request(&id).await?.workspace_id, &wid)?;
    repo.delete_request(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

fn req_to_new(wid: &Id, req: UpsertApiRequestReq, position: i64) -> NewApiRequest {
    NewApiRequest {
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
        position,
    }
}

// ===========================================================================
// Environments
// ===========================================================================

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
    let env = repo(&ctx)
        .create_environment(NewApiEnvironment {
            workspace_id: wid,
            name: req.name.trim().to_string(),
            variables: normalize_json_object(req.variables),
        })
        .await?;
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
    ensure_in_workspace(&repo.get_environment(&id).await?.workspace_id, &wid)?;
    let vars = normalize_json_object(req.variables);
    let env = repo
        .update_environment(&id, Some(req.name.trim()), Some(&vars))
        .await?;
    Ok(Json(env))
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
    Ok(Json(repo.set_active(&wid, &id).await?))
}

// ===========================================================================
// History
// ===========================================================================

#[derive(Debug, Deserialize)]
pub struct HistoryFilter {
    pub limit: Option<i64>,
}

/// `GET /workspaces/{wid}/api-client/history` (?limit)
pub async fn list_history(
    Path(wid): Path<Id>,
    Query(filter): Query<HistoryFilter>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<ApiHistoryEntry>>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Viewer).await?;
    let limit = filter
        .limit
        .unwrap_or(HISTORY_DEFAULT)
        .clamp(1, HISTORY_MAX);
    Ok(Json(repo(&ctx).list_history(&wid, limit).await?))
}

/// `DELETE /workspaces/{wid}/api-client/history`
pub async fn clear_history(
    Path(wid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<StatusCode> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    repo(&ctx).clear_history(&wid).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ===========================================================================
// OpenAPI export
// ===========================================================================

/// `GET /workspaces/{wid}/api-client/collections/{id}/openapi`
pub async fn export_openapi(
    Path((wid, id)): Path<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Value>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Viewer).await?;
    let repo = repo(&ctx);
    let collection = repo.get_collection(&id).await?;
    ensure_in_workspace(&collection.workspace_id, &wid)?;
    let requests = repo.list_requests(&wid, Some(&id)).await?;
    Ok(Json(collection_to_openapi(&collection, &requests)))
}

// ===========================================================================
// Import curl (workspace-agnostic)
// ===========================================================================

/// `POST /api-client/import-curl`
pub async fn import_curl(
    State(_ctx): State<ServerCtx>,
    CurrentUser(_user): CurrentUser,
    Json(req): Json<ImportCurlReq>,
) -> ApiResult<Json<ParsedCurl>> {
    Ok(Json(parse_curl(&req.curl)?))
}

// ===========================================================================
// Execute
// ===========================================================================

/// `POST /workspaces/{wid}/api-client/execute`
///
/// Resolves `{{var}}` placeholders from the selected (or active) environment,
/// builds and sends the request through the shared client, records a history
/// entry, and returns the response. Network failures map to a 502 `Problem`
/// (and are still recorded in history with a null status).
pub async fn execute(
    Path(wid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<ExecuteApiReq>,
) -> ApiResult<Json<ApiResponse>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    let repo = repo(&ctx);

    // Resolve the environment variable map.
    let vars = resolve_variables(&repo, &wid, req.environment_id.as_ref()).await?;

    // Snapshot of the request as executed (post-substitution view recorded too).
    let request_snapshot = json!({
        "method": req.method,
        "url": req.url,
        "headers": req.headers,
        "query": req.query,
        "body_mode": req.body_mode,
        "body": req.body,
        "auth": req.auth,
        "environment_id": req.environment_id,
    });

    match build_and_send(&req, &vars).await {
        Ok(resp) => {
            // Record success in history (best-effort; do not fail the request).
            let _ = repo
                .insert_history(NewApiHistory {
                    workspace_id: wid.clone(),
                    method: req.method.to_uppercase(),
                    url: substitute(&req.url, &vars),
                    status: Some(resp.status as i64),
                    duration_ms: Some(resp.duration_ms),
                    request: request_snapshot,
                    response: serde_json::to_value(&resp).unwrap_or(Value::Null),
                })
                .await;
            Ok(Json(resp))
        }
        Err(err) => {
            // Record the failure too, then surface as a 502.
            let _ = repo
                .insert_history(NewApiHistory {
                    workspace_id: wid.clone(),
                    method: req.method.to_uppercase(),
                    url: substitute(&req.url, &vars),
                    status: None,
                    duration_ms: None,
                    request: request_snapshot,
                    response: json!({ "error": err }),
                })
                .await;
            Err(ApiError(Error::Upstream(err)))
        }
    }
}

/// Resolve the variable map: explicit `environment_id`, else the workspace's
/// active environment, else empty.
async fn resolve_variables(
    repo: &ApiClientRepo,
    wid: &Id,
    environment_id: Option<&Id>,
) -> ApiResult<serde_json::Map<String, Value>> {
    let env = match environment_id {
        Some(eid) => {
            let env = repo.get_environment(eid).await?;
            ensure_in_workspace(&env.workspace_id, wid)?;
            Some(env)
        }
        None => repo.active_environment(wid).await?,
    };
    Ok(env
        .and_then(|e| e.variables.as_object().cloned())
        .unwrap_or_default())
}

/// Build the outbound request and send it, returning an `ApiResponse` or an
/// error message string (never panics).
async fn build_and_send(
    req: &ExecuteApiReq,
    vars: &serde_json::Map<String, Value>,
) -> Result<ApiResponse, String> {
    use reqwest::header::{HeaderMap, HeaderName, HeaderValue, AUTHORIZATION, CONTENT_TYPE};

    // --- method ---
    let url = substitute(&req.url, vars);
    let method = reqwest::Method::from_bytes(req.method.to_uppercase().as_bytes())
        .map_err(|_| format!("invalid HTTP method '{}'", req.method))?;

    // --- query params (enabled only) ---
    let mut query: Vec<(String, String)> = Vec::new();
    for (k, v) in enabled_kv(&req.query) {
        query.push((substitute(&k, vars), substitute(&v, vars)));
    }

    // --- headers (enabled only) ---
    let mut headers = HeaderMap::new();
    for (k, v) in enabled_kv(&req.headers) {
        let name = substitute(&k, vars);
        let value = substitute(&v, vars);
        if name.trim().is_empty() {
            continue;
        }
        let hn = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| format!("invalid header name '{name}'"))?;
        let hv = HeaderValue::from_str(&value)
            .map_err(|_| format!("invalid header value for '{name}'"))?;
        headers.insert(hn, hv);
    }

    // --- auth ---
    apply_auth(&req.auth, vars, &mut headers, &mut query)?;

    let client = http_client();
    let mut builder = client.request(method, &url).timeout(EXECUTE_TIMEOUT);

    // --- body per body_mode ---
    match req.body_mode.as_str() {
        "none" | "" => {}
        "json" => {
            let body = substitute(&req.body, vars);
            if !headers.contains_key(CONTENT_TYPE) {
                headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
            }
            builder = builder.body(body);
        }
        "graphql" => {
            let body = substitute(&req.body, vars);
            if !headers.contains_key(CONTENT_TYPE) {
                headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
            }
            builder = builder.body(body);
        }
        "form" => {
            let body = substitute(&req.body, vars);
            if !headers.contains_key(CONTENT_TYPE) {
                headers.insert(
                    CONTENT_TYPE,
                    HeaderValue::from_static("application/x-www-form-urlencoded"),
                );
            }
            builder = builder.body(body);
        }
        // raw or any other mode → send as text verbatim.
        _ => {
            let body = substitute(&req.body, vars);
            builder = builder.body(body);
        }
    }

    // Silence unused import when AUTHORIZATION is only referenced inside apply_auth.
    let _ = AUTHORIZATION;

    if !query.is_empty() {
        builder = builder.query(&query);
    }
    builder = builder.headers(headers);

    let started = Instant::now();
    let resp = builder
        .send()
        .await
        .map_err(|e| describe_reqwest_error(&e))?;
    let status = resp.status();
    let status_code = status.as_u16();
    let status_text = status
        .canonical_reason()
        .unwrap_or("")
        .to_string();

    // Response headers as [{key,value}].
    let resp_headers: Vec<Value> = resp
        .headers()
        .iter()
        .map(|(k, v)| {
            json!({
                "key": k.as_str(),
                "value": v.to_str().unwrap_or("").to_string(),
            })
        })
        .collect();
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    // Body: read bytes, render as UTF-8 text (lossy for binary).
    let bytes = resp.bytes().await.map_err(|e| describe_reqwest_error(&e))?;
    let size_bytes = bytes.len() as i64;
    let body = String::from_utf8_lossy(&bytes).into_owned();
    let duration_ms = started.elapsed().as_millis() as i64;

    Ok(ApiResponse {
        status: status_code,
        status_text,
        headers: Value::Array(resp_headers),
        body,
        duration_ms,
        size_bytes,
        content_type,
    })
}

/// Apply the `auth` object to headers/query. Shapes:
/// `{type:bearer, token}` → Authorization: Bearer …
/// `{type:basic, username, password}` → Authorization: Basic base64
/// `{type:api_key, in:"header"|"query", key, value}`
fn apply_auth(
    auth: &Value,
    vars: &serde_json::Map<String, Value>,
    headers: &mut reqwest::header::HeaderMap,
    query: &mut Vec<(String, String)>,
) -> Result<(), String> {
    use reqwest::header::{HeaderName, HeaderValue, AUTHORIZATION};

    let kind = auth.get("type").and_then(Value::as_str).unwrap_or("none");
    match kind {
        "bearer" => {
            let token = substitute(auth.get("token").and_then(Value::as_str).unwrap_or(""), vars);
            if !token.is_empty() {
                let hv = HeaderValue::from_str(&format!("Bearer {token}"))
                    .map_err(|_| "invalid bearer token".to_string())?;
                headers.insert(AUTHORIZATION, hv);
            }
        }
        "basic" => {
            use base64::engine::general_purpose::STANDARD as B64;
            use base64::Engine;
            let user = substitute(
                auth.get("username").and_then(Value::as_str).unwrap_or(""),
                vars,
            );
            let pass = substitute(
                auth.get("password").and_then(Value::as_str).unwrap_or(""),
                vars,
            );
            let encoded = B64.encode(format!("{user}:{pass}"));
            let hv = HeaderValue::from_str(&format!("Basic {encoded}"))
                .map_err(|_| "invalid basic auth".to_string())?;
            headers.insert(AUTHORIZATION, hv);
        }
        "api_key" => {
            let key = substitute(auth.get("key").and_then(Value::as_str).unwrap_or(""), vars);
            let value = substitute(auth.get("value").and_then(Value::as_str).unwrap_or(""), vars);
            if key.is_empty() {
                return Ok(());
            }
            let location = auth.get("in").and_then(Value::as_str).unwrap_or("header");
            if location == "query" {
                query.push((key, value));
            } else {
                let hn = HeaderName::from_bytes(key.as_bytes())
                    .map_err(|_| format!("invalid api_key header name '{key}'"))?;
                let hv = HeaderValue::from_str(&value)
                    .map_err(|_| "invalid api_key header value".to_string())?;
                headers.insert(hn, hv);
            }
        }
        _ => {}
    }
    Ok(())
}

/// Human-friendly message for a reqwest error (timeout / DNS / connect / …).
fn describe_reqwest_error(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        format!("request timed out after {}s", EXECUTE_TIMEOUT.as_secs())
    } else if e.is_connect() {
        format!("connection failed: {e}")
    } else if e.is_request() {
        format!("could not build request: {e}")
    } else if e.is_body() || e.is_decode() {
        format!("failed reading response: {e}")
    } else {
        format!("request failed: {e}")
    }
}

// ===========================================================================
// Automations (collection runner)
// ===========================================================================

/// `GET /workspaces/{wid}/api-client/automations`
pub async fn list_automations(
    Path(wid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<ApiAutomation>>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Viewer).await?;
    Ok(Json(repo(&ctx).list_automations(&wid).await?))
}

/// `POST /workspaces/{wid}/api-client/automations`
pub async fn create_automation(
    Path(wid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<UpsertApiAutomationReq>,
) -> ApiResult<Json<ApiAutomation>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    if req.name.trim().is_empty() {
        return Err(Error::Invalid("automation name must not be empty".into()).into());
    }
    let auto = repo(&ctx)
        .create_automation(NewApiAutomation {
            workspace_id: wid,
            name: req.name.trim().to_string(),
            steps: normalize_json_array(req.steps),
        })
        .await?;
    Ok(Json(auto))
}

/// `PATCH /workspaces/{wid}/api-client/automations/{id}`
pub async fn update_automation(
    Path((wid, id)): Path<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<UpsertApiAutomationReq>,
) -> ApiResult<Json<ApiAutomation>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    let repo = repo(&ctx);
    ensure_in_workspace(&repo.get_automation(&id).await?.workspace_id, &wid)?;
    let steps = normalize_json_array(req.steps);
    let auto = repo
        .update_automation(&id, Some(req.name.trim()), Some(&steps))
        .await?;
    Ok(Json(auto))
}

/// `DELETE /workspaces/{wid}/api-client/automations/{id}`
pub async fn delete_automation(
    Path((wid, id)): Path<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<StatusCode> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    let repo = repo(&ctx);
    ensure_in_workspace(&repo.get_automation(&id).await?.workspace_id, &wid)?;
    repo.delete_automation(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /workspaces/{wid}/api-client/automations/{id}/run`
///
/// Runs each step in order against its saved request. The effective variable
/// map starts from the workspace's active environment and is overlaid with
/// variables extracted from prior steps (chaining). Every step is attempted —
/// a failing step is recorded but never aborts the run, and a failure to load a
/// request, send it, or evaluate assertions never panics.
pub async fn run_automation(
    Path((wid, id)): Path<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<ApiRunResult>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    let repo = repo(&ctx);

    let automation = repo.get_automation(&id).await?;
    ensure_in_workspace(&automation.workspace_id, &wid)?;

    // Seed the chained variable map from the workspace's active environment.
    let mut vars = resolve_variables(&repo, &wid, None).await?;

    let steps = automation.steps.as_array().cloned().unwrap_or_default();
    let mut results: Vec<ApiRunStepResult> = Vec::with_capacity(steps.len());

    for step in &steps {
        results.push(run_step(&repo, &wid, step, &mut vars).await);
    }

    let passed = results.iter().all(|s| s.ok);
    Ok(Json(ApiRunResult {
        automation_id: id,
        steps: results,
        passed,
    }))
}

/// Run one automation step against its saved request, evaluating assertions and
/// applying extractions into `vars` for later steps. Resilient: any error is
/// captured into the returned [`ApiRunStepResult`] rather than propagated.
async fn run_step(
    repo: &ApiClientRepo,
    wid: &Id,
    step: &Value,
    vars: &mut serde_json::Map<String, Value>,
) -> ApiRunStepResult {
    let request_id = step
        .get("request_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    // Load the saved request (must belong to this workspace).
    let request = match load_step_request(repo, wid, &request_id).await {
        Ok(r) => r,
        Err(msg) => {
            return ApiRunStepResult {
                request_id,
                name: String::new(),
                status: None,
                duration_ms: 0,
                ok: false,
                assertions: Value::Array(Vec::new()),
                error: Some(msg),
            };
        }
    };

    let exec = request_to_execute(&request);

    // Send via the shared single-request path (same reqwest logic as /execute).
    match build_and_send(&exec, vars).await {
        Ok(resp) => {
            // Parse the body as JSON once for json_path assertions/extraction;
            // a non-JSON body yields Null (assertions/extracts simply miss).
            let body_json = serde_json::from_str::<Value>(&resp.body).unwrap_or(Value::Null);

            // Evaluate assertions.
            let (assertions_value, all_passed) = eval_step_assertions(
                step,
                Some(resp.status),
                resp.duration_ms,
                &body_json,
            );

            // Apply extractions into the chained variable map.
            apply_extractions(step, &body_json, vars);

            ApiRunStepResult {
                request_id,
                name: request.name,
                status: Some(resp.status),
                duration_ms: resp.duration_ms,
                ok: all_passed, // request succeeded; ok iff every assertion held
                assertions: assertions_value,
                error: None,
            }
        }
        Err(err) => {
            // Request failed: still evaluate assertions (against a null body and
            // unknown status) so the report is complete, but ok is false.
            let (assertions_value, _) =
                eval_step_assertions(step, None, 0, &Value::Null);
            ApiRunStepResult {
                request_id,
                name: request.name,
                status: None,
                duration_ms: 0,
                ok: false,
                assertions: assertions_value,
                error: Some(err),
            }
        }
    }
}

/// Load a step's saved request, validating presence and workspace ownership.
/// Returns a descriptive error string on any miss.
async fn load_step_request(
    repo: &ApiClientRepo,
    wid: &Id,
    request_id: &str,
) -> Result<ApiRequest, String> {
    if request_id.is_empty() {
        return Err("step is missing request_id".to_string());
    }
    let request = repo
        .get_request(&request_id.to_string())
        .await
        .map_err(|_| format!("request '{request_id}' not found"))?;
    if &request.workspace_id != wid {
        return Err(format!("request '{request_id}' not in this workspace"));
    }
    Ok(request)
}

/// Build an [`ExecuteApiReq`] from a saved request so the runner can reuse the
/// single-request execute path verbatim.
fn request_to_execute(request: &ApiRequest) -> ExecuteApiReq {
    ExecuteApiReq {
        method: request.method.clone(),
        url: request.url.clone(),
        headers: request.headers.clone(),
        query: request.query.clone(),
        body_mode: request.body_mode.clone(),
        body: request.body.clone(),
        auth: request.auth.clone(),
        // Variables are supplied by the runner's chained map, not an env id.
        environment_id: None,
    }
}

/// Evaluate a step's `assertions` array, returning the `[{desc,passed}]` JSON
/// and whether every assertion passed (vacuously true when none).
fn eval_step_assertions(
    step: &Value,
    status: Option<u16>,
    duration_ms: i64,
    body: &Value,
) -> (Value, bool) {
    let mut out: Vec<Value> = Vec::new();
    let mut all = true;
    if let Some(arr) = step.get("assertions").and_then(Value::as_array) {
        for assertion in arr {
            let r = eval_assertion(assertion, status, duration_ms, body);
            if !r.passed {
                all = false;
            }
            out.push(json!({ "desc": r.desc, "passed": r.passed }));
        }
    }
    (Value::Array(out), all)
}

/// Apply a step's `extract` list: evaluate each `path` against the JSON body and
/// store `var -> value` into the chained variable map. Missing paths are
/// skipped (the variable is simply not set).
fn apply_extractions(
    step: &Value,
    body: &Value,
    vars: &mut serde_json::Map<String, Value>,
) {
    if let Some(arr) = step.get("extract").and_then(Value::as_array) {
        for entry in arr {
            let var = entry.get("var").and_then(Value::as_str).unwrap_or("");
            let path = entry.get("path").and_then(Value::as_str).unwrap_or("");
            if var.is_empty() {
                continue;
            }
            if let Some(found) = json_path(body, path) {
                vars.insert(var.to_string(), found.clone());
            }
        }
    }
}

// ===========================================================================
// shared helpers
// ===========================================================================

/// 404 when an entity belongs to a different workspace than the path's `wid`.
fn ensure_in_workspace(entity_ws: &Id, wid: &Id) -> Result<(), ApiError> {
    if entity_ws == wid {
        Ok(())
    } else {
        Err(ApiError(Error::NotFound("not in this workspace".into())))
    }
}

/// Replace `{{name}}` occurrences with the corresponding variable value
/// (string-coerced). Unknown placeholders are left intact.
fn substitute(input: &str, vars: &serde_json::Map<String, Value>) -> String {
    if !input.contains("{{") {
        return input.to_string();
    }
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'{' && bytes[i + 1] == b'{' {
            if let Some(end) = input[i + 2..].find("}}") {
                let name = input[i + 2..i + 2 + end].trim();
                if let Some(v) = vars.get(name) {
                    out.push_str(&value_to_string(v));
                } else {
                    // leave placeholder as-is
                    out.push_str(&input[i..i + 2 + end + 2]);
                }
                i = i + 2 + end + 2;
                continue;
            }
        }
        // push the current char (handle utf-8 boundaries via char iteration)
        let ch = input[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn value_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Extract enabled `(key, value)` pairs from a `[{key,value,enabled}]` array.
/// A missing `enabled` field defaults to true.
fn enabled_kv(v: &Value) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if let Some(arr) = v.as_array() {
        for item in arr {
            let enabled = item.get("enabled").and_then(Value::as_bool).unwrap_or(true);
            if !enabled {
                continue;
            }
            let key = item.get("key").and_then(Value::as_str).unwrap_or("");
            if key.is_empty() {
                continue;
            }
            let value = item.get("value").and_then(Value::as_str).unwrap_or("");
            out.push((key.to_string(), value.to_string()));
        }
    }
    out
}

/// Default a JSON value to an empty array when it arrives as null (the DTOs use
/// `#[serde(default)]` so omitted fields become `Value::Null`).
fn normalize_json_array(v: Value) -> Value {
    if v.is_null() {
        Value::Array(Vec::new())
    } else {
        v
    }
}

/// Default a JSON value to an empty object when null.
fn normalize_json_object(v: Value) -> Value {
    if v.is_null() {
        Value::Object(serde_json::Map::new())
    } else {
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn substitute_replaces_known_and_keeps_unknown() {
        let mut vars = serde_json::Map::new();
        vars.insert("base".into(), json!("https://api.test"));
        vars.insert("ver".into(), json!(2));
        assert_eq!(
            substitute("{{base}}/v{{ver}}/users", &vars),
            "https://api.test/v2/users"
        );
        // unknown placeholder preserved
        assert_eq!(substitute("{{missing}}/x", &vars), "{{missing}}/x");
        // no placeholders → unchanged
        assert_eq!(substitute("plain", &vars), "plain");
    }

    #[test]
    fn enabled_kv_filters_disabled_and_empty() {
        let v = json!([
            {"key":"A","value":"1","enabled":true},
            {"key":"B","value":"2","enabled":false},
            {"key":"","value":"x","enabled":true},
            {"key":"C","value":"3"}
        ]);
        assert_eq!(
            enabled_kv(&v),
            vec![("A".into(), "1".into()), ("C".into(), "3".into())]
        );
    }
}
