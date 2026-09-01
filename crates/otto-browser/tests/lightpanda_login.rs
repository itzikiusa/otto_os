//! Gated end-to-end test: real `lightpanda` + a local login-form fixture
//! server, driving `BrowserEngine::login` directly. Skips (prints SKIP,
//! passes trivially) on any machine without a `lightpanda` binary — same
//! convention as `tests/lightpanda.rs`.
//!
//! Runs the engine directly rather than through the HTTP route/MCP tool: the
//! daemon's `otto_netguard::check_url` refuses loopback addresses by design
//! (SSRF guard), so a route-level test can never point at a local fixture.
//! This test bypasses that entirely and exercises exactly what
//! `crates/otto-server/src/routes/browser.rs`'s login route calls once past
//! its own netguard check: `otto_browser::BrowserEngine::login`.

use otto_browser::BrowserEngine;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const LOGIN_PAGE: &str = r#"<!doctype html><html><body>
<form action="/do-login" method="post">
  <input type="email" name="email" />
  <input type="password" name="password" />
  <button type="submit">Sign in</button>
</form>
</body></html>"#;

const WELCOME_PAGE: &str = r#"<!doctype html><html><body><h1>Welcome</h1></body></html>"#;

fn find_subslice(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Minimal raw HTTP/1.1 fixture, one task per connection: `GET /login`
/// serves the form; `POST /do-login` checks the urlencoded body against
/// fixed credentials and serves either the password-FREE welcome page
/// (success) or the login form again (failure) — exactly the "password
/// field gone" heuristic `drive_login` checks for.
async fn spawn_fixture() -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let mut buf = vec![0u8; 8192];
                let mut n = 0usize;
                loop {
                    let r = socket.read(&mut buf[n..]).await.unwrap_or(0);
                    if r == 0 {
                        return;
                    }
                    n += r;
                    if find_subslice(&buf[..n], b"\r\n\r\n").is_some() {
                        break;
                    }
                    if n == buf.len() {
                        buf.resize(buf.len() * 2, 0);
                    }
                }
                let Some(head_end) = find_subslice(&buf[..n], b"\r\n\r\n") else {
                    return;
                };
                let header_text = String::from_utf8_lossy(&buf[..head_end]).to_string();
                let mut lines = header_text.split("\r\n");
                let request_line = lines.next().unwrap_or_default().to_string();
                let content_length: usize = lines
                    .find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|v| v.trim().to_string())
                    })
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
                let mut body = buf[head_end + 4..n].to_vec();
                while body.len() < content_length {
                    let r = socket.read(&mut buf[..]).await.unwrap_or(0);
                    if r == 0 {
                        break;
                    }
                    body.extend_from_slice(&buf[..r]);
                }
                body.truncate(content_length);

                let (status, html) = if request_line.starts_with("GET /login") {
                    (200, LOGIN_PAGE.to_string())
                } else if request_line.starts_with("POST /do-login") {
                    let body_str = String::from_utf8_lossy(&body);
                    let ok = body_str.contains("email=test%40example.com")
                        && body_str.contains("password=hunter2");
                    if ok {
                        (200, WELCOME_PAGE.to_string())
                    } else {
                        (200, LOGIN_PAGE.to_string())
                    }
                } else {
                    (404, "not found".to_string())
                };
                let resp = format!(
                    "HTTP/1.1 {status} OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    html.len(),
                    html
                );
                let _ = socket.write_all(resp.as_bytes()).await;
            });
        }
    });
    addr
}

#[tokio::test]
async fn login_fills_submits_and_detects_success_over_real_cdp() {
    let Some(bin) = otto_browser::Lightpanda::locate(None) else {
        eprintln!("SKIP: no lightpanda binary on this machine");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let lp = otto_browser::Lightpanda::start(bin, tmp.path().into())
        .await
        .unwrap();
    let engine = otto_browser::LightpandaEngine::new(lp.cdp_url());

    let addr = spawn_fixture().await;
    let url = format!("http://{addr}/login");

    let logged_in = engine.login(&url, "test@example.com", "hunter2").await.unwrap();
    assert!(logged_in, "password field should be gone after a successful submit");

    lp.shutdown().await;
}

#[tokio::test]
async fn login_reports_failure_when_credentials_are_wrong() {
    let Some(bin) = otto_browser::Lightpanda::locate(None) else {
        eprintln!("SKIP: no lightpanda binary on this machine");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let lp = otto_browser::Lightpanda::start(bin, tmp.path().into())
        .await
        .unwrap();
    let engine = otto_browser::LightpandaEngine::new(lp.cdp_url());

    let addr = spawn_fixture().await;
    let url = format!("http://{addr}/login");

    let logged_in = engine.login(&url, "test@example.com", "wrong-password").await.unwrap();
    assert!(!logged_in, "password field should still be present after a failed submit");

    lp.shutdown().await;
}
