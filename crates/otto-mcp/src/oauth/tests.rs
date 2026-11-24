//! Sign-in against a local fake authorization server (discovery through
//! protected-resource + authorization-server metadata, dynamic registration,
//! the code exchange with PKCE, refresh with rotation, a revoked grant, a
//! state mismatch and a PKCE mismatch) plus the pure helpers.
use super::*;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[test]
fn pkce_s256_is_the_base64url_sha256_of_the_verifier() {
    assert_eq!(
        challenge("dBjftJeZ4CVP-mJ92K9b-pg6v12s3cJw6aQfhKdQ9sk"),
        // (node: crypto sha256 → base64url)
        "U9PAMMr4SJ6AHu1u3CTa62BT4hEnAMyL3BadSQSCCRA"
    );
    assert_ne!(random_token(), random_token());
}

#[test]
fn metadata_hints_and_well_known_paths() {
    assert_eq!(
        resource_metadata_hint(r#"Bearer realm="x", resource_metadata="https://h/.well-known/oauth-protected-resource/mcp""#).as_deref(),
        Some("https://h/.well-known/oauth-protected-resource/mcp")
    );
    assert_eq!(resource_metadata_hint("Bearer realm=x"), None);
    let u = reqwest::Url::parse("https://h.example/a/mcp").unwrap();
    assert_eq!(
        well_known(&u, "oauth-protected-resource").as_str(),
        "https://h.example/.well-known/oauth-protected-resource/a/mcp"
    );
    let root = reqwest::Url::parse("https://as.example/").unwrap();
    assert_eq!(
        well_known(&root, "oauth-authorization-server").as_str(),
        "https://as.example/.well-known/oauth-authorization-server"
    );
}

#[test]
fn the_callback_needs_the_exact_path_and_state() {
    assert_eq!(
        callback_code("GET /callback?code=c1&state=s1 HTTP/1.1", "s1").unwrap(),
        "c1"
    );
    assert!(
        callback_code("GET /callback?code=c1&state=s2 HTTP/1.1", "s1")
            .unwrap_err()
            .contains("state")
    );
    assert!(callback_code("GET /other?code=c1&state=s1 HTTP/1.1", "s1").is_err());
    assert!(
        callback_code("GET /callback?error=access_denied&state=s1 HTTP/1.1", "s1")
            .unwrap_err()
            .contains("access_denied")
    );
    assert!(!format!(
        "{:?}",
        Tokens {
            access_token: "secret-at".into(),
            refresh_token: Some("secret-rt".into()),
            expires_at: None,
            token_endpoint: String::new(),
            client_id: String::new(),
            client_secret: None,
            resource: String::new(),
            auth_host: String::new()
        }
    )
    .contains("secret"));
}

/// The fake authorization server's memory.
#[derive(Default)]
struct As {
    challenge: Option<String>,
    /// Pad the authorization-server metadata past the body cap.
    huge: bool,
    /// Metadata that names another issuer / another resource.
    wrong_issuer: bool,
    wrong_resource: bool,
    revoked: bool,
    tokens_issued: u32,
}

async fn fake_as() -> (String, Arc<Mutex<As>>, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let state = Arc::new(Mutex::new(As::default()));
    let (b, st) = (base.clone(), state.clone());
    let task = tokio::spawn(async move {
        while let Ok((mut s, _)) = listener.accept().await {
            let (b, st) = (b.clone(), st.clone());
            tokio::spawn(async move {
                let mut req = Vec::new();
                let mut buf = [0u8; 8192];
                let (head, body) = loop {
                    let n = s.read(&mut buf).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    req.extend_from_slice(&buf[..n]);
                    let t = String::from_utf8_lossy(&req).to_string();
                    if let Some(end) = t.find("\r\n\r\n") {
                        let len = t[..end]
                            .lines()
                            .find_map(|l| {
                                l.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                            })
                            .unwrap_or(0);
                        if req.len() >= end + 4 + len {
                            break (t[..end].to_string(), t[end + 4..].to_string());
                        }
                    }
                };
                let line = head.lines().next().unwrap_or_default().to_string();
                let path = line
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or_default()
                    .to_string();
                let form: std::collections::HashMap<String, String> =
                    url::form_urlencoded_parse(&body);
                let (status, extra, json) = if line.starts_with("POST /mcp") {
                    (401, format!("WWW-Authenticate: Bearer resource_metadata=\"{b}/.well-known/oauth-protected-resource/mcp\"\r\n"), json!({}))
                } else if path == "/.well-known/oauth-protected-resource/mcp" {
                    (
                        200,
                        String::new(),
                        json!({"resource": if st.lock().unwrap().wrong_resource { "https://other.example/mcp".to_string() } else { format!("{b}/mcp") }, "authorization_servers": [b.clone()]}),
                    )
                } else if path == "/.well-known/oauth-authorization-server"
                    && st.lock().unwrap().huge
                {
                    (
                        200,
                        String::new(),
                        json!({"pad": "x".repeat(2 * 1024 * 1024)}),
                    )
                } else if path == "/.well-known/oauth-authorization-server" {
                    (
                        200,
                        String::new(),
                        json!({"issuer": if st.lock().unwrap().wrong_issuer { "https://other.example".to_string() } else { b.clone() }, "authorization_endpoint": format!("{b}/authorize"), "token_endpoint": format!("{b}/token"), "registration_endpoint": format!("{b}/register"), "code_challenge_methods_supported": ["S256"]}),
                    )
                } else if path == "/register" {
                    (201, String::new(), json!({"client_id": "client-1"}))
                } else if path == "/token" {
                    let mut st = st.lock().unwrap();
                    match form.get("grant_type").map(String::as_str) {
                        Some("authorization_code") => {
                            let ok = form.get("code").map(String::as_str) == Some("good-code")
                                && st.challenge.as_deref()
                                    == form.get("code_verifier").map(|v| challenge(v)).as_deref()
                                && form.get("redirect_uri").is_some_and(|r| {
                                    r.starts_with("http://127.0.0.1:") && r.ends_with("/callback")
                                });
                            if ok {
                                st.tokens_issued += 1;
                                (
                                    200,
                                    String::new(),
                                    json!({"access_token": "at-1", "refresh_token": "rt-1", "expires_in": 3600, "token_type": "Bearer"}),
                                )
                            } else {
                                (
                                    400,
                                    String::new(),
                                    json!({"error": "invalid_grant", "error_description": "code at-leak-test"}),
                                )
                            }
                        }
                        Some("refresh_token")
                            if !st.revoked
                                && form.get("refresh_token").map(String::as_str)
                                    == Some("rt-1") =>
                        {
                            (
                                200,
                                String::new(),
                                json!({"access_token": "at-2", "refresh_token": "rt-2", "expires_in": 3600}),
                            )
                        }
                        _ => (400, String::new(), json!({"error": "invalid_grant"})),
                    }
                } else {
                    (404, String::new(), json!({}))
                };
                let body = json.to_string();
                let resp = format!("HTTP/1.1 {status} X\r\nContent-Type: application/json\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                let _ = s.write_all(resp.as_bytes()).await;
            });
        }
    });
    (base, state, task)
}

mod url {
    pub fn form_urlencoded_parse(body: &str) -> std::collections::HashMap<String, String> {
        reqwest::Url::parse(&format!("http://x/?{body}"))
            .map(|u| u.query_pairs().into_owned().collect())
            .unwrap_or_default()
    }
}

/// The browser: follow the authorization URL's redirect to the loopback
/// listener with `code` and the given state (the AS remembers the challenge).
async fn browser(auth_url: &str, st: &Arc<Mutex<As>>, code: &str, state_override: Option<&str>) {
    let u = reqwest::Url::parse(auth_url).unwrap();
    let q: std::collections::HashMap<String, String> = u.query_pairs().into_owned().collect();
    assert_eq!(q["code_challenge_method"], "S256");
    assert!(q["resource"].ends_with("/mcp"));
    st.lock().unwrap().challenge = Some(q["code_challenge"].clone());
    let state = state_override.unwrap_or(&q["state"]);
    let cb = format!("{}?code={code}&state={state}", q["redirect_uri"]);
    let _ = reqwest::get(cb).await;
}

fn now() -> i64 {
    1_000
}

#[tokio::test]
async fn sign_in_discovers_registers_exchanges_and_refreshes_with_rotation() {
    let (base, st, task) = fake_as().await;
    let server = format!("{base}/mcp");
    let (auth_url, pending) = start(&server, None).await.expect("start");
    assert!(
        auth_url.starts_with(&format!("{base}/authorize?")),
        "{auth_url}"
    );
    assert_eq!(pending.base.client_id, "client-1", "registered dynamically");
    let done = tokio::spawn(finish(pending, now));
    browser(&auth_url, &st, "good-code", None).await;
    let t = done.await.unwrap().expect("signed in");
    assert_eq!(
        (
            t.access_token.as_str(),
            t.refresh_token.as_deref(),
            t.expires_at
        ),
        ("at-1", Some("rt-1"), Some(4_600))
    );
    assert!(!t.needs_refresh(1_000) && t.needs_refresh(4_560));
    let r = refresh(&t, 2_000).await.expect("refresh");
    assert_eq!(
        (r.access_token.as_str(), r.refresh_token.as_deref()),
        ("at-2", Some("rt-2")),
        "rotated"
    );
    // A revoked grant: refresh fails, and the error carries no body text.
    st.lock().unwrap().revoked = true;
    let e = refresh(&t, 2_000).await.unwrap_err();
    assert!(e.contains("invalid_grant") && !e.contains("rt-1"), "{e}");
    task.abort();
}

#[tokio::test]
async fn a_wrong_state_is_refused_and_the_flow_keeps_waiting() {
    let (base, st, task) = fake_as().await;
    let (auth_url, pending) = start(&format!("{base}/mcp"), None).await.unwrap();
    let done = tokio::spawn(finish(pending, now));
    // An idle connection doesn't stall the listener.
    let _idle = tokio::net::TcpStream::connect(redirect_addr(&auth_url))
        .await
        .unwrap();
    browser(&auth_url, &st, "good-code", Some("forged")).await;
    assert_eq!(
        st.lock().unwrap().tokens_issued,
        0,
        "no exchange for a forged answer"
    );
    assert!(!done.is_finished());
    browser(&auth_url, &st, "good-code", None).await;
    let t = done.await.unwrap().expect("the real answer still signs in");
    assert_eq!(t.access_token, "at-1");
    assert_eq!(st.lock().unwrap().tokens_issued, 1);
    task.abort();
}

fn redirect_addr(auth_url: &str) -> String {
    let u = reqwest::Url::parse(auth_url).unwrap();
    let r = u
        .query_pairs()
        .find(|(k, _)| k == "redirect_uri")
        .unwrap()
        .1
        .to_string();
    let r = reqwest::Url::parse(&r).unwrap();
    format!("127.0.0.1:{}", r.port().unwrap())
}

#[tokio::test]
async fn a_pkce_mismatch_is_refused_by_the_token_endpoint() {
    let (base, st, task) = fake_as().await;
    let (auth_url, pending) = start(&format!("{base}/mcp"), None).await.unwrap();
    let done = tokio::spawn(finish(pending, now));
    browser(&auth_url, &st, "good-code", None).await;
    // (The AS saw another challenge than the verifier's.)
    st.lock().unwrap().challenge = Some("not-the-challenge".into());
    let e = done.await.unwrap().unwrap_err();
    assert!(e.contains("invalid_grant") && !e.contains("leak"), "{e}");
    task.abort();
}

#[tokio::test]
async fn an_oversized_metadata_answer_is_refused_not_buffered() {
    let (base, st, task) = fake_as().await;
    st.lock().unwrap().huge = true;
    let e = start(&format!("{base}/mcp"), None).await.err().unwrap();
    assert!(e.contains("no metadata"), "{e}");
    task.abort();
}

#[tokio::test]
async fn metadata_for_another_issuer_or_resource_is_refused() {
    let (base, st, task) = fake_as().await;
    st.lock().unwrap().wrong_issuer = true;
    let e = start(&format!("{base}/mcp"), None).await.err().unwrap();
    assert!(e.contains("issuer"), "{e}");
    {
        let mut s = st.lock().unwrap();
        (s.wrong_issuer, s.wrong_resource) = (false, true);
    }
    let e = start(&format!("{base}/mcp"), None).await.err().unwrap();
    assert!(e.contains("resource"), "{e}");
    task.abort();
}

#[tokio::test]
async fn manual_client_credentials_skip_registration() {
    let (base, _st, task) = fake_as().await;
    let (_url, pending) = start(
        &format!("{base}/mcp"),
        Some(("my-app".into(), Some("s3cret".into()))),
    )
    .await
    .unwrap();
    assert_eq!(pending.base.client_id, "my-app");
    assert_eq!(pending.base.client_secret.as_deref(), Some("s3cret"));
    task.abort();
}

#[test]
fn local_endpoints_only_for_a_server_as_local() {
    // A LAN server may name LAN https endpoints, never this Mac's loopback.
    assert!(checked_endpoint("http://127.0.0.1:9/token", Scope::Local).is_err());
    assert!(checked_endpoint("https://localhost/token", Scope::Local).is_err());
    assert!(checked_endpoint("https://10.0.0.5/token", Scope::Local).is_ok());
    assert!(checked_endpoint("http://10.0.0.5/token", Scope::Local).is_err());
    assert!(checked_endpoint("https://10.0.0.5/token", Scope::Remote).is_err());
    assert_eq!(
        server_scope("http://localhost:3000/mcp").unwrap(),
        Scope::Loopback
    );
}

#[test]
fn a_huge_expires_in_saturates_instead_of_overflowing() {
    let base = Tokens {
        access_token: String::new(),
        refresh_token: None,
        expires_at: None,
        token_endpoint: String::new(),
        client_id: String::new(),
        client_secret: None,
        resource: String::new(),
        auth_host: String::new(),
    };
    let t = tokens_from(
        &json!({"access_token": "a", "expires_in": i64::MAX}),
        &base,
        1_000,
    );
    assert_eq!(t.expires_at, Some(i64::MAX));
}

#[test]
fn remote_endpoints_must_be_https_and_not_local() {
    assert!(checked_endpoint("http://as.example/token", Scope::Remote).is_err());
    assert!(checked_endpoint("https://127.0.0.1/token", Scope::Remote).is_err());
    assert!(checked_endpoint("https://as.example/token", Scope::Remote).is_ok());
    // IP literals the resolver never sees: metadata, this host, link-local.
    for raw in [
        "https://169.254.169.254/x",
        "https://0.0.0.0:8443/x",
        "https://[fe80::1]/x",
        "https://[::]/x",
    ] {
        assert!(checked_endpoint(raw, Scope::Remote).is_err(), "{raw}");
    }
    assert!(checked_endpoint("http://127.0.0.1:9/token", Scope::Loopback).is_ok());
    // A local server's metadata still can't send Otto to plain http elsewhere.
    assert!(checked_endpoint("http://as.example/token", Scope::Loopback).is_err());
    assert!(checked_endpoint("https://as.example/token", Scope::Loopback).is_ok());
}

#[tokio::test]
async fn a_plain_http_server_off_loopback_cannot_sign_in() {
    let e = start("http://10.1.2.3/mcp", None).await.err().unwrap();
    assert!(e.contains("https"), "{e}");
}
