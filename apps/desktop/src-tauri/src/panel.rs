//! Non-activating panels for the assistant bar and the tray popover.
//!
//! A plain tao window activates Otto when it takes key focus, and activation
//! brings Otto's MAIN window forward over whatever app the user was in — the
//! opposite of a Spotlight/Raycast-style summon. AppKit's answer is an
//! `NSPanel` with `NSWindowStyleMaskNonactivatingPanel`: it becomes key (so
//! the webview gets the keyboard) while the other app stays active.
//!
//! tao has no panel option, so — like the `tauri-nspanel` crate — we swap the
//! window's class at runtime to a registered `NSPanel` subclass that answers
//! `canBecomeKeyWindow = YES` (a borderless panel otherwise refuses key), then
//! set the panel-only style bit. Caveats of the swap, all harmless for how the
//! two panels are used:
//!   * tao's `TaoWindow` overrides (`sendEvent:` drag-by-background, the
//!     `focusable` ivar) are gone — never call `set_focusable` on a panel
//!     (tao would look the ivar up on the new class and panic);
//!   * everything else (delegate, WKWebView, tauri events) is untouched.
//!
//! Every function here must run on the main thread (callers use
//! `run_on_main_thread`).

use std::sync::OnceLock;

use objc2::runtime::{AnyClass, AnyObject, Bool, ClassBuilder, Sel};
use objc2::{msg_send, sel};

/// `NSWindowStyleMaskNonactivatingPanel`.
const STYLE_NONACTIVATING_PANEL: usize = 1 << 7;
/// `NSStatusWindowLevel` — above normal + floating windows and the Dock, so
/// the bar shows over a full-screen app too (with `FullScreenAuxiliary`).
const LEVEL_STATUS: isize = 25;
/// `NSWindowCollectionBehavior` bits.
const BEHAVIOR_CAN_JOIN_ALL_SPACES: usize = 1 << 0;
const BEHAVIOR_IGNORES_CYCLE: usize = 1 << 6;
const BEHAVIOR_FULL_SCREEN_AUXILIARY: usize = 1 << 8;

extern "C-unwind" fn answer_yes(_this: &AnyObject, _cmd: Sel) -> Bool {
    Bool::YES
}

extern "C-unwind" fn answer_no(_this: &AnyObject, _cmd: Sel) -> Bool {
    Bool::NO
}

/// The `OttoPanel : NSPanel` class, registered once.
fn panel_class() -> Option<&'static AnyClass> {
    static CLASS: OnceLock<Option<&'static AnyClass>> = OnceLock::new();
    *CLASS.get_or_init(|| {
        let name = c"OttoNonactivatingPanel";
        if let Some(existing) = AnyClass::get(name) {
            return Some(existing);
        }
        let superclass = AnyClass::get(c"NSPanel")?;
        let mut builder = ClassBuilder::new(name, superclass)?;
        // SAFETY: both selectors are `- (BOOL)sel` on NSWindow; the function
        // signatures match (receiver, _cmd) -> BOOL.
        unsafe {
            builder.add_method(
                sel!(canBecomeKeyWindow),
                answer_yes as extern "C-unwind" fn(_, _) -> _,
            );
            // Never "main": main status is what the app's own windows hold,
            // and stealing it would restyle Otto's main window as inactive.
            builder.add_method(
                sel!(canBecomeMainWindow),
                answer_no as extern "C-unwind" fn(_, _) -> _,
            );
        }
        Some(builder.register())
    })
}

/// Turn a freshly built (hidden) window into a non-activating floating panel
/// that joins every Space and floats over full-screen apps. Returns false when
/// the conversion couldn't happen (the window still works, it just activates
/// Otto when focused).
pub fn make_nonactivating_panel(win: &tauri::WebviewWindow) -> bool {
    let Ok(ptr) = win.ns_window() else {
        return false;
    };
    let Some(cls) = panel_class() else {
        return false;
    };
    let ns = ptr.cast::<AnyObject>();
    if ns.is_null() {
        return false;
    }
    // SAFETY: `ns` is tao's live NSWindow (main thread). NSPanel adds no
    // instance storage over NSWindow, so re-classing the instance is sound —
    // the same technique tauri-nspanel ships.
    unsafe {
        objc2::ffi::object_setClass(ns, cls);
        let mask: usize = msg_send![ns, styleMask];
        let _: () = msg_send![ns, setStyleMask: mask | STYLE_NONACTIVATING_PANEL];
        // Setting the non-activating bit AFTER init doesn't reach the window
        // server on recent macOS; the private setter re-syncs it. Guarded, so
        // an OS without it just skips the call.
        let prevents: Bool = msg_send![ns, respondsToSelector: sel!(_setPreventsActivation:)];
        if prevents.as_bool() {
            let _: () = msg_send![ns, _setPreventsActivation: Bool::YES];
        }
        let _: () = msg_send![ns, setFloatingPanel: Bool::YES];
        let _: () = msg_send![ns, setBecomesKeyOnlyIfNeeded: Bool::NO];
        let _: () = msg_send![ns, setHidesOnDeactivate: Bool::NO];
        let _: () = msg_send![ns, setLevel: LEVEL_STATUS];
        let behavior =
            BEHAVIOR_CAN_JOIN_ALL_SPACES | BEHAVIOR_FULL_SCREEN_AUXILIARY | BEHAVIOR_IGNORES_CYCLE;
        let _: () = msg_send![ns, setCollectionBehavior: behavior];
    }
    true
}

/// Show a panel and make it key WITHOUT activating Otto (the frontmost app
/// stays frontmost; its windows don't reshuffle), then give the webview
/// first-responder status so DOM focus takes keystrokes.
pub fn show_without_activation(win: &tauri::WebviewWindow) {
    if let Ok(ptr) = win.ns_window() {
        let ns = ptr.cast::<AnyObject>();
        if !ns.is_null() {
            // SAFETY: live NSWindow/NSPanel on the main thread; both selectors
            // are plain `- (void)sel`.
            unsafe {
                let _: () = msg_send![ns, orderFrontRegardless];
                let _: () = msg_send![ns, makeKeyWindow];
            }
        }
    }
    // Webview focus = `makeFirstResponder:` only (wry) — no app activation,
    // unlike `WebviewWindow::set_focus`.
    let _ = AsRef::<tauri::Webview>::as_ref(win).set_focus();
}
