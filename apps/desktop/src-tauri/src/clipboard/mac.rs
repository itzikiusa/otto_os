//! The AppKit side of clipboard history: read the general pasteboard under
//! macOS's paste-access rule, name the frontmost app, fetch an app's icon,
//! write an entry back, and send ⌘V. What to record lives in history.rs.

use std::ffi::c_void;

use objc2::rc::Retained;
use objc2::runtime::{NSObjectProtocol, ProtocolObject};
use objc2::{sel, AllocAnyThread};
use objc2_app_kit::{
    NSAttributedStringAppKitDocumentFormats, NSBitmapImageRep, NSPasteboard,
    NSPasteboardAccessBehavior, NSPasteboardWriting, NSWorkspace,
};
use objc2_foundation::{
    NSArray, NSAttributedString, NSData, NSDictionary, NSNumber, NSString, NSURL,
};
use serde::Serialize;

use super::history::{self, Entry, Kind, Source, MAX_IMAGE_BYTES, MAX_TEXT_BYTES};

const T_STRING: &str = "public.utf8-plain-text";
const T_RTF: &str = "public.rtf";
const T_FILE_URL: &str = "public.file-url";
const T_PNG: &str = "public.png";
const T_TIFF: &str = "public.tiff";

fn ns(s: &str) -> Retained<NSString> {
    NSString::from_str(s)
}

/// May Otto read the general pasteboard? (macOS 15.4+ asks per app: System
/// Settings → Privacy & Security → Paste from Other Apps.)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Access {
    /// Always Allow — or never asked yet: then the first read shows macOS's
    /// one-time prompt, after which the setting exists.
    Allowed,
    /// "Ask": every read would pop an alert, so Otto does not read.
    Ask,
    Denied,
}

pub fn access(pb: &NSPasteboard) -> Access {
    if !pb.respondsToSelector(sel!(accessBehavior)) {
        return Access::Allowed; // before macOS 15.4 reads were free
    }
    match pb.accessBehavior() {
        NSPasteboardAccessBehavior::Ask => Access::Ask,
        NSPasteboardAccessBehavior::AlwaysDeny => Access::Denied,
        _ => Access::Allowed,
    }
}

pub struct Image {
    pub bytes: Vec<u8>,
    pub png: bool,
    pub width: u32,
    pub height: u32,
}

/// One copy, read per the capture policy: nothing at all from a concealed /
/// transient / password-manager copy, and nothing over the size caps.
#[derive(Default)]
pub struct Snapshot {
    pub files: Vec<String>,
    pub text: Option<String>,
    pub image: Option<Image>,
}

pub fn read(pb: &NSPasteboard) -> Snapshot {
    let types: Vec<String> = pb
        .types()
        .map(|t| t.iter().map(|s| s.to_string()).collect())
        .unwrap_or_default();
    let has = |t: &str| types.iter().any(|x| x == t);
    let mut snap = Snapshot::default();
    if history::skip(&types) {
        return snap;
    }
    if has(T_FILE_URL) {
        snap.files = pb
            .pasteboardItems()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|i| i.stringForType(&ns(T_FILE_URL)))
                    .filter_map(|u| NSURL::URLWithString(&u)?.path())
                    .map(|p| p.to_string())
                    .collect()
            })
            .unwrap_or_default();
    } else if let Some(s) = pb
        .stringForType(&ns(T_STRING))
        .map(|s| s.to_string())
        .or_else(|| rtf_text(pb))
    {
        if s.len() <= MAX_TEXT_BYTES && !s.trim().is_empty() {
            snap.text = Some(s);
        }
    } else if let Some((data, png)) = pb
        .dataForType(&ns(T_PNG))
        .map(|d| (d, true))
        .or_else(|| pb.dataForType(&ns(T_TIFF)).map(|d| (d, false)))
    {
        if data.len() <= MAX_IMAGE_BYTES {
            if let Some(rep) = NSBitmapImageRep::imageRepWithData(&data) {
                snap.image = Some(Image {
                    bytes: data.to_vec(),
                    png,
                    width: rep.pixelsWide().max(0) as u32,
                    height: rep.pixelsHigh().max(0) as u32,
                });
            }
        }
    }
    snap
}

/// Rich text with no plain-text flavour, as its text.
fn rtf_text(pb: &NSPasteboard) -> Option<String> {
    let data = pb.dataForType(&ns(T_RTF))?;
    // SAFETY: valid RTF data (or nil comes back); no document attributes wanted.
    let s = unsafe {
        NSAttributedString::initWithRTF_documentAttributes(NSAttributedString::alloc(), &data, None)
    }?;
    Some(s.string().to_string())
}

pub fn frontmost() -> Option<Source> {
    let app = NSWorkspace::sharedWorkspace().frontmostApplication()?;
    Some(Source {
        name: app.localizedName()?.to_string(),
        bundle_id: app.bundleIdentifier().map(|b| b.to_string()),
    })
}

/// An installed app's icon as TIFF (the caller shrinks it to a small PNG).
pub fn app_icon_tiff(bundle_id: &str) -> Option<Vec<u8>> {
    let ws = NSWorkspace::sharedWorkspace();
    let path = ws
        .URLForApplicationWithBundleIdentifier(&ns(bundle_id))?
        .path()?;
    Some(ws.iconForFile(&path).TIFFRepresentation()?.to_vec())
}

/// Put an entry back on the pasteboard (images are stored as PNG).
pub fn write(pb: &NSPasteboard, e: &Entry, png: Option<&[u8]>) -> bool {
    pb.clearContents();
    match e.kind {
        Kind::Text | Kind::Link => pb.setString_forType(&ns(&e.text), &ns(T_STRING)),
        Kind::File => {
            let urls: Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> = e
                .text
                .lines()
                .map(|p| ProtocolObject::from_retained(NSURL::fileURLWithPath(&ns(p))))
                .collect();
            pb.writeObjects(&NSArray::from_retained_slice(&urls))
        }
        Kind::Image => {
            png.is_some_and(|b| pb.setData_forType(Some(&NSData::with_bytes(b)), &ns(T_PNG)))
        }
    }
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrustedWithOptions(options: *const c_void) -> bool;
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventSourceCreate(state: i32) -> *mut c_void;
    fn CGEventCreateKeyboardEvent(source: *mut c_void, key: u16, down: bool) -> *mut c_void;
    fn CGEventSetFlags(event: *mut c_void, flags: u64);
    fn CGEventPost(tap: u32, event: *mut c_void);
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(cf: *mut c_void);
}

/// Accessibility (needed to send ⌘V). `prompt` shows macOS's dialog and
/// lists Otto in System Settings → Privacy & Security → Accessibility.
pub fn accessibility(prompt: bool) -> bool {
    let opts = NSDictionary::from_slices(
        &[&*ns("AXTrustedCheckOptionPrompt")],
        &[&*NSNumber::new_bool(prompt)],
    );
    // SAFETY: NSDictionary is toll-free bridged to CFDictionary; it outlives the call.
    unsafe { AXIsProcessTrustedWithOptions(Retained::as_ptr(&opts).cast()) }
}

/// ⌘V into whatever app is frontmost (the ANSI V key code).
pub fn send_cmd_v() {
    const KEY_V: u16 = 9;
    const FLAG_COMMAND: u64 = 1 << 20;
    const COMBINED_SESSION_STATE: i32 = 0;
    const SESSION_EVENT_TAP: u32 = 1;
    // SAFETY: CoreGraphics C API; every created object is released once.
    unsafe {
        let src = CGEventSourceCreate(COMBINED_SESSION_STATE);
        for down in [true, false] {
            let ev = CGEventCreateKeyboardEvent(src, KEY_V, down);
            if !ev.is_null() {
                CGEventSetFlags(ev, FLAG_COMMAND);
                CGEventPost(SESSION_EVENT_TAP, ev);
                CFRelease(ev);
            }
        }
        if !src.is_null() {
            CFRelease(src);
        }
    }
}

#[cfg(test)]
mod tests {
    //! Against a PRIVATE pasteboard (`pasteboardWithUniqueName`): macOS only
    //! gates the general one, so these never trigger a paste alert or touch
    //! what the person running them has copied.
    use super::*;

    fn private() -> Retained<NSPasteboard> {
        NSPasteboard::pasteboardWithUniqueName()
    }

    /// Drop the test pasteboard from the pasteboard server (`oneway`, so
    /// objc2 has no typed binding for it).
    fn release(pb: &NSPasteboard) {
        // SAFETY: `- (oneway void)releaseGlobally` on a live pasteboard.
        unsafe { objc2::msg_send![pb, releaseGlobally] }
    }

    fn entry(kind: Kind, text: &str) -> Entry {
        Entry {
            id: "x".into(),
            kind,
            text: text.into(),
            width: 0,
            height: 0,
            size: 0,
            source: None,
            first_ms: 0,
            last_ms: 0,
            count: 1,
            pinned: false,
        }
    }

    #[test]
    fn a_private_pasteboard_is_readable_and_text_round_trips() {
        let pb = private();
        assert_eq!(access(&pb), Access::Allowed);
        assert!(write(&pb, &entry(Kind::Text, "hello clipboard"), None));
        let s = read(&pb);
        assert_eq!(s.text.as_deref(), Some("hello clipboard"));
        assert!(s.files.is_empty() && s.image.is_none());
        release(&pb);
    }

    #[test]
    fn a_concealed_copy_is_not_read_at_all() {
        let pb = private();
        pb.clearContents();
        pb.setString_forType(&ns("hunter2"), &ns(T_STRING));
        pb.setString_forType(&ns(""), &ns("org.nspasteboard.ConcealedType"));
        let s = read(&pb);
        assert!(s.text.is_none() && s.files.is_empty() && s.image.is_none());
        release(&pb);
    }

    #[test]
    fn files_round_trip_as_paths_and_oversized_text_is_skipped() {
        let pb = private();
        assert!(write(
            &pb,
            &entry(Kind::File, "/tmp/a b.txt\n/etc/hosts"),
            None
        ));
        assert_eq!(read(&pb).files, ["/tmp/a b.txt", "/etc/hosts"]);
        assert!(write(
            &pb,
            &entry(Kind::Text, &"x".repeat(MAX_TEXT_BYTES + 1)),
            None
        ));
        assert!(read(&pb).text.is_none());
        release(&pb);
    }

    #[test]
    fn an_image_is_read_with_its_pixel_size() {
        const PNG: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/icons/32x32.png"));
        let pb = private();
        assert!(write(&pb, &entry(Kind::Image, ""), Some(PNG)));
        let img = read(&pb).image.expect("image read");
        assert_eq!((img.width, img.height, img.png), (32, 32, true));
        release(&pb);
    }

    #[test]
    fn the_frontmost_app_has_a_name() {
        // Whatever app is in front while the tests run.
        assert!(frontmost().is_some_and(|s| !s.name.is_empty()));
    }
}
