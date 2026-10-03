//! Isolated full-SPA native acceptance. Requires a freshly built ui/dist and
//! OTTO_PANE_UI_CONFIG pointing to {base, token, workspace} for a throwaway
//! daemon. Both host and child use nonpersistent WK stores; no installed app
//! state, daemon setup, capture devices, or window registry is touched.
#![allow(dead_code, unused_imports)]
#[path = "../src/panes.rs"]
mod panes;
#[path = "../src/panes_policy.rs"]
mod panes_policy;
#[path = "../src/popout.rs"]
mod popout;
#[path = "../src/windows.rs"]
mod windows;
mod bar {
    pub const LABEL: &str = "otto-bar";
    pub fn hide(_: &tauri::AppHandle) {}
}
mod tray {
    pub const POPOVER_LABEL: &str = "otto-tray";
    pub fn hide_popover(_: &tauri::AppHandle) {}
}
use tauri::{Emitter, Manager, Webview, WebviewUrl, WebviewWindowBuilder};
#[tauri::command(rename = "windows_registry")]
fn probe_windows_registry() -> Vec<String> {
    vec!["main".into()]
}
fn evaluate(view: &Webview, script: &str) -> serde_json::Value {
    let (tx, rx) = std::sync::mpsc::channel();
    view.eval_with_callback(script, move |v| {
        let _ = tx.send(v);
    })
    .unwrap();
    let raw = rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("JavaScript response");
    serde_json::from_str(&raw).unwrap_or_else(|_| panic!("Invalid JS response: {raw}"))
}
fn wait(view: &Webview, script: &str) {
    for _ in 0..300 {
        if evaluate(view, script) == true {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    panic!(
        "Timed out waiting for {script}; page={}",
        evaluate(view, "document.body.innerText.slice(0,1800)")
    );
}
fn menu(view: &Webview, label: &str) {
    evaluate(
        view,
        "document.querySelector('[data-testid=pane-window-menu]').click();true",
    );
    wait(
        view,
        "!!document.querySelector('.ctx-menu [role=menuitem]')",
    );
    evaluate(view,&format!("Array.from(document.querySelectorAll('.ctx-menu [role=menuitem]')).find(n=>n.textContent.trim()==={}).click();true",serde_json::to_string(label).unwrap()));
}
fn assert_host_fills_window(host: &Webview) {
    let native_width =
        host.window().inner_size().unwrap().width as f64 / host.window().scale_factor().unwrap();
    let script = format!("Math.abs(innerWidth-{native_width})<2");
    wait(host, &script);
    let view = host.size().unwrap();
    assert_eq!(
        view,
        host.window().inner_size().unwrap(),
        "primary native webview fills host after multiwebview transition"
    );
}
fn assert_native_content_filled(view: &Webview) {
    let window = view.window();
    let native = window.ns_window().unwrap() as usize;
    let (tx, rx) = std::sync::mpsc::channel();
    window
        .run_on_main_thread(move || unsafe {
            let window = &*(native as *mut objc2::runtime::AnyObject);
            let content: *mut objc2::runtime::AnyObject = objc2::msg_send![window, contentView];
            let bounds: objc2_foundation::NSRect = objc2::msg_send![content, bounds];
            let _ = tx.send((bounds.size.width, bounds.size.height));
        })
        .unwrap();
    let expected = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
    let actual = view
        .size()
        .unwrap()
        .to_logical::<f64>(window.scale_factor().unwrap());
    assert!(
        (actual.width - expected.0).abs() < 1.0 && (actual.height - expected.1).abs() < 1.0,
        "native content {expected:?}, child {actual:?}"
    );
}
static PROBE_RESULT: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(2);
#[cfg(debug_assertions)]
fn main() {
    let path = std::env::var("OTTO_PANE_UI_CONFIG").expect("isolated fixture metadata path");
    let metadata: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path).expect("fixture metadata")).unwrap();
    let base = metadata["base"].as_str().expect("fixture base");
    let url = base.parse::<tauri::Url>().unwrap();
    assert_eq!(url.host_str(), Some("127.0.0.1"));
    assert_ne!(url.port(), Some(7700));
    let values = serde_json::json!({"otto_base":base,"otto_token":metadata["token"],"otto_workspace":metadata["workspace"],"otto_rail_expanded":"1","otto_firstrun_dismissed":"1","otto_client_id":"detachable-native-probe"});
    let init=format!("window.__OTTO_PROBE_ISOLATED__=localStorage.length===0;for(const [key,value] of Object.entries({values}))localStorage.setItem(key,String(value));");
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    context.config_mut().identifier = format!("com.otto.pane-spa-probe-{}", std::process::id());
    context.config_mut().app.security.csp=Some(tauri::utils::config::Csp::Policy(format!("default-src 'self'; script-src 'self' 'wasm-unsafe-eval' 'unsafe-eval'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob: https:; font-src 'self' data:; connect-src 'self' {base} {} blob: data: ipc: http://ipc.localhost; worker-src 'self' blob:; frame-src 'self' blob: data: https: http:; media-src 'self' blob: data:; object-src 'none'; base-uri 'self'",base.replacen("http","ws",1))));
    tauri::Builder::default().invoke_handler(tauri::generate_handler![probe_windows_registry,panes::pane_open,panes::pane_layout,panes::pane_state,panes::pane_to_host,panes::pane_to_guest,panes::pane_detach,panes::pane_return,panes::pane_focus,panes::pane_window_action,panes::pane_monitors,panes::pane_close])
        .setup(move|app|{
            app.manage(panes::ProbeIsolation{initialization_script:init.clone()});
            let (loaded_tx,loaded_rx)=std::sync::mpsc::channel();
            WebviewWindowBuilder::new(app,"main",WebviewUrl::App("index.html#/agents".into())).incognito(true).initialization_script(&init).on_page_load(move |_,payload| {if payload.event()==tauri::webview::PageLoadEvent::Finished {let _=loaded_tx.send(());}}).title("Isolated Otto pane SPA acceptance").inner_size(1500.0,900.0).build()?;
            let app=app.handle().clone();std::thread::spawn(move||{
                let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||{
                    loaded_rx.recv_timeout(std::time::Duration::from_secs(20)).expect("bundled host page finished loading");
                    let host=app.get_webview("main").unwrap();
                    wait(&host,"!!document.querySelector('.navigator [data-nav-id=connections]')");
                    assert_eq!(evaluate(&host,"window.__OTTO_PROBE_ISOLATED__"),true);
                    evaluate(&host,"document.querySelector('.navigator [data-nav-id=connections]').dispatchEvent(new MouseEvent('click',{bubbles:true,altKey:true}));true");
                    wait(&host,"!!document.querySelector('[data-testid=side-pane]')&&!document.querySelector('[data-testid=side-pane-cover]')");
                    let child=app.get_webview("otto-pane-main").expect("live side webview");
                    wait(&child,"!!document.querySelector('[data-testid=pane-controls]')");
                    assert_eq!(evaluate(&child,"window.__OTTO_PROBE_ISOLATED__"),true);
                    assert_eq!(evaluate(&child,"window.__OTTO_NATIVE_PANE__.host"),"main");
                    assert_eq!(evaluate(&child,"localStorage.getItem('otto_client_id')"),evaluate(&host,"localStorage.getItem('otto_client_id')"));
                    assert_host_fills_window(&host);
                    for view in [&host,&child]{evaluate(view,"window.liveToken=crypto.randomUUID();window.liveNode=document.querySelector('main')||document.body;window.probeDraft=document.createElement('textarea');window.probeDraft.value='never saved';window.probeDraft.dataset.probe='draft';document.body.appendChild(window.probeDraft);true");}
                    let host_token=evaluate(&host,"window.liveToken");let child_token=evaluate(&child,"window.liveToken");
                    for which in ["side","main"]{
                        menu(&host,&format!("Detach {which} pane"));
                        wait(&host,"!document.querySelector('[data-testid=split-divider]')");
                        assert!(app.get_window("otto-pane-window-main").is_some());
                        assert_host_fills_window(&host);
                        assert_native_content_filled(&child);
                        assert_eq!(evaluate(&host,"window.liveToken"),host_token);assert_eq!(evaluate(&child,"window.liveToken"),child_token);
                        for view in [&host,&child]{assert_eq!(evaluate(view,"window.liveNode.isConnected&&window.probeDraft.value==='never saved'"),true);}
                        // Return from the child toolbar: real guest subscription and IPC.
                        menu(&child,"Return to split");
                        wait(&host,"!!document.querySelector('[data-testid=split-divider]')");
                        assert_eq!(evaluate(&child,"window.liveToken"),child_token);assert!(app.get_window("otto-pane-window-main").is_none());
                        assert_host_fills_window(&host);
                    }
                    let before=evaluate(&host,"innerWidth").as_f64().unwrap();
                    app.emit_to(tauri::EventTarget::webview("main"),"otto://menu","zoom-in").unwrap();
                    wait(&host,&format!("innerWidth<{before}"));
                    assert_eq!(evaluate(&child,"window.liveToken"),child_token);
                    println!("PASS full SPA: isolated nonpersistent stores; native child ready handshake; real toolbar detach both directions; child-toolbar Return; retained root/child DOM+drafts; native Webview menu listener");
                }));let code=if result.is_ok(){0}else{1};PROBE_RESULT.store(code,std::sync::atomic::Ordering::SeqCst);if code != 0 {std::process::exit(code)}app.exit(code);
            });Ok(())
        }).build(context).unwrap().run(|app,event|{
            if let tauri::RunEvent::WindowEvent{label,event,..}=event{match event{
                tauri::WindowEvent::CloseRequested{api,..}=>{if panes::intercept_close(app,&label){api.prevent_close()}},
                tauri::WindowEvent::Resized(_)=>panes::resized(app,&label),
                tauri::WindowEvent::Destroyed=>panes::destroyed(app,&label),_=>{}
            }}
        });
    std::process::exit(PROBE_RESULT.load(std::sync::atomic::Ordering::SeqCst));
}

#[cfg(not(debug_assertions))]
fn main() {
    eprintln!("This isolated pane probe requires a debug build");
}
