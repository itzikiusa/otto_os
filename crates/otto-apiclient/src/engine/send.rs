//! Building and sending the outbound request, and describing its failures.

use super::*;

/// Shared HTTP request preparation for long-lived transports. Secrets and
/// environment selection stay workspace-checked exactly as in execute.
///
/// `confirm_allowed` is `false` for a managed agent credential, which can
/// never confirm a new-host secret send (`request.confirm_new_host`).
pub async fn prepare_stream<C: ApiClientCtx>(
    ctx: &C,
    wid: &Id,
    request: &ExecuteApiReq,
    actor: &Id,
    confirm_allowed: bool,
) -> Result<reqwest::RequestBuilder, String> {
    let repo = ctx.repo();
    let (mut vars, environment, env_blob) =
        resolve_environment(ctx, &repo, wid, request.environment_id.as_ref())
            .await
            .map_err(|e| e.to_string())?;
    let env_vars = vars.clone();
    if let Some(Value::Object(overrides)) = &request.vars {
        vars.extend(overrides.clone());
    }
    // Same secret ↔ host binding as ad-hoc execute. The stream socket only
    // admits Editor credentials that are not MCP-restricted; a managed agent
    // still can't self-confirm (checked by the caller's token at upgrade).
    if let Some(host) = unbound_secret_host(
        &repo,
        wid,
        request,
        &vars,
        &env_vars,
        &env_blob,
        environment
            .as_ref()
            .map(|env| env.secret_keys.as_slice())
            .unwrap_or(&[]),
    )
    .await
    .map_err(|e| e.to_string())?
    {
        if !request.confirm_new_host || !confirm_allowed {
            return Err(new_host_conflict(&host, !confirm_allowed).to_string());
        }
    }
    let req = resolve_exec_auth(&repo, ctx.secrets(), wid, request).await?;
    let proxy = resolve_socks_proxy(ctx, wid, req.ssh_connection_id.as_ref(), actor).await?;
    let allow_local = workspace_allows_local(ctx, wid).await;
    let values: Vec<String> = vars
        .values()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .chain(
            req.auth
                .as_object()
                .into_iter()
                .flat_map(|o| o.values())
                .filter_map(Value::as_str)
                .map(str::to_string),
        )
        .collect();
    // No whole-body timeout: a stream's body never "finishes" — the caller
    // bounds only the time to the response head.
    prepare_request(
        &jar_scope(wid, actor),
        &req,
        &vars,
        proxy.as_deref(),
        allow_local,
        false,
    )
    .await
    .map(|p| p.0)
    .map_err(|e| api_secrets::scrub_str(&e, &values))
}

/// Build the outbound request. `wid` is the cookie-jar scope
/// ([`jar_scope`]: workspace + acting user). `whole_body_timeout`: apply the request's
/// timeout to connect → last body byte (one-shot sends); `false` for streams
/// (SSE / WebSocket), whose callers bound only the wait for the response head.
pub async fn prepare_request(
    wid: &Id,
    req: &ExecuteApiReq,
    vars: &serde_json::Map<String, Value>,
    proxy: Option<&str>,
    allow_local: bool,
    whole_body_timeout: bool,
) -> Result<(reqwest::RequestBuilder, String, usize), String> {
    use reqwest::header::{HeaderMap, HeaderName, HeaderValue, CONTENT_TYPE};

    // --- method ---
    let url = substitute(&req.url, vars);
    // SSRF guard: resolve + classify the target host before connecting.
    // Skipped when (a) the request rides an SSH tunnel — the FAR end resolves
    // and dials, so the local check only false-blocks bastion-only hostnames
    // and RFC1918 targets the tunnel exists to reach; or (b) the workspace has
    // explicitly opted in to local/private targets (dev servers on localhost
    // are the #1 thing an API client is pointed at).
    if proxy.is_none() && !allow_local {
        otto_netguard::check_url(&url).await?;
    }
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
        // `append`, not `insert`: two enabled rows with the same name (two
        // `Accept`s, repeated `X-Forwarded-For`) are both sent, as written.
        headers.append(hn, hv);
    }

    // --- auth ---
    apply_auth(&req.auth, vars, &mut headers, &mut query)?;

    // Per-request settings: a custom client when any non-default is set or the
    // request is tunnelled, otherwise the workspace's shared pooled client.
    let shared_client = http_client(wid, allow_local);
    let custom_client = build_settings_client(wid, req, proxy, allow_local)?;
    let client = custom_client.as_ref().unwrap_or(&shared_client);
    let mut builder = client.request(method, &url);
    if whole_body_timeout {
        builder = builder.timeout(effective_timeout(req.timeout_ms));
    }

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
            // The editor/importer stores percent-encoded pairs. Resolve each
            // decoded component before reqwest encodes it exactly once.
            let fields: Vec<(String, String)> = req
                .body
                .split('&')
                .filter(|p| !p.is_empty())
                .map(|pair| {
                    let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
                    (
                        substitute(&percent_decode(key), vars),
                        substitute(&percent_decode(value), vars),
                    )
                })
                .collect();
            if !headers.contains_key(CONTENT_TYPE) {
                headers.insert(
                    CONTENT_TYPE,
                    HeaderValue::from_static("application/x-www-form-urlencoded"),
                );
            }
            builder = builder.form(&fields);
        }
        // multipart/form-data: the body is either a JSON array of fields
        // (`[{key,type,value,filename}]`, where a `file` field's value is the
        // base64 of the file's bytes) or the legacy `k=v&...` text encoding.
        // reqwest sets the `Content-Type: multipart/form-data; boundary=…`
        // itself, so any explicit Content-Type must be dropped.
        "multipart" => {
            headers.remove(CONTENT_TYPE);
            builder = builder.multipart(build_multipart(&req.body, vars)?);
        }
        // raw or any other mode → send as text verbatim.
        _ => {
            let body = substitute(&req.body, vars);
            builder = builder.body(body);
        }
    }

    if !query.is_empty() {
        builder = builder.query(&query);
    }
    let header_count = headers.len();
    builder = builder.headers(headers);

    Ok((builder, url, header_count))
}

/// Build the outbound request and send it, returning an `ApiResponse` plus
/// the raw body bytes, or an error message string (never panics).
pub async fn build_and_send(
    wid: &Id,
    req: &ExecuteApiReq,
    vars: &serde_json::Map<String, Value>,
    proxy: Option<&str>,
    allow_local: bool,
) -> Result<(ApiResponse, Vec<u8>), String> {
    let (builder, url, header_count) =
        prepare_request(wid, req, vars, proxy, allow_local, true).await?;
    // Trace: resolved request + per-phase timing for the response "Trace" tab.
    let method_str = req.method.to_uppercase();
    let body_desc = match req.body_mode.as_str() {
        "none" | "" => String::new(),
        m => format!(", {m} body"),
    };
    let mut trace: Vec<otto_core::api::TraceStep> = vec![
        trace_step("Request", &format!("{method_str} {url}"), None, "info"),
        trace_step(
            "Sent request",
            &format!("{header_count} header(s){body_desc}"),
            None,
            "info",
        ),
    ];
    if proxy.is_some() {
        trace.push(trace_step(
            "SSH tunnel",
            "routed via SOCKS5 over SSH",
            None,
            "info",
        ));
    }

    let timeout = effective_timeout(req.timeout_ms);
    let started = Instant::now();
    let resp = builder
        .send()
        .await
        .map_err(|e| describe_reqwest_error(&e, timeout))?;
    let ttfb_ms = started.elapsed().as_millis() as i64;
    let final_url = resp.url().to_string();
    trace.push(trace_step(
        "Waiting (TTFB)",
        "time to first response byte (includes connect + TLS)",
        Some(ttfb_ms),
        "timing",
    ));
    if final_url != url {
        trace.push(trace_step(
            "Redirected",
            &format!("→ {final_url}"),
            None,
            "redirect",
        ));
    }
    let status = resp.status();
    let status_code = status.as_u16();
    let status_text = status.canonical_reason().unwrap_or("").to_string();

    // Response headers as [{key,value}].
    let resp_headers: Vec<Value> = resp
        .headers()
        .iter()
        .map(|(k, v)| {
            json!({
                "key": k.as_str(),
                // Lossy, not `to_str()`: a non-ASCII value (a UTF-8 filename
                // in Content-Disposition) must not show up blank.
                "value": header_value_text(v),
            })
        })
        .collect();
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    // Body: the UTF-8 (lossy) text capped for display goes in the response;
    // the raw bytes go back to the caller, which decides whether the UI gets
    // them (`attach_body`) — never base64 by default. Beyond MAX_INLINE we
    // keep neither.
    const MAX_INLINE: usize = 25 * 1024 * 1024; // 25 MB raw
    const MAX_TEXT_DISPLAY: usize = 512 * 1024; // 512 KB rendered as text
    let dl_start = Instant::now();
    let mut response = resp;
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| describe_reqwest_error(&e, timeout))?
    {
        let remaining = (MAX_INLINE + 1).saturating_sub(bytes.len());
        bytes.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
        if bytes.len() > MAX_INLINE {
            break;
        }
    }
    let download_ms = dl_start.elapsed().as_millis() as i64;
    let size_bytes = bytes.len() as i64;
    let duration_ms = started.elapsed().as_millis() as i64;
    trace.push(trace_step(
        "Downloaded",
        &human_bytes(bytes.len()),
        Some(download_ms),
        "timing",
    ));
    trace.push(trace_step(
        "Completed",
        &format!("{status_code} {status_text}"),
        Some(duration_ms),
        if status.is_success() {
            "success"
        } else {
            "error"
        },
    ));

    let (body, truncated, too_large) = if bytes.len() > MAX_INLINE {
        bytes = Vec::new();
        (String::new(), false, true)
    } else {
        // Decode only the displayed prefix (+ one char's worth of slack) —
        // not the whole body — then cut on a char boundary.
        let prefix = &bytes[..bytes.len().min(MAX_TEXT_DISPLAY + 4)];
        let text = String::from_utf8_lossy(prefix);
        if bytes.len() > prefix.len() || text.len() > MAX_TEXT_DISPLAY {
            let mut end = MAX_TEXT_DISPLAY.min(text.len());
            while end > 0 && !text.is_char_boundary(end) {
                end -= 1;
            }
            (text[..end].to_string(), true, false)
        } else {
            (text.into_owned(), false, false)
        }
    };

    Ok((
        ApiResponse {
            status: status_code,
            status_text,
            headers: Value::Array(resp_headers),
            body,
            body_base64: String::new(),
            body_id: None,
            truncated,
            too_large,
            duration_ms,
            size_bytes,
            content_type,
            trace,
        },
        bytes,
    ))
}

/// Parks a body in the response cache and returns its `body_id`.
pub type BodyParker<'a> = &'a dyn Fn(Option<String>, Vec<u8>) -> Id;

/// Largest `image/*` body inlined as base64 for the viewer's preview.
pub(crate) const INLINE_PREVIEW_MAX: usize = 5 * 1024 * 1024;

/// Give the UI what it needs to preview / save the body without shipping the
/// whole thing as base64: an image (≤ 5 MiB) is inlined for the preview; an
/// exact UTF-8 `body` already is the payload; anything else (truncated text,
/// binary) is parked in the bounded response cache behind `body_id` — only
/// when `park` is given (the interactive UI route:
/// [`ApiClientCtx::park_body`] for the caller's workspace + user).
pub fn attach_body(resp: &mut ApiResponse, raw: Vec<u8>, park: Option<BodyParker<'_>>) {
    if resp.too_large || raw.is_empty() {
        return;
    }
    let image = resp
        .content_type
        .as_deref()
        .is_some_and(|c| c.trim_start().to_ascii_lowercase().starts_with("image/"));
    if image && raw.len() <= INLINE_PREVIEW_MAX {
        use base64::engine::general_purpose::STANDARD as B64;
        use base64::Engine;
        resp.body_base64 = B64.encode(&raw);
        return;
    }
    let Some(park) = park else {
        return;
    };
    if !resp.truncated && std::str::from_utf8(&raw).is_ok_and(|t| t == resp.body) {
        return;
    }
    resp.body_id = Some(park(resp.content_type.clone(), raw));
}

pub(crate) fn trace_step(
    label: &str,
    detail: &str,
    ms: Option<i64>,
    level: &str,
) -> otto_core::api::TraceStep {
    otto_core::api::TraceStep {
        label: label.to_string(),
        detail: detail.to_string(),
        ms,
        level: level.to_string(),
    }
}

pub(crate) fn human_bytes(n: usize) -> String {
    if n < 1024 {
        format!("{n} B")
    } else if n < 1024 * 1024 {
        format!("{:.1} KB", n as f64 / 1024.0)
    } else {
        format!("{:.1} MB", n as f64 / (1024.0 * 1024.0))
    }
}

/// The per-request timeout actually applied: the request's `timeout_ms`
/// (0 = unset, never an instant failure), else [`EXECUTE_TIMEOUT`].
pub fn effective_timeout(timeout_ms: Option<u64>) -> Duration {
    timeout_ms
        .filter(|ms| *ms > 0)
        .map(Duration::from_millis)
        .unwrap_or(EXECUTE_TIMEOUT)
}

/// A response header value as display text (UTF-8, lossy).
pub(crate) fn header_value_text(value: &reqwest::header::HeaderValue) -> String {
    String::from_utf8_lossy(value.as_bytes()).into_owned()
}

/// "1.5s" / "60s" / "250ms" — a timeout as the user set it.
pub(crate) fn human_duration(d: Duration) -> String {
    let ms = d.as_millis();
    if ms < 1000 {
        format!("{ms}ms")
    } else if ms.is_multiple_of(1000) {
        format!("{}s", ms / 1000)
    } else {
        format!("{:.1}s", ms as f64 / 1000.0)
    }
}

/// Human-friendly message for a reqwest error (timeout / DNS / connect / …).
/// `timeout` is the limit that was applied to this request, so a timeout
/// reports the user's own setting rather than the default.
pub fn describe_reqwest_error(e: &reqwest::Error, timeout: Duration) -> String {
    if e.is_timeout() {
        format!("request timed out after {}", human_duration(timeout))
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
