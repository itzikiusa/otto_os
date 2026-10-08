//! Opt-in protocol acceptance against a real Chrome for Testing executable.
//! No remote site is contacted: the page is installed over CDP and its only
//! HTTP requests target a fixture listener that the production guard must deny.
use super::super::chrome::LaunchSpec;
use super::super::hooks::NoopHooks;
use super::super::proxy::GuardProxy;
use super::super::types::{ChromeBuild, DownloadPolicy};
use super::*;
use std::sync::atomic::AtomicUsize;

pub(super) async fn evaluate(session: &LiveSession, expression: String) -> Value {
    let reply = session
        .proc
        .conn
        .call_with_timeout(
            "Runtime.evaluate",
            json!({"expression": expression, "awaitPromise": true, "returnByValue": true}),
            Some(&session.sid),
            Duration::from_secs(15),
        )
        .await
        .expect("real Chromium must answer within the workload deadline");
    assert!(reply.get("exceptionDetails").is_none(), "{reply}");
    reply["result"]["value"].clone()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires OTTO_TEST_CHROME_BIN; isolated real Chromium resource workload"]
async fn real_chromium_resource_bursts_keep_navigation_and_frames_live() {
    let binary =
        PathBuf::from(std::env::var("OTTO_TEST_CHROME_BIN").expect("set OTTO_TEST_CHROME_BIN"));
    assert!(binary.is_file());
    let data = tempfile::tempdir().unwrap();
    let proxy = GuardProxy::start().await.unwrap();
    let forbidden = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let forbidden_port = forbidden.local_addr().unwrap().port();
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
        json!({"version":proc.version, "browser_fetch":proc.browser_fetch, "profile":data.path(), "test_pid":std::process::id()})
    );
    let session = LiveSession::create(
        proc.clone(),
        OpenParams {
            tab_id: "resource-probe".into(),
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

    // Observe real Network failures on a separate CDP session, without changing
    // Fetch interception on the production session.
    let attached = proc
        .conn
        .call(
            "Target.attachToTarget",
            json!({"targetId":session.target_id,"flatten":true}),
            None,
        )
        .await
        .unwrap();
    let observer_sid = attached["sessionId"].as_str().unwrap().to_string();
    let (tx, mut events) = mpsc::channel(super::super::conn::QUEUE_MESSAGES);
    proc.conn.route(&observer_sid, tx);
    let blocked = Arc::new(AtomicUsize::new(0));
    let count = blocked.clone();
    let observer = tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            if event.method == "Network.loadingFailed"
                && event.params["blockedReason"] == "inspector"
                && event.params["errorText"]
                    .as_str()
                    .is_some_and(|text| text.starts_with("net::ERR_BLOCKED_BY_CLIENT"))
            {
                count.fetch_add(1, Ordering::SeqCst);
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
    evaluate(
        &session,
        "document.body.innerHTML='<h1>Chromium resource probe</h1><input id=entry><main></main>'; true".into(),
    )
    .await;
    let mut expected_blocked = 0;
    let mut observations = Vec::new();
    for round in 0..12 {
        let n = [1, 32, 128][round % 3];
        let started = Instant::now();
        evaluate(&session, "document.querySelector('input').value=''; document.querySelector('input').focus(); true".into()).await;
        session
            .dispatch_input(ClientMsg::Text {
                text: format!("input-{round}"),
            })
            .await
            .unwrap();
        assert_eq!(
            evaluate(&session, "document.querySelector('input').value".into()).await,
            format!("input-{round}")
        );
        let result = evaluate(&session, format!(r#"(async () => {{
            const host=document.querySelector('main'); host.replaceChildren();
            const images=Array.from({{length:{n}}}, (_,i) => {{
                const image=new Image(24,24);
                image.src='data:image/svg+xml,'+encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24"><rect width="24" height="24" fill="'+(i%2?'blue':'green')+'"/></svg>');
                host.append(image); return image.decode();
            }});
            await Promise.all(images);
            const failed=await Promise.all(Array.from({{length:{n}}}, (_,i) => fetch('http://127.0.0.1:{forbidden_port}/blocked/{round}/'+i).then(()=>false,()=>true)));
            for(let i=0;i<8;i++) history.pushState(null,'','#round-{round}-'+i);
            document.title='round-{round}';
            document.body.style.backgroundColor={round}%2?'white':'silver';
            return {{images:host.children.length,failed:failed.filter(Boolean).length,hash:location.hash}};
        }})()"#)).await;
        assert_eq!(result["images"], n);
        assert_eq!(result["failed"], n);
        assert_eq!(result["hash"], format!("#round-{round}-7"));
        expected_blocked += n as usize;
        tokio::time::timeout(Duration::from_secs(5), async {
            while blocked.load(Ordering::SeqCst) < expected_blocked {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("all requests must fail specifically through the guard");
        let before = frames.load(Ordering::SeqCst);
        evaluate(
            &session,
            format!("document.querySelector('h1').textContent='paint {round}'; true"),
        )
        .await;
        tokio::time::timeout(Duration::from_secs(5), async {
            while frames.load(Ordering::SeqCst) == before {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("viewer must still receive and acknowledge new frames");
        assert!(session.is_live());
        assert!(!proc.conn.is_closed());
        assert_eq!(session.viewer_count(), 1);
        observations.push(json!({"round":round,"resources":n,"elapsed_ms":started.elapsed().as_secs_f64()*1000.0,"blocked_total":blocked.load(Ordering::SeqCst),"frames_total":frames.load(Ordering::SeqCst)}));
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(100), forbidden.accept())
            .await
            .is_err(),
        "guard must never contact the forbidden listener"
    );
    let screenshot = session
        .screenshot(&ScreenshotRequest::default())
        .await
        .unwrap();
    assert!(screenshot.bytes.len() > 100);
    session.close("closed").await;
    assert_eq!(proc.session_count(), 0);
    viewer_task.await.unwrap();
    proc.conn.unroute(&observer_sid);
    observer.abort();
    proc.shutdown().await;
    assert!(proc.conn.is_closed());
    println!(
        "CHROME_RESULT {}",
        json!({"observations":observations,"blocked":blocked.load(Ordering::SeqCst),"frames":frames.load(Ordering::SeqCst),"screenshot_bytes":screenshot.bytes.len(),"closed":true})
    );
}
