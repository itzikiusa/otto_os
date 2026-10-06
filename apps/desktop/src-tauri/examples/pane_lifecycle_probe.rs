//! Runs the production pane commands against synthetic bundled HTML only.
//! No daemon, registry restore/save, sessions, credentials or installed app.
#![allow(dead_code, unused_imports)]
#[path = "../src/throttle.rs"]
mod throttle;
use throttle::NO_THROTTLE;
#[path = "../src/panes.rs"]
mod panes;
#[path = "../src/panes_policy.rs"]
mod panes_policy;
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
#[path = "../src/popout.rs"]
mod popout;
use std::borrow::Cow;
use tauri::utils::assets::{AssetKey, AssetsIter, CspHash};
use tauri::{Assets, Manager, Webview, WebviewUrl, WebviewWindowBuilder};
struct Synthetic;
impl Assets<tauri::Wry> for Synthetic {
    fn get(&self, _: &AssetKey) -> Option<Cow<'_, [u8]>> {
        Some(Cow::Borrowed(b"<!doctype html><title>Isolated production pane probe</title><textarea id=draft>initial</textarea><div style='height:4000px'>Synthetic local document</div>"))
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
    view.eval_with_callback(script, move |value| {
        let _ = tx.send(value);
    })
    .unwrap();
    let raw = rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("JS callback");
    serde_json::from_str(&raw).unwrap_or_else(|_| panic!("invalid JS result {raw}"))
}
fn invoke(view: &Webview, command: &str, args: serde_json::Value) -> serde_json::Value {
    evaluate(view,&format!("window.result=null;window.__TAURI_INTERNALS__.invoke({},{}).then(value=>window.result={{ok:true,value}}).catch(error=>window.result={{ok:false,error:String(error)}});true",serde_json::to_string(command).unwrap(),args));
    for _ in 0..100 {
        std::thread::sleep(std::time::Duration::from_millis(40));
        let v = evaluate(view, "window.result");
        if !v.is_null() {
            return v;
        }
    }
    panic!("IPC timeout: {command}")
}
fn ok(view: &Webview, command: &str, args: serde_json::Value) -> serde_json::Value {
    let r = invoke(view, command, args);
    assert_eq!(r["ok"], true, "{command}: {r}");
    r["value"].clone()
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
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    context.config_mut().identifier =
        format!("com.otto.pane-lifecycle-probe-{}", std::process::id());
    context.set_assets(Box::new(Synthetic));
    tauri::Builder::default().invoke_handler(tauri::generate_handler![panes::pane_open,panes::pane_layout,panes::pane_state,panes::pane_to_host,panes::pane_to_guest,panes::pane_detach,panes::pane_return,panes::pane_focus,panes::pane_window_action,panes::pane_monitors,panes::pane_close])
        .setup(|app| {
            app.manage(panes::ProbeIsolation { initialization_script: String::new() });
            WebviewWindowBuilder::new(app,"main",WebviewUrl::App("index.html".into())).incognito(true)
            .title("Isolated pane lifecycle proof").inner_size(1000.0,700.0).build()?;
            WebviewWindowBuilder::new(app,"w2",WebviewUrl::App("index.html".into())).incognito(true).visible(false).build()?;
            let app=app.handle().clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(2));
                let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let host=app.get_webview("main").unwrap();
                    let logical_width=host.window().inner_size().unwrap().width as f64/host.window().scale_factor().unwrap();
                    println!("ZOOM before: logical_width={logical_width} css_width={}",evaluate(&host,"innerWidth"));
                    host.set_zoom(1.5).unwrap();std::thread::sleep(std::time::Duration::from_millis(100));
                    println!("ZOOM after_1.5: logical_width={logical_width} css_width={}",evaluate(&host,"innerWidth"));
                    host.set_zoom(1.0).unwrap();
                    let foreign=app.get_webview("w2").unwrap();
                    println!("LOCAL_URL {:?}",host.url());
                    let bounds=serde_json::json!({"x":500,"y":0,"width":500,"height":650});
                    let opened=ok(&host,"pane_open",serde_json::json!({"route":"agents","bounds":bounds}));
                    let child=app.get_webview(opened["child"].as_str().unwrap()).unwrap();
                    assert_eq!(windows::primary_window(&app).unwrap().label(),"main");
                    std::thread::sleep(std::time::Duration::from_millis(700));
                    assert!(!invoke(&foreign,"pane_to_guest",serde_json::json!({"message":{"ns":"otto-side","type":"navigate","route":"git"}}))["ok"].as_bool().unwrap());
                    assert!(!invoke(&child,"pane_open",serde_json::json!({"route":"git","bounds":bounds}))["ok"].as_bool().unwrap());
                    evaluate(&host,"window.events=[];window.__TAURI_INTERNALS__.invoke('plugin:event|listen',{event:'otto://pane-to-host',target:{kind:'Webview',label:'main'},handler:window.__TAURI_INTERNALS__.transformCallback(e=>window.events.push(e.payload))});true");
                    ok(&child,"pane_to_host",serde_json::json!({"message":{"ns":"otto-side","type":"ready","route":"agents"}}));
                    std::thread::sleep(std::time::Duration::from_millis(80));
                    assert_eq!(evaluate(&host,"window.events[0].type"),"ready");
                    evaluate(&child,"window.token=crypto.randomUUID();document.getElementById('draft').value='live unsaved query';window.scrollTo(0,200);true");
                    let before=evaluate(&child,"JSON.stringify({token:window.token,draft:document.getElementById('draft').value,scroll:window.scrollY})");
                    ok(&host,"pane_layout",serde_json::json!({"bounds":bounds,"visible":false}));
                    assert_eq!(ok(&child,"pane_state",serde_json::json!({}))["visible"],false);
                    // Creation failure must leave the original pair recoverable.
                    let collision=tauri::WindowBuilder::new(&app,"otto-pane-window-main").visible(false).build().unwrap();
                    assert_eq!(invoke(&host,"pane_detach",serde_json::json!({"pane":"side"}))["ok"],false);
                    assert_eq!(ok(&host,"pane_state",serde_json::json!({}))["mode"],"attached");
                    assert_eq!(child.window().label(),"main");collision.destroy().unwrap();
                    for mode in ["side","primary"] {
                        let s=ok(&host,"pane_detach",serde_json::json!({"pane":mode}));assert_eq!(s["mode"],mode);assert_eq!(s["visible"],true);
                        assert_eq!(ok(&child,"pane_detach",serde_json::json!({"pane":mode}))["mode"],mode);
                        ok(&host,"pane_focus",serde_json::json!({"pane":"side"}));
                        for _ in 0..60 {if evaluate(&child,"document.hasFocus()")==true {break}std::thread::sleep(std::time::Duration::from_millis(50));}
                        assert_eq!(evaluate(&child,"document.hasFocus()"),true,"{mode} side focus");
                        assert_eq!(evaluate(&host,"document.hasFocus()"),false,"{mode} primary must not retain focus");
                        assert_native_content_filled(&child);
                        println!("FOCUS {mode}: host={} child={}",evaluate(&host,"document.hasFocus()"),evaluate(&child,"document.hasFocus()"));
                        assert_eq!(before,evaluate(&child,"JSON.stringify({token:window.token,draft:document.getElementById('draft').value,scroll:window.scrollY})"));
                        if mode=="side" {
                            assert_eq!(ok(&child,"pane_window_action",serde_json::json!({"action":"toggle-top"}))["alwaysOnTop"],true);
                            assert_eq!(ok(&child,"pane_window_action",serde_json::json!({"action":"toggle-top"}))["alwaysOnTop"],false);
                            let monitors=ok(&child,"pane_monitors",serde_json::json!({}));let monitors=monitors.as_array().unwrap();
                            println!("DISPLAY_COUNT {}",monitors.len());assert!(!monitors.is_empty());
                            for monitor in monitors {
                                ok(&child,"pane_window_action",serde_json::json!({"action":"move-display","monitor":monitor["id"]}));
                                std::thread::sleep(std::time::Duration::from_millis(180));
                                let expected=app.available_monitors().unwrap()[monitor["id"].as_u64().unwrap() as usize].clone();
                                let w=child.window();let area=*expected.work_area();let pos=w.outer_position().unwrap();let size=w.outer_size().unwrap();
                                println!("MOVE_GEOMETRY id={} scale={} area={area:?} actual={pos:?} size={size:?}",monitor["id"],expected.scale_factor());
                                assert!(pos.x>=area.position.x && pos.y>=area.position.y,"display {} top-left",monitor["id"]);
                                assert!(pos.x as i64+size.width as i64<=area.position.x as i64+area.size.width as i64 && pos.y as i64+size.height as i64<=area.position.y as i64+area.size.height as i64,"display {} bottom-right",monitor["id"]);
                                assert_native_content_filled(&child);
                                println!("MOVED_DISPLAY {} work_area_fits=true",monitor["id"]);
                            }
                            ok(&child,"pane_window_action",serde_json::json!({"action":"maximize"}));std::thread::sleep(std::time::Duration::from_millis(250));
                            ok(&child,"pane_window_action",serde_json::json!({"action":"maximize"}));
                            ok(&child,"pane_window_action",serde_json::json!({"action":"toggle-fullscreen"}));
                            std::thread::sleep(std::time::Duration::from_millis(1000));
                            assert!(child.window().is_fullscreen().unwrap());
                            assert_native_content_filled(&child);
                        }
                        assert_eq!(panes::menu_host(&app,"otto-pane-window-main"),"main");
                        let original=panes::original_frame("main").expect("original frame during detach");assert!(original.w>=1000);
                        app.get_window(if mode=="side"{"otto-pane-window-main"}else{"main"}).unwrap().close().unwrap();
                        std::thread::sleep(std::time::Duration::from_millis(1000));
                        assert_eq!(ok(&host,"pane_state",serde_json::json!({}))["mode"],"attached");
                        assert!(app.get_window("otto-pane-window-main").is_none());
                        assert_eq!(before,evaluate(&child,"JSON.stringify({token:window.token,draft:document.getElementById('draft').value,scroll:window.scrollY})"));
                    }
                    let original_position=host.window().outer_position().unwrap();
                    let original_size=host.window().inner_size().unwrap();
                    for monitor in app.available_monitors().unwrap().iter().enumerate().map(|(id,_)|id) {
                        ok(&host,"pane_detach",serde_json::json!({"pane":"primary"}));
                        ok(&host,"pane_window_action",serde_json::json!({"action":"move-display","monitor":monitor}));
                        std::thread::sleep(std::time::Duration::from_millis(180));
                        ok(&host,"pane_return",serde_json::json!({}));
                        std::thread::sleep(std::time::Duration::from_millis(220));
                        let restored=host.window().outer_position().unwrap();let size=host.window().inner_size().unwrap();
                        assert!((restored.x-original_position.x).abs()<=2 && (restored.y-original_position.y).abs()<=2,"restore position from display {monitor}: expected {original_position:?} got {restored:?}");
                        assert_eq!(size,original_size,"restore size from display {monitor}");
                        assert_eq!(before,evaluate(&child,"JSON.stringify({token:window.token,draft:document.getElementById('draft').value,scroll:window.scrollY})"));
                        println!("PRIMARY_RESTORED display={monitor} position_and_size=true");
                    }
                    ok(&host,"pane_detach",serde_json::json!({"pane":"primary"}));
                    let persisted=windows::live_frame(&app.get_window("main").unwrap()).unwrap();
                    assert!(persisted.w>=1000,"quit snapshot keeps original full host frame");
                    windows::mark_quitting();assert!(!panes::intercept_close(&app,"main"));assert!(!panes::intercept_close(&app,"otto-pane-window-main"));
                    ok(&host,"pane_return",serde_json::json!({}));
                    ok(&host,"pane_close",serde_json::json!({}));assert!(app.get_webview(child.label()).is_none());
                    assert!(ok(&host,"pane_state",serde_json::json!({})).is_null());
                    println!("PASS production IPC: authenticated owner isolation, targeted JS event, hidden visibility, side+primary detach/idempotence, native focus, close-to-return, live draft+UUID+scroll, child cleanup");
                }));let code=if result.is_ok(){0}else{1};PROBE_RESULT.store(code,std::sync::atomic::Ordering::SeqCst);if code != 0 {std::process::exit(code)}app.exit(code);
            });Ok(())
        }).build(context).unwrap().run(|app,event| {
            if let tauri::RunEvent::WindowEvent{label,event,..}=event {match event {
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
