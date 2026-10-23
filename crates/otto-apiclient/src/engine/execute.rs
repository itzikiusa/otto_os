//! The two execute paths: ad-hoc `execute_service` and saved-request `run_saved`.

use super::*;

/// Run the saved request `id` (HTTP transport only) the way an agent or tool
/// replays it: its stored scripts, settings and GraphQL variables, the
/// environment (+ `vars` overrides), the confirmations (`confirm` for an
/// unsafe method, `confirm_new_host` for a new or unbound host), then send,
/// scrub every resolved secret and record history. The caller has checked
/// the Editor role (`POST …/requests/{id}/execute`).
pub async fn run_saved<C: ApiClientCtx>(
    ctx: &C,
    wid: &Id,
    id: &Id,
    user: &User,
    auth: &AuthContext,
    headers: &HeaderMap,
    req: RunSavedRequestReq,
) -> EngineResult<RunSavedRequestResp> {
    let repo = ctx.repo();
    let request = repo.get_request(id).await?;
    ensure_in_workspace(&request.workspace_id, wid)?;
    let extras = request.extras.clone().unwrap_or(Value::Null);
    let transport = extras
        .get("transport")
        .and_then(Value::as_str)
        .unwrap_or("http");
    if transport != "http" {
        return Err(EngineError::Invalid(format!(
            "'{transport}' requests are not runnable through saved execute (HTTP only)"
        )));
    }

    let mut exec = request_to_execute(&request);
    apply_extras_settings(&extras, &mut exec);
    let graphql_vars = if exec.body_mode == "graphql" {
        Some(parse_graphql_variables(&extras).map_err(EngineError::Invalid)?)
    } else {
        None
    };
    exec.timeout_ms = req
        .timeout_ms
        .or(exec.timeout_ms)
        .map(|timeout| timeout.clamp(1, 60_000));
    let (mut vars, environment, env_blob) =
        resolve_environment(ctx, &repo, wid, req.environment_id.as_ref()).await?;
    let env_vars = vars.clone();
    if let Some(overrides) = &req.vars {
        let overrides = overrides
            .as_object()
            .ok_or_else(|| EngineError::Invalid("vars must be a JSON object".into()))?;
        check_override_vars(overrides).map_err(EngineError::Invalid)?;
        for (key, value) in overrides {
            vars.insert(key.clone(), value.clone());
        }
    }

    // Pre-request script: same mutable request + variables surface as automation steps.
    let pre_code = extras
        .pointer("/scripts/pre")
        .and_then(Value::as_str)
        .unwrap_or("");
    if !pre_code.trim().is_empty() {
        let script_req = ScriptRequest {
            method: exec.method.clone(),
            url: exec.url.clone(),
            headers: exec.headers.clone(),
            body: exec.body.clone(),
        };
        let svars = string_vars(&vars);
        let out = api_scripts::run_pre_request_isolated(pre_code, &script_req, &svars).await;
        if let Some(error) = out.error {
            let error =
                api_secrets::scrub_str(&error, &env_blob.values().cloned().collect::<Vec<_>>());
            return Err(EngineError::Invalid(format!(
                "pre-request script failed: {error}"
            )));
        }
        if let Some(mutated) = out.request {
            exec.method = mutated.method;
            exec.url = mutated.url;
            exec.headers = mutated.headers;
            exec.body = mutated.body;
        }
        merge_string_vars(&mut vars, &out.vars);
    }

    if let Some(gql_vars) = graphql_vars {
        exec.body = json!({"query": exec.body, "variables": gql_vars}).to_string();
    }

    let env_secret_values: Vec<String> = env_blob.values().cloned().collect();
    let resolved_url_before_auth =
        api_secrets::scrub_str(&substitute(&exec.url, &vars), &env_secret_values);
    let method = exec.method.to_ascii_uppercase();
    if !is_safe_method(&method) && !req.confirm {
        return Err(EngineError::Conflict(format!(
            "needs_confirm=method: {method} '{}' → {resolved_url_before_auth} is not a safe method; re-send with confirm:true",
            request.name
        )));
    }

    if is_agent_authored(&request) && !req.confirm_new_host {
        if let Some(host) = host_of(&substitute(&exec.url, &vars)) {
            // Only the urls (r3-07-06) — not every request body and 500 full
            // history rows, ~35 MB of JSON per agent send at worst.
            let (request_urls, history_urls) = repo.known_host_urls(wid).await?;
            if !hosts_from_urls(&request_urls, &history_urls, &vars).contains(&host) {
                return Err(EngineError::Conflict(format!(
                    "needs_confirm=new_host: host '{host}' is not used by any human-authored request or run in this workspace; re-send with confirm_new_host:true"
                )));
            }
        }
    }

    // Secret ↔ host binding, exactly as ad-hoc execute: an agent can rewrite
    // a saved request (upsert), override `{{base_url}}` (`vars`) or mutate the
    // URL in a pre-request script, so a stored secret leaving its bound host
    // needs a PERSON's confirmation here too — the agent-authored new-host
    // speed bump above is self-confirmable and does not cover secrets.
    if let Some(host) = unbound_secret_host(
        &repo,
        wid,
        &exec,
        &vars,
        &env_vars,
        &env_blob,
        environment
            .as_ref()
            .map(|env| env.secret_keys.as_slice())
            .unwrap_or(&[]),
    )
    .await?
    {
        let agent = is_agent_caller(headers, auth);
        if agent || !req.confirm_new_host {
            return Err(new_host_conflict(&host, agent));
        }
    }

    exec = resolve_exec_auth(&repo, ctx.secrets(), wid, &exec)
        .await
        .map_err(EngineError::Invalid)?;
    let (secret_names, secret_values) = collect_secrets(
        &env_blob,
        environment
            .as_ref()
            .map(|env| env.secret_keys.as_slice())
            .unwrap_or(&[]),
        &exec.auth,
    );
    let resolved_url = api_secrets::scrub_str(&substitute(&exec.url, &vars), &secret_values);
    let request_snapshot = json!({
        "request_id": request.id,
        "name": request.name,
        "method": exec.method,
        "url": api_secrets::scrub_str(&exec.url, &secret_values),
        "headers": api_secrets::mask_kv_rows(&exec.headers),
        "query": api_secrets::mask_kv_rows(&exec.query),
        "body_mode": exec.body_mode,
        "body": api_secrets::scrub_str(&exec.body, &secret_values),
        "auth": api_secrets::redact_auth(&exec.auth),
        "environment_id": req.environment_id,
        "ssh_connection_id": exec.ssh_connection_id,
    });
    let (source, session_id) = caller_source(headers);
    let proxy = match resolve_socks_proxy(ctx, wid, exec.ssh_connection_id.as_ref(), &user.id).await
    {
        Ok(proxy) => proxy,
        Err(error) => {
            let error = api_secrets::scrub_str(&error, &secret_values);
            record_history(
                ctx,
                &repo,
                NewApiHistory {
                    workspace_id: wid.clone(),
                    method: method.clone(),
                    url: resolved_url,
                    status: None,
                    duration_ms: None,
                    request: request_snapshot,
                    response: json!({"error": error}),
                },
                &source,
                session_id,
                Some(request.id.clone()),
            )
            .await;
            return Err(EngineError::Upstream(error));
        }
    };
    let allow_local = workspace_allows_local(ctx, wid).await;
    let mut response = match build_and_send(
        &jar_scope(wid, &user.id),
        &exec,
        &vars,
        proxy.as_deref(),
        allow_local,
    )
    .await
    {
        Ok((mut response, raw)) => {
            // Image preview only: this route serves agents/tools, never the
            // UI's "Save to disk", so nothing is parked for download.
            attach_body(&mut response, raw, None);
            response
        }
        Err(error) => {
            let error = api_secrets::scrub_str(&error, &secret_values);
            record_history(
                ctx,
                &repo,
                NewApiHistory {
                    workspace_id: wid.clone(),
                    method: method.clone(),
                    url: resolved_url,
                    status: None,
                    duration_ms: None,
                    request: request_snapshot,
                    response: json!({"error": error}),
                },
                &source,
                session_id,
                Some(request.id.clone()),
            )
            .await;
            return Err(EngineError::Upstream(error));
        }
    };

    let mut warnings = Vec::new();
    let mut tests = Vec::new();
    let post_code = extras
        .pointer("/scripts/post")
        .and_then(Value::as_str)
        .unwrap_or("");
    if !post_code.trim().is_empty() {
        let mut response_headers = BTreeMap::new();
        if let Some(headers) = response.headers.as_array() {
            for header in headers {
                let key = header.get("key").and_then(Value::as_str).unwrap_or("");
                let value = header.get("value").and_then(Value::as_str).unwrap_or("");
                if !key.is_empty() {
                    response_headers.insert(key.to_ascii_lowercase(), value.to_string());
                }
            }
        }
        let script_response = ScriptResponse {
            code: response.status,
            status: response.status_text.clone(),
            response_time: response.duration_ms,
            headers: response_headers,
            body_text: response.body.clone(),
        };
        let out = api_scripts::run_post_response_isolated(
            post_code,
            &script_response,
            &string_vars(&vars),
        )
        .await;
        if out.error.is_none() {
            merge_string_vars(&mut vars, &out.vars);
        }
        tests.extend(
            out.tests
                .into_iter()
                .map(|test| json!({"desc": test.name, "passed": test.passed})),
        );
        if let Some(error) = out.error {
            warnings.push(format!("post-response script failed: {error}"));
        }
    }

    api_secrets::scrub_secrets(&mut response, &secret_values);
    if req.shape.as_deref() == Some("agent") {
        shape_agent(&mut response);
    }
    let jwt_claims = if req.decode_jwt == Some(false) {
        None
    } else {
        let claims = api_secrets::jwt_claims(&response.body);
        if claims.is_some() {
            response.body = api_secrets::mask_jwts(&response.body);
            if let Some(headers) = response.headers.as_array_mut() {
                for header in headers {
                    if let Some(Value::String(value)) = header.get_mut("value") {
                        *value = api_secrets::mask_jwts(value);
                    }
                }
            }
        }
        claims
    };
    let mut history_response = response.clone();
    history_response.body_base64.clear();
    let history_id = record_history(
        ctx,
        &repo,
        NewApiHistory {
            workspace_id: wid.clone(),
            method: method.clone(),
            url: resolved_url.clone(),
            status: Some(response.status as i64),
            duration_ms: Some(response.duration_ms),
            request: request_snapshot,
            response: serde_json::to_value(&history_response).unwrap_or(Value::Null),
        },
        &source,
        session_id,
        Some(request.id.clone()),
    )
    .await
    .unwrap_or_default();

    let result = RunSavedRequestResp {
        history_id,
        request_id: request.id,
        name: request.name,
        response,
        resolved: ApiResolvedRequest {
            method,
            url: resolved_url,
            environment_id: environment.as_ref().map(|env| env.id.clone()),
            environment: environment.map(|env| env.name),
            secrets_used: secret_names,
        },
        jwt_claims,
        warnings,
        script_tests: Value::Array(tests),
    };
    let mut serialized =
        serde_json::to_value(result).map_err(|e| EngineError::Internal(e.to_string()))?;
    api_secrets::scrub_json(&mut serialized, &secret_values);
    serde_json::from_value(serialized).map_err(|e| EngineError::Internal(e.to_string()))
}

/// The execute path itself, for internal callers (a workflow `api_run` step):
/// they call this, never the HTTP handler with fabricated extractors.
/// `headers` carries the caller attribution (`x-otto-agent`, session id) that
/// history and the agent secret-scrubbing rules read; `auth` is the request's
/// credential (an automation passes [`AuthContext::automation`]).
pub async fn execute_service<C: ApiClientCtx>(
    ctx: &C,
    wid: &Id,
    user: &User,
    auth: &AuthContext,
    headers: &HeaderMap,
    req: ExecuteApiReq,
) -> EngineResult<ApiResponse> {
    ctx.require_editor(user, wid).await?;
    let repo = ctx.repo();

    // Resolve the environment variable map (Keychain-backed secrets included),
    // then layer runtime overrides (from post-response scripts / chaining).
    let (mut vars, environment, env_blob) =
        resolve_environment(ctx, &repo, wid, req.environment_id.as_ref()).await?;
    let env_vars = vars.clone();
    if let Some(Value::Object(overrides)) = &req.vars {
        for (k, v) in overrides {
            vars.insert(k.clone(), v.clone());
        }
    }

    // Secret ↔ host binding: the caller chooses the URL here, so a stored
    // secret (a `$secret` marker or a Keychain env variable) must not be
    // resolved for a host it isn't bound to without a PERSON's confirmation —
    // before anything is resolved or sent.
    if let Some(host) = unbound_secret_host(
        &repo,
        wid,
        &req,
        &vars,
        &env_vars,
        &env_blob,
        environment
            .as_ref()
            .map(|env| env.secret_keys.as_slice())
            .unwrap_or(&[]),
    )
    .await?
    {
        let agent = is_agent_caller(headers, auth);
        if agent || !req.confirm_new_host {
            return Err(new_host_conflict(&host, agent));
        }
    }

    // Snapshot of the request as executed. Secret auth members are REDACTED —
    // history must never become the plaintext copy that defeats the Keychain.
    let (source, session_id) = caller_source(headers);
    let request_snapshot = json!({
        "method": req.method,
        "url": req.url,
        "headers": api_secrets::mask_kv_rows(&req.headers),
        "query": api_secrets::mask_kv_rows(&req.query),
        "body_mode": req.body_mode,
        "body": req.body,
        "auth": api_secrets::redact_auth(&req.auth),
        "environment_id": req.environment_id,
        "ssh_connection_id": req.ssh_connection_id,
    });

    // Resolve `$secret` auth markers in-memory, immediately before send. The
    // referenced request must live in this workspace.
    let exec_req = match resolve_exec_auth(&repo, ctx.secrets(), wid, &req).await {
        Ok(r) => Some(r),
        Err(msg) => {
            let secret_values: Vec<String> = env_blob.values().cloned().collect();
            record_history(
                ctx,
                &repo,
                NewApiHistory {
                    workspace_id: wid.clone(),
                    method: req.method.to_uppercase(),
                    url: api_secrets::scrub_str(&substitute(&req.url, &vars), &secret_values),
                    status: None,
                    duration_ms: None,
                    request: request_snapshot.clone(),
                    response: json!({ "error": msg }),
                },
                &source,
                session_id.clone(),
                None,
            )
            .await;
            return Err(EngineError::Invalid(msg));
        }
    };
    let exec_req = exec_req.expect("checked above");

    // Resolve the optional SSH tunnel first; a resolution failure flows through
    // the same error/history path as a network failure below.
    let send = match resolve_socks_proxy(ctx, wid, req.ssh_connection_id.as_ref(), &user.id).await {
        Ok(proxy) => {
            let allow_local = workspace_allows_local(ctx, wid).await;
            build_and_send(
                &jar_scope(wid, &user.id),
                &exec_req,
                &vars,
                proxy.as_deref(),
                allow_local,
            )
            .await
        }
        Err(msg) => Err(msg),
    };
    match send {
        Ok((mut resp, raw)) => {
            let (_, secret_values) = collect_secrets(
                &env_blob,
                environment
                    .as_ref()
                    .map(|env| env.secret_keys.as_slice())
                    .unwrap_or(&[]),
                &exec_req.auth,
            );
            // The history copy never keeps raw bytes: lift them out first so
            // the copy doesn't carry them only to clear them. One copy only —
            // straight to the stored JSON, scrubbed in place (perf N7: it was
            // a typed clone of the ≤512 KB body, then a second copy to JSON).
            let body_base64 = std::mem::take(&mut resp.body_base64);
            let mut stored_resp = serde_json::to_value(&resp).unwrap_or(Value::Null);
            resp.body_base64 = body_base64;
            api_secrets::scrub_response_json(&mut stored_resp, &secret_values);
            // Record success in history (best-effort; do not fail the request).
            record_history(
                ctx,
                &repo,
                NewApiHistory {
                    workspace_id: wid.clone(),
                    method: req.method.to_uppercase(),
                    url: api_secrets::scrub_str(&substitute(&req.url, &vars), &secret_values),
                    status: Some(resp.status as i64),
                    duration_ms: Some(resp.duration_ms),
                    request: request_snapshot,
                    response: stored_resp,
                },
                &source,
                session_id,
                None,
            )
            .await;
            if is_agent_caller(headers, auth) {
                // An agent reads the response too: a bound host that echoes
                // the request (a `/debug/headers` endpoint) must not hand it
                // the Keychain value. No raw bytes either — neither inline
                // base64 nor a `body_id` download — since those are unscrubbed.
                api_secrets::scrub_secrets(&mut resp, &secret_values);
                resp.body_base64.clear();
            } else {
                attach_body(
                    &mut resp,
                    raw,
                    Some(&|content_type, raw| ctx.park_body(wid, &user.id, content_type, raw)),
                );
            }
            Ok(resp)
        }
        Err(err) => {
            let (_, secret_values) = collect_secrets(
                &env_blob,
                environment
                    .as_ref()
                    .map(|env| env.secret_keys.as_slice())
                    .unwrap_or(&[]),
                &exec_req.auth,
            );
            let err = api_secrets::scrub_str(&err, &secret_values);
            // Record the failure too, then surface as a 502.
            record_history(
                ctx,
                &repo,
                NewApiHistory {
                    workspace_id: wid.clone(),
                    method: req.method.to_uppercase(),
                    url: api_secrets::scrub_str(&substitute(&req.url, &vars), &secret_values),
                    status: None,
                    duration_ms: None,
                    request: request_snapshot,
                    response: json!({ "error": err }),
                },
                &source,
                session_id,
                None,
            )
            .await;
            Err(EngineError::Upstream(err))
        }
    }
}
