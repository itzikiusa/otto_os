//! One automation (collection runner) step: load, script, bind, send, assert, extract.

use super::*;

/// Secret ↔ host binding inputs for an automation run an AGENT started (see
/// [`unbound_secret_host`]): the environment's own variables (no dataset rows,
/// no chained extractions) and its Keychain blob. A person's run carries none —
/// they chose the automation, its dataset and its targets themselves.
pub struct StepSecretBinding {
    pub env_vars: serde_json::Map<String, Value>,
    pub env_blob: BTreeMap<String, String>,
    pub secret_keys: Vec<String>,
    /// [`bound_env_secret_hosts`] for `env_vars`, computed once per run
    /// rather than once per step (perf F3).
    pub bound_hosts: BTreeSet<String>,
}

impl StepSecretBinding {
    /// The binding `api_automation_runs::start` gives a run: `Some` for an
    /// AGENT caller (every stored secret stays on its bound host, see
    /// [`run_step`]), `None` for a person (their run is theirs to aim). One
    /// constructor, so the run route and its test share the decision.
    pub async fn for_caller(
        agent: bool,
        api: &ApiClientRepo,
        wid: &Id,
        env_vars: &serde_json::Map<String, Value>,
        environment: Option<&ApiEnvironment>,
        env_blob: &BTreeMap<String, String>,
    ) -> EngineResult<Option<Self>> {
        if !agent {
            return Ok(None);
        }
        Ok(Some(Self {
            bound_hosts: bound_env_secret_hosts(api, wid, env_vars).await?,
            env_vars: env_vars.clone(),
            env_blob: env_blob.clone(),
            secret_keys: environment
                .map(|env| env.secret_keys.clone())
                .unwrap_or_default(),
        }))
    }
}

/// Run one automation step against its saved request, evaluating assertions and
/// applying extractions into `vars` for later steps. Resilient: any error is
/// captured into the returned [`ApiRunStepResult`] rather than propagated.
#[allow(clippy::too_many_arguments)]
pub async fn run_step<C: ApiClientCtx>(
    ctx: &C,
    repo: &ApiClientRepo,
    wid: &Id,
    step: &Value,
    vars: &mut serde_json::Map<String, Value>,
    pinned: Option<ApiRequest>,
    actor: &Id,
    binding: Option<&StepSecretBinding>,
) -> ApiRunStepResult {
    let request_id = step
        .get("request_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    // Load the saved request (must belong to this workspace).
    let request = match match pinned {
        Some(request) => Ok(request),
        None => load_step_request(repo, wid, &request_id).await,
    } {
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

    // Honour the saved request's SSH tunnel choice, if any.
    let proxy = match resolve_socks_proxy(ctx, wid, request.ssh_connection_id.as_ref(), actor).await
    {
        Ok(p) => p,
        Err(msg) => {
            return ApiRunStepResult {
                request_id,
                name: request.name,
                status: None,
                duration_ms: 0,
                ok: false,
                assertions: Value::Array(Vec::new()),
                error: Some(msg),
            };
        }
    };

    // The persisted `extras` unlock scripts / settings / GraphQL variables for
    // stored-request replay (this is what automations previously lost).
    let extras = request.extras.clone().unwrap_or(Value::Null);
    let transport = extras
        .get("transport")
        .and_then(Value::as_str)
        .unwrap_or("http");
    if transport != "http" {
        return ApiRunStepResult {
            request_id,
            name: request.name,
            status: None,
            duration_ms: 0,
            ok: false,
            assertions: Value::Array(Vec::new()),
            error: Some(format!(
                "'{transport}' requests are not runnable in automations (HTTP only)"
            )),
        };
    }

    let mut exec = request_to_execute(&request);
    apply_extras_settings(&extras, &mut exec);
    let graphql_vars = if exec.body_mode == "graphql" {
        match parse_graphql_variables(&extras) {
            Ok(vars) => Some(vars),
            Err(error) => {
                return ApiRunStepResult {
                    request_id,
                    name: request.name,
                    status: None,
                    duration_ms: 0,
                    ok: false,
                    assertions: Value::Array(Vec::new()),
                    error: Some(error),
                };
            }
        }
    } else {
        None
    };
    let mut script_asserts: Vec<Value> = Vec::new();
    let mut scripts_ok = true;

    // Pre-request script: may mutate method/url/headers/body and set vars.
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
        let mut svars = string_vars(vars);
        let out = api_scripts::run_pre_request_isolated(pre_code, &script_req, &svars).await;
        if let Some(err) = out.error {
            return ApiRunStepResult {
                request_id,
                name: request.name,
                status: None,
                duration_ms: 0,
                ok: false,
                assertions: Value::Array(Vec::new()),
                error: Some(format!("pre-request script failed: {err}")),
            };
        }
        if let Some(mutated) = out.request {
            exec.method = mutated.method;
            exec.url = mutated.url;
            exec.headers = mutated.headers;
            exec.body = mutated.body;
        }
        svars = out.vars;
        merge_string_vars(vars, &svars);
    }

    // GraphQL: combine the query body + stored variables into the standard
    // JSON payload, exactly like the interactive runner.
    if let Some(gql_vars) = graphql_vars {
        exec.body = json!({ "query": exec.body, "variables": gql_vars }).to_string();
    }

    // Agent-started run: a stored secret must not leave its bound host (a
    // rewritten request, a dataset row or a chained value can retarget the
    // URL). The step fails instead of sending; a person can run it from the UI.
    if let Some(binding) = binding {
        let unbound = unbound_secret_host_with(
            repo,
            wid,
            &exec,
            vars,
            &binding.env_vars,
            &binding.env_blob,
            &binding.secret_keys,
            Some(&binding.bound_hosts),
        )
        .await;
        let error = match unbound {
            Ok(None) => None,
            Ok(Some(host)) => Some(new_host_conflict(&host, true).to_string()),
            Err(error) => Some(error.to_string()),
        };
        if let Some(error) = error {
            return ApiRunStepResult {
                request_id,
                name: request.name,
                status: None,
                duration_ms: 0,
                ok: false,
                assertions: Value::Array(Vec::new()),
                error: Some(error),
            };
        }
    }

    // A `{{var}}` nothing defines (an earlier extraction missed, a dataset
    // column is absent) fails the step — never send the literal placeholder.
    let unresolved = unresolved_placeholders(&exec, vars);
    if !unresolved.is_empty() {
        let names: Vec<String> = unresolved.iter().map(|n| format!("{{{{{n}}}}}")).collect();
        return ApiRunStepResult {
            request_id,
            name: request.name,
            status: None,
            duration_ms: 0,
            ok: false,
            assertions: Value::Array(Vec::new()),
            error: Some(format!(
                "Not sent: no value for {} (set it in the environment, the dataset or an earlier step's extraction)",
                names.join(", ")
            )),
        };
    }

    // Resolve `$secret` auth markers (this stored request's own ref only).
    if let Err(msg) = api_secrets::resolve_auth_markers_async(
        ctx.secrets(),
        &mut exec.auth,
        std::slice::from_ref(&request.id),
    )
    .await
    {
        return ApiRunStepResult {
            request_id,
            name: request.name,
            status: None,
            duration_ms: 0,
            ok: false,
            assertions: Value::Array(Vec::new()),
            error: Some(msg),
        };
    }

    // Send via the shared single-request path (same reqwest logic as /execute).
    let allow_local = workspace_allows_local(ctx, wid).await;
    match build_and_send(
        &jar_scope(wid, actor),
        &exec,
        vars,
        proxy.as_deref(),
        allow_local,
    )
    .await
    {
        // Steps only read status/body/timing — the raw bytes are dropped here.
        Ok((resp, _raw)) => {
            // Parse the body as JSON once for json_path assertions/extraction;
            // a non-JSON body yields Null (assertions/extracts simply miss).
            let body_json = serde_json::from_str::<Value>(&resp.body).unwrap_or(Value::Null);

            // Evaluate assertions.
            let outcome = StepOutcome {
                status: Some(resp.status),
                duration_ms: resp.duration_ms,
                body: &body_json,
                body_text: &resp.body,
                headers: &resp.headers,
            };
            let (assertions_value, mut all_passed) = eval_step_assertions(step, &outcome);

            // Apply extractions into the chained variable map. A miss is a
            // failed line in the report and fails the step — otherwise the
            // next step would silently send a literal `{{token}}`.
            let misses = apply_extractions(step, &outcome, vars);
            if !misses.is_empty() {
                all_passed = false;
            }
            script_asserts.extend(
                misses
                    .into_iter()
                    .map(|desc| json!({ "desc": desc, "passed": false, "actual": Value::Null })),
            );

            // Post-response script: chaining (set vars) + pm.test results,
            // reported alongside the step's declarative assertions.
            let post_code = extras
                .pointer("/scripts/post")
                .and_then(Value::as_str)
                .unwrap_or("");
            if !post_code.trim().is_empty() {
                let mut headers_map = BTreeMap::new();
                if let Some(arr) = resp.headers.as_array() {
                    for h in arr {
                        let k = h.get("key").and_then(Value::as_str).unwrap_or("");
                        let v = h.get("value").and_then(Value::as_str).unwrap_or("");
                        if !k.is_empty() {
                            headers_map.insert(k.to_ascii_lowercase(), v.to_string());
                        }
                    }
                }
                let script_resp = ScriptResponse {
                    code: resp.status,
                    status: resp.status_text.clone(),
                    response_time: resp.duration_ms,
                    headers: headers_map,
                    body_text: resp.body.clone(),
                };
                let svars = string_vars(vars);
                let out =
                    api_scripts::run_post_response_isolated(post_code, &script_resp, &svars).await;
                if out.error.is_none() {
                    merge_string_vars(vars, &out.vars);
                }
                for t in &out.tests {
                    if !t.passed {
                        scripts_ok = false;
                    }
                    script_asserts.push(json!({
                        "desc": format!("script: {}", t.name),
                        "passed": t.passed,
                    }));
                }
                if let Some(err) = out.error {
                    scripts_ok = false;
                    script_asserts.push(json!({
                        "desc": format!("post-response script error: {err}"),
                        "passed": false,
                    }));
                }
            }

            let mut all_asserts = assertions_value.as_array().cloned().unwrap_or_default();
            all_asserts.extend(script_asserts);

            ApiRunStepResult {
                request_id,
                name: request.name,
                status: Some(resp.status),
                duration_ms: resp.duration_ms,
                // request succeeded; ok iff every assertion AND script test held
                ok: all_passed && scripts_ok,
                assertions: Value::Array(all_asserts),
                error: None,
            }
        }
        Err(err) => {
            // Request failed: still evaluate assertions (against a null body and
            // unknown status) so the report is complete, but ok is false.
            let (assertions_value, _) = eval_step_assertions(
                step,
                &StepOutcome {
                    status: None,
                    duration_ms: 0,
                    body: &Value::Null,
                    body_text: "",
                    headers: &Value::Null,
                },
            );
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

/// Map `extras.settings` onto the per-request execution settings.
pub fn apply_extras_settings(extras: &Value, exec: &mut ExecuteApiReq) {
    let Some(settings) = extras.get("settings") else {
        return;
    };
    if let Some(ms) = settings.get("timeout_ms").and_then(Value::as_u64) {
        exec.timeout_ms = Some(ms);
    }
    if let Some(b) = settings.get("follow_redirects").and_then(Value::as_bool) {
        exec.follow_redirects = Some(b);
    }
    if let Some(b) = settings.get("tls_verify").and_then(Value::as_bool) {
        exec.verify_ssl = Some(b);
    }
}

/// Match interactive execution: missing/blank variables mean {}, but malformed
/// JSON and non-object values must fail before scripts or outbound execution.
pub fn parse_graphql_variables(extras: &Value) -> Result<Value, String> {
    let text = extras
        .get("graphql_variables")
        .and_then(Value::as_str)
        .unwrap_or("");
    if text.trim().is_empty() {
        return Ok(json!({}));
    }
    let value: Value = serde_json::from_str(text)
        .map_err(|error| format!("GraphQL variables must be a valid JSON object: {error}"))?;
    if !value.is_object() {
        return Err("GraphQL variables must be a JSON object".into());
    }
    Ok(value)
}

/// Load a step's saved request, validating presence and workspace ownership.
/// Returns a descriptive error string on any miss.
pub async fn load_step_request(
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
pub fn request_to_execute(request: &ApiRequest) -> ExecuteApiReq {
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
        timeout_ms: None,
        follow_redirects: None,
        verify_ssl: None,
        vars: None,
        ssh_connection_id: request.ssh_connection_id.clone(),
        confirm_new_host: false,
    }
}

/// Evaluate a step's `assertions` array, returning the
/// `[{desc,passed,actual}]` JSON and whether every assertion passed
/// (vacuously true when none).
pub fn eval_step_assertions(step: &Value, outcome: &StepOutcome<'_>) -> (Value, bool) {
    let mut out: Vec<Value> = Vec::new();
    let mut all = true;
    if let Some(arr) = step.get("assertions").and_then(Value::as_array) {
        for assertion in arr {
            let r = eval_assertion_with(assertion, outcome);
            if !r.passed {
                all = false;
            }
            out.push(json!({ "desc": r.desc, "passed": r.passed, "actual": r.actual }));
        }
    }
    (Value::Array(out), all)
}

/// Apply a step's `extract` list into the chained variable map. `path` is a
/// JSON path into the body, `header:<name>` for a response header, or
/// `status` for the status code. Returns one worded line per MISS ("Save
/// {{token}} from $.data.token: not found") — the caller fails the step.
pub fn apply_extractions(
    step: &Value,
    outcome: &StepOutcome<'_>,
    vars: &mut serde_json::Map<String, Value>,
) -> Vec<String> {
    let mut misses = Vec::new();
    if let Some(arr) = step.get("extract").and_then(Value::as_array) {
        for entry in arr {
            let var = entry.get("var").and_then(Value::as_str).unwrap_or("");
            let path = entry.get("path").and_then(Value::as_str).unwrap_or("");
            if var.is_empty() {
                continue;
            }
            let found = if path.trim() == "status" {
                outcome.status.map(Value::from)
            } else if let Some(name) = path.trim().strip_prefix("header:") {
                header_value(outcome.headers, name).map(Value::String)
            } else {
                json_path(outcome.body, path).cloned()
            };
            match found {
                Some(v) => {
                    vars.insert(var.to_string(), v);
                }
                None => misses.push(format!("Save {{{{{var}}}}} from {path}: not found")),
            }
        }
    }
    misses
}

/// `{{name}}` placeholders still unresolved in what an automation step is
/// about to send (URL, enabled query/headers, body, auth strings) — no value in
/// `vars` and not a built-in dynamic (`$guid` & co). Sorted, deduplicated.
pub fn unresolved_placeholders(
    exec: &ExecuteApiReq,
    vars: &serde_json::Map<String, Value>,
) -> Vec<String> {
    fn scan(text: &str, vars: &serde_json::Map<String, Value>, out: &mut BTreeSet<String>) {
        let resolved = substitute(text, vars);
        let mut rest = resolved.as_str();
        while let Some(i) = rest.find("{{") {
            let after = &rest[i + 2..];
            let Some(end) = after.find("}}") else { break };
            let name = after[..end].trim();
            // Only identifier-shaped names — `{{ }}` or prose isn't a variable.
            let ident = !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | '$'));
            if ident {
                out.insert(name.to_string());
            }
            rest = &after[end + 2..];
        }
    }
    fn scan_value(v: &Value, vars: &serde_json::Map<String, Value>, out: &mut BTreeSet<String>) {
        match v {
            Value::String(s) => scan(s, vars, out),
            Value::Array(a) => a.iter().for_each(|x| scan_value(x, vars, out)),
            Value::Object(m) => m.values().for_each(|x| scan_value(x, vars, out)),
            _ => {}
        }
    }
    let mut out = BTreeSet::new();
    scan(&exec.url, vars, &mut out);
    for (k, v) in enabled_kv(&exec.query)
        .iter()
        .chain(enabled_kv(&exec.headers).iter())
    {
        scan(k, vars, &mut out);
        scan(v, vars, &mut out);
    }
    if !matches!(exec.body_mode.as_str(), "none" | "") {
        if exec.body_mode == "form" {
            scan(&percent_decode(&exec.body), vars, &mut out);
        } else {
            scan(&exec.body, vars, &mut out);
        }
    }
    scan_value(&exec.auth, vars, &mut out);
    out.into_iter().collect()
}
