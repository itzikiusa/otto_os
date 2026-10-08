use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// A normal resource burst must not depend on spawned guard workers winning
/// the scheduler before the next already-buffered browser event is consumed.
#[tokio::test]
async fn cached_public_resource_burst_does_not_fail_before_workers_are_polled() {
    const REQUESTS: usize = 128;
    let (ours, browser) = tokio::io::duplex(1 << 20);
    let (read, write) = tokio::io::split(ours);
    let (mut browser_read, mut browser_write) = tokio::io::split(browser);
    let (conn, mut events) = CdpConn::start(read, write);
    let mut cache = VerdictCache::default();
    cache.insert(
        guard::origin_key("https://example.com/"),
        true,
        Instant::now(),
    );
    let proc = process_fixture(conn, cache);
    for index in 0..REQUESTS {
        let mut frame = serde_json::to_vec(&json!({
            "method": "Fetch.requestPaused",
            "sessionId": "page-session",
            "params": {
                "requestId": format!("resource-{index}"),
                "resourceType": "Image",
                "request": {
                    "url": format!("https://example.com/image-{index}.png"),
                    "method": "GET"
                }
            }
        }))
        .unwrap();
        frame.push(0);
        browser_write.write_all(&frame).await.unwrap();
    }
    // Collect real decoded events first, then drive the synchronous pump. On a
    // current-thread runtime none of its spawned workers run inside this loop.
    let mut buffered = Vec::new();
    for _ in 0..REQUESTS {
        buffered.push(events.recv().await.unwrap());
    }
    for event in buffered {
        proc.on_browser_event(event);
    }
    let mut continued = 0;
    let mut failed = 0;
    tokio::time::timeout(Duration::from_secs(2), async {
        for _ in 0..REQUESTS {
            let mut frame = Vec::new();
            loop {
                let byte = browser_read.read_u8().await.unwrap();
                if byte == 0 {
                    break;
                }
                frame.push(byte);
            }
            let command: Value = serde_json::from_slice(&frame).unwrap();
            assert_eq!(command["sessionId"], "page-session");
            match command["method"].as_str().unwrap() {
                "Fetch.continueRequest" => continued += 1,
                "Fetch.failRequest" => failed += 1,
                other => panic!("unexpected command: {other}"),
            }
        }
    })
    .await
    .expect("a cached public burst must settle promptly");
    proc.conn.shutdown();
    assert_eq!(
        (continued, failed),
        (REQUESTS, 0),
        "cached public resources were rejected solely because guard workers had not run"
    );
}

fn process_fixture(conn: Arc<CdpConn>, cache: VerdictCache) -> Arc<ChromeProcess> {
    Arc::new(ChromeProcess {
        key: ProcessKey::Ephemeral,
        build: ChromeBuild::Chrome,
        version: "burst-fixture".into(),
        headed: false,
        conn,
        download_policy: DownloadPolicy::Block,
        downloads_dir: PathBuf::new(),
        browser_fetch: true,
        child: AsyncMutex::new(None),
        targets: StdMutex::new(HashMap::new()),
        popups: StdMutex::new(HashMap::new()),
        downloads: StdMutex::new(HashMap::new()),
        verdicts: StdMutex::new(cache),
        guard_slots: Arc::new(Semaphore::new(GUARD_SLOTS)),
        empty_since: StdMutex::new(None),
        dead: AtomicBool::new(false),
        loop_task: StdMutex::new(None),
    })
}

/// The inline path may bypass worker admission only for a fresh positive
/// verdict and a safe method. All other cases still meet fail-closed admission.
#[tokio::test]
async fn cached_fast_path_preserves_expiry_denial_and_outward_guarding() {
    for (case, method, verdict, age) in [
        ("denied", "GET", Some(false), Duration::ZERO),
        ("expired", "GET", Some(true), guard::VERDICT_TTL),
        ("uncached", "GET", None, Duration::ZERO),
        ("outward-post", "POST", Some(true), Duration::ZERO),
        ("outward-delete", "DELETE", Some(true), Duration::ZERO),
    ] {
        let (ours, mut browser) = tokio::io::duplex(4096);
        let (read, write) = tokio::io::split(ours);
        let (conn, mut events) = CdpConn::start(read, write);
        let mut cache = VerdictCache::default();
        if let Some(allowed) = verdict {
            cache.insert(
                guard::origin_key("https://example.com/"),
                allowed,
                Instant::now() - age,
            );
        }
        let proc = process_fixture(conn, cache);
        let _busy = proc
            .guard_slots
            .clone()
            .acquire_many_owned(GUARD_SLOTS as u32)
            .await
            .unwrap();
        let mut event = serde_json::to_vec(&json!({
            "method": "Fetch.requestPaused", "sessionId": "page-session",
            "params": {"requestId": case, "resourceType": "Document",
                "request": {"url": "https://example.com/", "method": method}}
        }))
        .unwrap();
        event.push(0);
        browser.write_all(&event).await.unwrap();
        proc.on_browser_event(events.recv().await.unwrap());
        let command = tokio::time::timeout(Duration::from_secs(1), async {
            let mut frame = Vec::new();
            loop {
                let byte = browser.read_u8().await.unwrap();
                if byte == 0 {
                    break;
                }
                frame.push(byte);
            }
            serde_json::from_slice::<Value>(&frame).unwrap()
        })
        .await
        .expect("non-fast-path requests must respect bounded guard admission");
        assert_eq!(command["method"], "Fetch.failRequest", "case: {case}");
        assert_eq!(command["params"]["requestId"], case);
        assert_eq!(command["sessionId"], "page-session");
        proc.conn.shutdown();
    }
}
