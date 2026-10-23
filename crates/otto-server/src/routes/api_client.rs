//! API client ("Postman" section) endpoints, workspace-scoped under
//! `/api/v1/workspaces/{wid}/api-client/...` (plus the workspace-agnostic
//! `POST /api-client/import-curl`). Editor role is required for every
//! workspace-scoped route; mutations and reads alike (the API client is a
//! collaborative authoring surface).
//!
//! Execution runs through the daemon via a shared `reqwest` client (mirroring
//! the browser proxy), so requests dodge webview CORS/CSP and get real timing.
//! The execute engine itself (environment + secret resolution, the secret ↔
//! host binding, cookie jars, tunnels, send, history, automation steps) is
//! `otto_apiclient::engine`, behind the `ApiClientCtx` this module implements
//! for `ServerCtx` (`api_client/ctx.rs`); these are the axum handlers.

use std::collections::BTreeMap;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use otto_apiclient::secrets as api_secrets;
use otto_core::api::{
    ApiClientStorage, ApiOverview, ApiOverviewAutomation, ApiOverviewCollection,
    ApiOverviewEnvironment, ApiOverviewRequest, ApiResponse, ApiRunResult, ExecuteApiReq,
    ImportCurlReq, ParsedCurl, RunSavedRequestReq, RunSavedRequestResp, UpsertApiAutomationReq,
    UpsertApiCollectionReq, UpsertApiEnvironmentReq, UpsertApiRequestReq,
};
use otto_core::domain::{
    ApiAutomation, ApiCollection, ApiEnvironment, ApiHistoryEntry, ApiHistorySummary, ApiRequest,
    WorkspaceRole,
};
use otto_core::event::Event;
use otto_core::{Error, Id};
use otto_state::api_client::ApiHistoryQuery;
use otto_state::{
    ApiClientRepo, NewApiAutomation, NewApiCollection, NewApiEnvironment, NewApiRequest,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::api_helpers::{collection_to_openapi, parse_curl};
use crate::auth::{require_ws_role, CurrentAuthContext, CurrentUser};
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;

// The engine's names stay importable from here (`routes::api_client::…`) for
// the handler groups below and the server's other API-client routes.
pub(crate) use otto_apiclient::engine::*;

// Handler groups live in `routes/api_client/*`; re-exported so
// `routes/mod.rs` (and every other importer) keeps the old paths.
mod automations;
mod collections;
mod ctx;
mod environments;
mod history;
mod requests;
pub use automations::*;
pub use collections::*;
pub use environments::*;
pub use history::*;
pub use requests::*;

/// Hard cap for a request's `extras` extension object (serialized).
const EXTRAS_MAX_BYTES: usize = 256 * 1024;

/// Hard cap for `GET .../history?limit=`.
const HISTORY_MAX: i64 = 500;
const HISTORY_DEFAULT: i64 = 100;
/// In-band marker appended to an agent-shaped request body that hit the cap.
const AGENT_BODY_TRUNCATED: &str = "\n…[truncated]";

fn repo(ctx: &ServerCtx) -> ApiClientRepo {
    ApiClientRepo::new(ctx.pool.clone())
}

// ===========================================================================
// Agent-facing overview
// ===========================================================================

#[derive(Debug, Deserialize)]
pub struct OverviewQuery {
    pub q: Option<String>,
    pub collection_id: Option<Id>,
    pub kind: Option<String>,
}

/// `GET /workspaces/{wid}/api-client/overview`
pub async fn overview(
    Path(wid): Path<Id>,
    Query(query): Query<OverviewQuery>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<ApiOverview>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Viewer).await?;
    let repo = repo(&ctx);
    let collections = repo.list_collections(&wid).await?;
    let requests = repo
        .list_request_summaries(&wid, query.collection_id.as_ref())
        .await?;
    let environments = repo.list_environments(&wid).await?;
    let automations = repo.list_automations(&wid).await?;
    Ok(Json(build_overview(
        collections,
        requests,
        environments,
        automations,
        query.q.as_deref(),
        query.kind.as_deref().unwrap_or("all"),
    )))
}

/// Build the compact, secret-free API-client discovery view.
pub(crate) fn build_overview(
    collections: Vec<ApiCollection>,
    requests: Vec<ApiOverviewRequest>,
    environments: Vec<ApiEnvironment>,
    automations: Vec<ApiAutomation>,
    q: Option<&str>,
    kind: &str,
) -> ApiOverview {
    let needle = q.unwrap_or_default().to_ascii_lowercase();
    let matches = |values: &[&str]| {
        needle.is_empty()
            || values
                .iter()
                .any(|value| value.to_ascii_lowercase().contains(&needle))
    };
    let include_requests = matches!(kind, "all" | "requests");
    let include_environments = matches!(kind, "all" | "environments");
    let include_automations = matches!(kind, "all" | "automations");

    ApiOverview {
        collections: if include_requests {
            collections
                .into_iter()
                .filter(|collection| matches(&[&collection.name]))
                .map(|collection| ApiOverviewCollection {
                    id: collection.id,
                    name: collection.name,
                    parent_id: collection.parent_id,
                })
                .collect()
        } else {
            Vec::new()
        },
        requests: if include_requests {
            requests
                .into_iter()
                .filter(|request| matches(&[&request.name, &request.method, &request.url]))
                .collect()
        } else {
            Vec::new()
        },
        environments: if include_environments {
            environments
                .into_iter()
                .filter(|environment| matches(&[&environment.name]))
                .map(|environment| ApiOverviewEnvironment {
                    id: environment.id,
                    name: environment.name,
                    is_active: environment.is_active,
                    variables: api_secrets::mask_variables(&environment.variables),
                    secret_keys: environment.secret_keys,
                })
                .collect()
        } else {
            Vec::new()
        },
        automations: if include_automations {
            automations
                .into_iter()
                .filter(|automation| matches(&[&automation.name]))
                .map(|automation| ApiOverviewAutomation {
                    id: automation.id,
                    name: automation.name,
                    steps: automation.steps.as_array().map_or(0, Vec::len),
                })
                .collect()
        } else {
            Vec::new()
        },
    }
}

/// `GET /workspaces/{wid}/api-client/storage` — history / run-report sizes
/// (cached 60 s per workspace). Read-only; nothing is ever deleted here.
pub async fn storage(
    Path(wid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<ApiClientStorage>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Viewer).await?;
    Ok(Json(storage_gauge(&repo(&ctx), &wid).await?))
}

/// `POST /workspaces/{wid}/api-client/storage/prune` — apply the workspace's
/// CONFIGURED retention now (`settings.api_client.history_max_rows` /
/// `history_max_days` / `automation_runs_keep`), instead of waiting for the
/// next Send or run. With no limit configured it deletes nothing — retention
/// stays opt-in. Returns the fresh gauge.
pub async fn prune_storage(
    Path(wid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<ApiClientStorage>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    let repo = repo(&ctx);
    let (max_rows, max_days) = history_retention(&ctx, &wid).await;
    if max_rows > 0 || max_days > 0 {
        repo.prune_history(&wid, max_rows, max_days).await?;
    }
    let keep = super::api_automation_runs::runs_keep(&ctx, &wid).await;
    if keep > 0 {
        let runs = otto_state::api_runs::ApiRunsRepo(ctx.pool.clone());
        for automation in repo.run_automation_ids(&wid).await? {
            runs.prune_runs(&wid, &automation, keep)
                .await
                .map_err(|e| ApiError(Error::Internal(format!("prune api runs: {e}"))))?;
        }
    }
    invalidate_storage_gauge(&wid);
    Ok(Json(storage_gauge(&repo, &wid).await?))
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
// Postman account sync
// ===========================================================================

/// Keychain ref holding the user's Postman API key (blob key `api_key`) when
/// they opt into "remember" — resyncs then need no key re-entry.
const POSTMAN_SECRET_REF: &str = "apiclient-postman";

/// Hard cap on fetched collections/environments per sync — the Postman API is
/// rate-limited (~60 req/min on free plans; one call per item), so an
/// unbounded account would stall the request and burn the quota.
const POSTMAN_SYNC_MAX: usize = 200;

#[derive(Debug, Deserialize)]
pub struct PostmanSyncReq {
    /// Postman API key (postman.co → Settings → API keys). Optional when a
    /// previous sync stored one with `remember`.
    pub api_key: Option<String>,
    /// Persist the key in the macOS Keychain for future one-click resyncs.
    #[serde(default)]
    pub remember: bool,
}

#[derive(Debug, serde::Serialize)]
pub struct PostmanSyncFailure {
    pub name: String,
    pub error: String,
}

#[derive(Debug, serde::Serialize)]
pub struct PostmanSyncResult {
    /// Full Postman v2.1 collection documents, importable as-is.
    pub collections: Vec<Value>,
    /// Postman environment exports (`{name, values: […]}`).
    pub environments: Vec<Value>,
    /// Items the Postman API listed but failed to deliver (rate limits, perms).
    pub failed: Vec<PostmanSyncFailure>,
    /// Whether a remembered key exists after this call.
    pub remembered: bool,
}

/// One authenticated GET against the Postman API, JSON-decoded. Errors carry
/// the Postman error message when the body has one (their 4xx bodies do).
async fn postman_get(client: &reqwest::Client, key: &str, url: &str) -> Result<Value, String> {
    // The API key rides the request header — refuse anything but TLS so an
    // interpolated uid can never downgrade the transport.
    if !url.starts_with("https://") {
        return Err("postman api url must be https".into());
    }
    let resp = client
        .get(url)
        .header("x-api-key", key)
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        let msg = body
            .pointer("/error/message")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        return Err(if msg.is_empty() {
            format!("HTTP {status}")
        } else {
            format!("HTTP {status}: {msg}")
        });
    }
    Ok(body)
}

/// `POST /workspaces/{wid}/api-client/postman/sync` — fetch EVERY collection
/// and environment from the user's Postman ACCOUNT (api.getpostman.com) in one
/// shot, so nothing has to be exported file-by-file. Returns the raw Postman
/// documents; the UI funnels them through its existing import pipeline (same
/// mapping as file import — one parser, no drift). The fixed public host is
/// daemon-chosen (never user input), so the SSRF guard does not apply.
pub async fn postman_sync(
    Path(wid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<PostmanSyncReq>,
) -> ApiResult<Json<PostmanSyncResult>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;

    // Resolve the key: explicit in the request, else the remembered one.
    let stored = api_secrets::load_blob_async(&ctx.secrets, POSTMAN_SECRET_REF).await;
    let key = match req
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|k| !k.is_empty())
    {
        Some(k) => k.to_string(),
        None => stored.get("api_key").cloned().ok_or_else(|| {
            ApiError(Error::Invalid(
                "no Postman API key — provide one (postman.co → Settings → API keys)".into(),
            ))
        })?,
    };

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| ApiError(Error::Internal(format!("http client: {e}"))))?;

    // Listing first — an invalid key fails fast here with Postman's message.
    let listing = postman_get(&client, &key, "https://api.getpostman.com/collections")
        .await
        .map_err(|e| ApiError(Error::Upstream(format!("Postman API: {e}"))))?;

    let mut collections = Vec::new();
    let mut environments = Vec::new();
    let mut failed = Vec::new();

    let listed = listing["collections"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if listed.len() > POSTMAN_SYNC_MAX {
        failed.push(PostmanSyncFailure {
            name: format!("(cap) {} more collections", listed.len() - POSTMAN_SYNC_MAX),
            error: format!("sync fetches at most {POSTMAN_SYNC_MAX} per run"),
        });
    }
    for item in listed.iter().take(POSTMAN_SYNC_MAX) {
        let uid = item["uid"].as_str().unwrap_or_default();
        let name = item["name"].as_str().unwrap_or(uid).to_string();
        match postman_get(
            &client,
            &key,
            &format!("https://api.getpostman.com/collections/{uid}"),
        )
        .await
        {
            Ok(mut v) => match v.get_mut("collection") {
                Some(c) => collections.push(c.take()),
                None => failed.push(PostmanSyncFailure {
                    name,
                    error: "response had no 'collection'".into(),
                }),
            },
            Err(error) => failed.push(PostmanSyncFailure { name, error }),
        }
    }

    // Environments are optional extras — a failure to LIST them shouldn't sink
    // the collections that already fetched fine.
    match postman_get(&client, &key, "https://api.getpostman.com/environments").await {
        Ok(listing) => {
            let listed = listing["environments"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            for item in listed.iter().take(POSTMAN_SYNC_MAX) {
                let uid = item["uid"].as_str().unwrap_or_default();
                let name = item["name"].as_str().unwrap_or(uid).to_string();
                match postman_get(
                    &client,
                    &key,
                    &format!("https://api.getpostman.com/environments/{uid}"),
                )
                .await
                {
                    Ok(mut v) => match v.get_mut("environment") {
                        Some(e) => environments.push(e.take()),
                        None => failed.push(PostmanSyncFailure {
                            name,
                            error: "response had no 'environment'".into(),
                        }),
                    },
                    Err(error) => failed.push(PostmanSyncFailure { name, error }),
                }
            }
        }
        Err(error) => failed.push(PostmanSyncFailure {
            name: "(environments)".into(),
            error,
        }),
    }

    // Remember the key only after it proved valid (the listing succeeded).
    let mut remembered = stored.contains_key("api_key");
    if req.remember && req.api_key.is_some() {
        let mut blob = BTreeMap::new();
        blob.insert("api_key".to_string(), key);
        api_secrets::store_blob_async(&ctx.secrets, POSTMAN_SECRET_REF, &blob).await?;
        remembered = true;
    }

    Ok(Json(PostmanSyncResult {
        collections,
        environments,
        failed,
        remembered,
    }))
}

/// `GET /workspaces/{wid}/api-client/cookies` — list the CALLER's jar in this
/// workspace (never another user's captured sessions).
pub async fn list_cookies(
    Path(wid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Value>> {
    // Editor-gated: cookie values are live credentials (session tokens), not
    // something a read-only viewer should be able to exfiltrate.
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    let jar = cookie_jar(&jar_scope(&wid, &user.id));
    let store = jar
        .lock()
        .map_err(|_| ApiError(Error::Internal("cookie jar poisoned".into())))?;
    let cookies: Vec<Value> = store
        .iter_any()
        .map(|c| {
            json!({
                "name": c.name(),
                "value": c.value(),
                "domain": c.domain().unwrap_or(""),
                "path": c.path().unwrap_or("/"),
            })
        })
        .collect();
    Ok(Json(Value::Array(cookies)))
}

/// `DELETE /workspaces/{wid}/api-client/cookies` — clear the caller's jar in
/// this workspace.
pub async fn clear_cookies(
    Path(wid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<StatusCode> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    let jar = cookie_jar(&jar_scope(&wid, &user.id));
    jar.lock()
        .map_err(|_| ApiError(Error::Internal("cookie jar poisoned".into())))?
        .clear();
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct OAuth2TokenReq {
    grant: String,
    token_url: String,
    #[serde(default)]
    client_id: String,
    /// Plain string, or a `{"$secret": …}` marker when the stored request's
    /// Keychain-backed value is being replayed (resolved server-side).
    #[serde(default)]
    client_secret: Value,
    #[serde(default)]
    scope: String,
    #[serde(default)]
    username: String,
    #[serde(default)]
    password: Value,
    #[serde(default)]
    refresh_token: Value,
    /// Confirms sending a `$secret`-marker value to a token endpoint other
    /// than the one saved on the owning request (person's credential only).
    #[serde(default)]
    confirm_new_host: bool,
}

/// `POST /workspaces/{wid}/api-client/oauth2/token` — perform an OAuth 2.0
/// token request (client_credentials / password / refresh_token) server-side
/// and return the token JSON. Authorization-code grant (browser redirect) is
/// not handled here.
pub async fn oauth2_token(
    Path(wid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    CurrentAuthContext(auth): CurrentAuthContext,
    headers: HeaderMap,
    Json(req): Json<OAuth2TokenReq>,
) -> ApiResult<Json<Value>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;

    // Secret ↔ host binding: a stored client_secret / password /
    // refresh_token belongs to the token endpoint saved on its request — the
    // caller picks `token_url` here, so any other host needs a person's
    // confirmation before the marker is resolved.
    if let Some(host) = unbound_oauth_secret_host(
        &repo(&ctx),
        &wid,
        &req.token_url,
        [&req.client_secret, &req.password, &req.refresh_token],
    )
    .await
    {
        let agent = is_agent_caller(&headers, &auth);
        if agent || !req.confirm_new_host {
            return Err(new_host_conflict(&host, agent).into());
        }
    }

    // Secret-shaped fields may arrive as `$secret` markers (stored request
    // auth replayed by the UI) — resolve them from the Keychain, workspace-
    // checked, exactly like execute.
    let client_secret =
        resolve_marker_or_string(&ctx, &wid, &req.client_secret, "client_secret").await?;
    let password = resolve_marker_or_string(&ctx, &wid, &req.password, "password").await?;
    let refresh_token =
        resolve_marker_or_string(&ctx, &wid, &req.refresh_token, "refresh_token").await?;

    let mut form: Vec<(&str, &str)> = Vec::new();
    match req.grant.as_str() {
        "client_credentials" => form.push(("grant_type", "client_credentials")),
        "password" => {
            form.push(("grant_type", "password"));
            form.push(("username", &req.username));
            form.push(("password", &password));
        }
        "refresh_token" => {
            form.push(("grant_type", "refresh_token"));
            form.push(("refresh_token", &refresh_token));
        }
        other => {
            return Err(ApiError(Error::Invalid(format!(
                "unsupported grant: {other}"
            ))));
        }
    }
    if !req.scope.is_empty() {
        form.push(("scope", &req.scope));
    }
    if !req.client_id.is_empty() {
        form.push(("client_id", &req.client_id));
    }
    if !client_secret.is_empty() {
        form.push(("client_secret", &client_secret));
    }

    // SSRF guard: never let the token endpoint point at an internal address —
    // unless the workspace opted in to local/private targets, exactly like the
    // requests the token is for (a dev Keycloak on localhost is the usual case).
    let allow_local = workspace_allows_local(&ctx, &wid).await;
    if !allow_local {
        otto_netguard::check_url(&req.token_url)
            .await
            .map_err(|m| ApiError(Error::Invalid(m)))?;
    }

    // A dedicated client: no redirects — a 307/308 would resend the form (the
    // client secret / password) to wherever the endpoint points, around the
    // host binding above — and the connect-time SSRF guard unless opted out.
    let client = if allow_local {
        reqwest::Client::builder()
    } else {
        otto_netguard::guarded_client_builder()
    }
    .user_agent("Otto-ApiClient/1.0")
    .redirect(reqwest::redirect::Policy::none())
    .build()
    .map_err(|e| ApiError(Error::Internal(format!("http client: {e}"))))?;
    let mut resp = client
        .post(&req.token_url)
        .header("Accept", "application/json")
        .timeout(OAUTH_TOKEN_TIMEOUT)
        .form(&form)
        .send()
        .await
        .map_err(|e| {
            ApiError(Error::Upstream(describe_reqwest_error(
                &e,
                OAUTH_TOKEN_TIMEOUT,
            )))
        })?;
    let status = resp.status().as_u16();
    // A token response is a small JSON document; read at most a bounded
    // prefix so a misconfigured URL (a 50 MB download) can't balloon memory.
    let mut raw = Vec::new();
    while let Ok(Some(chunk)) = resp.chunk().await {
        raw.extend_from_slice(&chunk[..chunk.len().min(OAUTH_TOKEN_BODY_MAX + 1 - raw.len())]);
        if raw.len() > OAUTH_TOKEN_BODY_MAX {
            break;
        }
    }
    let text = String::from_utf8_lossy(&raw).into_owned();
    let parsed: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if status >= 400 || parsed.get("access_token").is_none() {
        // A misbehaving endpoint may echo the form back — never the secrets.
        let message = api_secrets::scrub_str(
            &oauth_error_message(status, &parsed, &text),
            &[
                client_secret.clone(),
                password.clone(),
                refresh_token.clone(),
            ],
        );
        return Err(ApiError(Error::Upstream(message)));
    }
    Ok(Json(parsed))
}

/// Budget for one OAuth 2.0 token request (connect → full body).
const OAUTH_TOKEN_TIMEOUT: Duration = Duration::from_secs(30);
/// Largest token-endpoint body read.
const OAUTH_TOKEN_BODY_MAX: usize = 256 * 1024;
/// Raw body excerpt quoted in a token error when the endpoint sent no
/// `error` / `error_description` (an HTML login page, a proxy error).
const OAUTH_ERROR_EXCERPT: usize = 300;

/// The message for a failed token request: the endpoint's own
/// `error_description` / `error` when present, else the status and a short
/// excerpt of the body (never the whole page).
pub(crate) fn oauth_error_message(status: u16, parsed: &Value, text: &str) -> String {
    if let Some(msg) = parsed
        .get("error_description")
        .or_else(|| parsed.get("error"))
        .and_then(Value::as_str)
        .filter(|m| !m.trim().is_empty())
    {
        return msg.to_string();
    }
    let mut excerpt = text.trim().to_string();
    let cut = excerpt.len() > OAUTH_ERROR_EXCERPT;
    truncate_string(&mut excerpt, OAUTH_ERROR_EXCERPT);
    if cut {
        excerpt.push('…');
    }
    if status < 400 {
        format!("token endpoint returned {status} without an access_token: {excerpt}")
    } else {
        format!("token endpoint returned {status}: {excerpt}")
    }
}

/// Resolve one string-or-marker field (`oauth2/token`): plain strings pass
/// through; a `{"$secret": "otto.api.request.<id>"}` marker is looked up in
/// the Keychain after verifying the referenced request lives in `wid`. The
/// blob member is `field` (member names match the oauth2 auth object).
async fn resolve_marker_or_string(
    ctx: &ServerCtx,
    wid: &Id,
    v: &Value,
    field: &str,
) -> Result<String, ApiError> {
    match v {
        Value::Null => Ok(String::new()),
        Value::String(s) => Ok(s.clone()),
        other => {
            let Some(r) = api_secrets::marker_ref(other) else {
                return Err(ApiError(Error::Invalid(format!(
                    "{field} must be a string or a $secret marker"
                ))));
            };
            let rid = api_secrets::parse_request_ref(r).ok_or_else(|| {
                ApiError(Error::Invalid(format!(
                    "{field} has an unsupported secret ref"
                )))
            })?;
            let request = repo(ctx).get_request(&rid.to_string()).await.map_err(|_| {
                ApiError(Error::Invalid(format!(
                    "{field} references an unknown request"
                )))
            })?;
            ensure_in_workspace(&request.workspace_id, wid)?;
            let blob = api_secrets::load_blob_async(&ctx.secrets, r).await;
            Ok(blob.get(field).cloned().unwrap_or_default())
        }
    }
}

/// `POST /workspaces/{wid}/api-client/requests/{id}/execute`
pub async fn run_saved_request(
    Path((wid, id)): Path<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    CurrentAuthContext(auth): CurrentAuthContext,
    headers: HeaderMap,
    Json(req): Json<RunSavedRequestReq>,
) -> ApiResult<Json<RunSavedRequestResp>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    run_saved(&ctx, &wid, &id, &user, &auth, &headers, req)
        .await
        .map(Json)
        .map_err(ApiError::from)
}

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
    CurrentAuthContext(auth): CurrentAuthContext,
    headers: HeaderMap,
    Json(req): Json<ExecuteApiReq>,
) -> ApiResult<Json<ApiResponse>> {
    execute_service(&ctx, &wid, &user, &auth, &headers, req)
        .await
        .map(Json)
        .map_err(ApiError::from)
}

// ===========================================================================
// secure-all (bulk lazy-migration sweep)
// ===========================================================================

/// `POST /workspaces/{wid}/api-client/secure-all` — sweep every saved request
/// and environment in the workspace, moving plaintext secrets to the Keychain
/// in one pass (the bulk counterpart of the lazy on-save migration). Requests:
/// every secret auth member. Environments: variables whose NAME is
/// secret-shaped become `secret_keys`. Idempotent — markers already in place
/// are left alone. No automatic sweep ever runs at boot (Keychain prompts at
/// daemon start are a known launchd/E2E hazard).
pub async fn secure_all(
    Path(wid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Value>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    let (requests_secured, env_keys_secured) =
        secure_all_sweep(&repo(&ctx), &ctx.secrets, &wid).await?;
    if requests_secured > 0 {
        notify_changed(&ctx, &wid, "request", None, false);
    }
    if env_keys_secured > 0 {
        notify_changed(&ctx, &wid, "environment", None, false);
    }
    Ok(Json(json!({
        "requests_secured": requests_secured,
        "env_keys_secured": env_keys_secured,
    })))
}

/// The sweep itself, separated from the HTTP handler so it is testable
/// against a bare repo + secret store.
async fn secure_all_sweep(
    repo: &ApiClientRepo,
    secrets: &std::sync::Arc<dyn otto_core::secrets::SecretStore>,
    wid: &Id,
) -> Result<(usize, usize), ApiError> {
    let mut requests_secured = 0usize;
    for request in repo.list_requests(wid, None).await? {
        let _credential_guard = api_secrets::request_guard(&request.id).await;
        let request = repo.get_request(&request.id).await?;
        let own_ref = api_secrets::request_ref(&request.id);
        let existing_blob = api_secrets::load_blob_checked_async(secrets, &own_ref).await?;
        let (auth_row, blob) =
            api_secrets::split_auth_secrets(&request.auth, &own_ref, &existing_blob)
                .map_err(|m| ApiError(Error::Internal(m)))?;
        if auth_row == request.auth {
            continue; // nothing plaintext left — already secured (idempotency)
        }
        api_secrets::store_blob_async(secrets, &own_ref, &blob).await?;
        repo.update_request(
            &request.id,
            NewApiRequest {
                id: None,
                workspace_id: request.workspace_id.clone(),
                collection_id: request.collection_id.clone(),
                name: request.name.clone(),
                method: request.method.clone(),
                url: request.url.clone(),
                headers: request.headers.clone(),
                query: request.query.clone(),
                body_mode: request.body_mode.clone(),
                body: request.body.clone(),
                auth: auth_row,
                ssh_connection_id: request.ssh_connection_id.clone(),
                extras: request.extras.clone(),
                position: request.position,
            },
        )
        .await?;
        requests_secured += 1;
    }

    let mut env_keys_secured = 0usize;
    for env in repo.list_environments(wid).await? {
        let candidates: Vec<(String, String)> = env
            .variables
            .as_object()
            .map(|o| {
                o.iter()
                    .filter(|(k, v)| {
                        api_secrets::secret_shaped(k)
                            && v.as_str().is_some_and(|s| !s.is_empty())
                            && !env.secret_keys.contains(*k)
                    })
                    .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string()))
                    .collect()
            })
            .unwrap_or_default();
        if candidates.is_empty() {
            continue;
        }
        let sref = api_secrets::env_ref(&env.id);
        let mut blob = api_secrets::load_blob_async(secrets, &sref).await;
        let mut secret_keys = env.secret_keys.clone();
        for (k, v) in &candidates {
            blob.insert(k.clone(), v.clone());
            secret_keys.push(k.clone());
        }
        api_secrets::store_blob_async(secrets, &sref, &blob).await?;
        let vars = api_secrets::strip_secret_variables(&env.variables, &secret_keys);
        repo.update_environment(&env.id, None, Some(&vars), Some(&secret_keys))
            .await?;
        env_keys_secured += candidates.len();
    }

    Ok((requests_secured, env_keys_secured))
}

// ===========================================================================
// shared helpers
// ===========================================================================

/// Merge the server-owned agent authorship stamp into request extras.
/// `session_id` is `None` for an outward MCP caller (no Otto session).
pub(crate) fn stamp_agent(extras: Option<Value>, session_id: Option<&str>) -> Value {
    let mut extras = extras
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_else(|| serde_json::Map::from_iter([("v".into(), json!(1))]));
    extras.insert(
        "agent".into(),
        json!({
            "session_id": session_id,
            "at": chrono::Utc::now().to_rfc3339(),
        }),
    );
    Value::Object(extras)
}

/// 404 when an entity belongs to a different workspace than the path's `wid`.
fn ensure_in_workspace(entity_ws: &Id, wid: &Id) -> Result<(), ApiError> {
    if entity_ws == wid {
        Ok(())
    } else {
        Err(ApiError(Error::NotFound("not in this workspace".into())))
    }
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
#[path = "../../tests/unit/api_client.rs"]
mod tests;
