//! OAuth 2.1 sign-in for a governed remote MCP server (the MCP authorization
//! spec): protected-resource metadata → authorization-server metadata →
//! dynamic client registration when offered (else the client id / secret the
//! person typed) → authorization code + PKCE (S256) in the SYSTEM browser,
//! redirected to a one-shot loopback listener on `127.0.0.1:<random port>`
//! with a state check → tokens, kept ONLY in the Keychain
//! ([`token_key`]; the DB holds nothing). The governed client attaches the
//! access token itself ([`crate::service::McpService`]); agents never see it.
//! A token near expiry is refreshed (a rotated refresh token replaces the old
//! one); a 401 refreshes once and retries; sign-out deletes the tokens.
//!
//! Every discovery, registration and token request is https through the
//! SSRF guard; only a server whose registered URL is itself an owner-named
//! local target may name local endpoints (a local dev server with its own
//! local authorization server), and nothing else ever may. Errors never carry a token, a code or a
//! response body — only the OAuth `error` code.

use std::time::Duration;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// How long the loopback listener waits for the browser to come back.
pub const SIGN_IN_WINDOW: Duration = Duration::from_secs(300);
/// A token this close to expiry is refreshed before use.
const REFRESH_EARLY_SECS: i64 = 60;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
/// The most a discovery, registration or token answer may be (decoded).
const MAX_BODY_BYTES: usize = 1024 * 1024;

/// A JSON answer read in chunks up to [`MAX_BODY_BYTES`] (a compressed body
/// can't inflate past it); `Null` for anything larger or not JSON.
async fn read_json(mut resp: reqwest::Response) -> Value {
    let mut bytes = Vec::new();
    while let Ok(Some(chunk)) = resp.chunk().await {
        if chunk.len() > MAX_BODY_BYTES - bytes.len() {
            return Value::Null;
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

/// The Keychain entry of a server's OAuth tokens and client registration.
pub fn token_key(server_id: &str) -> String {
    format!("otto/mcp-oauth/{server_id}")
}

/// What the Keychain keeps for one signed-in server.
#[derive(Clone, Serialize, Deserialize)]
pub struct Tokens {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// Unix seconds; `None` = the server gave no lifetime.
    #[serde(default)]
    pub expires_at: Option<i64>,
    pub token_endpoint: String,
    pub client_id: String,
    #[serde(default)]
    pub client_secret: Option<String>,
    /// The MCP server (RFC 8707 `resource`).
    pub resource: String,
    /// Shown to the person ("signed in at …").
    pub auth_host: String,
}

impl std::fmt::Debug for Tokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Tokens { .. }") // never print a token
    }
}

impl Tokens {
    pub fn needs_refresh(&self, now: i64) -> bool {
        self.expires_at
            .is_some_and(|t| t - REFRESH_EARLY_SECS <= now)
    }
}

/// Random URL-safe value (state, PKCE verifier): 32 bytes.
pub(crate) fn random_token() -> String {
    URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>())
}

/// RFC 7636 S256 challenge of `verifier`.
pub(crate) fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// Where the server the person registered lives: which local endpoints its
/// metadata may name.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Scope {
    /// `localhost` / a loopback literal: a dev server on this Mac.
    Loopback,
    /// An owner-named private target (LAN, tailnet).
    Local,
    Remote,
}

fn is_loopback(url: &reqwest::Url) -> bool {
    let host = url.host_str().unwrap_or_default();
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    host.eq_ignore_ascii_case("localhost")
        || bare
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

fn scope_of(url: &reqwest::Url) -> Scope {
    if is_loopback(url) {
        Scope::Loopback
    } else if otto_netguard::is_explicit_local_target(url) {
        Scope::Local
    } else {
        Scope::Remote
    }
}

fn server_scope(server_url: &str) -> Result<Scope, String> {
    let parsed = reqwest::Url::parse(server_url).map_err(|e| format!("invalid url: {e}"))?;
    Ok(scope_of(&parsed))
}

/// A URL a sign-in may call: a loopback endpoint only for a loopback server,
/// a private one only for a private (or loopback) server, and everything
/// but loopback over https past the SSRF guard's literal check — whatever
/// the server's metadata says.
fn checked_endpoint(raw: &str, server: Scope) -> Result<reqwest::Url, String> {
    let url = reqwest::Url::parse(raw)
        .map_err(|_| "the authorization server named an invalid URL".to_string())?;
    let allowed = match scope_of(&url) {
        Scope::Loopback => server == Scope::Loopback,
        Scope::Local => server != Scope::Remote,
        Scope::Remote => true,
    };
    if !allowed {
        return Err("the authorization server named a local address; refused".into());
    }
    if scope_of(&url) != Scope::Loopback && url.scheme() != "https" {
        return Err("the authorization server named a non-https endpoint; refused".into());
    }
    if scope_of(&url) == Scope::Remote && !otto_netguard::check_url_literal(&url) {
        // Link-local / metadata / 0.0.0.0 / NAT64… literals never reach a resolver.
        return Err("the authorization server named a blocked address; refused".into());
    }
    Ok(url)
}

/// The client for one request: the host is resolved and vetted once and
/// those addresses are pinned (no second DNS answer, no redirect, reaches
/// metadata or Otto's own daemon; a remote one no private address either).
async fn client_for(target: &reqwest::Url) -> Result<reqwest::Client, String> {
    let local = scope_of(target) != Scope::Remote;
    otto_netguard::pinned_client_builder_for(target.as_str(), local)
        .await?
        .timeout(REQUEST_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| format!("http client: {e}"))
}

/// The authorization server's endpoints for a server.
#[derive(Debug, Clone)]
pub struct AuthServer {
    pub authorization_endpoint: reqwest::Url,
    pub token_endpoint: reqwest::Url,
    pub registration_endpoint: Option<reqwest::Url>,
}

/// `resource_metadata="<url>"` from a 401's `WWW-Authenticate`. Pure.
pub(crate) fn resource_metadata_hint(header: &str) -> Option<String> {
    let at = header.find("resource_metadata=")? + "resource_metadata=".len();
    let rest = &header[at..];
    let value = if let Some(q) = rest.strip_prefix('"') {
        q.split('"').next()?
    } else {
        rest.split([',', ' ']).next()?
    };
    (!value.is_empty()).then(|| value.to_string())
}

/// RFC 8414 / 9728 well-known URL with the path inserted after the prefix:
/// `https://h/a/b` + `oauth-protected-resource` → `https://h/.well-known/oauth-protected-resource/a/b`.
pub(crate) fn well_known(base: &reqwest::Url, name: &str) -> reqwest::Url {
    let mut u = base.clone();
    let path = base.path().trim_end_matches('/');
    u.set_path(&format!("/.well-known/{name}{path}"));
    u.set_query(None);
    u
}

async fn get_json(url: reqwest::Url) -> Option<Value> {
    let resp = client_for(&url)
        .await
        .ok()?
        .get(url)
        .header("Accept", "application/json")
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    Some(read_json(resp).await).filter(|v| !v.is_null())
}

/// Find the authorization server of the MCP server at `server_url`.
pub async fn discover(server_url: &str) -> Result<AuthServer, String> {
    // The metadata of a cleartext server could be rewritten on the path.
    otto_netguard::require_tls_or_loopback(server_url)?;
    let local = server_scope(server_url)?;
    if local == Scope::Remote {
        otto_netguard::check_url(server_url).await?;
    }
    let server = reqwest::Url::parse(server_url).map_err(|e| format!("invalid url: {e}"))?;
    // 1. An unauthenticated request: its 401 may point at the metadata.
    let probe = client_for(&server)
        .await?
        .post(server.clone())
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .json(&json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"otto","version":"1"}}}))
        .send()
        .await
        .ok();
    let hint = probe
        .as_ref()
        .and_then(|r| r.headers().get("www-authenticate"))
        .and_then(|v| v.to_str().ok())
        .and_then(resource_metadata_hint);
    // 2. Protected-resource metadata → its authorization server.
    let mut prm = None;
    if let Some(h) = hint {
        if let Ok(u) = checked_endpoint(&h, local) {
            prm = get_json(u).await;
        }
    }
    if prm.is_none() {
        prm = get_json(well_known(&server, "oauth-protected-resource")).await;
    }
    let issuer = match prm
        .as_ref()
        .and_then(|p| p["authorization_servers"][0].as_str())
    {
        Some(i) => checked_endpoint(i, local)?,
        // An older server (2025-03-26): its own origin is the authorization server.
        None => {
            let mut o = server.clone();
            o.set_path("/");
            o.set_query(None);
            o
        }
    };
    // 3. Authorization-server metadata (OAuth, then OpenID Connect).
    let meta = match get_json(well_known(&issuer, "oauth-authorization-server")).await {
        Some(m) => m,
        None => get_json(well_known(&issuer, "openid-configuration"))
            .await
            .ok_or("the server's authorization server publishes no metadata")?,
    };
    let s256 = meta["code_challenge_methods_supported"]
        .as_array()
        .is_some_and(|m| m.iter().any(|v| v == "S256"));
    if !s256 {
        return Err("the authorization server doesn't support PKCE (S256); refused".into());
    }
    let ep = |k: &str| meta[k].as_str().map(|s| checked_endpoint(s, local));
    Ok(AuthServer {
        authorization_endpoint: ep("authorization_endpoint")
            .ok_or("no authorization_endpoint")??,
        token_endpoint: ep("token_endpoint").ok_or("no token_endpoint")??,
        registration_endpoint: ep("registration_endpoint").transpose()?,
    })
}

/// RFC 7591 dynamic client registration of a public client.
pub async fn register(
    endpoint: &reqwest::Url,
    redirect_uri: &str,
) -> Result<(String, Option<String>), String> {
    let resp = client_for(endpoint)
        .await?
        .post(endpoint.clone())
        .json(&json!({
            "client_name": "Otto",
            "redirect_uris": [redirect_uri],
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
            "token_endpoint_auth_method": "none",
        }))
        .send()
        .await
        .map_err(|_| "client registration failed (network)".to_string())?;
    let status = resp.status();
    let v = read_json(resp).await;
    let id = v["client_id"].as_str().filter(|_| status.is_success());
    match id {
        Some(id) => Ok((
            id.to_string(),
            v["client_secret"].as_str().map(str::to_string),
        )),
        None => Err(format!("client registration refused ({status})")),
    }
}

/// The token endpoint's answer → [`Tokens`] (keeping `prev`'s refresh token
/// when the server didn't rotate it). Errors carry only the OAuth error code.
async fn token_request(
    server_url: &str,
    endpoint: &str,
    form: Vec<(&str, String)>,
    client_secret: Option<&str>,
    client_id: &str,
) -> Result<Value, String> {
    let url = checked_endpoint(endpoint, server_scope(server_url)?)?;
    let mut rb = client_for(&url)
        .await?
        .post(url)
        .header("Accept", "application/json");
    if let Some(secret) = client_secret {
        rb = rb.basic_auth(client_id, Some(secret));
    }
    let resp = rb
        .form(&form)
        .send()
        .await
        .map_err(|_| "the token request failed (network)".to_string())?;
    let status = resp.status();
    let v = read_json(resp).await;
    if status.is_success() && v["access_token"].is_string() {
        return Ok(v);
    }
    let code = v["error"].as_str().unwrap_or("unknown_error");
    let code: String = code
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .take(40)
        .collect();
    Err(format!(
        "the authorization server refused the token request ({code})"
    ))
}

fn tokens_from(v: &Value, base: &Tokens, now: i64) -> Tokens {
    Tokens {
        access_token: v["access_token"].as_str().unwrap_or_default().to_string(),
        refresh_token: v["refresh_token"]
            .as_str()
            .map(str::to_string)
            .or_else(|| base.refresh_token.clone()),
        expires_at: v["expires_in"].as_i64().map(|s| now + s),
        ..base.clone()
    }
}

/// Exchange the authorization code (PKCE verifier, the exact redirect URI).
pub async fn exchange(pending: &Pending, code: &str, now: i64) -> Result<Tokens, String> {
    let mut form = vec![
        ("grant_type", "authorization_code".to_string()),
        ("code", code.to_string()),
        ("redirect_uri", pending.redirect_uri.clone()),
        ("code_verifier", pending.verifier.clone()),
        ("resource", pending.base.resource.clone()),
    ];
    if pending.base.client_secret.is_none() {
        form.push(("client_id", pending.base.client_id.clone()));
    }
    let v = token_request(
        &pending.base.resource,
        &pending.base.token_endpoint,
        form,
        pending.base.client_secret.as_deref(),
        &pending.base.client_id,
    )
    .await?;
    Ok(tokens_from(&v, &pending.base, now))
}

/// Refresh (rotation honoured). `Err` = the grant is gone: sign in again.
pub async fn refresh(t: &Tokens, now: i64) -> Result<Tokens, String> {
    let rt = t
        .refresh_token
        .clone()
        .ok_or("no refresh token; sign in again")?;
    let mut form = vec![
        ("grant_type", "refresh_token".to_string()),
        ("refresh_token", rt),
        ("resource", t.resource.clone()),
    ];
    if t.client_secret.is_none() {
        form.push(("client_id", t.client_id.clone()));
    }
    let v = token_request(
        &t.resource,
        &t.token_endpoint,
        form,
        t.client_secret.as_deref(),
        &t.client_id,
    )
    .await?;
    Ok(tokens_from(&v, t, now))
}

/// A started sign-in: what the callback needs to finish it.
pub struct Pending {
    pub state: String,
    pub verifier: String,
    pub redirect_uri: String,
    /// Everything but the tokens themselves.
    pub base: Tokens,
    pub listener: tokio::net::TcpListener,
}

/// Start a sign-in for the server at `server_url`: discover, register (or
/// take `manual` client credentials), bind the loopback listener and build
/// the authorization URL. Returns the URL for the browser and the pending flow.
pub async fn start(
    server_url: &str,
    manual: Option<(String, Option<String>)>,
) -> Result<(String, Pending), String> {
    let asrv = discover(server_url).await?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| format!("loopback listener: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let redirect_uri = format!("http://127.0.0.1:{port}/callback");
    let (client_id, client_secret) = match (manual, &asrv.registration_endpoint) {
        (Some((id, secret)), _) if !id.trim().is_empty() => {
            (id.trim().to_string(), secret.filter(|s| !s.is_empty()))
        }
        (_, Some(reg)) => register(reg, &redirect_uri).await?,
        _ => {
            return Err(
                "the authorization server offers no client registration: enter a client id".into(),
            )
        }
    };
    let state = random_token();
    let verifier = random_token();
    let mut url = asrv.authorization_endpoint.clone();
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", &client_id)
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("state", &state)
        .append_pair("code_challenge", &challenge(&verifier))
        .append_pair("code_challenge_method", "S256")
        .append_pair("resource", server_url);
    let auth_host = asrv
        .authorization_endpoint
        .host_str()
        .unwrap_or_default()
        .to_string();
    let base = Tokens {
        access_token: String::new(),
        refresh_token: None,
        expires_at: None,
        token_endpoint: asrv.token_endpoint.to_string(),
        client_id,
        client_secret,
        resource: server_url.to_string(),
        auth_host,
    };
    Ok((
        url.to_string(),
        Pending {
            state,
            verifier,
            redirect_uri,
            base,
            listener,
        },
    ))
}

/// Constant-time string equality (the state check).
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

/// `code` from `GET /callback?code=…&state=…` when the state matches. Pure.
pub(crate) fn callback_code(request_line: &str, state: &str) -> Result<String, String> {
    let target = request_line
        .split_whitespace()
        .nth(1)
        .ok_or("bad request")?;
    let url =
        reqwest::Url::parse(&format!("http://127.0.0.1{target}")).map_err(|_| "bad request")?;
    if url.path() != "/callback" {
        return Err("not the callback".into());
    }
    let q: std::collections::HashMap<String, String> = url.query_pairs().into_owned().collect();
    if !q.get("state").is_some_and(|s| same(s, state)) {
        return Err("the sign-in answer didn't match (state); refused".into());
    }
    if let Some(e) = q.get("error") {
        let e: String = e
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
            .take(40)
            .collect();
        return Err(format!("sign-in was declined ({e})"));
    }
    q.get("code")
        .filter(|c| !c.is_empty())
        .cloned()
        .ok_or_else(|| "no code in the answer".into())
}

/// Wait (≤ [`SIGN_IN_WINDOW`]) for the browser's redirect, answer it with a
/// short page, then exchange the code. One answer ends the flow either way.
pub async fn finish(pending: Pending, now: impl Fn() -> i64) -> Result<Tokens, String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let wait = async {
        loop {
            let (mut s, _) = pending.listener.accept().await.map_err(|e| e.to_string())?;
            let mut buf = vec![0u8; 8192];
            let n = s.read(&mut buf).await.unwrap_or(0);
            let text = String::from_utf8_lossy(&buf[..n]).to_string();
            let line = text.lines().next().unwrap_or_default().to_string();
            // A favicon or a stray probe: not the answer, keep waiting.
            if !line.contains("/callback") {
                let _ = s
                    .write_all(
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    )
                    .await;
                continue;
            }
            let result = callback_code(&line, &pending.state);
            let body = match &result {
                Ok(_) => "<p>Signed in. You can close this tab and go back to Otto.</p>",
                Err(_) => "<p>Sign-in failed. Go back to Otto and try again.</p>",
            };
            let page = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n{body}",
                body.len()
            );
            let _ = s.write_all(page.as_bytes()).await;
            return result;
        }
    };
    let code = tokio::time::timeout(SIGN_IN_WINDOW, wait)
        .await
        .map_err(|_| "sign-in timed out".to_string())??;
    exchange(&pending, &code, now()).await
}

#[cfg(test)]
mod tests;
