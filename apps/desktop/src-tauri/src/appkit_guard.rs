//! Guards against AppKit raising Objective-C exceptions inside event dispatch.
//!
//! Three otto-desktop aborts on 2026-09-28/29 had the same cause:
//! `*** Assertion failure in NSCampoLightweightUIController.m:1429` on a
//! "Mouse entered" event. "Campo" is macOS 27's Writing Tools lightweight UI —
//! the "Edit with Siri" affordance AppKit floats beside the caret or a
//! selection in any editable text, WKWebView textareas included. The system
//! log showed Otto driving it ~10× more than any other app (the composer,
//! editors and every xterm helper textarea are editable), and it asserts when
//! a mouse-entered event reaches a tracking area whose remote view it already
//! tore down.
//!
//! A plain Cocoa app survives that: `-[NSApplication run]` reports the
//! exception and keeps going. Under tao the exception unwinds into tao's
//! `extern "C" send_event` override, which cannot unwind, so Rust aborts the
//! whole app.
//!
//! - [`disable_writing_tools_affordance`] removes the trigger: the controller's
//!   eligibility class methods answer NO in this process, so the affordance
//!   never shows in Otto. Writing Tools stays in the Edit/context menus.
//! - [`install_exception_guard`] is the net for anything else AppKit throws:
//!   the superclass `sendEvent:` implementations tao calls into are wrapped in
//!   an Objective-C `@try`, so an exception is caught below tao's frame, logged
//!   to `desktop-panic.log`, and only that event is dropped.
//!
//! Both are no-ops when the class or selector is missing (older/newer macOS).

use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;

use objc2::runtime::{AnyClass, AnyObject, Bool, Imp, Sel};
use objc2::sel;

use crate::panic_guard;

type SendEvent = unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject);

static APP_SEND_EVENT: OnceLock<SendEvent> = OnceLock::new();
static WINDOW_SEND_EVENT: OnceLock<SendEvent> = OnceLock::new();
static CONTAINED: AtomicUsize = AtomicUsize::new(0);

/// Make macOS's Writing Tools affordance ineligible in this process.
/// Call on the main thread before the event loop starts.
pub fn disable_writing_tools_affordance() {
    let Some(cls) = AnyClass::get(c"NSCampoLightweightUIController") else {
        return;
    };
    let imp: Imp = unsafe {
        std::mem::transmute::<extern "C-unwind" fn(&AnyClass, Sel) -> Bool, Imp>(not_eligible)
    };
    for sel in [
        sel!(isEligible),
        sel!(isHostProcessEligible),
        sel!(_isLightweightUIEnabledForWritingTools),
    ] {
        if let Some(m) = cls.class_method(sel) {
            // SAFETY: each is a `+(BOOL)` taking no arguments; `not_eligible`
            // has that exact signature.
            unsafe { m.set_implementation(imp) };
        }
    }
}

extern "C-unwind" fn not_eligible(_cls: &AnyClass, _cmd: Sel) -> Bool {
    Bool::NO
}

/// Wrap `-[NSApplication sendEvent:]` and `-[NSWindow sendEvent:]` in an
/// Objective-C exception handler. Call once, on the main thread, before the
/// event loop starts.
pub fn install_exception_guard() {
    wrap(c"NSApplication", &APP_SEND_EVENT, app_send_event);
    wrap(c"NSWindow", &WINDOW_SEND_EVENT, window_send_event);
}

fn wrap(class: &std::ffi::CStr, slot: &'static OnceLock<SendEvent>, replacement: SendEvent) {
    if slot.get().is_some() {
        return;
    }
    let Some(method) = AnyClass::get(class).and_then(|c| c.instance_method(sel!(sendEvent:)))
    else {
        return;
    };
    // Record the original before swapping, so the wrapper never runs without it.
    // SAFETY: `sendEvent:` is `-(void)sendEvent:(NSEvent *)`, matching `SendEvent`.
    let original = unsafe { std::mem::transmute::<Imp, SendEvent>(method.implementation()) };
    if slot.set(original).is_err() {
        return;
    }
    unsafe { method.set_implementation(std::mem::transmute::<SendEvent, Imp>(replacement)) };
}

unsafe extern "C-unwind" fn app_send_event(this: *mut AnyObject, cmd: Sel, event: *mut AnyObject) {
    if let Some(original) = APP_SEND_EVENT.get() {
        call_guarded("NSApplication sendEvent:", *original, this, cmd, event);
    }
}

unsafe extern "C-unwind" fn window_send_event(
    this: *mut AnyObject,
    cmd: Sel,
    event: *mut AnyObject,
) {
    if let Some(original) = WINDOW_SEND_EVENT.get() {
        call_guarded("NSWindow sendEvent:", *original, this, cmd, event);
    }
}

/// Call `original`; an Objective-C exception is caught, logged and swallowed.
/// Returns true when one was contained.
fn call_guarded(
    what: &str,
    original: SendEvent,
    this: *mut AnyObject,
    cmd: Sel,
    event: *mut AnyObject,
) -> bool {
    // SAFETY: forwarding the exact receiver/selector/argument AppKit gave us.
    let result =
        objc2::exception::catch(AssertUnwindSafe(|| unsafe { original(this, cmd, event) }));
    let Err(exception) = result else { return false };
    // Log the first few in full, then every hundredth, so a tight loop of
    // repeated exceptions can't grow the log without bound.
    let n = CONTAINED.fetch_add(1, Ordering::Relaxed);
    if n < 20 || n.is_multiple_of(100) {
        let detail = match &exception {
            Some(e) => format!("{e:?}"),
            None => "nil exception".to_string(),
        };
        panic_guard::append(&format!(
            "=== contained: Objective-C exception in {what} (#{}, event dropped)\n{detail}\n\n",
            n + 1
        ));
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use objc2::rc::Retained;
    use objc2::runtime::NSObject;

    unsafe extern "C-unwind" fn raises(this: *mut AnyObject, _cmd: Sel, _event: *mut AnyObject) {
        // NSObject doesn't implement copyWithZone: → NSInvalidArgumentException.
        let _: Option<Retained<AnyObject>> = unsafe { objc2::msg_send![this, copy] };
    }

    unsafe extern "C-unwind" fn fine(_this: *mut AnyObject, _cmd: Sel, _event: *mut AnyObject) {}

    #[test]
    fn an_objc_exception_is_contained_not_propagated() {
        let obj = NSObject::new();
        let this = Retained::as_ptr(&obj) as *mut AnyObject;
        let null = std::ptr::null_mut();
        assert!(call_guarded("test", raises, this, sel!(sendEvent:), null));
        assert!(!call_guarded("test", fine, this, sel!(sendEvent:), null));
    }

    #[test]
    fn writing_tools_affordance_reports_ineligible() {
        let Some(cls) = AnyClass::get(c"NSCampoLightweightUIController") else {
            return; // macOS without the Campo UI: nothing to disable
        };
        disable_writing_tools_affordance();
        if cls.class_method(sel!(isEligible)).is_some() {
            let eligible: bool = unsafe { objc2::msg_send![cls, isEligible] };
            assert!(!eligible);
        }
        if cls.class_method(sel!(isHostProcessEligible)).is_some() {
            let eligible: bool = unsafe { objc2::msg_send![cls, isHostProcessEligible] };
            assert!(!eligible);
        }
    }
}
