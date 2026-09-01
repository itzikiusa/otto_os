//! Gated end-to-end test against a real `lightpanda` binary.
//!
//! Install locally with `brew install lightpanda-io/tap/lightpanda` (or
//! download a release from https://github.com/lightpanda-io/browser) to run
//! this for real; on any machine without the binary it prints SKIP and
//! passes trivially — CI has no lightpanda install, so this never gates the
//! build there.
//!
//! Hermetic: serves a tiny fixture page over a local loopback `TcpListener`
//! (same raw-HTTP-fixture shape `tests/lightpanda_login.rs` uses) rather than
//! fetching a real internet host — a machine with the binary installed but no
//! network access (this sandbox, some CI-adjacent environments) must still
//! be able to run this. `otto_netguard` is never in this path (this drives
//! `LightpandaEngine` directly, not the HTTP route), so a loopback target is
//! fine here.

use otto_browser::BrowserEngine;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const FIXTURE_HTML: &str =
    r#"<!doctype html><html><head><title>Otto Fixture Page</title></head><body><h1>hi</h1></body></html>"#;

/// Minimal raw HTTP/1.1 fixture: serves `FIXTURE_HTML` for any request. Same
/// shape as `tests/lightpanda_login.rs::spawn_fixture`, trimmed to a single
/// static response since this test only needs one page, not a form flow.
async fn spawn_fixture() -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                // Drain (and discard) the request — a single static page has
                // nothing to branch on.
                let mut buf = [0u8; 4096];
                let _ = socket.read(&mut buf).await;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    FIXTURE_HTML.len(),
                    FIXTURE_HTML
                );
                let _ = socket.write_all(resp.as_bytes()).await;
            });
        }
    });
    addr
}

#[tokio::test]
async fn sidecar_serves_a_page_over_cdp() {
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
    let url = format!("http://{addr}/");

    let page = engine.fetch_page(&url).await.unwrap();
    assert_eq!(page.title, "Otto Fixture Page");
    assert!(!page.degraded);
    lp.shutdown().await;
}
