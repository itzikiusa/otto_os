//! A Confluence site reached over plain http (not loopback) never receives the
//! account's Basic credentials: EVERY public call refuses before sending,
//! including the paged walks that send the auth header themselves (5d19db346).
//!
//! The site is TEST-NET-1 (192.0.2.1, never routed): a call that skipped the
//! gate would fail as a connect error/timeout (`Upstream`), not the `Invalid`
//! "confluence base url" asserted here.

use otto_core::Error;
use otto_issues::ConfluenceClient;

fn refused<T: std::fmt::Debug>(what: &str, r: otto_core::Result<T>) {
    match r {
        Err(Error::Invalid(m)) if m.contains("confluence base url") => {}
        other => panic!("{what}: expected the TLS refusal, got {other:?}"),
    }
}

#[tokio::test]
async fn every_call_refuses_a_cleartext_non_loopback_site() {
    for base in [
        "http://192.0.2.1",
        "http://192.0.2.1/wiki/",
        "ws://192.0.2.1",
    ] {
        let c = ConfluenceClient::new(base, "me@example.com", "secret-token");
        refused("get_page", c.get_page("1").await);
        refused("create_page", c.create_page("S", "T", "<p/>", None).await);
        refused("update_page", c.update_page("1", "T", "<p/>", 3).await);
        refused("add_comment", c.add_comment("1", "<p/>").await);
        refused("list_spaces", c.list_spaces().await);
        refused("list_spaces_paged", c.list_spaces_paged().await);
        refused("search_pages", c.search_pages(Some("S"), "q").await);
        refused("search_cql", c.search_cql("type=page", None, 0, 25).await);
        refused("list_comments", c.list_comments("1").await);
        refused(
            "list_comments_since",
            c.list_comments_since("1", "2026-01-01T00:00:00.000Z").await,
        );
    }
}

#[tokio::test]
async fn loopback_dev_servers_may_stay_on_plain_http() {
    // Passes the gate, then fails to connect (closed port): an upstream error,
    // not the TLS refusal.
    let c = ConfluenceClient::new("http://127.0.0.1:1", "me@example.com", "t");
    assert!(matches!(c.get_page("1").await, Err(Error::Upstream(_))));
}
