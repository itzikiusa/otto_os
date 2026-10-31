//! Certificate pinning for room invitations from an Otto on this network.
//!
//! An Otto's network listener serves HTTPS with a SELF-SIGNED certificate.
//! WKWebView refuses it without a prompt (a blank room window), and wry
//! exposes no certificate-challenge hook. The invitation itself carries the
//! host certificate's SHA-256 (`#/room/<id>/<code>/<sha256>`, minted by the
//! host next to the invite code), so the joining app can accept EXACTLY that
//! certificate for EXACTLY that host:port — and nothing else.
//!
//! `-webView:didReceiveAuthenticationChallenge:completionHandler:` is added to
//! wry's navigation-delegate class (it implements none). Every challenge that
//! is not a server-trust challenge for a pinned host:port gets WebKit's
//! default handling, so all other sites keep normal certificate validation.
#![cfg(target_os = "macos")]

use std::collections::HashMap;
use std::ffi::{c_char, c_void, CStr};
use std::sync::{Mutex, OnceLock};

use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
use objc2::{msg_send, sel};

/// `host:port` → pinned leaf-certificate SHA-256.
static PINS: OnceLock<Mutex<HashMap<String, [u8; 32]>>> = OnceLock::new();
static INSTALLED: OnceLock<()> = OnceLock::new();

fn pins() -> std::sync::MutexGuard<'static, HashMap<String, [u8; 32]>> {
    PINS.get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Lowercase-hex SHA-256 → bytes.
pub fn parse_fingerprint(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

/// Accept `fingerprint` for `host:port` from now on (this app run only).
pub fn pin(host: &str, port: u16, fingerprint: [u8; 32]) {
    pins().insert(format!("{}:{port}", host.to_ascii_lowercase()), fingerprint);
}

/// Add the challenge handler to the class of `webview`'s navigation delegate
/// (wry's, shared by every Otto webview). Main thread; idempotent. Must run
/// before the pinned window is created: WebKit caches which optional
/// delegate methods exist when the delegate is assigned.
pub fn install(wk_webview: *mut c_void) {
    if INSTALLED.get().is_some() || wk_webview.is_null() {
        return;
    }
    let webview = wk_webview.cast::<AnyObject>();
    // SAFETY: a live WKWebView from Tauri's `with_webview`, on the main thread.
    let delegate: *mut AnyObject = unsafe { msg_send![webview, navigationDelegate] };
    if delegate.is_null() {
        return;
    }
    let cls = unsafe { objc2::ffi::object_getClass(delegate) } as *mut AnyClass;
    let imp: Imp =
        unsafe { std::mem::transmute::<ChallengeImp, Imp>(did_receive_challenge as ChallengeImp) };
    // v@:@@@? — void, self, _cmd, WKWebView*, NSURLAuthenticationChallenge*, block.
    let added = unsafe {
        objc2::ffi::class_addMethod(
            cls,
            sel!(webView:didReceiveAuthenticationChallenge:completionHandler:),
            imp,
            c"v@:@@@?".as_ptr(),
        )
    };
    if added.as_bool() {
        let _ = INSTALLED.set(());
    }
}

type ChallengeImp =
    unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, *mut AnyObject, *mut Block);

/// The Objective-C block ABI's fixed header; `invoke`'s first argument is the
/// block itself. NSURLSessionAuthChallengeDisposition is an NSInteger.
#[repr(C)]
struct Block {
    isa: *const c_void,
    flags: i32,
    reserved: i32,
    invoke: unsafe extern "C" fn(*mut Block, isize, *mut AnyObject),
}

const USE_CREDENTIAL: isize = 0;
const DEFAULT_HANDLING: isize = 1;
const CANCEL: isize = 2;

#[link(name = "Security", kind = "framework")]
extern "C" {
    fn SecTrustGetCertificateCount(trust: *mut c_void) -> isize;
    fn SecTrustGetCertificateAtIndex(trust: *mut c_void, ix: isize) -> *mut c_void;
    fn SecCertificateCopyData(cert: *mut c_void) -> *mut c_void;
}
#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFDataGetLength(data: *mut c_void) -> isize;
    fn CFDataGetBytePtr(data: *mut c_void) -> *const u8;
    fn CFRelease(cf: *mut c_void);
}
extern "C" {
    fn CC_SHA256(data: *const c_void, len: u32, md: *mut u8) -> *mut u8;
}

unsafe fn ns_string(s: *mut AnyObject) -> Option<String> {
    if s.is_null() {
        return None;
    }
    let p: *const c_char = msg_send![s, UTF8String];
    (!p.is_null()).then(|| CStr::from_ptr(p).to_string_lossy().into_owned())
}

/// SHA-256 of the trust's leaf certificate (DER).
unsafe fn leaf_sha256(trust: *mut c_void) -> Option<[u8; 32]> {
    if trust.is_null() || SecTrustGetCertificateCount(trust) < 1 {
        return None;
    }
    let cert = SecTrustGetCertificateAtIndex(trust, 0);
    if cert.is_null() {
        return None;
    }
    let data = SecCertificateCopyData(cert);
    if data.is_null() {
        return None;
    }
    let len = CFDataGetLength(data);
    let mut out = [0u8; 32];
    CC_SHA256(CFDataGetBytePtr(data).cast(), len as u32, out.as_mut_ptr());
    CFRelease(data);
    Some(out)
}

unsafe extern "C-unwind" fn did_receive_challenge(
    _this: *mut AnyObject,
    _cmd: Sel,
    _webview: *mut AnyObject,
    challenge: *mut AnyObject,
    handler: *mut Block,
) {
    let reply = |disposition: isize, credential: *mut AnyObject| {
        if !handler.is_null() {
            ((*handler).invoke)(handler, disposition, credential);
        }
    };
    let space: *mut AnyObject = msg_send![challenge, protectionSpace];
    let method = ns_string(msg_send![space, authenticationMethod]);
    if method.as_deref() != Some("NSURLAuthenticationMethodServerTrust") {
        return reply(DEFAULT_HANDLING, std::ptr::null_mut());
    }
    let host = ns_string(msg_send![space, host]).unwrap_or_default();
    let port: isize = msg_send![space, port];
    let Some(pinned) = pins()
        .get(&format!("{}:{port}", host.to_ascii_lowercase()))
        .copied()
    else {
        return reply(DEFAULT_HANDLING, std::ptr::null_mut());
    };
    let trust: *mut c_void = msg_send![space, serverTrust];
    if leaf_sha256(trust) != Some(pinned) {
        // A pinned host presenting any other certificate is refused outright.
        return reply(CANCEL, std::ptr::null_mut());
    }
    let Some(cls) = AnyClass::get(c"NSURLCredential") else {
        return reply(CANCEL, std::ptr::null_mut());
    };
    let credential: *mut AnyObject = msg_send![cls, credentialForTrust: trust];
    reply(USE_CREDENTIAL, credential);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprints_parse_only_as_64_hex() {
        let hex = "00ff".repeat(16);
        let fp = parse_fingerprint(&hex).unwrap();
        assert_eq!(fp[0], 0x00);
        assert_eq!(fp[1], 0xff);
        assert!(parse_fingerprint(&hex[..62]).is_none());
        assert!(parse_fingerprint(&format!("{}zz", &hex[..62])).is_none());
        assert!(parse_fingerprint(&format!("{hex}00")).is_none());
    }
}
