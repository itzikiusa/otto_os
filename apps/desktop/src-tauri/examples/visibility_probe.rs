//! Observe actual WebKit visibility with Otto's production scheduling policy.
//! Synthetic incognito document only: no daemon, user profile or app state.
use std::borrow::Cow;
use tauri::utils::assets::{AssetKey, AssetsIter, CspHash};
use tauri::{Assets, WebviewUrl, WebviewWindowBuilder};
#[path = "../src/throttle.rs"]
mod throttle;
struct Synthetic;
impl Assets<tauri::Wry> for Synthetic {
    fn get(&self, _: &AssetKey) -> Option<Cow<'_, [u8]>> {
        Some(Cow::Borrowed(b"<!doctype html><title>Isolated visibility probe</title><p>Synthetic local visibility probe</p>"))
    }
    fn iter(&self) -> Box<AssetsIter<'_>> {
        Box::new(std::iter::empty())
    }
    fn csp_hashes(&self, _: &AssetKey) -> Box<dyn Iterator<Item = CspHash<'_>> + '_> {
        Box::new(std::iter::empty())
    }
}
fn observe(view: &tauri::WebviewWindow, phase: &str) {
    let (tx, rx) = std::sync::mpsc::channel();
    view.eval_with_callback("JSON.stringify({hidden:document.hidden,state:document.visibilityState,events:window.events})", move |v| { let _ = tx.send(v); }).unwrap();
    println!(
        "VISIBILITY {phase}: {}",
        rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap()
    );
}
fn main() {
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    context.config_mut().identifier = format!("com.otto.visibility-probe-{}", std::process::id());
    context.set_assets(Box::new(Synthetic));
    tauri::Builder::default()
        .setup(|app| {
            let view = WebviewWindowBuilder::new(
                app,
                "visibility-probe",
                WebviewUrl::App("index.html".into()),
            )
            .incognito(true)
            .background_throttling(throttle::NO_THROTTLE)
            .title("Isolated visibility proof")
            .inner_size(400.0, 200.0)
            .build()?;
            let app = app.handle().clone();
            std::thread::spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let settle = || std::thread::sleep(std::time::Duration::from_millis(750));
                // Install through eval after navigation; production CSP rightly
                // disallows an un-nonced inline script in synthetic assets.
                settle();
                view.eval("window.events=[];document.addEventListener('visibilitychange',()=>events.push({hidden:document.hidden,state:document.visibilityState}));").unwrap();
                settle();
                    observe(&view, "visible");
                    view.hide().unwrap();
                    settle();
                    observe(&view, "hidden");
                    view.show().unwrap();
                    settle();
                    observe(&view, "shown");
                    view.minimize().unwrap();
                    settle();
                    observe(&view, "minimized");
                    view.unminimize().unwrap();
                    settle();
                    observe(&view, "restored");
                }));
                if result.is_err() {
                    std::process::exit(1);
                }
                app.exit(0);
            });
            Ok(())
        })
        .run(context)
        .unwrap();
}
