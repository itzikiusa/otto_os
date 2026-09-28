//! Isolated acceptance fixture: production room window and ACL, no Otto startup.
//! Run through scripts/room-media-probe/native.py, never the installed app.
#[path = "../src/rooms.rs"]
mod rooms;

use std::{
    borrow::Cow,
    collections::BTreeMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

const PROBE: &str = include_str!("../../../../scripts/room-media-probe/native.js");
const PHYSICAL: &str = include_str!("../../../../scripts/room-media-probe/physical.js");
const WORKLET: &[u8] = include_bytes!("../../../../ui/public/room-recap-worklet.js");
static CUSTOM_CALLS: AtomicUsize = AtomicUsize::new(0);

// Deliberately has no caller guard: the runtime must reject remote custom IPC
// before reaching this harmless stand-in for the real supervisor command.
#[tauri::command]
fn daemon_restart() -> usize {
    CUSTOM_CALLS.fetch_add(1, Ordering::SeqCst) + 1
}

fn main() {
    let invitation = std::env::var("OTTO_ROOM_PROBE_INVITATION").expect("Use native.py");
    let expected = tauri::Url::parse(&invitation).expect("fixture invitation");
    assert_eq!(expected.host_str(), Some("127.0.0.1"));
    let physical = std::env::var("OTTO_ROOM_PROBE_PHYSICAL").is_ok_and(|value| value == "1");
    let cycles: usize = std::env::var("OTTO_ROOM_PROBE_CYCLES")
        .unwrap_or_else(|_| "1".into())
        .parse()
        .expect("cycles");
    assert!((1..=20).contains(&cycles));
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    context.config_mut().identifier = "com.otto.room-native-probe".into();
    let results = Arc::new(Mutex::new(BTreeMap::<String, serde_json::Value>::new()));
    let collected = results.clone();
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![rooms::open_room_window, daemon_restart])
        .on_page_load(move |view, payload| {
            if payload.event() != tauri::webview::PageLoadEvent::Finished { return; }
            let role = if view.label() == "main" { "local" } else { "guest" };
            let script = format!("window.__probeRole={};window.__probeInvitation={};window.__probeCycles={};{};void 0",
                serde_json::to_string(role).unwrap(), serde_json::to_string(&invitation).unwrap(), cycles, if physical { PHYSICAL } else { PROBE });
            view.eval(script).expect("inject fixture");
        })
        .setup(move |app| {
            // Blank bundled-origin document; never boot the product SPA or supervisor.
            WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                .title("Otto isolated native room probe")
                .incognito(true)
                .on_web_resource_request(|request, response| {
                    let worklet = request.uri().path().ends_with("room-recap-worklet.js");
                    *response.status_mut() = tauri::http::StatusCode::OK;
                    response.headers_mut().insert("Content-Type", if worklet { "text/javascript" } else { "text/html" }.parse().unwrap());
                    *response.body_mut() = Cow::Borrowed(if worklet { WORKLET } else { b"<!doctype html><title>Isolated native room probe</title>" });
                })
                .build()?;
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                let start = Instant::now();
                loop {
                    for (label, window) in handle.webview_windows() {
                        let collected = collected.clone();
                        let key = label.clone();
                        let _ = window.eval_with_callback("window.__probeResult || null", move |raw| {
                            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) {
                                if value.is_object() { collected.lock().unwrap().insert(key.clone(), value); }
                            }
                        });
                    }
                    let snapshot = collected.lock().unwrap().clone();
                    let local = snapshot.get("main");
                    let guest = snapshot.iter().find(|(key, _)| key.starts_with("room-"));
                    if let (Some(_), Some((guest_label, _))) = (local, guest) {
                        let guest_url = handle.get_webview_window(guest_label).and_then(|w| w.url().ok());
                        let confined = guest_url.as_ref() == Some(&expected);
                        let window_count = handle.webview_windows().len();
                        // Exactly one call must come from the local positive control.
                        let custom_calls = CUSTOM_CALLS.load(Ordering::SeqCst);
                        let passed = snapshot.values().all(|v| v["passed"] == true) && confined && window_count == 2 && custom_calls == 1;
                        println!("{}", serde_json::json!({"event":"done","passed":passed,"navigation_confined":confined,"window_count":window_count,"custom_command_calls":custom_calls,"results":snapshot}));
                        handle.exit(if passed { 0 } else { 1 });
                        break;
                    }
                    if start.elapsed() > Duration::from_secs(if physical { 900 } else { 35 }) {
                        println!("{}", serde_json::json!({"event":"timeout","results":snapshot}));
                        handle.exit(2); break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            });
            Ok(())
        })
        .run(context)
        .expect("native fixture runtime");
}
