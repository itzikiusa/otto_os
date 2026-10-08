//! Real Chromium positive-network acceptance. Only the final socket address is
//! substituted in a cfg(test) proxy; Fetch and proxy policy remain unchanged.
use super::super::chrome::LaunchSpec;
use super::super::hooks::NoopHooks;
use super::super::proxy::GuardProxy;
use super::super::types::{ChromeBuild, DownloadPolicy};
use super::chromium_tests::evaluate;
use super::*;
use std::collections::HashSet;
use std::sync::atomic::AtomicUsize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires OTTO_TEST_CHROME_BIN; real Chromium positive HTTP resource workload"]
async fn real_chromium_positive_http_bursts_preserve_bodies_input_and_frames() {
    let binary =
        PathBuf::from(std::env::var("OTTO_TEST_CHROME_BIN").expect("set OTTO_TEST_CHROME_BIN"));
    assert!(binary.is_file());
    let data = tempfile::tempdir().unwrap();
    let profile = data.path().to_path_buf();
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let local = listener.local_addr().unwrap();
    // A literal avoids DNS and is accepted by the production netguard policy.
    // No packet goes to this address: the test proxy maps it after vetting and
    // rejects every other destination. Documentation ranges are guard-blocked.
    let public = "198.20.0.1:8123".parse().unwrap();
    let proxy = GuardProxy::start_fixture(public, local).await.unwrap();
    let proxy_port = proxy.port();
    let paths = Arc::new(StdMutex::new(HashSet::new()));
    let served_paths = paths.clone();
    let fixture = tokio::spawn(async move {
        let mut sockets = JoinSet::new();
        loop {
            tokio::select! {
                Some(result) = sockets.join_next(), if !sockets.is_empty() => { result.unwrap(); },
                accepted = listener.accept() => {
                    let (mut stream, _) = accepted.unwrap();
                    let paths = served_paths.clone();
                    sockets.spawn(async move {
                        let mut bytes = Vec::new();
                        let complete = tokio::time::timeout(Duration::from_secs(5), async {
                            while !bytes.ends_with(b"\r\n\r\n") {
                                let Ok(byte) = stream.read_u8().await else { return false; };
                                bytes.push(byte);
                                assert!(bytes.len() < 8192);
                            }
                            true
                        }).await.unwrap();
                        if !complete { return; }
                        let request = String::from_utf8(bytes).unwrap();
                        let path = request.split_whitespace().nth(1).unwrap();
                        let body = if path == "/" {
                            "<!doctype html><title>Owned HTTP fixture</title><link rel=icon href=data:,><h1>Healthy HTTP bursts</h1><input id=entry><main></main>".to_owned()
                        } else if path.starts_with("/asset/") {
                            assert!(paths.lock().unwrap().insert(path.to_owned()), "asset fetched twice: {path}");
                            format!("owned-body:{path}")
                        } else {
                            panic!("unexpected fixture request: {path}");
                        };
                        let response = format!("HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}", if path == "/" { "text/html" } else { "text/plain" }, body.len());
                        stream.write_all(response.as_bytes()).await.unwrap();
                        stream.shutdown().await.unwrap();
                    });
                }
            }
        }
    });
    let proc = ChromeProcess::start(
        ProcessKey::Ephemeral,
        LaunchSpec {
            binary,
            build: ChromeBuild::Chrome,
            headed: false,
            user_data_dir: ProcessKey::Ephemeral.user_data_dir(data.path()),
            viewport: Viewport::default(),
            extra_args: proxy.chrome_args(),
            log_path: data.path().join("chrome.log"),
        },
        DownloadPolicy::Block,
        data.path().join("downloads"),
    )
    .await
    .unwrap();
    println!(
        "CHROME_START {}",
        json!({"version":proc.version,"browser_fetch":proc.browser_fetch,"profile":profile,"test_pid":std::process::id(),"fixture":local,"vetted_literal":public})
    );
    let session = LiveSession::create(
        proc.clone(),
        OpenParams {
            tab_id: "positive-http-probe".into(),
            workspace_id: "fixture".into(),
            owner_id: "fixture".into(),
            profile: "ephemeral".into(),
            viewport: Viewport::default(),
            url: None,
        },
        data.path().to_path_buf(),
        Arc::new(NoopHooks),
    )
    .await
    .unwrap();
    let attached = proc
        .conn
        .call(
            "Target.attachToTarget",
            json!({"targetId":session.target_id,"flatten":true}),
            None,
        )
        .await
        .unwrap();
    let observer_sid = attached["sessionId"].as_str().unwrap().to_owned();
    let (tx, mut events) = mpsc::channel(super::super::conn::QUEUE_MESSAGES);
    proc.conn.route(&observer_sid, tx);
    let failures = Arc::new(AtomicUsize::new(0));
    let failed = failures.clone();
    let observer = tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            if event.params["type"] == "Document" {
                println!("DOCUMENT_EVENT {} {}", event.method, event.params);
            }
            if event.method == "Network.loadingFailed" {
                failed.fetch_add(1, Ordering::SeqCst);
                println!("NETWORK_FAILURE {}", event.params);
            }
        }
    });
    proc.conn
        .call("Network.enable", json!({}), Some(&observer_sid))
        .await
        .unwrap();
    let mut viewer = session.attach("fixture", true).unwrap();
    let frames = Arc::new(AtomicUsize::new(0));
    let frame_count = frames.clone();
    let viewer_task = tokio::spawn(async move {
        while let Some(out) = viewer.recv().await {
            if let ViewerOut::Frame(bytes) = out {
                let header_len = u32::from_be_bytes(bytes[1..5].try_into().unwrap()) as usize;
                let header: Value = serde_json::from_slice(&bytes[5..5 + header_len]).unwrap();
                frame_count.fetch_add(1, Ordering::SeqCst);
                viewer
                    .handle(ClientMsg::Ack {
                        seq: header["seq"].as_u64().unwrap(),
                    })
                    .await;
            }
        }
    });
    session
        .navigate(NavAction::Goto, Some(format!("http://{public}/")))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if evaluate(&session, "!!document.querySelector('#entry')".into()).await == true {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        evaluate(&session, "location.origin".into()).await,
        "http://198.20.0.1:8123"
    );
    let mut observations = Vec::new();
    for round in 0..3 {
        let started = Instant::now();
        let bodies = evaluate(
            &session,
            format!(
                r#"Promise.all(Array.from({{length:128}}, async (_,i) => {{
            const path='/asset/{round}/'+i;
            const response=await fetch(path, {{cache:'no-store'}});
            if(!response.ok) throw new Error('HTTP '+response.status);
            const body=await response.text();
            if(body!=='owned-body:'+path) throw new Error('wrong body '+path);
            return body;
        }}))"#
            ),
        )
        .await;
        assert_eq!(bodies.as_array().unwrap().len(), 128);
        for i in 0..128 {
            assert_eq!(bodies[i], format!("owned-body:/asset/{round}/{i}"));
        }
        assert_eq!(paths.lock().unwrap().len(), (round + 1) * 128);
        evaluate(&session, "document.querySelector('input').value=''; document.querySelector('input').focus(); true".into()).await;
        session
            .dispatch_input(ClientMsg::Text {
                text: format!("round-{round}"),
            })
            .await
            .unwrap();
        assert_eq!(
            evaluate(&session, "document.querySelector('input').value".into()).await,
            format!("round-{round}")
        );
        let before = frames.load(Ordering::SeqCst);
        evaluate(
            &session,
            format!("document.querySelector('h1').textContent='HTTP round {round}'; true"),
        )
        .await;
        tokio::time::timeout(Duration::from_secs(5), async {
            while frames.load(Ordering::SeqCst) == before {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("fresh input/render must progress after each burst");
        assert!(session.is_live());
        assert!(!proc.conn.is_closed());
        assert_eq!(failures.load(Ordering::SeqCst), 0);
        observations.push(json!({"round":round,"bodies":128,"elapsed_ms":started.elapsed().as_secs_f64()*1000.0,"frames_total":frames.load(Ordering::SeqCst)}));
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    let screenshot = session
        .screenshot(&ScreenshotRequest::default())
        .await
        .unwrap();
    assert!(screenshot.bytes.len() > 100);
    session.close("closed").await;
    viewer_task.await.unwrap();
    assert_eq!(proc.session_count(), 0);
    proc.conn.unroute(&observer_sid);
    observer.abort();
    proc.shutdown().await;
    assert!(proc.conn.is_closed());
    drop(proxy);
    fixture.abort();
    assert!(fixture.await.unwrap_err().is_cancelled());
    // Both listeners must be reclaimed, not merely idle.
    tokio::time::timeout(Duration::from_secs(2), async {
        while TcpStream::connect(("127.0.0.1", proxy_port)).await.is_ok() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(TcpStream::connect(local).await.is_err());
    data.close().unwrap();
    assert!(!profile.exists());
    println!(
        "CHROME_RESULT {}",
        json!({"observations":observations,"bodies":paths.lock().unwrap().len(),"network_failures":failures.load(Ordering::SeqCst),"frames":frames.load(Ordering::SeqCst),"screenshot_bytes":screenshot.bytes.len(),"closed":true,"profile_removed":true})
    );
}
