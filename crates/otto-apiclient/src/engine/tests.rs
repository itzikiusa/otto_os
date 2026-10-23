//! Unit tests for the REST engine's pure and network pieces (moved with the
//! engine from `otto-server`'s `tests/unit/api_client.rs`; the handler-level
//! and server-context-backed tests stay there).

use super::*;
use serde_json::json;

#[test]
fn script_variable_unset_survives_server_replay() {
    let mut vars = json!({"token": "old", "keep": "yes"})
        .as_object()
        .unwrap()
        .clone();
    let out = api_scripts::run_pre_request(
        "pm.environment.unset('token'); pm.variables.set('next', 'new');",
        &ScriptRequest {
            method: "GET".into(),
            url: "https://example.test".into(),
            headers: json!([]),
            body: String::new(),
        },
        &string_vars(&vars),
    );
    assert!(out.error.is_none(), "{:?}", out.error);
    merge_string_vars(&mut vars, &out.vars);
    assert_eq!(
        vars,
        json!({"keep": "yes", "next": "new"})
            .as_object()
            .unwrap()
            .clone()
    );
}

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
fn cookie_jars_are_workspace_isolated() {
    let w1: Id = "ws-cookie-a".to_string();
    let w2: Id = "ws-cookie-b".to_string();
    // Same wid → same Arc; different wid → different store.
    assert!(Arc::ptr_eq(&cookie_jar(&w1), &cookie_jar(&w1)));
    assert!(!Arc::ptr_eq(&cookie_jar(&w1), &cookie_jar(&w2)));

    // A cookie set in workspace A never shows up in workspace B's jar.
    let url = "https://cookies.test/".parse().unwrap();
    {
        let jar = cookie_jar(&w1);
        let mut store = jar.lock().unwrap();
        let cookie = reqwest_cookie_store::RawCookie::parse("sess=abc; Path=/").unwrap();
        store.insert_raw(&cookie, &url).unwrap();
    }
    assert_eq!(cookie_jar(&w1).lock().unwrap().iter_any().count(), 1);
    assert_eq!(cookie_jar(&w2).lock().unwrap().iter_any().count(), 0);

    // Clearing B leaves A intact (list/clear operate on the caller's jar).
    cookie_jar(&w2).lock().unwrap().clear();
    assert_eq!(cookie_jar(&w1).lock().unwrap().iter_any().count(), 1);
    cookie_jar(&w1).lock().unwrap().clear();
}

/// S6-19: on a shared workspace each user has their own jar — user A's
/// captured session is neither replayed for nor listed to user B.
#[test]
fn cookie_jars_are_per_user_within_a_workspace() {
    let wid: Id = "ws-cookie-shared".to_string();
    let (a, b): (Id, Id) = ("user-a".into(), "user-b".into());
    let url = "https://cookies.test/".parse().unwrap();
    {
        let jar = cookie_jar(&jar_scope(&wid, &a));
        let cookie = reqwest_cookie_store::RawCookie::parse("sess=a-secret; Path=/").unwrap();
        jar.lock().unwrap().insert_raw(&cookie, &url).unwrap();
    }
    assert_eq!(
        cookie_jar(&jar_scope(&wid, &a))
            .lock()
            .unwrap()
            .iter_any()
            .count(),
        1
    );
    assert_eq!(
        cookie_jar(&jar_scope(&wid, &b))
            .lock()
            .unwrap()
            .iter_any()
            .count(),
        0
    );
    assert_ne!(jar_scope(&wid, &a), jar_scope(&wid, &b));
    cookie_jar(&jar_scope(&wid, &a)).lock().unwrap().clear();
}

/// S6-310: an idle jar no client holds is dropped; one a live client
/// still holds, or one used recently, is kept.
#[test]
fn idle_cookie_jars_are_swept_unless_held() {
    let t0 = Instant::now();
    let later = t0 + JAR_IDLE_TTL + Duration::from_secs(1);
    let jar = || Arc::new(CookieJar::new(reqwest_cookie_store::CookieStore::default()));
    let held = jar();
    let _client_ref = held.clone();
    let mut map: JarMap = HashMap::new();
    map.insert("idle".into(), (jar(), t0));
    map.insert("held".into(), (held, t0));
    map.insert("fresh".into(), (jar(), later));
    sweep_idle_jars(&mut map, later);
    let mut keys: Vec<&Id> = map.keys().collect();
    keys.sort();
    assert_eq!(keys, ["fresh", "held"]);
}

#[test]
fn apply_extras_settings_maps_onto_exec() {
    let mut exec = ExecuteApiReq {
        method: "GET".into(),
        url: "https://x.test".into(),
        headers: json!([]),
        query: json!([]),
        body_mode: "none".into(),
        body: String::new(),
        auth: json!({"type":"none"}),
        environment_id: None,
        timeout_ms: None,
        follow_redirects: None,
        verify_ssl: None,
        vars: None,
        ssh_connection_id: None,
        confirm_new_host: false,
    };
    apply_extras_settings(
        &json!({"settings": {"timeout_ms": 1500, "follow_redirects": false, "tls_verify": false}}),
        &mut exec,
    );
    assert_eq!(exec.timeout_ms, Some(1500));
    assert_eq!(exec.follow_redirects, Some(false));
    assert_eq!(exec.verify_ssl, Some(false));
    // No settings object → untouched.
    let mut exec2 = exec.clone();
    exec2.timeout_ms = None;
    apply_extras_settings(&json!({}), &mut exec2);
    assert_eq!(exec2.timeout_ms, None);
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

#[test]
fn check_override_vars_rejects_braces() {
    let mut vars = serde_json::Map::new();
    vars.insert("safe".into(), json!("plain"));
    assert!(check_override_vars(&vars).is_ok());
    vars.insert(
        "base_url".into(),
        json!("https://evil.test/?t={{api_token}}"),
    );
    assert_eq!(
        check_override_vars(&vars).unwrap_err(),
        "vars override 'base_url' must not contain '{{' (no nested substitution)"
    );
}

#[test]
fn safe_methods_need_no_confirm() {
    for method in ["GET", "head", "Options"] {
        assert!(is_safe_method(method), "{method}");
    }
    for method in ["POST", "PUT", "PATCH", "DELETE"] {
        assert!(!is_safe_method(method), "{method}");
    }
}

fn domain_request(id: &str, url: &str, extras: Option<Value>) -> ApiRequest {
    ApiRequest {
        id: id.into(),
        workspace_id: "ws1".into(),
        collection_id: None,
        name: id.into(),
        method: "GET".into(),
        url: url.into(),
        headers: json!([]),
        query: json!([]),
        body_mode: "none".into(),
        body: String::new(),
        auth: json!({"type":"none"}),
        ssh_connection_id: None,
        extras,
        position: 0,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    }
}

fn history_entry(id: &str, url: &str, request: Value) -> ApiHistoryEntry {
    ApiHistoryEntry {
        id: id.into(),
        workspace_id: "ws1".into(),
        method: "GET".into(),
        url: url.into(),
        status: Some(200),
        duration_ms: Some(1),
        request,
        response: json!({"status":200}),
        executed_at: chrono::Utc::now(),
    }
}

#[test]
fn known_hosts_collects_human_requests_and_history() {
    let requests = vec![
        domain_request("human", "https://{{host}}/v1", None),
        domain_request(
            "agent",
            "https://agent-only.test/v1",
            Some(json!({"agent":{"session_id":"s1"}})),
        ),
    ];
    let history = vec![
        history_entry(
            "human-run",
            "https://human-run.test/x",
            json!({"source":{"kind":"human"}}),
        ),
        history_entry(
            "agent-run",
            "https://agent-run.test/x",
            json!({"source":{"kind":"agent"}}),
        ),
        history_entry("legacy-run", "https://legacy.test/x", json!({})),
    ];
    let vars = serde_json::Map::from_iter([("host".into(), json!("saved-human.test"))]);
    let hosts = known_hosts(&requests, &history, &vars);
    assert_eq!(
        hosts,
        BTreeSet::from_iter([
            "human-run.test".into(),
            "legacy.test".into(),
            "saved-human.test".into(),
        ])
    );
}

fn api_response(body: String, body_base64: String) -> ApiResponse {
    ApiResponse {
        status: 200,
        status_text: "OK".into(),
        headers: json!([]),
        body,
        body_base64,
        body_id: None,
        truncated: false,
        too_large: false,
        duration_ms: 1,
        size_bytes: 1,
        content_type: Some("text/plain".into()),
        trace: Vec::new(),
    }
}

#[test]
fn shape_agent_caps_body_and_drops_base64() {
    let mut response = api_response("é".repeat(AGENT_BODY_MAX), "binary".into());
    shape_agent(&mut response);
    assert!(response.body.len() <= AGENT_BODY_MAX);
    assert!(response.truncated);
    assert!(response.body_base64.is_empty());
    assert!(response.body.is_char_boundary(response.body.len()));
}

#[test]
fn attach_body_ships_only_what_the_viewer_needs() {
    // The UI route's parker: hands out an id for whatever it is given.
    let park = |_content_type: Option<String>, _raw: Vec<u8>| -> Id { "body-1".into() };
    // Exact UTF-8 text: `body` already is the payload — no base64, no id.
    let mut text = api_response("{\"a\":1}".into(), String::new());
    attach_body(&mut text, b"{\"a\":1}".to_vec(), Some(&park));
    assert!(text.body_base64.is_empty());
    assert!(text.body_id.is_none());

    // A small image is inlined for the preview.
    let mut img = api_response(String::new(), String::new());
    img.content_type = Some("Image/PNG".into());
    attach_body(&mut img, vec![0x89, b'P', b'N', b'G'], Some(&park));
    assert!(!img.body_base64.is_empty());
    assert!(img.body_id.is_none());

    // Truncated text: parked behind a body id (UI route) …
    let mut big = api_response("x".repeat(10), String::new());
    big.truncated = true;
    attach_body(&mut big, vec![b'x'; 20], Some(&park));
    assert!(big.body_base64.is_empty());
    assert!(big.body_id.is_some());
    // … but never for a caller that can't download it (agents/tools).
    let mut tool = api_response("x".repeat(10), String::new());
    tool.truncated = true;
    attach_body(&mut tool, vec![b'x'; 20], None);
    assert!(tool.body_id.is_none() && tool.body_base64.is_empty());

    // Non-UTF-8 bytes (lossy `body`) are downloadable too; too_large is not.
    let mut bin = api_response("\u{FFFD}".into(), String::new());
    attach_body(&mut bin, vec![0xff], Some(&park));
    assert!(bin.body_id.is_some());
    let mut huge = api_response(String::new(), String::new());
    huge.too_large = true;
    attach_body(&mut huge, Vec::new(), Some(&park));
    assert!(huge.body_id.is_none());
}

#[test]
fn caller_source_reads_session_header() {
    let mut headers = HeaderMap::new();
    headers.insert("x-otto-session", " session-1 ".parse().unwrap());
    let (source, session_id) = caller_source(&headers);
    assert_eq!(source, json!({"kind":"agent","session_id":"session-1"}));
    assert_eq!(session_id.as_deref(), Some("session-1"));

    headers.insert("x-otto-session", "   ".parse().unwrap());
    let (source, session_id) = caller_source(&headers);
    assert_eq!(source, json!({"kind":"human","session_id":null}));
    assert!(session_id.is_none());

    // Outward MCP stamps only `X-Otto-Agent` — still an agent caller.
    headers.insert("x-otto-agent", "mcp-outward".parse().unwrap());
    let (source, session_id) = caller_source(&headers);
    assert_eq!(
        source,
        json!({"kind":"agent","session_id":null,"via":"mcp-outward"})
    );
    assert!(session_id.is_none());

    // A session header wins over the agent marker.
    headers.insert("x-otto-session", "session-2".parse().unwrap());
    let (source, session_id) = caller_source(&headers);
    assert_eq!(source, json!({"kind":"agent","session_id":"session-2"}));
    assert_eq!(session_id.as_deref(), Some("session-2"));
}

fn exec_req(url: &str) -> ExecuteApiReq {
    ExecuteApiReq {
        method: "GET".into(),
        url: url.into(),
        headers: json!([]),
        query: json!([]),
        body_mode: "none".into(),
        body: String::new(),
        auth: json!({"type":"none"}),
        environment_id: None,
        timeout_ms: None,
        follow_redirects: None,
        verify_ssl: None,
        vars: None,
        ssh_connection_id: None,
        confirm_new_host: false,
    }
}

#[test]
fn history_rows_keep_only_a_body_preview() {
    let mut small = json!({"status": 200, "body": "ok", "truncated": false});
    cap_history_body(&mut small);
    assert_eq!(small["body"], "ok");
    assert_eq!(small["truncated"], false);

    let big = "é".repeat(HISTORY_BODY_MAX); // 2 bytes each → over the cap
    let mut resp = json!({"status": 200, "body": big});
    cap_history_body(&mut resp);
    let body = resp["body"].as_str().unwrap();
    assert!(body.len() <= HISTORY_BODY_MAX && body.len() > HISTORY_BODY_MAX - 2);
    assert_eq!(resp["truncated"], true);
    // A non-object / bodiless response is left alone.
    let mut err = json!({"error": "boom"});
    cap_history_body(&mut err);
    assert_eq!(err, json!({"error": "boom"}));
}

#[test]
fn env_secret_probe_sees_nested_and_ignores_replaced_keys() {
    let mut vars = serde_json::Map::new();
    vars.insert("TOKEN".into(), json!("s3cret"));
    vars.insert("ALIAS".into(), json!("{{TOKEN}}"));
    let mut blob = BTreeMap::new();
    blob.insert("TOKEN".to_string(), "s3cret".to_string());
    let keys = vec!["TOKEN".to_string()];

    let mut exec = exec_req("https://x.example/");
    exec.body = "{\"t\":\"{{ALIAS}}\"}".into();
    assert!(
        uses_env_secret(&exec, &vars, &blob, &keys),
        "nested reference"
    );
    exec.body = "{{OTHER}}".into();
    assert!(!uses_env_secret(&exec, &vars, &blob, &keys));
    exec.url = "https://x.example/?k={{ TOKEN }}".into();
    assert!(
        uses_env_secret(&exec, &vars, &blob, &keys),
        "spaced placeholder"
    );
    // A runtime override that REPLACED the secret carries no secret.
    vars.insert("TOKEN".into(), json!("public"));
    assert!(!uses_env_secret(&exec, &vars, &blob, &keys));
}

#[test]
fn oauth_secrets_bind_to_the_saved_token_endpoint() {
    assert!(oauth_token_url_bound(
        "https://auth.test/oauth/token",
        "https://auth.test/oauth/token"
    ));
    assert!(oauth_token_url_bound(
        "https://auth.test/oauth/token",
        "https://AUTH.test/v2/token"
    ));
    assert!(!oauth_token_url_bound(
        "https://auth.test/oauth/token",
        "https://attacker.example/token"
    ));
    assert!(!oauth_token_url_bound("", "https://attacker.example/token"));
    assert!(oauth_token_url_bound("{{AUTH}}/token", "{{AUTH}}/token"));
}

#[test]
fn agent_callers_are_recognised_for_new_host_confirmation() {
    let user = otto_core::domain::User {
        id: "fixture".into(),
        username: "fixture".into(),
        display_name: "Fixture".into(),
        is_root: false,
        disabled: false,
        created_at: chrono::Utc::now(),
    };
    let person = AuthContext {
        real_user: user.clone(),
        effective_user: user,
        scope: None,
        mcp_only: false,
        mcp_scope: None,
        mcp_internal: false,
        mcp_session_id: None,
        managed_session_id: None,
    };
    let empty = HeaderMap::new();
    assert!(!is_agent_caller(&empty, &person));
    let mut bridged = HeaderMap::new();
    bridged.insert("x-otto-agent", "mcp".parse().unwrap());
    assert!(is_agent_caller(&bridged, &person));
    let mut managed = person.clone();
    managed.managed_session_id = Some("s1".into());
    assert!(is_agent_caller(&empty, &managed));
    let mut mcp = person;
    mcp.mcp_only = true;
    assert!(is_agent_caller(&empty, &mcp));
}

#[test]
fn timeouts_report_the_limit_that_applied() {
    assert_eq!(effective_timeout(None), EXECUTE_TIMEOUT);
    // 0 means "unset", never an instant failure.
    assert_eq!(effective_timeout(Some(0)), EXECUTE_TIMEOUT);
    assert_eq!(effective_timeout(Some(1500)), Duration::from_millis(1500));
    assert_eq!(human_duration(Duration::from_millis(250)), "250ms");
    assert_eq!(human_duration(Duration::from_secs(60)), "60s");
    assert_eq!(human_duration(Duration::from_millis(1500)), "1.5s");
}

#[tokio::test]
async fn repeated_request_headers_are_all_sent() {
    let mut exec = exec_req("http://dup.example/x");
    exec.headers = json!([
        {"key":"Accept","value":"application/json","enabled":true},
        {"key":"Accept","value":"text/plain","enabled":true},
        {"key":"X-Off","value":"no","enabled":false}
    ]);
    let wid = otto_core::new_id();
    let (builder, _, count) =
        prepare_request(&wid, &exec, &serde_json::Map::new(), None, true, true)
            .await
            .unwrap();
    let built = builder.build().unwrap();
    let accepts: Vec<_> = built
        .headers()
        .get_all("accept")
        .iter()
        .map(|v| v.to_str().unwrap().to_string())
        .collect();
    assert_eq!(accepts, vec!["application/json", "text/plain"]);
    assert!(built.headers().get("x-off").is_none());
    assert_eq!(count, 2);
}

/// One-shot loopback HTTP server: answers the first request with `reply`
/// (or never answers when `None`) and returns its address.
async fn one_shot_server(reply: Option<Vec<u8>>) -> std::net::SocketAddr {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 4096];
        let _ = sock.read(&mut buf).await;
        match reply {
            Some(bytes) => {
                let _ = sock.write_all(&bytes).await;
                let _ = sock.shutdown().await;
            }
            None => tokio::time::sleep(Duration::from_secs(10)).await,
        }
    });
    addr
}

#[tokio::test]
async fn non_ascii_response_headers_are_kept() {
    let mut reply =
        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nContent-Disposition: attachment; filename=\""
            .to_vec();
    reply.extend_from_slice("résumé.pdf".as_bytes());
    reply.extend_from_slice(b"\"\r\n\r\nok");
    let addr = one_shot_server(Some(reply)).await;
    let exec = exec_req(&format!("http://{addr}/file"));
    let wid = otto_core::new_id();
    let (resp, raw) = build_and_send(&wid, &exec, &serde_json::Map::new(), None, true)
        .await
        .unwrap();
    assert_eq!(raw, b"ok");
    let disposition = resp
        .headers
        .as_array()
        .unwrap()
        .iter()
        .find(|h| h["key"] == "content-disposition")
        .unwrap();
    assert_eq!(disposition["value"], "attachment; filename=\"résumé.pdf\"");
}

#[tokio::test]
async fn a_timeout_names_the_requests_own_limit() {
    let addr = one_shot_server(None).await;
    let mut exec = exec_req(&format!("http://{addr}/slow"));
    exec.timeout_ms = Some(200);
    let wid = otto_core::new_id();
    let err = build_and_send(&wid, &exec, &serde_json::Map::new(), None, true)
        .await
        .unwrap_err();
    assert_eq!(err, "request timed out after 200ms");
}

/// A tunnel URL reqwest rejects must FAIL the send — never fall back to a
/// direct, unguarded egress (or the shared client, dropping the tunnel).
#[tokio::test]
async fn an_invalid_tunnel_proxy_fails_instead_of_going_direct() {
    let wid = otto_core::new_id();
    let exec = exec_req("http://example.invalid/x");
    let err = build_settings_client(&wid, &exec, Some("not a url"), false).unwrap_err();
    assert!(err.contains("SSH tunnel"), "{err}");
    let err = build_and_send(
        &wid,
        &exec,
        &serde_json::Map::new(),
        Some("not a url"),
        false,
    )
    .await
    .unwrap_err();
    assert!(err.contains("SSH tunnel"), "{err}");
    // No proxy and default settings → the shared client, as before.
    assert!(build_settings_client(&wid, &exec, None, false)
        .unwrap()
        .is_none());
}

#[test]
fn extraction_misses_are_reported_and_headers_status_extract() {
    let body = json!({"data":{"token":"t-1"}});
    let headers = json!([{"key":"X-Request-Id","value":"r-9"}]);
    let outcome = StepOutcome {
        status: Some(201),
        duration_ms: 1,
        body: &body,
        body_text: "",
        headers: &headers,
    };
    let step = json!({"extract":[
        {"var":"token","path":"$.data.token"},
        {"var":"rid","path":"header:x-request-id"},
        {"var":"code","path":"status"},
        {"var":"gone","path":"$.data.nope"},
    ]});
    let mut vars = serde_json::Map::new();
    let misses = apply_extractions(&step, &outcome, &mut vars);
    assert_eq!(vars["token"], "t-1");
    assert_eq!(vars["rid"], "r-9");
    assert_eq!(vars["code"], 201);
    assert!(!vars.contains_key("gone"));
    assert_eq!(misses, vec!["Save {{gone}} from $.data.nope: not found"]);
}

#[test]
fn unresolved_placeholders_are_found_where_the_step_would_send_them() {
    let mut vars = serde_json::Map::new();
    vars.insert("host".into(), json!("api.test"));
    let mut exec = exec_req("https://{{host}}/x?id={{id}}");
    exec.headers = json!([
        {"key":"Authorization","value":"Bearer {{token}}"},
        {"key":"X-Off","value":"{{off}}","enabled":false},
        {"key":"X-Id","value":"{{$guid}}"},
    ]);
    exec.body_mode = "json".into();
    exec.body = r#"{"note":"{{ }}","id":"{{id}}"}"#.into();
    exec.auth = json!({"type":"basic","username":"{{user}}","password":"x"});
    assert_eq!(
        unresolved_placeholders(&exec, &vars),
        vec!["id".to_string(), "token".into(), "user".into()]
    );
    vars.insert("id".into(), json!(7));
    vars.insert("token".into(), json!("t"));
    vars.insert("user".into(), json!("u"));
    assert!(unresolved_placeholders(&exec, &vars).is_empty());
}

// ── perf N4 / N5 route-level guards ──────────────────────────────────────

#[tokio::test]
async fn form_variables_are_decoded_substituted_then_encoded() {
    let request: ExecuteApiReq = serde_json::from_value(json!({
        "method":"POST", "url":"http://127.0.0.1/", "body_mode":"form",
        "body":"token=%7B%7Btoken%7D%7D&repeat=a&repeat=b&literal=%2520"
    }))
    .unwrap();
    let vars = json!({"token":"abc+&=%"}).as_object().unwrap().clone();
    let (builder, _, _) = prepare_request(&"form-test".into(), &request, &vars, None, true, true)
        .await
        .unwrap();
    let request = builder.build().unwrap();
    assert_eq!(
        request.body().unwrap().as_bytes().unwrap(),
        b"token=abc%2B%26%3D%25&repeat=a&repeat=b&literal=%2520"
    );
}

#[tokio::test]
async fn malformed_or_missing_multipart_content_is_not_sent() {
    for body in [
        "[{broken".to_string(),
        json!([{ "key":"f","type":"file","filename":"f.txt","value":"","missing_content":true }])
            .to_string(),
    ] {
        let request: ExecuteApiReq = serde_json::from_value(
            json!({"method":"POST","url":"http://127.0.0.1/","body_mode":"multipart","body":body}),
        )
        .unwrap();
        assert!(prepare_request(
            &"multipart-invalid".into(),
            &request,
            &serde_json::Map::new(),
            None,
            true,
            true
        )
        .await
        .is_err());
    }
}

#[test]
fn history_marks_stripped_file_content_even_for_small_requests() {
    let mut request = json!({"body_mode":"multipart","body":json!([
        {"key":"f","type":"file","filename":"f.txt","value":"aGVsbG8="}
    ]).to_string()})
    .as_object()
    .unwrap()
    .clone();
    cap_history_request(&mut request);
    assert_eq!(request["request_truncated"], true);
    let fields: Value = serde_json::from_str(request["body"].as_str().unwrap()).unwrap();
    assert_eq!(fields[0]["missing_content"], true);
    assert_eq!(fields[0]["value"], "");
    assert_eq!(fields[0]["size"], 5);
}

#[test]
fn encoded_form_variables_keep_secret_binding_and_missing_variable_checks() {
    let request: ExecuteApiReq = serde_json::from_value(json!({"method":"POST","url":"https://new.test","body_mode":"form","body":"token=%7B%7Btoken%7D%7D"})).unwrap();
    let vars = json!({"token":"secret"}).as_object().unwrap().clone();
    assert!(uses_env_secret(
        &request,
        &vars,
        &BTreeMap::from([("token".into(), "secret".into())]),
        &["token".into()]
    ));
    assert_eq!(
        unresolved_placeholders(&request, &serde_json::Map::new()),
        vec!["token"]
    );
}
