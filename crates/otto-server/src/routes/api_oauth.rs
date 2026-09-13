//! Browser authorization-code + S256 PKCE, with one-use, expiring state.
//! Tokens are exchanged by the daemon and stored only in Keychain.
use crate::{
    api_secrets,
    auth::{require_ws_role, CurrentUser},
    error::{ApiError, ApiResult},
    state::ServerCtx,
};
use axum::{
    extract::{Path, Query, State},
    response::Html,
    Json,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use otto_core::{
    domain::{User, WorkspaceRole},
    Error, Id,
};
use otto_state::{ApiClientRepo, UsersRepo};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::Row;
use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

#[derive(Clone)]
struct Flow {
    wid: Id,
    actor: User,
    request_id: Id,
    auth: Value,
    revision: String,
    secret_fingerprint: String,
    verifier: String,
    redirect_uri: String,
    created: Instant,
    status: String,
    error: Option<String>,
}
fn flows() -> &'static Mutex<HashMap<String, Flow>> {
    static FLOWS: OnceLock<Mutex<HashMap<String, Flow>>> = OnceLock::new();
    FLOWS.get_or_init(|| Mutex::new(HashMap::new()))
}
fn fingerprint(blob: &std::collections::BTreeMap<String, String>) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(serde_json::to_vec(blob).unwrap_or_default()))
}
fn random_token() -> String {
    URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>())
}
fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}
fn purge(map: &mut HashMap<String, Flow>) {
    map.retain(|_, f| f.created.elapsed() < Duration::from_secs(600));
}
fn claim(map: &mut HashMap<String, Flow>, state: &str) -> Option<Flow> {
    purge(map);
    let flow = map.get_mut(state)?;
    if flow.status != "pending" {
        return None;
    }
    flow.status = "exchanging".into();
    Some(flow.clone())
}
fn unchanged_auth(current: &Value, expected: &Value) -> Result<(), String> {
    if current == expected {
        Ok(())
    } else {
        Err("Request authorization changed while the browser was open. Start again to avoid overwriting it.".into())
    }
}
fn invalid(message: &str) -> ApiError {
    ApiError(Error::Invalid(message.into()))
}
#[derive(Deserialize)]
pub struct StartReq {
    pub request_id: Id,
}
pub async fn start(
    Path(wid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<StartReq>,
) -> ApiResult<Json<Value>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    let _guard = api_secrets::request_guard(&req.request_id).await;
    let request = ApiClientRepo::new(ctx.pool.clone())
        .get_request(&req.request_id)
        .await?;
    if request.workspace_id != wid {
        return Err(ApiError(Error::NotFound("request".into())));
    }
    let secret_fingerprint = fingerprint(&api_secrets::load_blob_checked(
        ctx.secrets.as_ref(),
        &api_secrets::request_ref(&req.request_id),
    )?);
    let revision = request.updated_at.to_rfc3339();
    let auth = request.auth;
    if auth["type"] != "oauth2" || auth["grant"] != "authorization_code" {
        return Err(invalid("Save an OAuth authorization-code request first."));
    }
    let authorization_url = auth["authorization_url"].as_str().unwrap_or("");
    let token_url = auth["token_url"].as_str().unwrap_or("");
    otto_netguard::require_tls_or_loopback(authorization_url).map_err(|e| invalid(&e))?;
    otto_netguard::require_tls_or_loopback(token_url).map_err(|e| invalid(&e))?;
    let mut url =
        reqwest::Url::parse(authorization_url).map_err(|_| invalid("Invalid authorization URL"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(invalid("Authorization URL must use HTTP or HTTPS"));
    }
    // Token exchange retains the same SSRF default as other API client traffic.
    if !super::api_client::workspace_allows_local(&ctx, &wid).await {
        super::api_client::net_guard::check_url(token_url)
            .await
            .map_err(|e| invalid(&e))?;
    }
    if auth["client_id"].as_str().unwrap_or("").is_empty() {
        return Err(invalid("Client ID is required"));
    }
    let state = random_token();
    let verifier = random_token();
    let redirect_uri = format!(
        "{}/api/v1/api-client/oauth2/callback",
        ctx.base_url.trim_end_matches('/')
    );
    let extras: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(key, _)| {
            !matches!(
                key.as_ref(),
                "response_type"
                    | "client_id"
                    | "redirect_uri"
                    | "state"
                    | "code_challenge"
                    | "code_challenge_method"
                    | "scope"
            )
        })
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    url.set_query(None);
    url.set_fragment(None);
    url.query_pairs_mut().extend_pairs(extras);
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", auth["client_id"].as_str().unwrap_or(""))
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("state", &state)
        .append_pair("code_challenge", &challenge(&verifier))
        .append_pair("code_challenge_method", "S256");
    if let Some(scope) = auth["scope"].as_str().filter(|s| !s.is_empty()) {
        url.query_pairs_mut().append_pair("scope", scope);
    }
    let mut map = flows().lock().unwrap();
    purge(&mut map);
    if map.len() >= 256 {
        return Err(invalid(
            "Too many pending authorizations. Retry after an existing flow finishes.",
        ));
    }
    map.insert(
        state.clone(),
        Flow {
            wid,
            actor: user,
            request_id: req.request_id,
            auth,
            revision,
            secret_fingerprint,
            verifier,
            redirect_uri: redirect_uri.clone(),
            created: Instant::now(),
            status: "pending".into(),
            error: None,
        },
    );
    Ok(Json(
        json!({"flow_id":state,"authorization_url":url.as_str(),"redirect_uri":redirect_uri,"expires_in":600}),
    ))
}
#[derive(Deserialize)]
pub struct Callback {
    pub state: String,
    pub code: Option<String>,
    pub error: Option<String>,
}
pub async fn callback(
    Query(q): Query<Callback>,
    State(ctx): State<ServerCtx>,
) -> Html<&'static str> {
    let flow = {
        let mut map = flows().lock().unwrap();
        purge(&mut map);
        match claim(&mut map,&q.state) {
            Some(flow) => flow,
            None => return Html("<!doctype html><title>Authorization expired</title><p>This authorization expired or was already used. Return to Otto and start again.</p>"),
        }
    };
    // The exchange owns its lifetime: closing the browser tab must not cancel
    // credential persistence or leave a one-use flow stuck in "exchanging".
    let (done, wait) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let result = if q.error.is_some() {
            Err("Authorization was declined by the provider.".into())
        } else if let Some(code) = q.code {
            exchange(&ctx, &flow, &code).await
        } else {
            Err("Provider returned no authorization code.".into())
        };
        let success = result.is_ok();
        if let Some(flow) = flows().lock().unwrap().get_mut(&q.state) {
            flow.status = if success { "completed" } else { "failed" }.into();
            flow.error = result.err();
            flow.verifier.clear();
        }
        let _ = done.send(success);
    });
    let success = wait.await.unwrap_or(false);
    if success {
        Html("<!doctype html><title>Otto authorized</title><p>Authorization complete. The token is stored in Keychain. Return to Otto; you may close this tab.</p>")
    } else {
        Html("<!doctype html><title>Authorization failed</title><p>Authorization could not be completed. Return to Otto for details.</p>")
    }
}
async fn fresh_actor(pool: &sqlx::SqlitePool, id: &Id) -> Result<User, String> {
    let actor = UsersRepo::new(pool.clone())
        .get(id)
        .await
        .map_err(|_| "Authorization user no longer exists".to_string())?;
    if actor.disabled {
        return Err("Authorization user was disabled.".into());
    }
    Ok(actor)
}
async fn authorize_actor(ctx: &ServerCtx, flow: &Flow) -> Result<(), String> {
    let actor = fresh_actor(&ctx.pool, &flow.actor.id).await?;
    ctx.roles
        .check(&actor, &flow.wid, WorkspaceRole::Editor)
        .await
        .map_err(|e| e.to_string())?;
    let cap = otto_state::GrantsRepo::new(ctx.pool.clone())
        .capability_of(&actor, otto_core::domain::Feature::ApiClient)
        .await
        .map_err(|e| e.to_string())?;
    if cap < otto_core::domain::Capability::Edit {
        return Err("API Client access was revoked.".into());
    }
    Ok(())
}
async fn exchange(ctx: &ServerCtx, flow: &Flow, code: &str) -> Result<(), String> {
    authorize_actor(ctx, flow).await?;
    let mut auth = flow.auth.clone();
    {
        let _guard = api_secrets::request_guard(&flow.request_id).await;
        let blob = api_secrets::load_blob_checked(
            ctx.secrets.as_ref(),
            &api_secrets::request_ref(&flow.request_id),
        )
        .map_err(|e| e.to_string())?;
        if fingerprint(&blob) != flow.secret_fingerprint {
            return Err("Request credentials changed. Start authorization again.".into());
        }
        for member in api_secrets::secret_members("oauth2") {
            if let Some(reference) = api_secrets::marker_ref(&auth[*member]) {
                if reference != api_secrets::request_ref(&flow.request_id) {
                    return Err("Invalid request secret reference".into());
                }
                auth[*member] = json!(blob
                    .get(*member)
                    .ok_or("A stored credential is missing from Keychain")?);
            }
        }
    }
    let token_url = auth["token_url"].as_str().unwrap_or("");
    let allow_local = super::api_client::workspace_allows_local(ctx, &flow.wid).await;
    if !allow_local {
        super::api_client::net_guard::check_url(token_url).await?;
    }
    let mut form = vec![
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", flow.redirect_uri.as_str()),
        ("client_id", auth["client_id"].as_str().unwrap_or("")),
        ("code_verifier", flow.verifier.as_str()),
    ];
    if let Some(secret) = auth["client_secret"].as_str().filter(|s| !s.is_empty()) {
        form.push(("client_secret", secret));
    }
    otto_netguard::require_tls_or_loopback(token_url)?;
    let parsed = reqwest::Url::parse(token_url).map_err(|_| "Invalid token URL".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("Token endpoint must use HTTP or HTTPS".into());
    }
    let host = parsed.host_str().ok_or("Token URL needs a host")?;
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    let port = parsed.port_or_known_default().ok_or("Invalid token port")?;
    let addrs: Vec<std::net::SocketAddr> = tokio::net::lookup_host((bare, port))
        .await
        .map_err(|_| "Token endpoint DNS lookup failed".to_string())?
        .collect();
    if addrs.is_empty()
        || (!allow_local && addrs.iter().any(|a| otto_netguard::is_blocked_ip(a.ip())))
    {
        return Err("Token endpoint resolves to a blocked address".into());
    }
    let client = reqwest::Client::builder()
        .no_proxy()
        .resolve_to_addrs(host, &addrs)
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.to_string())?;
    let mut response = client
        .post(token_url)
        .form(&form)
        .send()
        .await
        .map_err(|e| e.without_url().to_string())?;
    if !response.status().is_success() {
        return Err(format!(
            "Token endpoint returned HTTP {}",
            response.status()
        ));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| e.without_url().to_string())?
    {
        if bytes.len() + chunk.len() > 1024 * 1024 {
            return Err("Token response exceeds 1 MiB".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let token: Value = serde_json::from_slice(&bytes)
        .map_err(|_| "Token endpoint returned invalid JSON".to_string())?;
    let access = token["access_token"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or("Token endpoint returned no access token")?;
    let _credential_guard = api_secrets::request_guard(&flow.request_id).await;
    authorize_actor(ctx, flow).await?;
    let own_ref = api_secrets::request_ref(&flow.request_id);
    // Take SQLite's write lock before checking auth, so a concurrent Save cannot
    // be overwritten during the Keychain/row update. Only auth columns change.
    let mut tx = ctx.pool.begin().await.map_err(|e| e.to_string())?;
    let changed =
        sqlx::query("UPDATE api_requests SET updated_at=updated_at WHERE id=? AND workspace_id=?")
            .bind(&flow.request_id)
            .bind(&flow.wid)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
    if changed.rows_affected() != 1 {
        return Err("Saved request was removed.".into());
    }
    let row = sqlx::query("SELECT auth_json,updated_at FROM api_requests WHERE id=?")
        .bind(&flow.request_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    let current: Value = serde_json::from_str(row.get("auth_json"))
        .map_err(|_| "Invalid stored auth".to_string())?;
    unchanged_auth(&current, &flow.auth)?;
    if row.get::<String, _>("updated_at") != flow.revision {
        return Err(
            "Saved request changed while authorization was in progress. Start again.".into(),
        );
    }
    let old_blob = api_secrets::load_blob_checked(ctx.secrets.as_ref(), &own_ref)
        .map_err(|e| e.to_string())?;
    if fingerprint(&old_blob) != flow.secret_fingerprint {
        return Err("Request credentials changed. Start authorization again.".into());
    }
    let mut updated = current;
    updated["access_token"] = json!(access);
    updated["token_type"] = json!(token["token_type"].as_str().unwrap_or("Bearer"));
    if let Some(refresh) = token["refresh_token"].as_str() {
        updated["refresh_token"] = json!(refresh);
    }
    let (stored, blob) = api_secrets::split_auth_secrets(&updated, &own_ref, &old_blob)?;
    sqlx::query("UPDATE api_requests SET auth_json=?,updated_at=? WHERE id=?")
        .bind(stored.to_string())
        .bind(chrono::Utc::now().to_rfc3339())
        .bind(&flow.request_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    api_secrets::store_blob(ctx.secrets.as_ref(), &own_ref, &blob).map_err(|e| e.to_string())?;
    if let Err(e) = tx.commit().await {
        let _ = api_secrets::store_blob(ctx.secrets.as_ref(), &own_ref, &old_blob);
        return Err(e.to_string());
    }
    Ok(())
}
pub async fn status(
    Path((wid, id)): Path<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Value>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    let map = flows().lock().unwrap();
    let flow = map
        .get(&id)
        .filter(|f| {
            f.wid == wid && f.actor.id == user.id && f.created.elapsed() < Duration::from_secs(600)
        })
        .ok_or_else(|| ApiError(Error::NotFound("OAuth flow expired".into())))?;
    Ok(Json(
        json!({"status":flow.status,"error":flow.error,"request_id":flow.request_id}),
    ))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn s256_matches_rfc7636_vector() {
        assert_eq!(
            challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }
    #[test]
    fn verifier_has_required_entropy_and_url_safe_encoding() {
        let a = random_token();
        assert_eq!(a.len(), 43);
        assert!(a
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
        assert_ne!(a, random_token());
    }
    fn sample_flow() -> Flow {
        Flow {
            wid: "w".into(),
            actor: User {
                id: "u".into(),
                username: "u".into(),
                display_name: "u".into(),
                is_root: false,
                disabled: false,
                created_at: chrono::Utc::now(),
            },
            request_id: "r".into(),
            auth: json!({}),
            revision: "r1".into(),
            secret_fingerprint: "fp".into(),
            verifier: "v".into(),
            redirect_uri: "http://localhost/callback".into(),
            created: Instant::now(),
            status: "pending".into(),
            error: None,
        }
    }
    #[test]
    fn callback_state_is_single_use_and_expires() {
        let mut map = HashMap::new();
        map.insert("s".into(), sample_flow());
        assert!(claim(&mut map, "s").is_some());
        assert!(claim(&mut map, "s").is_none());
        let mut expired = sample_flow();
        expired.created = Instant::now() - Duration::from_secs(601);
        map.insert("old".into(), expired);
        assert!(claim(&mut map, "old").is_none());
    }
    #[test]
    fn changed_authorization_is_not_overwritten() {
        assert!(unchanged_auth(&json!({"client_id":"new"}), &json!({"client_id":"old"})).is_err());
        assert!(unchanged_auth(&json!({"client_id":"same"}), &json!({"client_id":"same"})).is_ok());
    }

    #[tokio::test]
    async fn actor_is_reloaded_so_disabled_or_demoted_root_cannot_retain_privileges() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::query("CREATE TABLE users(id TEXT,username TEXT,display_name TEXT,is_root INTEGER,disabled INTEGER,created_at TEXT)").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO users VALUES ('u','u','u',1,0,?)")
            .bind(chrono::Utc::now().to_rfc3339())
            .execute(&pool)
            .await
            .unwrap();
        assert!(fresh_actor(&pool, &"u".into()).await.unwrap().is_root);
        sqlx::query("UPDATE users SET is_root=0")
            .execute(&pool)
            .await
            .unwrap();
        assert!(!fresh_actor(&pool, &"u".into()).await.unwrap().is_root);
        sqlx::query("UPDATE users SET disabled=1")
            .execute(&pool)
            .await
            .unwrap();
        assert!(fresh_actor(&pool, &"u".into()).await.is_err());
    }
}
