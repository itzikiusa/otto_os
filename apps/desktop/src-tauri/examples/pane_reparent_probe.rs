//! Isolated synthetic WKWebView proof. Does not start ottod or load Otto state.
//! Run: cargo run --example pane_reparent_probe
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tauri::{
    LogicalPosition, LogicalSize, Manager, WebviewBuilder, WebviewUrl, WebviewWindowBuilder,
    WindowBuilder,
};

fn evaluate(view: &tauri::Webview, script: &str) -> String {
    let (tx, rx) = std::sync::mpsc::channel();
    view.eval_with_callback(script, move |value| {
        let _ = tx.send(value);
    })
    .unwrap();
    rx.recv_timeout(std::time::Duration::from_secs(10))
        .expect("JavaScript callback")
}

static PROBE_RESULT: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(2);
fn main() {
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    context.config_mut().identifier =
        format!("com.otto.pane-reparent-probe-{}", std::process::id());
    let loads = Arc::new(AtomicUsize::new(0));
    tauri::Builder::default()
        .register_uri_scheme_protocol("paneproof", |_, _| {
            tauri::http::Response::builder().header("Content-Type", "text/html").body(b"<!doctype html><title>Isolated pane proof</title><textarea id=draft>unsaved draft</textarea><div style='height:4000px'>Live document</div>".to_vec()).unwrap()
        })
        .setup(move |app| {
            let _host = WebviewWindowBuilder::new(app, "main", WebviewUrl::External("paneproof://localhost/host".parse().unwrap())).incognito(true).title("Isolated pane proof host").inner_size(800.0,600.0).build()?;
            let detached = WindowBuilder::new(app, "proof-detached").title("Isolated pane proof detached").inner_size(600.0,450.0).build()?;
            let counter = loads.clone();
            let host_window = app.get_window("main").unwrap();
            let child = host_window.add_child(WebviewBuilder::new("proof-child", WebviewUrl::External("paneproof://localhost/child".parse().unwrap())).incognito(true).on_page_load(move |_, payload| {
                if payload.event() == tauri::webview::PageLoadEvent::Finished { counter.fetch_add(1, Ordering::SeqCst); }
            }), LogicalPosition::new(400.0,0.0), LogicalSize::new(400.0,600.0))?;
            let app = app.handle().clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(2));
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    println!("HOST_LOOKUP after_add_child={} windows={:?}", app.get_webview_window("main").is_some(), app.webview_windows().keys().collect::<Vec<_>>());
                    child.hide().unwrap();
                    println!("HIDDEN_VISIBILITY {}", evaluate(&child,"document.visibilityState"));
                    child.show().unwrap();
                    let before = evaluate(&child, "window.liveToken=crypto.randomUUID(); window.liveCounter=0; window.timer=setInterval(()=>window.liveCounter++,20); document.getElementById('draft').value='edited but never saved'; window.scrollTo(0,300); JSON.stringify({token:window.liveToken,draft:document.getElementById('draft').value})");
                    for _ in 0..3 {
                        child.reparent(&detached).expect("detach");
                        child.set_position(LogicalPosition::new(0.0,0.0)).unwrap();
                        child.set_size(LogicalSize::new(600.0,450.0)).unwrap();
                        std::thread::sleep(std::time::Duration::from_millis(120));
                        assert_eq!(before, evaluate(&child,"JSON.stringify({token:window.liveToken,draft:document.getElementById('draft').value})"));
                        child.reparent(&host_window).expect("return");
                        assert_eq!(before, evaluate(&child,"JSON.stringify({token:window.liveToken,draft:document.getElementById('draft').value})"));
                    }
                    assert_eq!(loads.load(Ordering::SeqCst),1,"child must load exactly once");
                    let counter = evaluate(&child,"window.liveCounter");
                    assert!(counter.parse::<u32>().unwrap()>0,"timer stays live");
                    println!("PASS: 3 detach/return cycles; one page load; unsaved draft+document UUID retained; timer={counter}; scroll={}",evaluate(&child,"window.scrollY"));
                }));
                let code=if result.is_ok(){0}else{1};PROBE_RESULT.store(code,std::sync::atomic::Ordering::SeqCst);if code != 0 {std::process::exit(code)}app.exit(code);
            });
            Ok(())
        })
        .run(context).expect("isolated probe runtime");
    std::process::exit(PROBE_RESULT.load(std::sync::atomic::Ordering::SeqCst));
}
