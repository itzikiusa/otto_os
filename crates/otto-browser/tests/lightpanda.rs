//! Gated end-to-end test against a real `lightpanda` binary.
//!
//! Install locally with `brew install lightpanda-io/tap/lightpanda` (or
//! download a release from https://github.com/lightpanda-io/browser) to run
//! this for real; on any machine without the binary it prints SKIP and
//! passes trivially — CI has no lightpanda install, so this never gates the
//! build there.

use otto_browser::BrowserEngine;

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
    let page = engine.fetch_page("https://example.com").await.unwrap();
    assert!(page.title.to_lowercase().contains("example"));
    assert!(!page.degraded);
    lp.shutdown().await;
}
