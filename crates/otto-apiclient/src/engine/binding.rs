//! The secret ↔ host binding, caller attribution and the small request/response
//! shaping helpers the execute paths share.

use super::*;

/// Safe, read-only methods that need no explicit execute confirmation.
pub fn is_safe_method(method: &str) -> bool {
    matches!(
        method.to_ascii_uppercase().as_str(),
        "GET" | "HEAD" | "OPTIONS"
    )
}

/// Lower-cased host from an absolute HTTP URL.
pub fn host_of(url: &str) -> Option<String> {
    reqwest::Url::parse(url)
        .ok()?
        .host_str()
        .map(str::to_ascii_lowercase)
}

/// Whether a saved request carries the server-owned agent authorship stamp.
pub fn is_agent_authored(request: &ApiRequest) -> bool {
    request
        .extras
        .as_ref()
        .and_then(|extras| extras.get("agent"))
        .is_some_and(Value::is_object)
}

/// Hosts established by human-authored saved requests or human/legacy runs,
/// from full rows — the reference the narrow production path
/// ([`hosts_from_urls`] over `known_host_urls`) is tested against.
#[cfg(test)]
pub fn known_hosts(
    requests: &[ApiRequest],
    history: &[ApiHistoryEntry],
    vars: &serde_json::Map<String, Value>,
) -> BTreeSet<String> {
    let request_urls: Vec<String> = requests
        .iter()
        .filter(|request| !is_agent_authored(request))
        .map(|request| request.url.clone())
        .collect();
    let history_urls: Vec<String> = history
        .iter()
        .filter(|entry| {
            entry
                .request
                .pointer("/source/kind")
                .and_then(Value::as_str)
                != Some("agent")
        })
        .map(|entry| entry.url.clone())
        .collect();
    hosts_from_urls(&request_urls, &history_urls, vars)
}

/// [`known_hosts`] over urls already filtered to human-authored requests and
/// human / legacy runs (see `ApiClientRepo::known_host_urls`). Request urls
/// get the workspace variables; run urls are already resolved.
pub fn hosts_from_urls(
    request_urls: &[String],
    history_urls: &[String],
    vars: &serde_json::Map<String, Value>,
) -> BTreeSet<String> {
    let mut hosts = BTreeSet::new();
    for url in request_urls {
        if let Some(host) = host_of(&substitute(url, vars)) {
            hosts.insert(host);
        }
    }
    for url in history_urls {
        if let Some(host) = host_of(url) {
            hosts.insert(host);
        }
    }
    hosts
}

/// Cap the response body for agent consumption and remove binary duplication.
pub fn shape_agent(response: &mut ApiResponse) {
    if response.body.len() > AGENT_BODY_MAX {
        truncate_string(&mut response.body, AGENT_BODY_MAX);
        response.truncated = true;
    }
    response.body_base64.clear();
}

/// Collect names and values of every secret resolved for one saved send.
pub fn collect_secrets(
    env_secret_blob: &BTreeMap<String, String>,
    secret_keys: &[String],
    resolved_auth: &Value,
) -> (Vec<String>, Vec<String>) {
    let mut names = Vec::new();
    let mut values = Vec::new();
    for key in secret_keys {
        if let Some(value) = env_secret_blob.get(key) {
            names.push(key.clone());
            values.push(value.clone());
        }
    }
    let auth_type = resolved_auth
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("none");
    for member in api_secrets::secret_members(auth_type) {
        if let Some(value) = resolved_auth.get(*member).and_then(Value::as_str) {
            if !value.is_empty() {
                names.push(format!("auth.{member}"));
                values.push(value.to_string());
            }
        }
    }
    (names, values)
}

// ===========================================================================
// Secret ↔ host binding for caller-built requests (ad-hoc execute, streams,
// OAuth token). A stored secret may only travel to the host it belongs to:
//   - a `{"$secret": "otto.api.request.<id>"}` marker → the host of that saved
//     request's URL (after variable substitution);
//   - a Keychain-backed ENVIRONMENT variable → the hosts of the workspace's
//     human-authored saved requests (the environment's established targets).
// Anything else needs an explicit `confirm_new_host`, which only a person's
// credential can give — an agent can never self-confirm.
// ===========================================================================

/// `true` when the caller is an agent: the bridge/MCP-executor headers
/// (`caller_source`), an Otto-minted managed-session credential, or a
/// restricted MCP token. Such callers can't confirm a new-host secret send.
pub fn is_agent_caller(headers: &HeaderMap, auth: &AuthContext) -> bool {
    let (source, _) = caller_source(headers);
    source.get("kind").and_then(Value::as_str) == Some("agent")
        || auth.managed_session_id.is_some()
        || auth.mcp_only
}

/// Probe value substituted for every environment secret by [`uses_env_secret`];
/// control characters keep it from ever colliding with real request text.
pub(crate) const SECRET_PROBE: &str = "\u{1}otto-secret-probe\u{1}";

/// Whether sending `req` would expand any Keychain-backed environment variable
/// — directly (`{{API_TOKEN}}`), nested through another variable, or via a
/// runtime override that references one — anywhere in the URL, query,
/// headers, body or auth. Only keys whose resolved value is still the
/// Keychain value count (a runtime override that REPLACED a secret key
/// carries no secret).
pub fn uses_env_secret(
    req: &ExecuteApiReq,
    vars: &serde_json::Map<String, Value>,
    env_blob: &BTreeMap<String, String>,
    secret_keys: &[String],
) -> bool {
    let mut probe = vars.clone();
    let mut any = false;
    for key in secret_keys {
        let Some(secret) = env_blob.get(key) else {
            continue;
        };
        if vars.get(key).and_then(Value::as_str) == Some(secret.as_str()) {
            probe.insert(key.clone(), Value::String(SECRET_PROBE.into()));
            any = true;
        }
    }
    if !any {
        return false;
    }
    let body = if req.body_mode == "form" {
        percent_decode(&req.body)
    } else {
        req.body.clone()
    };
    let text = serde_json::to_string(&json!([req.url, req.query, req.headers, body, req.auth]))
        .unwrap_or_default();
    substitute(&text, &probe).contains(SECRET_PROBE)
}

/// Hosts an environment's secrets are bound to: the hosts of the workspace's
/// human-authored saved requests (substituted with the same variables).
/// `urls` are the raw urls of those requests
/// ([`ApiClientRepo::request_host_urls`] — a one-column read, not full rows).
pub fn env_secret_hosts(
    urls: &[String],
    vars: &serde_json::Map<String, Value>,
) -> BTreeSet<String> {
    urls.iter()
        .filter_map(|url| host_of(&substitute(url, vars)))
        .collect()
}

/// [`env_secret_hosts`] for `wid` with `bind_vars` — what an agent-started
/// automation computes ONCE per run ([`StepSecretBinding::bound_hosts`]).
pub async fn bound_env_secret_hosts(
    repo: &ApiClientRepo,
    wid: &Id,
    bind_vars: &serde_json::Map<String, Value>,
) -> EngineResult<BTreeSet<String>> {
    let urls = repo.request_host_urls(wid).await?;
    Ok(env_secret_hosts(&urls, bind_vars))
}

/// The host a stored secret in `req` would be sent to OUTSIDE its binding
/// (see the section comment), or `None` when every secret stays home (or none
/// is used). Marker refs that are malformed / foreign are skipped here —
/// [`resolve_exec_auth`] rejects those outright.
///
/// `vars` is the EFFECTIVE map the send will use (environment + runtime
/// overrides / dataset / chained values) — it decides the target host and
/// which secrets are expanded. `bind_vars` is the environment's OWN map,
/// without caller-supplied overrides: the binding side (a marker owner's host,
/// the human-authored requests' hosts) is resolved with it, so overriding
/// `{{base_url}}` can't move the binding along with the target.
pub async fn unbound_secret_host(
    repo: &ApiClientRepo,
    wid: &Id,
    req: &ExecuteApiReq,
    vars: &serde_json::Map<String, Value>,
    bind_vars: &serde_json::Map<String, Value>,
    env_blob: &BTreeMap<String, String>,
    secret_keys: &[String],
) -> EngineResult<Option<String>> {
    unbound_secret_host_with(repo, wid, req, vars, bind_vars, env_blob, secret_keys, None).await
}

/// [`unbound_secret_host`] with the environment-secret host set precomputed
/// (`bound_hosts`, from [`bound_env_secret_hosts`] with the same `bind_vars`)
/// — `None` reads it when an env secret is actually used.
#[allow(clippy::too_many_arguments)]
pub async fn unbound_secret_host_with(
    repo: &ApiClientRepo,
    wid: &Id,
    req: &ExecuteApiReq,
    vars: &serde_json::Map<String, Value>,
    bind_vars: &serde_json::Map<String, Value>,
    env_blob: &BTreeMap<String, String>,
    secret_keys: &[String],
    bound_hosts: Option<&BTreeSet<String>>,
) -> EngineResult<Option<String>> {
    let Some(target) = host_of(&substitute(&req.url, vars)) else {
        return Ok(None);
    };
    let refs: Vec<String> = req
        .auth
        .as_object()
        .map(|o| {
            o.values()
                .filter_map(api_secrets::marker_ref)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    for r in &refs {
        let Some(rid) = api_secrets::parse_request_ref(r) else {
            continue;
        };
        let Ok(owner) = repo.get_request(&rid.to_string()).await else {
            continue;
        };
        if &owner.workspace_id != wid {
            continue;
        }
        if host_of(&substitute(&owner.url, bind_vars)).as_deref() != Some(target.as_str()) {
            return Ok(Some(target));
        }
    }
    if uses_env_secret(req, vars, env_blob, secret_keys) {
        let bound = match bound_hosts {
            Some(hosts) => hosts.contains(&target),
            None => bound_env_secret_hosts(repo, wid, bind_vars)
                .await?
                .contains(&target),
        };
        if !bound {
            return Ok(Some(target));
        }
    }
    Ok(None)
}

/// The 409 for a secret leaving its bound host. `agent` callers are told a
/// person has to send it; people are told how to confirm.
pub fn new_host_conflict(host: &str, agent: bool) -> EngineError {
    let how = if agent {
        "a person must send it from the Otto UI (agent credentials cannot confirm)"
    } else {
        "re-send with confirm_new_host:true"
    };
    EngineError::Conflict(format!(
        "needs_confirm=new_host: a stored secret would be sent to host '{host}', which it is not bound to; {how}"
    ))
}

/// OAuth flavour of [`unbound_secret_host`]: each `$secret` marker among
/// `values` is bound to the `token_url` saved on its owning request (same
/// string, or same host). Returns the offending target host. Malformed /
/// foreign refs are skipped here — `resolve_marker_or_string` rejects them.
pub async fn unbound_oauth_secret_host(
    repo: &ApiClientRepo,
    wid: &Id,
    token_url: &str,
    values: [&Value; 3],
) -> Option<String> {
    for value in values {
        let Some(r) = api_secrets::marker_ref(value) else {
            continue;
        };
        let Some(rid) = api_secrets::parse_request_ref(r) else {
            continue;
        };
        let Ok(owner) = repo.get_request(&rid.to_string()).await else {
            continue;
        };
        if &owner.workspace_id != wid {
            continue;
        }
        let saved = owner
            .auth
            .get("token_url")
            .and_then(Value::as_str)
            .unwrap_or("");
        if !oauth_token_url_bound(saved, token_url) {
            return Some(host_of(token_url).unwrap_or_else(|| token_url.to_string()));
        }
    }
    None
}

/// Whether `requested` is the token endpoint `saved` on the secret's request:
/// the same string, or the same (parseable) host.
pub fn oauth_token_url_bound(saved: &str, requested: &str) -> bool {
    if saved == requested {
        return true;
    }
    match (host_of(saved), host_of(requested)) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

/// Audit source derived from the inward bridge's session header, or — for an
/// outward MCP client, which has no Otto session — the executor's `X-Otto-Agent`
/// marker. Either way the caller is an agent, and the new-host rule applies.
pub fn caller_source(headers: &HeaderMap) -> (Value, Option<Id>) {
    let hdr = |name: &str| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    match (hdr("x-otto-session"), hdr("x-otto-agent")) {
        (Some(session_id), _) => (
            json!({"kind": "agent", "session_id": session_id}),
            Some(session_id),
        ),
        (None, Some(via)) => (
            json!({"kind": "agent", "session_id": null, "via": via}),
            None,
        ),
        (None, None) => (json!({"kind": "human", "session_id": null}), None),
    }
}
