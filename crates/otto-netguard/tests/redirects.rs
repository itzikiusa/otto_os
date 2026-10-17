//! The redirect policies and the guarded client, over real sockets: an
//! upstream 30x must not bounce a fetch into loopback (or onto a non-http
//! scheme), and a guarded client must refuse a name that resolves to
//! loopback at connect time.
//!
//! The "upstream" is itself on loopback, so it is fetched with a PLAIN
//! client carrying only the policy under test: the first hop is allowed by
//! construction, every redirect hop is vetted by the policy.

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// A one-route HTTP/1.1 server answering every request with `response`
/// (a full head + body). Returns its origin.
async fn serve(response: String) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                return;
            };
            let response = response.clone();
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let _ = sock.read(&mut buf).await;
                let _ = sock.write_all(response.as_bytes()).await;
                let _ = sock.shutdown().await;
            });
        }
    });
    format!("http://{addr}")
}

fn redirect_to(location: &str) -> String {
    format!(
        "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )
}

fn ok_body(body: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

#[tokio::test]
async fn both_policies_stop_a_redirect_into_loopback() {
    let secret = serve(ok_body("internal secret")).await;
    for (name, policy) in [
        ("redirect_policy", otto_netguard::redirect_policy()),
        (
            "guarded_redirect_policy",
            otto_netguard::guarded_redirect_policy(),
        ),
    ] {
        let bounce = serve(redirect_to(&format!("{secret}/admin"))).await;
        let client = reqwest::Client::builder().redirect(policy).build().unwrap();
        let resp = client.get(&bounce).send().await.unwrap();
        // The hop was not followed: the caller sees the 302, never the body.
        assert_eq!(resp.status().as_u16(), 302, "{name}");
        assert_ne!(resp.text().await.unwrap(), "internal secret", "{name}");
    }
}

#[tokio::test]
async fn redirects_to_metadata_ipv6_loopback_and_other_schemes_are_stopped() {
    for target in [
        "http://169.254.169.254/latest/meta-data/",
        "http://[::1]:7700/api/v1/sessions",
        "http://2130706433/",
        "file:///etc/passwd",
        "ftp://example.com/x",
    ] {
        let bounce = serve(redirect_to(target)).await;
        let client = reqwest::Client::builder()
            .redirect(otto_netguard::guarded_redirect_policy())
            .build()
            .unwrap();
        let r = client.get(&bounce).send().await;
        // Either reqwest refuses the scheme itself or the policy stops the
        // hop; what must never happen is a followed redirect — which would
        // show up as a response from the target or a failure to CONNECT to it.
        match r {
            Ok(resp) => assert_eq!(resp.status().as_u16(), 302, "{target}"),
            Err(e) => assert!(
                !e.is_connect() && !e.is_timeout(),
                "{target}: the hop was dialled: {e}"
            ),
        }
    }
}

#[tokio::test]
async fn a_plain_client_without_the_policy_would_have_followed() {
    // Control: proves the fixture really redirects, so the tests above are
    // measuring the policy and not a broken server.
    let secret = serve(ok_body("internal secret")).await;
    let bounce = serve(redirect_to(&format!("{secret}/admin"))).await;
    let resp = reqwest::get(&bounce).await.unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    assert_eq!(resp.text().await.unwrap(), "internal secret");
}

#[tokio::test]
async fn the_guarded_client_refuses_a_name_that_resolves_to_loopback() {
    let origin = serve(ok_body("local")).await;
    let port = origin.rsplit(':').next().unwrap();
    let client = otto_netguard::guarded_client_builder().build().unwrap();
    // `localhost` goes through the resolver (no network), which vets the
    // addresses hyper would dial and refuses loopback.
    let err = client
        .get(format!("http://localhost:{port}/"))
        .send()
        .await
        .unwrap_err();
    assert!(err.is_connect() || err.to_string().contains("dns"), "{err}");
}
