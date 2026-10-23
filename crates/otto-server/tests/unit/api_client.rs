use super::*;
use otto_core::auth::AuthContext;
use serde_json::json;
use std::sync::Mutex as StdMutex;

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
    use otto_netguard::check_url;
    assert!(check_url("http://127.0.0.1/x").await.is_err());
    assert!(check_url("http://169.254.169.254/latest/meta-data")
        .await
        .is_err());
    assert!(check_url("http://[::1]:8080/").await.is_err());
    // Non-fetchable schemes are rejected outright.
    assert!(check_url("file:///etc/passwd").await.is_err());
    assert!(check_url("not a url").await.is_err());
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

#[tokio::test]
async fn multipart_substitutes_fields_without_corrupting_container_or_files() {
    use axum::{body::Bytes, routing::post, Router};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().route("/", post(|body: Bytes| async { body })),
        )
        .await
        .unwrap();
    });
    let request: ExecuteApiReq = serde_json::from_value(json!({
        "method":"POST", "url":format!("http://{addr}/"), "body_mode":"multipart",
        "body":json!([
            {"key":"note","type":"text","value":"{{note}}"},
            {"key":"file","type":"file","filename":"hello.txt","value":"aGVsbG8="}
        ]).to_string()
    }))
    .unwrap();
    let vars = json!({"note":"a\"b\n\\&"}).as_object().unwrap().clone();
    let result = build_and_send(&"multipart-test".into(), &request, &vars, None, true).await;
    server.abort();
    let (response, _) = result.unwrap();
    assert!(
        response.body.contains("name=\"note\"\r\n\r\na\"b\n\\&\r\n"),
        "{}",
        response.body
    );
    assert!(response.body.contains("filename=\"hello.txt\""));
    assert!(response.body.contains("\r\n\r\nhello\r\n"));
}

#[tokio::test]
async fn http_non_success_status_remains_a_response_not_an_execution_error() {
    use axum::{routing::post, Router};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().route(
                "/",
                post(|| async { (StatusCode::UNPROCESSABLE_ENTITY, "invalid input") }),
            ),
        )
        .await
        .unwrap();
    });
    let request: ExecuteApiReq =
        serde_json::from_value(json!({"method":"POST","url":format!("http://{addr}/")})).unwrap();
    let result = build_and_send(
        &"http-status-test".into(),
        &request,
        &serde_json::Map::new(),
        None,
        true,
    )
    .await;
    server.abort();
    let (response, _) = result.expect("HTTP failure status is still a successful transport");
    assert_eq!(response.status, 422);
    assert_eq!(response.body, "invalid input");
}

// ── characterization: the REST engine's contract with its callers ────────
// Written against the pre-move engine (architecture-1-07r) and kept
// unchanged across the move into `otto-apiclient`: every status, problem
// body and merge rule below is what the routes, workflows and automation
// runs depended on before the engine left this file.

/// The status and JSON problem body an error renders as on the wire.
async fn wire(error: impl Into<ApiError>) -> (u16, Value) {
    use axum::response::IntoResponse;
    let response = error.into().into_response();
    let status = response.status().as_u16();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

/// A routes-level fixture: a seeded workspace (no `allow_local`) on a
/// throwaway `ServerCtx`.
async fn engine_fixture(wid: &str) -> (tempfile::TempDir, ServerCtx, Id) {
    use crate::routes::browser::tests::{mem_pool, seed_workspace, test_ctx};
    let tmp = tempfile::TempDir::new().unwrap();
    let pool = mem_pool().await;
    seed_workspace(&pool, wid).await;
    let ctx = test_ctx(&pool, tmp.path().to_path_buf()).await;
    (tmp, ctx, wid.to_string())
}

async fn new_env(ctx: &ServerCtx, wid: &Id, body: Value) -> ApiEnvironment {
    let user = crate::routes::browser::tests::root_user();
    let Json(env) = create_environment(
        Path(wid.clone()),
        State(ctx.clone()),
        CurrentUser(user),
        Json(serde_json::from_value(body).unwrap()),
    )
    .await
    .unwrap();
    env
}

/// `POST …/execute` as a person (or, with `x-otto-agent`, an agent) that
/// must fail; returns the wire status and problem body.
async fn execute_failure(
    ctx: &ServerCtx,
    wid: &Id,
    headers: HeaderMap,
    exec: Value,
) -> (u16, Value) {
    let user = crate::routes::browser::tests::root_user();
    let result = execute(
        Path(wid.clone()),
        State(ctx.clone()),
        CurrentUser(user.clone()),
        CurrentAuthContext(auth_for(&user)),
        headers,
        Json(serde_json::from_value(exec).unwrap()),
    )
    .await;
    let Err(error) = result else {
        panic!("execute must fail")
    };
    wire(error).await
}

#[test]
fn substitute_resolves_nested_bounded_and_dynamic_variables() {
    let vars = json!({
        "host": "api.test",
        "base_url": "https://{{host}}/v1",
        "a": "{{b}}", "b": "{{c}}", "c": "{{d}}", "d": "deep",
        "loop": "{{loop}}",
        "$timestamp": "mine",
        "n": 3, "z": null,
    })
    .as_object()
    .unwrap()
    .clone();
    // Nested definitions re-expand…
    assert_eq!(
        substitute("{{base_url}}/users", &vars),
        "https://api.test/v1/users"
    );
    // …for at most three passes, and a self-reference stops at once.
    assert_eq!(substitute("{{a}}", &vars), "{{d}}");
    assert_eq!(substitute("{{loop}}", &vars), "{{loop}}");
    // A user/env variable wins over the built-in dynamic of the same name.
    assert_eq!(substitute("{{$timestamp}}", &vars), "mine");
    // Non-strings are coerced, null is empty, names are trimmed.
    assert_eq!(substitute("{{ n }}-{{z}}-", &vars), "3--");
    let none = serde_json::Map::new();
    let guid = substitute("{{$guid}}", &none);
    assert!(uuid::Uuid::parse_str(&guid).is_ok(), "{guid}");
    let ts = substitute("{{$timestamp}}", &none);
    assert!(
        !ts.is_empty() && ts.chars().all(|c| c.is_ascii_digit()),
        "{ts}"
    );
    assert!(substitute("{{$isoTimestamp}}", &none).contains('T'));
    let n: u32 = substitute("{{$randomInt}}", &none).parse().unwrap();
    assert!(n < 1000);
    assert_eq!(substitute("{{$unknown}}", &none), "{{$unknown}}");
}

#[test]
fn retention_reads_non_negative_limits_and_clamps_days() {
    assert_eq!(retention_from_settings(None), (0, 0));
    assert_eq!(retention_from_settings(Some(&json!({}))), (0, 0));
    assert_eq!(
        retention_from_settings(Some(
            &json!({"api_client": {"history_max_rows": 50, "history_max_days": 7}})
        )),
        (50, 7)
    );
    // Negative / non-integer values fall back to the (opt-in, 0) defaults.
    assert_eq!(
        retention_from_settings(Some(
            &json!({"api_client": {"history_max_rows": -1, "history_max_days": "7"}})
        )),
        (HISTORY_KEEP_ROWS_DEFAULT, HISTORY_KEEP_DAYS_DEFAULT)
    );
    assert_eq!(
        retention_from_settings(Some(
            &json!({"api_client": {"history_max_days": 99_999_999}})
        )),
        (0, HISTORY_KEEP_DAYS_MAX)
    );
}

#[tokio::test]
async fn new_host_conflicts_render_as_409_problems() {
    assert_eq!(
        wire(new_host_conflict("evil.test", true)).await,
        (
            409,
            json!({"code": "conflict", "message": "conflict: needs_confirm=new_host: a stored secret would be sent to host 'evil.test', which it is not bound to; a person must send it from the Otto UI (agent credentials cannot confirm)"})
        )
    );
    assert_eq!(
        wire(new_host_conflict("evil.test", false)).await,
        (
            409,
            json!({"code": "conflict", "message": "conflict: needs_confirm=new_host: a stored secret would be sent to host 'evil.test', which it is not bound to; re-send with confirm_new_host:true"})
        )
    );
}

#[tokio::test]
async fn resolve_environment_prefers_explicit_then_active_and_overlays_secrets() {
    let (_tmp, ctx, wid) = engine_fixture("ws-resolve-env").await;
    let repo = repo(&ctx);
    // No environment at all → empty map, no row, no blob.
    let (vars, env, blob) = resolve_environment(&ctx, &repo, &wid, None).await.unwrap();
    assert!(vars.is_empty() && env.is_none() && blob.is_empty());

    let explicit = new_env(
        &ctx,
        &wid,
        json!({"name": "explicit", "variables": {"base": "https://e.test", "token": "row-value"},
               "secret_keys": ["token"], "secret_values": {"token": "kc-value"}}),
    )
    .await;
    let active = new_env(
        &ctx,
        &wid,
        json!({"name": "active", "variables": {"base": "https://a.test"}}),
    )
    .await;
    repo.set_active(&wid, &active.id).await.unwrap();

    // No id → the active environment.
    let (vars, env, blob) = resolve_environment(&ctx, &repo, &wid, None).await.unwrap();
    assert_eq!(env.map(|e| e.id), Some(active.id.clone()));
    assert_eq!(vars["base"], "https://a.test");
    assert!(blob.is_empty());

    // An explicit id wins over the active one, and its Keychain values are
    // laid over the row's variables (the row never holds the secret).
    let (vars, env, blob) = resolve_environment(&ctx, &repo, &wid, Some(&explicit.id))
        .await
        .unwrap();
    assert_eq!(env.map(|e| e.id), Some(explicit.id.clone()));
    assert_eq!(vars["base"], "https://e.test");
    assert_eq!(vars["token"], "kc-value");
    assert_eq!(blob.get("token").map(String::as_str), Some("kc-value"));

    // An environment of another workspace is a 404, never resolved.
    let Err(error) =
        resolve_environment(&ctx, &repo, &"ws-elsewhere".to_string(), Some(&explicit.id)).await
    else {
        panic!("a foreign environment must not resolve")
    };
    assert_eq!(
        wire(error).await,
        (
            404,
            json!({"code": "not_found", "message": "not found: not in this workspace"})
        )
    );
}

#[tokio::test]
async fn execute_errors_keep_their_status_and_problem_body() {
    let (_tmp, ctx, wid) = engine_fixture("ws-exec-errors").await;

    // Local / private target without the workspace opt-in → 502 with the
    // net guard's own reason; a URL that doesn't parse fails the same way.
    for url in ["http://127.0.0.1:9/x", "not a url"] {
        let reason = otto_netguard::check_url(url).await.unwrap_err();
        assert_eq!(
            execute_failure(
                &ctx,
                &wid,
                HeaderMap::new(),
                json!({"method": "GET", "url": url})
            )
            .await,
            (
                502,
                json!({"code": "upstream", "message": format!("upstream: {reason}")})
            ),
            "{url}"
        );
    }

    // A Keychain env secret aimed at a host none of the workspace's human
    // requests use → 409; a person is told how to confirm, an agent is not.
    let env = new_env(
        &ctx,
        &wid,
        json!({"name": "prod", "variables": {"base": "https://unbound.invalid"},
               "secret_keys": ["token"], "secret_values": {"token": "s3cr3t"}}),
    )
    .await;
    let exec = json!({"method": "GET", "url": "{{base}}/me", "environment_id": env.id,
                      "headers": [{"key": "Authorization", "value": "Bearer {{token}}", "enabled": true}]});
    assert_eq!(
        execute_failure(&ctx, &wid, HeaderMap::new(), exec.clone()).await,
        (
            409,
            json!({"code": "conflict", "message": "conflict: needs_confirm=new_host: a stored secret would be sent to host 'unbound.invalid', which it is not bound to; re-send with confirm_new_host:true"})
        )
    );
    let mut agent = HeaderMap::new();
    agent.insert("x-otto-agent", "mcp".parse().unwrap());
    let mut confirmed = exec;
    confirmed["confirm_new_host"] = json!(true);
    assert_eq!(
        execute_failure(&ctx, &wid, agent, confirmed).await,
        (
            409,
            json!({"code": "conflict", "message": "conflict: needs_confirm=new_host: a stored secret would be sent to host 'unbound.invalid', which it is not bound to; a person must send it from the Otto UI (agent credentials cannot confirm)"})
        )
    );

    // A `$secret` marker for a request that doesn't exist → 400, and the
    // refused send is still recorded in history (no status, the error).
    let marker = json!({"$secret": api_secrets::request_ref("no-such-request")});
    assert_eq!(
        execute_failure(
            &ctx,
            &wid,
            HeaderMap::new(),
            json!({"method": "GET", "url": "https://x.test/", "auth": {"type": "bearer", "token": marker}})
        )
        .await,
        (400, json!({"code": "invalid", "message": "invalid: auth references a secret for an unknown request"}))
    );
    let history = repo(&ctx).list_history(&wid, 10).await.unwrap();
    let refused = history
        .iter()
        .find(|h| h.url == "https://x.test/")
        .expect("the refused send is recorded");
    assert_eq!(refused.status, None);
    assert_eq!(
        refused.response,
        json!({"error": "auth references a secret for an unknown request"})
    );
}
