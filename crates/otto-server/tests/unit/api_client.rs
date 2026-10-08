use super::*;
use serde_json::json;

/// In-memory `SecretStore` stand-in for the Keychain in tests.
struct MemStore(StdMutex<BTreeMap<String, String>>);
impl MemStore {
    fn new() -> Self {
        Self(StdMutex::new(BTreeMap::new()))
    }
}
impl otto_core::secrets::SecretStore for MemStore {
    fn put(&self, k: &str, v: &str) -> otto_core::Result<()> {
        self.0.lock().unwrap().insert(k.into(), v.into());
        Ok(())
    }
    fn get(&self, k: &str) -> otto_core::Result<Option<String>> {
        Ok(self.0.lock().unwrap().get(k).cloned())
    }
    fn delete(&self, k: &str) -> otto_core::Result<()> {
        self.0.lock().unwrap().remove(k);
        Ok(())
    }
}

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

async fn mk_repo() -> (otto_state::DbPool, ApiClientRepo, Id) {
    let opts = sqlx::sqlite::SqliteConnectOptions::new()
        .in_memory(true)
        .foreign_keys(true);
    let pool = otto_state::DbPool::from(
        sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap(),
    );
    sqlx::migrate!("../otto-state/migrations")
        .run(&pool)
        .await
        .unwrap();
    let ws = otto_core::new_id();
    let user = otto_core::new_id();
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query("INSERT INTO users (id, username, password_hash, created_at) VALUES (?, ?, '', ?)")
        .bind(&user)
        .bind(format!("u-{user}"))
        .bind(&now)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO workspaces (id, name, root_path, created_at) VALUES (?, 'ws', '/tmp', ?)",
    )
    .bind(&ws)
    .bind(&now)
    .execute(&pool)
    .await
    .unwrap();
    (pool.clone(), ApiClientRepo::new(pool), ws)
}

fn legacy_request(ws: &Id, name: &str, auth: Value) -> NewApiRequest {
    NewApiRequest {
        id: None,
        workspace_id: ws.clone(),
        collection_id: None,
        name: name.into(),
        method: "GET".into(),
        url: "https://api.test/x".into(),
        headers: json!([]),
        query: json!([]),
        body_mode: "none".into(),
        body: String::new(),
        auth,
        ssh_connection_id: None,
        extras: None,
        position: 0,
    }
}

#[tokio::test]
async fn secure_all_sweeps_and_is_idempotent() {
    let (_pool, repo, ws) = mk_repo().await;
    let store = std::sync::Arc::new(MemStore::new());

    // Legacy rows: plaintext bearer token + a secret-shaped env variable.
    let req = repo
        .create_request(legacy_request(
            &ws,
            "legacy",
            json!({"type":"bearer","token":"tk-1"}),
        ))
        .await
        .unwrap();
    let env = repo
        .create_environment(NewApiEnvironment {
            workspace_id: ws.clone(),
            name: "prod".into(),
            variables: json!({"base":"https://x","api_token":"sekret"}),
            secret_keys: Vec::new(),
        })
        .await
        .unwrap();

    let (r, e) = secure_all_sweep(
        &repo,
        &(store.clone() as std::sync::Arc<dyn otto_core::secrets::SecretStore>),
        &ws,
    )
    .await
    .unwrap();
    assert_eq!((r, e), (1, 1));

    // Row now carries a marker; the value lives only in the store.
    let after = repo.get_request(&req.id).await.unwrap();
    let own_ref = api_secrets::request_ref(&req.id);
    assert_eq!(after.auth["token"], json!({"$secret": own_ref.clone()}));
    assert!(store
        .0
        .lock()
        .unwrap()
        .get(&own_ref)
        .unwrap()
        .contains("tk-1"));
    assert!(!after.auth.to_string().contains("tk-1"));

    // Environment: key moved to secret_keys, value stripped from the row.
    let env_after = repo.get_environment(&env.id).await.unwrap();
    assert_eq!(env_after.secret_keys, vec!["api_token".to_string()]);
    assert!(env_after.variables.get("api_token").is_none());
    assert_eq!(env_after.variables["base"], "https://x");
    assert!(store
        .0
        .lock()
        .unwrap()
        .get(&api_secrets::env_ref(&env.id))
        .unwrap()
        .contains("sekret"));

    // Second sweep finds nothing to do (idempotent).
    let (r2, e2) = secure_all_sweep(
        &repo,
        &(store.clone() as std::sync::Arc<dyn otto_core::secrets::SecretStore>),
        &ws,
    )
    .await
    .unwrap();
    assert_eq!((r2, e2), (0, 0));

    // The OpenAPI export of the secured request contains marker refs only.
    let col = otto_core::domain::ApiCollection {
        id: "c".into(),
        workspace_id: ws.clone(),
        name: "col".into(),
        parent_id: None,
        position: 0,
        created_at: chrono::Utc::now(),
    };
    let doc = crate::api_helpers::collection_to_openapi(&col, &[after]);
    assert!(!doc.to_string().contains("tk-1"));
}

#[tokio::test]
async fn resolve_exec_auth_resolves_own_and_rejects_foreign() {
    let (pool, repo, ws) = mk_repo().await;
    let store = std::sync::Arc::new(MemStore::new());

    let req = repo
        .create_request(legacy_request(
            &ws,
            "mine",
            json!({"type":"bearer","token":"live-tok"}),
        ))
        .await
        .unwrap();
    secure_all_sweep(
        &repo,
        &(store.clone() as std::sync::Arc<dyn otto_core::secrets::SecretStore>),
        &ws,
    )
    .await
    .unwrap();
    let stored = repo.get_request(&req.id).await.unwrap();

    let exec = ExecuteApiReq {
        method: "GET".into(),
        url: "https://api.test/x".into(),
        headers: json!([]),
        query: json!([]),
        body_mode: "none".into(),
        body: String::new(),
        auth: stored.auth.clone(),
        environment_id: None,
        timeout_ms: None,
        follow_redirects: None,
        verify_ssl: None,
        vars: None,
        ssh_connection_id: None,
        confirm_new_host: false,
    };
    // Same-workspace marker resolves in-memory only.
    let resolved = resolve_exec_auth(
        &repo,
        &(store.clone() as std::sync::Arc<dyn otto_core::secrets::SecretStore>),
        &ws,
        &exec,
    )
    .await
    .unwrap();
    assert_eq!(resolved.auth["token"], "live-tok");
    assert!(repo.get_request(&req.id).await.unwrap().auth["token"].is_object());

    // A caller in another workspace replaying the marker is rejected.
    let ws2 = otto_core::new_id();
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO workspaces (id, name, root_path, created_at) VALUES (?, 'w2', '/tmp', ?)",
    )
    .bind(&ws2)
    .bind(&now)
    .execute(&pool)
    .await
    .unwrap();
    let err = resolve_exec_auth(
        &repo,
        &(store.clone() as std::sync::Arc<dyn otto_core::secrets::SecretStore>),
        &ws2,
        &exec,
    )
    .await
    .unwrap_err();
    assert!(err.contains("outside this workspace"), "{err}");

    // History snapshots redact markers AND plaintext to ***.
    let red = api_secrets::redact_auth(&stored.auth);
    assert_eq!(red["token"], api_secrets::REDACTED);
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
fn net_guard_blocks_internal_addresses() {
    use otto_netguard::is_blocked_ip;
    use std::net::IpAddr;
    let blocked = [
        "127.0.0.1",
        "::1",
        "10.0.0.5",
        "172.16.3.4",
        "192.168.1.1",
        "169.254.169.254", // cloud metadata
        "169.254.1.1",     // link-local
        "100.64.0.1",      // CGNAT
        "0.0.0.0",
        "::",
        "::ffff:127.0.0.1", // v4-mapped loopback
        "fd00::1",          // ULA
        "fe80::1",          // link-local v6
    ];
    for s in blocked {
        let ip: IpAddr = s.parse().unwrap();
        assert!(is_blocked_ip(ip), "{s} should be blocked");
    }
    let allowed = ["8.8.8.8", "1.1.1.1", "93.184.216.34", "2606:2800:220:1::1"];
    for s in allowed {
        let ip: IpAddr = s.parse().unwrap();
        assert!(!is_blocked_ip(ip), "{s} should be allowed");
    }
}

#[tokio::test]
async fn net_guard_check_url_rejects_loopback_and_schemes() {
    use super::net_guard::check_url;
    assert!(check_url("http://127.0.0.1/x").await.is_err());
    assert!(check_url("http://169.254.169.254/latest/meta-data")
        .await
        .is_err());
    assert!(check_url("http://[::1]:8080/").await.is_err());
    // Non-fetchable schemes are rejected outright.
    assert!(check_url("file:///etc/passwd").await.is_err());
    assert!(check_url("not a url").await.is_err());
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
    let (wid, user) = ("w-attach".to_string(), "u-attach".to_string());
    // Exact UTF-8 text: `body` already is the payload — no base64, no id.
    let mut text = api_response("{\"a\":1}".into(), String::new());
    attach_body(&mut text, b"{\"a\":1}".to_vec(), Some((&wid, &user)));
    assert!(text.body_base64.is_empty());
    assert!(text.body_id.is_none());

    // A small image is inlined for the preview.
    let mut img = api_response(String::new(), String::new());
    img.content_type = Some("Image/PNG".into());
    attach_body(&mut img, vec![0x89, b'P', b'N', b'G'], Some((&wid, &user)));
    assert!(!img.body_base64.is_empty());
    assert!(img.body_id.is_none());

    // Truncated text: parked behind a body id (UI route) …
    let mut big = api_response("x".repeat(10), String::new());
    big.truncated = true;
    attach_body(&mut big, vec![b'x'; 20], Some((&wid, &user)));
    assert!(big.body_base64.is_empty());
    assert!(big.body_id.is_some());
    // … but never for a caller that can't download it (agents/tools).
    let mut tool = api_response("x".repeat(10), String::new());
    tool.truncated = true;
    attach_body(&mut tool, vec![b'x'; 20], None);
    assert!(tool.body_id.is_none() && tool.body_base64.is_empty());

    // Non-UTF-8 bytes (lossy `body`) are downloadable too; too_large is not.
    let mut bin = api_response("\u{FFFD}".into(), String::new());
    attach_body(&mut bin, vec![0xff], Some((&wid, &user)));
    assert!(bin.body_id.is_some());
    let mut huge = api_response(String::new(), String::new());
    huge.too_large = true;
    attach_body(&mut huge, Vec::new(), Some((&wid, &user)));
    assert!(huge.body_id.is_none());
}

#[test]
fn shape_request_agent_redacts_auth_headers_and_body() {
    let mut request = domain_request("r1", "https://api.test", None);
    request.auth = json!({"type":"bearer","token":"live-token"});
    request.headers = json!([
        {"key":"Authorization","value":"Bearer live-token","enabled":true},
        {"key":"Accept","value":"application/json","enabled":true}
    ]);
    request.query = json!([{"key":"api_key","value":"live-token","enabled":true}]);
    request.body = "a".repeat(AGENT_BODY_MAX + 10);
    let shaped = shape_request_agent(request);
    assert_eq!(shaped.auth["token"], api_secrets::MASK);
    assert_eq!(shaped.headers[0]["value"], api_secrets::MASK);
    assert_eq!(shaped.headers[1]["value"], "application/json");
    assert_eq!(shaped.query[0]["value"], api_secrets::MASK);
    assert_eq!(
        shaped.body.len(),
        AGENT_BODY_MAX + AGENT_BODY_TRUNCATED.len()
    );
    assert!(shaped.body.ends_with(AGENT_BODY_TRUNCATED));
}

#[test]
fn build_overview_filters_and_masks() {
    let now = chrono::Utc::now();
    let collections = vec![ApiCollection {
        id: "c1".into(),
        workspace_id: "ws1".into(),
        name: "Payments".into(),
        parent_id: None,
        position: 0,
        created_at: now,
    }];
    let mut request = domain_request("r1", "https://api.test/login", None);
    request.name = "Login".into();
    request.auth = json!({"type":"bearer","token":"never returned"});
    let environments = vec![ApiEnvironment {
        id: "e1".into(),
        workspace_id: "ws1".into(),
        name: "Staging".into(),
        variables: json!({"base_url":"https://api.test","api_token":"legacy-secret"}),
        secret_keys: vec!["client_secret".into()],
        is_active: true,
        created_at: now,
    }];
    let automations = vec![ApiAutomation {
        id: "a1".into(),
        workspace_id: "ws1".into(),
        name: "Smoke".into(),
        steps: json!([{"request_id":"r1"}]),
        created_at: now,
    }];
    let overview = build_overview(
        collections,
        vec![overview_row(&request)],
        environments,
        automations,
        None,
        "all",
    );
    assert_eq!(overview.requests[0].auth_type, "bearer");
    assert_eq!(
        overview.environments[0].variables["api_token"],
        api_secrets::MASK
    );
    assert_eq!(
        overview.environments[0].variables["base_url"],
        "https://api.test"
    );
    assert_eq!(overview.automations[0].steps, 1);
    assert_eq!(overview.collections.len(), 1);

    let filtered = build_overview(
        Vec::new(),
        vec![overview_row(&domain_request(
            "orders",
            "https://api.test/orders",
            None,
        ))],
        Vec::new(),
        Vec::new(),
        Some("ORDERS"),
        "requests",
    );
    assert_eq!(filtered.requests.len(), 1);
    assert!(filtered.environments.is_empty());
}

/// The Rust reading of a request's overview row — the reference the
/// SQL projection (`list_request_summaries`) is checked against.
fn overview_row(request: &ApiRequest) -> ApiOverviewRequest {
    ApiOverviewRequest {
        id: request.id.clone(),
        name: request.name.clone(),
        method: request.method.clone(),
        url: request.url.clone(),
        collection_id: request.collection_id.clone(),
        auth_type: request
            .auth
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("none")
            .to_string(),
        has_ssh: request.ssh_connection_id.is_some(),
        agent_authored: is_agent_authored(request),
        updated_at: request.updated_at,
        position: request.position,
    }
}

#[test]
fn stamp_agent_merges_into_existing_extras() {
    let stamped = stamp_agent(Some(json!({"v":2,"docs_md":"hello"})), Some("s1"));
    assert_eq!(stamped["v"], 2);
    assert_eq!(stamped["docs_md"], "hello");
    assert_eq!(stamped["agent"]["session_id"], "s1");
    assert!(stamped["agent"]["at"].as_str().is_some());
    assert_eq!(stamp_agent(None, Some("s2"))["v"], 1);
    // Outward MCP: no session, still agent-authored.
    let outward = stamp_agent(None, None);
    assert_eq!(outward["agent"]["session_id"], Value::Null);
    assert!(outward["agent"].is_object());
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

#[test]
fn merged_auth_for_update_keeps_when_absent() {
    assert!(merged_auth_for_update(&Value::Null).is_none());
    let incoming = json!({"type":"none"});
    assert_eq!(merged_auth_for_update(&incoming), Some(incoming));
}
#[test]
fn history_summary_query_limits_keep_existing_contract() {
    for (requested, expected) in [(None, 100), (Some(0), 1), (Some(-2), 1), (Some(800), 500)] {
        let filter = HistoryFilter {
            limit: requested,
            q: Some("literal_a%".into()),
            status: Some(201),
            request_id: Some("req".into()),
            source: Some("agent".into()),
        };
        let query = history_query(filter);
        assert_eq!(query.limit, expected);
        assert_eq!(query.q.as_deref(), Some("literal_a%"));
        assert_eq!(query.status, Some(201));
        assert_eq!(query.request_id.as_deref(), Some("req"));
        assert_eq!(query.source.as_deref(), Some("agent"));
    }
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

#[tokio::test]
async fn secrets_stay_bound_to_their_hosts() {
    let (_pool, repo, ws) = mk_repo().await;
    // Human-authored saved request on api.test owns the marker.
    let owner = repo
        .create_request(legacy_request(
            &ws,
            "owner",
            json!({"type":"bearer","token":"tk"}),
        ))
        .await
        .unwrap();
    let marker = json!({"$secret": api_secrets::request_ref(&owner.id)});
    let none_vars = serde_json::Map::new();
    let no_blob = BTreeMap::new();

    // Marker → bound to the owning request's host only.
    let mut exec = exec_req("https://api.test/other");
    exec.auth = json!({"type":"bearer","token": marker});
    assert_eq!(
        unbound_secret_host(&repo, &ws, &exec, &none_vars, &none_vars, &no_blob, &[])
            .await
            .unwrap(),
        None
    );
    exec.url = "https://attacker.example/x".into();
    assert_eq!(
        unbound_secret_host(&repo, &ws, &exec, &none_vars, &none_vars, &no_blob, &[])
            .await
            .unwrap()
            .as_deref(),
        Some("attacker.example")
    );

    // Env secret → bound to the hosts of human-authored saved requests.
    let mut vars = serde_json::Map::new();
    vars.insert("TOKEN".into(), json!("s3cret"));
    let mut blob = BTreeMap::new();
    blob.insert("TOKEN".to_string(), "s3cret".to_string());
    let keys = vec!["TOKEN".to_string()];
    let mut exec = exec_req("https://attacker.example/x");
    exec.headers = json!([{"key":"Authorization","value":"Bearer {{TOKEN}}","enabled":true}]);
    assert_eq!(
        unbound_secret_host(&repo, &ws, &exec, &vars, &vars, &blob, &keys)
            .await
            .unwrap()
            .as_deref(),
        Some("attacker.example")
    );
    exec.url = "https://api.test/y".into();
    assert_eq!(
        unbound_secret_host(&repo, &ws, &exec, &vars, &vars, &blob, &keys)
            .await
            .unwrap(),
        None
    );
    // No secret referenced → any host is fine.
    let exec = exec_req("https://elsewhere.example/");
    assert_eq!(
        unbound_secret_host(&repo, &ws, &exec, &vars, &vars, &blob, &keys)
            .await
            .unwrap(),
        None
    );
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
fn renaming_a_secret_moves_its_keychain_value() {
    let stored = |pairs: &[(&str, &str)]| -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    };
    let keys = |ks: &[&str]| -> Vec<String> { ks.iter().map(|k| k.to_string()).collect() };

    // Rename without retyping: the value follows the new name.
    let mut blob = stored(&[("API_TOKEN", "tk"), ("OTHER", "o")]);
    let renames = stored(&[("API_TOKEN", "API_TOKEN_PROD")]);
    apply_secret_changes(
        &mut blob,
        &keys(&["API_TOKEN_PROD", "OTHER"]),
        &renames,
        BTreeMap::new(),
    );
    assert_eq!(blob, stored(&[("API_TOKEN_PROD", "tk"), ("OTHER", "o")]));

    // A freshly typed value for the new name wins over the moved one.
    let mut blob = stored(&[("A", "old")]);
    apply_secret_changes(
        &mut blob,
        &keys(&["B"]),
        &stored(&[("A", "B")]),
        stored(&[("B", "new")]),
    );
    assert_eq!(blob, stored(&[("B", "new")]));

    // Without a rename, a key dropped from secret_keys loses its value
    // (explicit un-marking), and untouched secrets are kept.
    let mut blob = stored(&[("A", "a"), ("B", "b")]);
    apply_secret_changes(&mut blob, &keys(&["B"]), &BTreeMap::new(), BTreeMap::new());
    assert_eq!(blob, stored(&[("B", "b")]));

    // A rename onto a key that is not secret moves nothing.
    let mut blob = stored(&[("A", "a")]);
    apply_secret_changes(
        &mut blob,
        &keys(&[]),
        &stored(&[("A", "PLAIN")]),
        BTreeMap::new(),
    );
    assert!(blob.is_empty());
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

/// Overriding `{{base_url}}` must not drag the secret's binding along: the
/// bound hosts are resolved with the environment's own variables.
#[tokio::test]
async fn overriding_a_variable_does_not_rebind_secrets() {
    let (_pool, repo, ws) = mk_repo().await;
    let mut human = legacy_request(&ws, "users", json!({"type":"none"}));
    human.url = "{{base_url}}/users".into();
    repo.create_request(human).await.unwrap();

    let mut env_vars = serde_json::Map::new();
    env_vars.insert("base_url".into(), json!("https://api.test"));
    env_vars.insert("TOKEN".into(), json!("s3cret-token"));
    let mut blob = BTreeMap::new();
    blob.insert("TOKEN".to_string(), "s3cret-token".to_string());
    let keys = vec!["TOKEN".to_string()];
    let mut exec = exec_req("{{base_url}}/anything");
    exec.headers = json!([{"key":"Authorization","value":"Bearer {{TOKEN}}","enabled":true}]);

    // Home host: fine.
    assert_eq!(
        unbound_secret_host(&repo, &ws, &exec, &env_vars, &env_vars, &blob, &keys)
            .await
            .unwrap(),
        None
    );
    // Runtime override retargets the send — the binding stays put.
    let mut overridden = env_vars.clone();
    overridden.insert("base_url".into(), json!("https://attacker.example"));
    assert_eq!(
        unbound_secret_host(&repo, &ws, &exec, &overridden, &env_vars, &blob, &keys)
            .await
            .unwrap()
            .as_deref(),
        Some("attacker.example")
    );

    // Same for a `$secret` marker whose owner URL is variable-based.
    let mut owner = legacy_request(&ws, "owner", json!({"type":"none"}));
    owner.url = "{{base_url}}/me".into();
    let owner = repo.create_request(owner).await.unwrap();
    let mut marked = exec_req("{{base_url}}/me");
    marked.auth =
        json!({"type":"bearer","token": {"$secret": api_secrets::request_ref(&owner.id)}});
    assert_eq!(
        unbound_secret_host(
            &repo,
            &ws,
            &marked,
            &overridden,
            &env_vars,
            &BTreeMap::new(),
            &[]
        )
        .await
        .unwrap()
        .as_deref(),
        Some("attacker.example")
    );
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

#[test]
fn oauth_errors_prefer_the_endpoint_message_and_bound_the_excerpt() {
    let described = json!({"error":"invalid_client","error_description":"bad secret"});
    assert_eq!(oauth_error_message(401, &described, ""), "bad secret");
    let bare = json!({"error":"invalid_grant"});
    assert_eq!(oauth_error_message(400, &bare, ""), "invalid_grant");
    let page = format!("<html>{}</html>", "x".repeat(10_000));
    let msg = oauth_error_message(502, &Value::Null, &page);
    assert!(msg.starts_with("token endpoint returned 502: <html>"));
    assert!(msg.ends_with('…'));
    assert!(msg.len() < 400, "excerpt must stay short: {}", msg.len());
    let msg = oauth_error_message(200, &json!({"ok":true}), r#"{"ok":true}"#);
    assert!(msg.contains("without an access_token"));
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

fn auth_for(user: &otto_core::domain::User) -> AuthContext {
    AuthContext {
        real_user: user.clone(),
        effective_user: user.clone(),
        scope: None,
        mcp_only: false,
        mcp_scope: None,
        mcp_internal: false,
        mcp_session_id: None,
        managed_session_id: None,
    }
}

fn upsert_req(name: &str, url: &str) -> UpsertApiRequestReq {
    serde_json::from_value(json!({ "name": name, "method": "GET", "url": url }))
        .expect("upsert dto")
}

#[tokio::test]
async fn invalid_graphql_variables_fail_saved_and_automation_replay() {
    use crate::routes::browser::tests::{mem_pool, root_user, seed_workspace, test_ctx};
    let tmp = tempfile::TempDir::new().unwrap();
    let pool = mem_pool().await;
    let wid: Id = "ws-graphql-replay".into();
    seed_workspace(&pool, &wid).await;
    let ctx = test_ctx(&pool, tmp.path().to_path_buf()).await;
    let user = root_user();
    for input in ["{bad", "[]", "null", "123"] {
        let mut req = upsert_req("Invalid variables", "http://127.0.0.1:1/graphql");
        req.body_mode = "graphql".into();
        req.body = "query { value }".into();
        req.extras = Some(json!({"v":1,"graphql_variables":input}));
        let Json(saved) = create_request(
            Path(wid.clone()),
            State(ctx.clone()),
            CurrentUser(user.clone()),
            HeaderMap::new(),
            Json(req),
        )
        .await
        .unwrap();
        let result = run_saved_request(
            Path((wid.clone(), saved.id.clone())),
            State(ctx.clone()),
            CurrentUser(user.clone()),
            CurrentAuthContext(auth_for(&user)),
            HeaderMap::new(),
            Json(serde_json::from_value(json!({})).unwrap()),
        )
        .await;
        let error = result.expect_err("invalid variables must fail before sending");
        assert!(
            error.0.to_string().contains("GraphQL variables"),
            "{input}: {}",
            error.0
        );
        let step = run_step(
            &ctx,
            &repo(&ctx),
            &wid,
            &json!({"request_id":saved.id}),
            &mut serde_json::Map::new(),
            Some(saved),
            &user.id,
            None,
        )
        .await;
        assert!(!step.ok);
        assert!(
            step.error
                .as_deref()
                .unwrap_or("")
                .contains("GraphQL variables"),
            "{input}: {:?}",
            step.error
        );
    }
}

/// perf N4: every saved-request write (the agent's `api_upsert_request`
/// goes through these same routes) tells the workspace's clients, so the
/// UI's 60 s list cache can't serve a stale tree.
#[tokio::test]
async fn request_writes_emit_api_client_changed() {
    use crate::routes::browser::tests::{mem_pool, root_user, seed_workspace, test_ctx};
    let tmp = tempfile::TempDir::new().unwrap();
    let pool = mem_pool().await;
    seed_workspace(&pool, "ws-n4").await;
    let ctx = test_ctx(&pool, tmp.path().to_path_buf()).await;
    let mut rx = ctx.events.subscribe();
    let wid: Id = "ws-n4".into();
    let user = root_user();

    let Json(created) = create_request(
        Path(wid.clone()),
        State(ctx.clone()),
        CurrentUser(user.clone()),
        HeaderMap::new(),
        Json(upsert_req("one", "https://a.example/x")),
    )
    .await
    .unwrap();
    let _ = update_request(
        Path((wid.clone(), created.id.clone())),
        State(ctx.clone()),
        CurrentUser(user.clone()),
        HeaderMap::new(),
        Json(upsert_req("one renamed", "https://a.example/x")),
    )
    .await
    .unwrap();
    delete_request(
        Path((wid.clone(), created.id.clone())),
        State(ctx.clone()),
        CurrentUser(user.clone()),
    )
    .await
    .unwrap();

    let mut seen = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        if let Event::ApiClientChanged {
            workspace_id,
            kind,
            id,
            deleted,
        } = ev
        {
            assert_eq!(workspace_id, wid);
            seen.push((kind, id, deleted));
        }
    }
    let some = Some(created.id.clone());
    assert_eq!(
        seen,
        vec![
            ("request".to_string(), some.clone(), false),
            ("request".to_string(), some.clone(), false),
            ("request".to_string(), some, true),
        ]
    );
}

/// perf N5: `/execute` with a Keychain-backed env secret over a workspace
/// of many saved requests checks the secret's host binding from the URL
/// projection — it never reads full request rows (`SELECT *`).
#[tokio::test]
async fn execute_with_env_secret_never_reads_full_request_rows() {
    use crate::routes::browser::tests::{mem_pool, root_user, seed_workspace, test_ctx};
    let tmp = tempfile::TempDir::new().unwrap();
    let pool = mem_pool().await;
    seed_workspace(&pool, "ws-n5").await;
    let ctx = test_ctx(&pool, tmp.path().to_path_buf()).await;
    let wid: Id = "ws-n5".into();
    let user = root_user();
    for i in 0..200 {
        let _ = create_request(
            Path(wid.clone()),
            State(ctx.clone()),
            CurrentUser(user.clone()),
            HeaderMap::new(),
            Json(upsert_req(
                &format!("r{i}"),
                &format!("https://h{i}.example/v1"),
            )),
        )
        .await
        .unwrap();
    }
    let env_req: UpsertApiEnvironmentReq = serde_json::from_value(json!({
        "name": "prod",
        "variables": { "base": "https://unbound.invalid" },
        "secret_keys": ["token"],
        "secret_values": { "token": "s3cr3t" },
    }))
    .unwrap();
    let Json(env) = create_environment(
        Path(wid.clone()),
        State(ctx.clone()),
        CurrentUser(user.clone()),
        Json(env_req),
    )
    .await
    .unwrap();

    let before = otto_state::api_client::list_requests_calls_on_this_thread();
    let exec: ExecuteApiReq = serde_json::from_value(json!({
        "method": "GET",
        "url": "{{base}}/me",
        "headers": [{ "key": "Authorization", "value": "Bearer {{token}}", "enabled": true }],
        "environment_id": env.id,
    }))
    .unwrap();
    // The outcome (an unbound-secret refusal, or a send error) doesn't
    // matter here — only what the path read to get there.
    let _ = execute(
        Path(wid.clone()),
        State(ctx.clone()),
        CurrentUser(user.clone()),
        CurrentAuthContext(auth_for(&user)),
        HeaderMap::new(),
        Json(exec),
    )
    .await;
    assert_eq!(
        otto_state::api_client::list_requests_calls_on_this_thread() - before,
        0,
        "/execute must not read every saved request in full"
    );
}
