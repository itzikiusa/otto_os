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
        || auth.mcp_internal
        || auth.mcp_session_id.is_some()
}

/// `Some(session)` when the caller is an agent ([`is_agent_caller`]), with the
/// session it acts for: the bridge's `X-Otto-Session` header, else the
/// credential's own session binding (`None` for an outward MCP client). `None`
/// for a person. A save stamps `extras.agent` from this — by CREDENTIAL, so a
/// raw PATCH with a session's token counts as the agent it is.
pub fn agent_author(headers: &HeaderMap, auth: &AuthContext) -> Option<Option<Id>> {
    if !is_agent_caller(headers, auth) {
        return None;
    }
    let (_, session) = caller_source(headers);
    Some(
        session
            .or_else(|| auth.managed_session_id.clone())
            .or_else(|| auth.mcp_session_id.clone()),
    )
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
///
/// Without the secret store this can't see the url a person recorded for a
/// `$secret` marker, so it applies the legacy rule only (see
/// [`marker_bound_host`]); the engine's own paths call
/// [`find_unbound_secret`] with the store.
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
    let found = find_unbound_secret(
        repo,
        None,
        wid,
        req,
        vars,
        bind_vars,
        env_blob,
        secret_keys,
        bound_hosts,
    )
    .await?;
    Ok(found.map(|unbound| unbound.target))
}

/// A stored secret on its way to a host it is not bound to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnboundSecret {
    /// The host the send would reach.
    pub target: String,
    /// The host the secret may go to; `None` for an environment secret (the
    /// workspace's saved-request hosts) or a marker with no trusted binding.
    pub bound: Option<String>,
}

/// The saved requests (in `wid`) whose `$secret` markers `req.auth` carries,
/// one per distinct ref. Malformed / foreign refs are skipped —
/// [`resolve_exec_auth`] rejects those outright.
pub(crate) async fn marker_owners(
    repo: &ApiClientRepo,
    wid: &Id,
    req: &ExecuteApiReq,
) -> Vec<ApiRequest> {
    let refs: BTreeSet<String> = req
        .auth
        .as_object()
        .map(|o| {
            o.values()
                .filter_map(api_secrets::marker_ref)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let mut owners = Vec::new();
    for r in &refs {
        let Some(rid) = api_secrets::parse_request_ref(r) else {
            continue;
        };
        let Ok(owner) = repo.get_request(&rid.to_string()).await else {
            continue;
        };
        if &owner.workspace_id == wid {
            owners.push(owner);
        }
    }
    owners
}

/// The host a `$secret` marker owned by `owner` may be sent to: the url
/// recorded in its Keychain `blob` ([`api_secrets::bound_url`] — set by a
/// person's save or send, never moved by an agent's edit). A legacy blob with
/// none recorded (or no store at hand, `None`) falls back to the owner's
/// current url — unless an agent last wrote the request, whose url then
/// proves nothing about where the secret belongs. `bind_vars` (the
/// environment's own variables) resolves `{{vars}}` in that url.
pub fn marker_bound_host(
    owner: &ApiRequest,
    blob: Option<&BTreeMap<String, String>>,
    bind_vars: &serde_json::Map<String, Value>,
) -> Option<String> {
    if let Some(url) = blob.and_then(api_secrets::bound_url) {
        return host_of(&substitute(url, bind_vars));
    }
    if is_agent_authored(owner) {
        return None;
    }
    host_of(&substitute(&owner.url, bind_vars))
}

/// The hosts an AGENT's send may take the `$secret` marker owned by `owner`
/// to once its binding is strict ([`strict_agent_binding`]): the hosts a
/// person's sends resolved it to ([`api_secrets::sent_hosts`]), plus the bound
/// url's host when the url names it outright — a `{{base}}` url then no longer
/// follows the environment an agent selects (F5).
pub fn agent_marker_hosts(
    owner: &ApiRequest,
    blob: Option<&BTreeMap<String, String>>,
) -> BTreeSet<String> {
    let mut hosts = blob.map(api_secrets::sent_hosts).unwrap_or_default();
    let url = match blob.and_then(api_secrets::bound_url) {
        Some(url) => Some(url),
        None if is_agent_authored(owner) => None,
        None => Some(owner.url.as_str()),
    };
    if let Some(host) = url.and_then(literal_host) {
        hosts.insert(host);
    }
    hosts
}

/// The host of `url` when no `{{variable}}` can change it (none in its scheme
/// or authority); `None` for a templated host.
fn literal_host(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if scheme.contains("{{") || authority.contains("{{") {
        return None;
    }
    host_of(url)
}

/// Whether an `agent`'s send is held to [`agent_marker_hosts`]: only once a
/// person's send has recorded a host for the binding. A binding recorded
/// before sent hosts existed (an upgrade) keeps following the selected
/// environment until a person sends it — unattended workflows and
/// automations keep working; the person's next send makes it strict.
pub fn strict_agent_binding(agent: bool, blob: Option<&BTreeMap<String, String>>) -> bool {
    agent && blob.is_some_and(|blob| !api_secrets::sent_hosts(blob).is_empty())
}

/// Whether the marker owned by `owner` may go to `target`: the bound url
/// resolved with the environment's own variables ([`marker_bound_host`]), or
/// for a strict agent binding [`agent_marker_hosts`] only.
pub fn marker_send_bound(
    agent: bool,
    owner: &ApiRequest,
    blob: Option<&BTreeMap<String, String>>,
    bind_vars: &serde_json::Map<String, Value>,
    target: &str,
) -> bool {
    if strict_agent_binding(agent, blob) {
        agent_marker_hosts(owner, blob).contains(target)
    } else {
        marker_bound_host(owner, blob, bind_vars).as_deref() == Some(target)
    }
}

/// The secret in `req` that would leave its bound host, if any (see the
/// section comment and [`marker_bound_host`]), under a PERSON's rule — see
/// [`find_unbound_secret_as`]. `secrets` lets the check read each marker's
/// recorded binding; `None` applies the legacy rule.
#[allow(clippy::too_many_arguments)]
pub async fn find_unbound_secret(
    repo: &ApiClientRepo,
    secrets: Option<&Arc<dyn SecretStore>>,
    wid: &Id,
    req: &ExecuteApiReq,
    vars: &serde_json::Map<String, Value>,
    bind_vars: &serde_json::Map<String, Value>,
    env_blob: &BTreeMap<String, String>,
    secret_keys: &[String],
    bound_hosts: Option<&BTreeSet<String>>,
) -> EngineResult<Option<UnboundSecret>> {
    find_unbound_secret_as(
        false,
        repo,
        secrets,
        wid,
        req,
        vars,
        bind_vars,
        env_blob,
        secret_keys,
        bound_hosts,
    )
    .await
}

/// [`find_unbound_secret`] for an `agent` caller or a person: an agent's
/// `$secret` markers are held to [`agent_marker_hosts`] ([`marker_send_bound`]).
#[allow(clippy::too_many_arguments)]
pub async fn find_unbound_secret_as(
    agent: bool,
    repo: &ApiClientRepo,
    secrets: Option<&Arc<dyn SecretStore>>,
    wid: &Id,
    req: &ExecuteApiReq,
    vars: &serde_json::Map<String, Value>,
    bind_vars: &serde_json::Map<String, Value>,
    env_blob: &BTreeMap<String, String>,
    secret_keys: &[String],
    bound_hosts: Option<&BTreeSet<String>>,
) -> EngineResult<Option<UnboundSecret>> {
    let Some(target) = host_of(&substitute(&req.url, vars)) else {
        return Ok(None);
    };
    for owner in marker_owners(repo, wid, req).await {
        let blob = match secrets {
            Some(store) => Some(
                api_secrets::load_blob_async(store, &api_secrets::request_ref(&owner.id)).await,
            ),
            None => None,
        };
        if !marker_send_bound(agent, &owner, blob.as_ref(), bind_vars, &target) {
            let bound = if strict_agent_binding(agent, blob.as_ref()) {
                agent_marker_hosts(&owner, blob.as_ref()).into_iter().next()
            } else {
                marker_bound_host(&owner, blob.as_ref(), bind_vars)
            };
            return Ok(Some(UnboundSecret { target, bound }));
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
            return Ok(Some(UnboundSecret {
                target,
                bound: None,
            }));
        }
    }
    Ok(None)
}

/// What a PERSON's send records as the binding of a marker owned by `owner`
/// (`Some(url)` only when it changes): a legacy blob the send relied on gets
/// the owner's url recorded; a send the person confirmed for a new host
/// (`confirmed_url`, the url they sent) rebinds the secret to it.
pub fn person_send_binding(
    owner: &ApiRequest,
    blob: &BTreeMap<String, String>,
    target: &str,
    bind_vars: &serde_json::Map<String, Value>,
    confirmed_url: Option<&str>,
) -> Option<String> {
    if !api_secrets::has_secret_values(blob) {
        return None;
    }
    if marker_bound_host(owner, Some(blob), bind_vars).as_deref() == Some(target) {
        return api_secrets::bound_url(blob)
            .is_none()
            .then(|| owner.url.clone());
    }
    confirmed_url.map(str::to_string)
}

/// Record [`person_send_binding`] for every marker `req` carries, under each
/// owner's credential lock. Best-effort: a locked or unreadable Keychain
/// leaves the binding as it was (the send itself was already allowed).
pub async fn record_person_bindings(
    repo: &ApiClientRepo,
    secrets: &Arc<dyn SecretStore>,
    wid: &Id,
    req: &ExecuteApiReq,
    vars: &serde_json::Map<String, Value>,
    bind_vars: &serde_json::Map<String, Value>,
    confirmed_url: Option<&str>,
) {
    let Some(target) = host_of(&substitute(&req.url, vars)) else {
        return;
    };
    for owner in marker_owners(repo, wid, req).await {
        let _guard = api_secrets::request_guard(&owner.id).await;
        let own_ref = api_secrets::request_ref(&owner.id);
        let Ok(mut blob) = api_secrets::load_blob_checked_async(secrets, &own_ref).await else {
            continue;
        };
        let rebind = person_send_binding(&owner, &blob, &target, bind_vars, confirmed_url);
        if let Some(url) = &rebind {
            api_secrets::set_bound_url(&mut blob, url);
        }
        let sent = person_sent_host(&owner, &mut blob, &target, bind_vars);
        if rebind.is_some() || sent {
            let _ = api_secrets::store_blob_async(secrets, &own_ref, &blob).await;
        }
    }
}

/// Record `target` as a host a person's send resolved the marker's binding
/// to (what an agent's later send may reach, [`agent_marker_hosts`]); `true`
/// when the blob changed. Only a send that stayed on its binding counts.
pub fn person_sent_host(
    owner: &ApiRequest,
    blob: &mut BTreeMap<String, String>,
    target: &str,
    bind_vars: &serde_json::Map<String, Value>,
) -> bool {
    api_secrets::has_secret_values(blob)
        && marker_bound_host(owner, Some(&*blob), bind_vars).as_deref() == Some(target)
        && api_secrets::add_sent_host(blob, target)
}

/// The 409 for a secret leaving its bound host. `agent` callers are told a
/// person has to send it; people are told how to confirm.
pub fn new_host_conflict(host: &str, agent: bool) -> EngineError {
    unbound_secret_conflict(
        &UnboundSecret {
            target: host.to_string(),
            bound: None,
        },
        agent,
    )
}

/// [`new_host_conflict`] naming the host the secret is bound to as well. The
/// target stays the FIRST `host '…'` in the text — the UI reads it from there.
pub fn unbound_secret_conflict(unbound: &UnboundSecret, agent: bool) -> EngineError {
    let how = if agent {
        "a person must send it from the Otto UI (agent credentials cannot confirm)"
    } else {
        "re-send with confirm_new_host:true"
    };
    let bound = match &unbound.bound {
        Some(bound) => format!(" (its bound host is '{bound}')"),
        None => String::new(),
    };
    EngineError::Conflict(format!(
        "needs_confirm=new_host: a stored secret would be sent to host '{}', which it is not bound to{bound}; {how}",
        unbound.target
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

/// What an approval card for running saved request `request_id` (in `wid`)
/// should name: its method and the host it goes to (see
/// [`approval_target`]), with the variables of `environment_id` — else the
/// active environment — substituted. Plain variables only: a card never reads
/// the Keychain. `None` when the request is unknown or in another workspace.
/// The governed invoke's arguments carry only the request id, so without this
/// the approver can't see where an agent's `api_execute` goes. (A pre-request
/// script can still retarget the send; the secret ↔ host binding covers that.)
pub async fn saved_request_target(
    repo: &ApiClientRepo,
    wid: &Id,
    request_id: &Id,
    environment_id: Option<&Id>,
) -> Option<String> {
    let request = repo.get_request(request_id).await.ok()?;
    if &request.workspace_id != wid {
        return None;
    }
    let env = match environment_id {
        Some(eid) => repo.get_environment(eid).await.ok(),
        None => repo.active_environment(wid).await.ok().flatten(),
    };
    let vars = env
        .filter(|env| &env.workspace_id == wid)
        .and_then(|env| env.variables.as_object().cloned())
        .unwrap_or_default();
    Some(approval_target(&request.method, &request.url, &vars))
}

/// `"POST api.example.com"`: the method and the host of `url` after `vars`;
/// the substituted url itself when it has no parseable host (a variable the
/// environment doesn't define).
pub fn approval_target(method: &str, url: &str, vars: &serde_json::Map<String, Value>) -> String {
    let url = substitute(url, vars);
    let target = host_of(&url).unwrap_or(url);
    format!("{} {target}", method.to_ascii_uppercase())
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
