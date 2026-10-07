//! Read-only, in-process AppKit accessibility acceptance for the disposable SPA
//! probe. No AX client permission request, VoiceOver toggle, accessibility setter,
//! action invocation or focus change. WK content may cross a remote AX boundary:
//! an unavailable tree is an explicit failure, never a DOM-based substitute.
use objc2::{msg_send, runtime::AnyObject, sel};
use objc2_foundation::NSString;
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    panic::AssertUnwindSafe,
    time::{Duration, Instant},
};
use tauri::Webview;

const MAX_NODES: usize = 1024;
const MAX_DEPTH: usize = 32;

type CfObject = *const std::ffi::c_void;

// Public client-side APIs, scoped to this disposable process. In particular,
// no WithOptions(prompt=true), system-
// wide element, action/set-attribute API, private remote token or global timeout.
#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> u8;
    fn AXUIElementCreateApplication(pid: i32) -> CfObject;
    fn AXUIElementSetMessagingTimeout(element: CfObject, timeout: f32) -> i32;
    fn AXUIElementGetTypeID() -> usize;
    fn AXUIElementGetPid(element: CfObject, pid: *mut i32) -> i32;
    fn AXUIElementCopyAttributeValue(
        element: CfObject,
        attribute: CfObject,
        value: *mut CfObject,
    ) -> i32;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(value: CfObject);
    fn CFRetain(value: CfObject) -> CfObject;
    fn CFGetTypeID(value: CfObject) -> usize;
    fn CFArrayGetTypeID() -> usize;
    fn CFArrayGetCount(array: CfObject) -> isize;
    fn CFArrayGetValueAtIndex(array: CfObject, index: isize) -> CfObject;
    fn CFStringGetTypeID() -> usize;
    fn CFBooleanGetTypeID() -> usize;
    fn CFBooleanGetValue(value: CfObject) -> u8;
}

/// Owns one Create/Copy/Retain reference. Never constructed from a borrowed AX
/// child without CFRetain; arrays and their elements remain live during walks.
struct OwnedCf(CfObject);
impl Drop for OwnedCf {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CFRelease(self.0) };
        }
    }
}

struct ClientWalk {
    state: Snapshot,
    deadline: Instant,
    deadline_exceeded: bool,
    errors: Vec<i32>,
    retained: Vec<OwnedCf>,
    ancestry: usize,
    own_ancestry: bool,
}

impl ClientWalk {
    fn ready(&mut self) -> bool {
        if Instant::now() >= self.deadline {
            self.deadline_exceeded = true;
            return false;
        }
        true
    }

    fn remember_error(&mut self, status: i32) {
        // Missing optional attributes are ordinary leaf/capability results.
        if status != 0 && ![-25205, -25212].contains(&status) && !self.errors.contains(&status) {
            self.errors.push(status);
        }
    }

    fn prepare(&mut self, element: CfObject) -> bool {
        if !self.ready()
            || element.is_null()
            || unsafe { CFGetTypeID(element) != AXUIElementGetTypeID() }
        {
            return false;
        }
        let status = unsafe { AXUIElementSetMessagingTimeout(element, 0.25) };
        self.remember_error(status);
        status == 0
    }

    fn copy(&mut self, element: CfObject, name: &str) -> Option<OwnedCf> {
        if !self.prepare(element) || !self.ready() {
            return None;
        }
        let key = NSString::from_str(name);
        let mut value = std::ptr::null();
        let status = unsafe {
            AXUIElementCopyAttributeValue(element, (&*key as *const NSString).cast(), &mut value)
        };
        let owned = OwnedCf(value);
        self.remember_error(status);
        (status == 0 && !value.is_null()).then_some(owned)
    }

    fn text(&mut self, element: CfObject, name: &str) -> String {
        let Some(value) = self.copy(element, name) else {
            return String::new();
        };
        if unsafe { CFGetTypeID(value.0) } != unsafe { CFStringGetTypeID() } {
            return String::new();
        }
        // Public CFString/NSString toll-free bridge only, never an AX element.
        unsafe { (&*(value.0 as *const NSString)).to_string() }
    }

    fn enabled(&mut self, element: CfObject) -> Option<bool> {
        let value = self.copy(element, "AXEnabled")?;
        (unsafe { CFGetTypeID(value.0) } == unsafe { CFBooleanGetTypeID() })
            .then(|| unsafe { CFBooleanGetValue(value.0) != 0 })
    }

    fn visit(&mut self, element: CfObject, depth: usize, in_dialog: bool) {
        if !self.ready() || self.state.seen.contains(&(element as usize)) {
            return;
        }
        if depth > MAX_DEPTH || self.state.seen.len() + self.ancestry >= MAX_NODES {
            self.state.truncated = true;
            return;
        }
        if !self.prepare(element) {
            return;
        }
        // Retaining every visited element prevents address reuse from making
        // the pointer-based visited set incorrectly skip a later sibling.
        self.retained.push(OwnedCf(unsafe { CFRetain(element) }));
        self.state.seen.insert(element as usize);
        let role = self.text(element, "AXRole");
        let subrole = self.text(element, "AXSubrole");
        let title = self.text(element, "AXTitle");
        let description = self.text(element, "AXDescription");
        let linked_label = if role == "AXTextField" {
            self.copy(element, "AXTitleUIElement")
                .map(|label| self.text(label.0, "AXValue"))
                .unwrap_or_default()
        } else {
            String::new()
        };
        let names = [title.as_str(), description.as_str(), linked_label.as_str()];
        let dialog = is_workspace_dialog(&role, &subrole, &names);
        let value_matches = in_dialog
            && role == "AXTextField"
            && names.contains(&"Name")
            && self.text(element, "AXValue") == "Unsubmitted native zoom draft";
        let enabled = if in_dialog && role == "AXButton" {
            self.enabled(element)
        } else {
            None
        };
        self.state
            .observe(&role, &names, dialog, in_dialog, value_matches, enabled);
        let Some(children) = self.copy(element, "AXChildren") else {
            return;
        };
        if unsafe { CFGetTypeID(children.0) != CFArrayGetTypeID() } {
            return;
        }
        let count = unsafe { CFArrayGetCount(children.0) };
        for index in 0..count {
            if !self.ready() {
                break;
            }
            let child = unsafe { CFArrayGetValueAtIndex(children.0, index) };
            self.visit(child, depth + 1, in_dialog || dialog);
            if self.state.truncated {
                break;
            }
        }
    }
}

fn is_workspace_dialog(role: &str, subrole: &str, names: &[&str]) -> bool {
    (role == "AXDialog" || matches!(subrole, "AXDialog" | "AXApplicationDialog"))
        && names.contains(&"Add Workspace")
}

fn own_process_dialog_snapshot() -> Value {
    if unsafe { AXIsProcessTrusted() } == 0 {
        return json!({"pass": false, "available": false, "reason": "AX client is not already trusted; no permission requested"});
    }
    let mut walk = ClientWalk {
        state: Snapshot::default(),
        deadline: Instant::now() + Duration::from_secs(3),
        deadline_exceeded: false,
        errors: Vec::new(),
        retained: Vec::new(),
        ancestry: 0,
        own_ancestry: false,
    };
    let application = OwnedCf(unsafe { AXUIElementCreateApplication(std::process::id() as i32) });
    // Start with this application's focused element, not the system-wide focus.
    // Find its nearest named dialog, then continue upward to prove the chain
    // ends in OUR application before reading that dialog's descendants.
    let mut current = walk.copy(application.0, "AXFocusedUIElement");
    let mut dialog = None;
    while let Some(element) = current {
        if !walk.ready() {
            break;
        }
        if walk.ancestry >= MAX_DEPTH {
            walk.state.truncated = true;
            break;
        }
        walk.ancestry += 1;
        let role = walk.text(element.0, "AXRole");
        if role == "AXApplication" {
            let mut pid = 0;
            if walk.ready() {
                let status = unsafe { AXUIElementGetPid(element.0, &mut pid) };
                walk.remember_error(status);
                walk.own_ancestry = status == 0 && pid == std::process::id() as i32;
            }
            break;
        }
        if role == "AXWebArea" {
            walk.state.web_areas += 1;
        }
        if dialog.is_none() {
            let subrole = walk.text(element.0, "AXSubrole");
            let title = walk.text(element.0, "AXTitle");
            let description = walk.text(element.0, "AXDescription");
            if is_workspace_dialog(&role, &subrole, &[&title, &description]) {
                dialog = Some(OwnedCf(unsafe { CFRetain(element.0) }));
            }
        }
        current = walk.copy(element.0, "AXParent");
    }
    if walk.own_ancestry {
        if let Some(dialog) = dialog {
            walk.visit(dialog.0, 0, false);
        }
    }
    // The final successful IPC may itself have crossed the deadline.
    let _ = walk.ready();
    let mut report = walk.state.report();
    report["transport"] = json!("public_AXUIElement_own_application");
    report["own_application_ancestry"] = json!(walk.own_ancestry);
    report["ancestry_nodes"] = json!(walk.ancestry);
    report["deadline_exceeded"] = json!(walk.deadline_exceeded);
    report["error_codes"] = json!(walk.errors);
    report["deadline_ms"] = json!(3000);
    report["per_element_timeout_ms"] = json!(250);
    if !walk.own_ancestry || walk.deadline_exceeded || !walk.errors.is_empty() {
        report["pass"] = json!(false);
    }
    report
}

fn own_process_client_capability() -> Value {
    // This runs on the probe worker, leaving AppKit's main thread free to answer
    // a self-directed message. A nonzero AX status is data, never ignored success.
    unsafe {
        let trusted = AXIsProcessTrusted() != 0;
        let application = AXUIElementCreateApplication(std::process::id() as i32);
        if application.is_null() {
            return json!({"trusted": trusted, "available": false, "reason": "own process AX element unavailable"});
        }
        let timeout_status = AXUIElementSetMessagingTimeout(application, 0.25);
        if timeout_status != 0 {
            CFRelease(application);
            return json!({"trusted": trusted, "available": false, "timeout_status": timeout_status});
        }
        let key = NSString::from_str("AXChildren");
        // NSString/CFString are publicly documented toll-free bridged types.
        // This does NOT cast any NSAccessibility element into an AXUIElementRef.
        let mut children = std::ptr::null();
        let status = AXUIElementCopyAttributeValue(
            application,
            (&*key as *const NSString).cast(),
            &mut children,
        );
        let count =
            if status == 0 && !children.is_null() && CFGetTypeID(children) == CFArrayGetTypeID() {
                Some(CFArrayGetCount(children))
            } else {
                None
            };
        if !children.is_null() {
            CFRelease(children);
        }
        CFRelease(application);
        json!({"trusted": trusted, "copy_status": status, "children": count,
            "own_pid_only": true, "messaging_timeout_ms": 250,
            "available": status == 0 && count.is_some(),
            "acceptance": "diagnostic_only_not_a_dialog_pass"})
    }
}

// The legacy public NSAccessibility protocol remains WebKit's bridge to remote
// content. These selectors are deprecated, not private. Fall back to modern
// NSAccessibility object getters when a receiver provides only that protocol.
// Check every receiver before messaging; never cast a remote ObjC object into
// an AXUIElementRef (those are different public APIs and object representations).
unsafe fn attribute(object: *mut AnyObject, name: &str) -> *mut AnyObject {
    if object.is_null() {
        return std::ptr::null_mut();
    }
    let supports: bool =
        unsafe { msg_send![object, respondsToSelector: sel!(accessibilityAttributeValue:)] };
    if supports {
        let key = NSString::from_str(name);
        let value: *mut AnyObject =
            unsafe { msg_send![object, accessibilityAttributeValue: &*key] };
        if !value.is_null() {
            return value;
        }
    }
    let selector = match name {
        "AXChildren" => sel!(accessibilityChildren),
        "AXRole" => sel!(accessibilityRole),
        "AXSubrole" => sel!(accessibilitySubrole),
        "AXTitle" => sel!(accessibilityTitle),
        "AXDescription" => sel!(accessibilityLabel),
        "AXTitleUIElement" => sel!(accessibilityTitleUIElement),
        "AXValue" => sel!(accessibilityValue),
        _ => return std::ptr::null_mut(),
    };
    let supports: bool = unsafe { msg_send![object, respondsToSelector: selector] };
    if !supports {
        return std::ptr::null_mut();
    }
    // Every selector above is a public, zero-argument OBJECT getter. Boolean
    // getters have their correct ABI in enabled(), not performSelector:.
    unsafe { msg_send![object, performSelector: selector] }
}

unsafe fn string(object: *mut AnyObject) -> Option<String> {
    if object.is_null() {
        return None;
    }
    let is_string: bool = unsafe { msg_send![object, isKindOfClass: objc2::class!(NSString)] };
    is_string.then(|| unsafe { (&*(object as *const NSString)).to_string() })
}

unsafe fn text(object: *mut AnyObject, name: &str) -> String {
    unsafe { string(attribute(object, name)) }.unwrap_or_default()
}

unsafe fn enabled(object: *mut AnyObject) -> Option<bool> {
    let value = unsafe { attribute(object, "AXEnabled") };
    if value.is_null() {
        let supports: bool =
            unsafe { msg_send![object, respondsToSelector: sel!(isAccessibilityEnabled)] };
        return supports.then(|| unsafe { msg_send![object, isAccessibilityEnabled] });
    }
    let is_number: bool = unsafe { msg_send![value, isKindOfClass: objc2::class!(NSNumber)] };
    is_number.then(|| unsafe { msg_send![value, boolValue] })
}

#[derive(Default)]
struct Snapshot {
    seen: HashSet<usize>,
    truncated: bool,
    dialog: bool,
    named_field: bool,
    draft_value: bool,
    cancel_enabled: bool,
    create_enabled: bool,
    web_areas: usize,
    remote_objects: usize,
    remote_legacy_readers: usize,
    remote_modern_children_readers: usize,
}

impl Snapshot {
    // Shared semantic rules for local protocol and public client transports.
    // A successful transport/capability read alone never satisfies these.
    fn observe(
        &mut self,
        role: &str,
        names: &[&str],
        is_dialog: bool,
        in_dialog: bool,
        value_matches: bool,
        enabled: Option<bool>,
    ) {
        self.dialog |= is_dialog;
        if role == "AXWebArea" {
            self.web_areas += 1;
        }
        if !(in_dialog || is_dialog) {
            return;
        }
        if role == "AXTextField" && names.contains(&"Name") {
            self.named_field = true;
            self.draft_value |= value_matches;
        }
        if role == "AXButton" {
            self.cancel_enabled |= names.contains(&"Cancel") && enabled == Some(true);
            self.create_enabled |= names.contains(&"Create Workspace") && enabled == Some(true);
        }
    }

    /// Only returned accessibility children are walked: never DOM nodes or the
    /// AppKit subview tree. A pointer is used only within its main-thread callback;
    /// the returned NSArray keeps its children alive during this recursive call.
    unsafe fn visit(&mut self, object: *mut AnyObject, depth: usize, in_dialog: bool) {
        if object.is_null() || self.seen.contains(&(object as usize)) {
            return;
        }
        if depth > MAX_DEPTH || self.seen.len() >= MAX_NODES {
            self.truncated = true;
            return;
        }
        self.seen.insert(object as usize);
        let role = unsafe { text(object, "AXRole") };
        let subrole = unsafe { text(object, "AXSubrole") };
        let title = unsafe { text(object, "AXTitle") };
        let description = unsafe { text(object, "AXDescription") };
        // WebKit may expose a text field's label as AXTitleUIElement rather
        // than AXDescription. Read the linked label, without traversing parents.
        let label = unsafe { attribute(object, "AXTitleUIElement") };
        let linked_label = unsafe { text(label, "AXValue") };
        let names = [title.as_str(), description.as_str(), linked_label.as_str()];
        let is_dialog = is_workspace_dialog(&role, &subrole, &names);
        let in_dialog = in_dialog || is_dialog;
        // Diagnostic only: no private class lookup, initialization or methods.
        if unsafe { &*object }
            .class()
            .name()
            .to_string_lossy()
            .contains("Remote")
        {
            self.remote_objects += 1;
            let legacy: bool = unsafe {
                msg_send![object, respondsToSelector: sel!(accessibilityAttributeValue:)]
            };
            let modern: bool =
                unsafe { msg_send![object, respondsToSelector: sel!(accessibilityChildren)] };
            self.remote_legacy_readers += usize::from(legacy);
            self.remote_modern_children_readers += usize::from(modern);
        }
        let value_matches = in_dialog
            && role == "AXTextField"
            && names.contains(&"Name")
            && unsafe { text(object, "AXValue") } == "Unsubmitted native zoom draft";
        let enabled = if in_dialog && role == "AXButton" {
            unsafe { enabled(object) }
        } else {
            None
        };
        self.observe(&role, &names, is_dialog, in_dialog, value_matches, enabled);
        let children = unsafe { attribute(object, "AXChildren") };
        if children.is_null() {
            return;
        }
        let is_array: bool = unsafe { msg_send![children, isKindOfClass: objc2::class!(NSArray)] };
        if !is_array {
            return;
        }
        let count: usize = unsafe { msg_send![children, count] };
        for index in 0..count {
            if self.seen.len() >= MAX_NODES {
                self.truncated = true;
                break;
            }
            let child: *mut AnyObject = unsafe { msg_send![children, objectAtIndex: index] };
            unsafe { self.visit(child, depth + 1, in_dialog) };
        }
    }

    fn report(&self) -> Value {
        json!({
            "available": self.web_areas > 0 && !self.truncated,
            "nodes": self.seen.len(), "max_nodes": MAX_NODES, "max_depth": MAX_DEPTH,
            "truncated": self.truncated, "web_areas": self.web_areas,
            "remote_objects": self.remote_objects, "dialog": self.dialog,
            "remote_legacy_readers": self.remote_legacy_readers,
            "remote_modern_children_readers": self.remote_modern_children_readers,
            "name_field": self.named_field, "fixture_value_matches": self.draft_value,
            "cancel_enabled": self.cancel_enabled, "create_enabled": self.create_enabled,
            "covered_background_ax_exclusion": "not_asserted_without_remote_provenance",
            "pass": self.web_areas > 0 && !self.truncated && self.dialog && self.named_field
                && self.draft_value && self.cancel_enabled && self.create_enabled,
        })
    }
}

fn snapshot(view: &Webview) -> Value {
    let (tx, rx) = std::sync::mpsc::channel();
    view.with_webview(move |platform| {
        let result = objc2::exception::catch(AssertUnwindSafe(|| unsafe {
            let webview = platform.inner() as *mut AnyObject;
            let window: *mut AnyObject = msg_send![webview, window];
            let mut snapshot = Snapshot::default();
            snapshot.visit(window, 0, false);
            // Query the owned WK view directly too: this requests its public AX
            // children and permits on-demand WebKit AX initialization. It never
            // changes system accessibility preferences. The next bounded retry
            // lets the normal AppKit event loop receive remote-tree updates.
            snapshot.visit(webview, 0, false);
            snapshot.report()
        }));
        let report = result.unwrap_or_else(|_| {
            json!({"pass": false, "available": false, "reason": "Objective-C exception querying owned AX tree"})
        });
        let _ = tx.send(report);
    }).expect("dispatch owned native accessibility query");
    rx.recv_timeout(Duration::from_secs(5)).unwrap_or_else(|_| {
        json!({"pass": false, "available": false, "reason": "owned AX query exceeded five-second callback deadline"})
    })
}

/// Run only after the real Add Workspace dialog has its known unsaved draft.
/// Never log field values or arbitrary UI names; report fixture matches only.
pub fn assert_workspace_dialog(view: &Webview, scale: &str) {
    let mut last = Value::Null;
    let started = Instant::now();
    for attempt in 0..10 {
        if started.elapsed() >= Duration::from_secs(3) {
            break;
        }
        last = snapshot(view);
        if last["pass"] == true {
            println!("NATIVE_ACCESSIBILITY_STATE scale={scale} {last}");
            return;
        }
        // A timed-out main-thread callback must not enqueue nine more queries.
        if last.get("reason").is_some() || last["truncated"] == true {
            break;
        }
        if attempt < 9 {
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    println!(
        "NATIVE_ACCESSIBILITY_CLIENT_CAPABILITY {}",
        own_process_client_capability()
    );
    let client = own_process_dialog_snapshot();
    println!("NATIVE_ACCESSIBILITY_CLIENT_STATE scale={scale} {client}");
    if client["pass"] == true {
        return;
    }
    println!("NATIVE_ACCESSIBILITY_UNAVAILABLE_OR_FAILED scale={scale} {last}");
    panic!("Native accessibility dialog acceptance unavailable or failed at {scale}: local={last}, client={client}");
}
