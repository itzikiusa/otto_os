//! Isolated native acceptance of production hosted-room commands. All bundled
//! assets are replaced by a blank fixture; no product boot, daemon, real room,
//! persistent credentials, microphone, screen capture or registry is touched.
#[path = "../src/throttle.rs"]
mod throttle;
use throttle::NO_THROTTLE;
#[path = "../src/host_rooms.rs"]
mod host_rooms;

use std::borrow::Cow;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::{Duration, Instant};
use tauri::utils::assets::{AssetKey, AssetsIter, CspHash};
use tauri::{Manager, Webview, WebviewUrl, WebviewWindowBuilder};

struct BlankAssets;
impl tauri::Assets<tauri::Wry> for BlankAssets {
    fn get(&self, _: &AssetKey) -> Option<Cow<'_, [u8]>> {
        Some(Cow::Borrowed(b"<!doctype html><title>Isolated hosted room probe</title><p>Hosted room native fixture</p>"))
    }
    fn iter(&self) -> Box<AssetsIter<'_>> {
        Box::new(std::iter::empty())
    }
    fn csp_hashes(&self, _: &AssetKey) -> Box<dyn Iterator<Item = CspHash<'_>> + '_> {
        Box::new(std::iter::empty())
    }
}

fn evaluate(view: &Webview, script: &str) -> serde_json::Value {
    let (tx, rx) = std::sync::mpsc::channel();
    view.eval_with_callback(script, move |raw| {
        let _ = tx.send(raw);
    })
    .unwrap();
    let raw = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("JavaScript response");
    serde_json::from_str(&raw).expect("JSON response")
}

fn wait(view: &Webview, script: &str) {
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(15) {
        if evaluate(view, script) == true {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("Timed out waiting for fixture assertion: {script}");
}

static RESULT: AtomicI32 = AtomicI32::new(2);

fn main() {
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    context.config_mut().identifier = format!("com.otto.host-room-probe-{}", std::process::id());
    context.set_assets(Box::new(BlankAssets));
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![host_rooms::open_host_room_window, host_rooms::get_host_room_context])
        .setup(|app| {
            let (tx, rx) = std::sync::mpsc::channel();
            WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html#/agents".into()))
                .title("Otto isolated hosted-room acceptance")
                .incognito(true)
                .on_page_load(move |_, event| { if event.event() == tauri::webview::PageLoadEvent::Finished { let _ = tx.send(()); } })
                .build()?;
            let app = app.handle().clone();
            std::thread::spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    rx.recv_timeout(Duration::from_secs(15)).expect("Main fixture page loaded");
                    let main = app.get_webview("main").unwrap();
                    evaluate(&main, r#"window.args={origin:'http://127.0.0.1:7700',credential:{room_id:'probe_room',member_id:'probe_host',token:'0123456789abcdef0123456789abcdef'}};window.mainDocument=document.body;window.draft=document.createElement('textarea');window.draft.value='unsaved agent work';document.body.appendChild(window.draft);window.openResults=null;Promise.all(Array.from({length:8},()=>window.__TAURI_INTERNALS__.invoke('open_host_room_window',window.args))).then(values=>window.openResults=values).catch(()=>window.openFailed=true);true"#);
                    wait(&main, "window.openResults!==null || window.openFailed===true");
                    assert_eq!(evaluate(&main, "!window.openFailed && new Set(window.openResults).size===1"), true, "concurrent calls focus one room window");
                    let label = evaluate(&main, "window.openResults[0]").as_str().unwrap().to_string();
                    let host = app.get_webview(&label).expect("Hosted native view");
                    wait(&host, "!!document.body && !!window.__TAURI_INTERNALS__");
                    assert_eq!(host.url().unwrap().as_str(), "tauri://localhost/index.html#/room-host/probe_room");
                    assert_eq!(app.webview_windows().len(), 2);
                    assert_eq!(evaluate(&main, "document.body===window.mainDocument && window.draft.value==='unsaved agent work' && location.hash==='#/agents'"), true);
                    evaluate(&main, "window.contextDenied=false;window.__TAURI_INTERNALS__.invoke('get_host_room_context').then(()=>window.contextLeaked=true).catch(()=>window.contextDenied=true);true");
                    wait(&main, "window.contextDenied===true");
                    evaluate(&host, "window.contextOk=false;window.__TAURI_INTERNALS__.invoke('get_host_room_context').then(value=>{window.contextOk=value.origin==='http://127.0.0.1:7700' && value.credential.room_id==='probe_room' && value.credential.member_id==='probe_host' && value.credential.token==='0123456789abcdef0123456789abcdef'});true");
                    wait(&host, "window.contextOk===true");
                    // A fragment mutation may be a same-document navigation in
                    // WebKit. The context command must independently reject it.
                    evaluate(&host, "location.hash='#/agents';window.wrongRouteDenied=false;window.__TAURI_INTERNALS__.invoke('get_host_room_context').then(()=>window.wrongRouteLeaked=true).catch(()=>window.wrongRouteDenied=true);true");
                    wait(&host, "window.wrongRouteDenied===true || location.hash==='#/room-host/probe_room'");
                    if evaluate(&host, "location.hash==='#/agents'") == true {
                        assert_eq!(evaluate(&host, "window.wrongRouteDenied===true && !window.wrongRouteLeaked"), true);
                    }
                    evaluate(&host, "location.hash='#/room-host/probe_room';true");
                    evaluate(&host, "location.href='https://example.invalid/';true");
                    std::thread::sleep(Duration::from_millis(150));
                    assert_eq!(host.url().unwrap().as_str(), "tauri://localhost/index.html#/room-host/probe_room");
                    evaluate(&host, "window.open('https://example.invalid/','_blank');true");
                    std::thread::sleep(Duration::from_millis(150));
                    assert_eq!(app.webview_windows().len(), 2);
                    evaluate(&host, "window.spawnDenied=false;window.__TAURI_INTERNALS__.invoke('get_host_room_context').then(value=>window.__TAURI_INTERNALS__.invoke('open_host_room_window',value)).then(()=>window.spawnLeaked=true).catch(()=>window.spawnDenied=true);true");
                    wait(&host, "window.spawnDenied===true");
                    app.get_webview_window(&label).unwrap().destroy().unwrap();
                    let deadline = Instant::now() + Duration::from_secs(5);
                    while app.get_webview(&label).is_some() && Instant::now() < deadline { std::thread::sleep(Duration::from_millis(50)); }
                    assert!(app.get_webview(&label).is_none());
                    evaluate(&main, "window.reopened=null;window.__TAURI_INTERNALS__.invoke('open_host_room_window',window.args).then(value=>window.reopened=value);true");
                    wait(&main, "window.reopened!==null");
                    assert_ne!(evaluate(&main, "window.reopened"), label);
                    assert_eq!(evaluate(&main, "document.body===window.mainDocument && window.draft.value==='unsaved agent work' && location.hash==='#/agents'"), true);
                    println!("PASS hosted room: eight concurrent opens produce one native window; correct-view-only credential IPC; wrong route and native spawn denied; external navigation/new windows denied; close/reopen new identity; source document/draft/navigation preserved");
                }));
                let code = if result.is_ok() { 0 } else { 1 };
                RESULT.store(code, Ordering::SeqCst);
                app.exit(code);
            });
            Ok(())
        })
        .build(context).expect("Probe runtime")
        .run(|_, event| {
            if let tauri::RunEvent::WindowEvent { label, event: tauri::WindowEvent::Destroyed, .. } = event {
                host_rooms::destroyed(&label);
            }
        });
    std::process::exit(RESULT.load(Ordering::SeqCst));
}
