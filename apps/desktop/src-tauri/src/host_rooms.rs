//! Local, ephemeral hosted-room windows. Credentials cross only the invoking
//! view's IPC channel and native memory; never URLs, scripts or persisted state.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use tauri::webview::NewWindowResponse;
use tauri::{Manager, Url, WebviewUrl, WebviewWindowBuilder};

const PREFIX: &str = "host-room-";
static NEXT_WINDOW: AtomicU64 = AtomicU64::new(1);
static OPEN: OnceLock<Mutex<HashMap<String, HostRoomContext>>> = OnceLock::new();
// Serialize opens without holding the registry mutex across native callbacks.
// A second request therefore focuses the completed first window, even if both
// requests arrive while its WebKit view is still being created.
static OPENING: tauri::async_runtime::Mutex<()> = tauri::async_runtime::Mutex::const_new(());

#[derive(Clone, Deserialize, Serialize, PartialEq)]
pub struct RoomCredential {
    room_id: String,
    member_id: String,
    token: String,
}

#[derive(Clone, Serialize)]
pub struct HostRoomContext {
    origin: String,
    credential: RoomCredential,
}

fn opaque(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn parse_context(origin: &str, credential: RoomCredential) -> Result<HostRoomContext, String> {
    let invalid = || "Hosted room context is invalid".to_string();
    if origin.len() > 2048
        || origin.chars().any(char::is_whitespace)
        || !opaque(&credential.room_id, 128)
        || !opaque(&credential.member_id, 128)
        || !opaque(&credential.token, 512)
        || credential.token.len() < 16
    {
        return Err(invalid());
    }
    let url = Url::parse(origin).map_err(|_| invalid())?;
    let host = url.host_str().unwrap_or_default();
    let loopback = host == "localhost"
        || host
            .trim_matches(['[', ']'])
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback());
    if host.is_empty()
        || host == "tauri.localhost"
        || !(url.scheme() == "https" || (url.scheme() == "http" && loopback))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid());
    }
    Ok(HostRoomContext {
        origin: url.origin().ascii_serialization(),
        credential,
    })
}

fn bundled(url: &Url) -> bool {
    ((url.scheme() == "tauri" && url.host_str() == Some("localhost"))
        || (url.scheme() == "https" && url.host_str() == Some("tauri.localhost")))
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && matches!(url.path(), "/" | "/index.html")
}

fn numbered(label: &str, prefix: &str) -> bool {
    label
        .strip_prefix(prefix)
        .is_some_and(|suffix| !suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_digit()))
}

fn trusted_caller(label: &str, url: &Url) -> bool {
    let host = label.strip_prefix("otto-pane-").unwrap_or(label);
    (host == "main" || numbered(host, "w") || numbered(label, "popout-")) && bundled(url)
}

fn allows_navigation(room: &str, url: &Url) -> bool {
    bundled(url)
        && url.path() == "/index.html"
        && url.query().is_none()
        && url.fragment() == Some(format!("/room-host/{room}").as_str())
}

fn context_for(
    open: &HashMap<String, HostRoomContext>,
    label: &str,
    url: &Url,
) -> Option<HostRoomContext> {
    if !numbered(label, PREFIX) {
        return None;
    }
    open.get(label)
        .filter(|context| allows_navigation(&context.credential.room_id, url))
        .cloned()
}

fn existing_label(
    open: &HashMap<String, HostRoomContext>,
    context: &HostRoomContext,
) -> Option<String> {
    open.iter()
        .find(|(_, candidate)| {
            candidate.origin == context.origin
                && candidate.credential.room_id == context.credential.room_id
        })
        .map(|(label, _)| label.clone())
}

fn with_open<R>(f: impl FnOnce(&mut HashMap<String, HostRoomContext>) -> R) -> R {
    let mut open = OPEN
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    f(&mut open)
}

/// Drop the only native credential copy when the actual window is destroyed,
/// not CloseRequested (which can still be prevented).
pub fn destroyed(label: &str) {
    if label.starts_with(PREFIX) {
        with_open(|open| open.remove(label));
    }
}

#[tauri::command]
pub async fn open_host_room_window(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    origin: String,
    credential: RoomCredential,
) -> Result<String, String> {
    let context = parse_context(&origin, credential)?;
    let _opening = OPENING.lock().await;
    let caller = webview
        .url()
        .map_err(|_| "Unable to validate the hosting window".to_string())?;
    if !trusted_caller(webview.label(), &caller) {
        return Err("Open hosted rooms from Otto's local session screen".into());
    }
    if let Some(label) = with_open(|open| existing_label(open, &context)) {
        if let Some(window) = app.get_webview_window(&label) {
            // A stale or replaced credential cannot overwrite a live host.
            let _ = window.unminimize();
            let _ = window.show();
            let _ = window.set_focus();
            return Ok(label);
        }
        destroyed(&label);
    }
    let label = format!("{PREFIX}{}", NEXT_WINDOW.fetch_add(1, Ordering::Relaxed));
    let room_id = context.credential.room_id.clone();
    let url = format!("index.html#/room-host/{room_id}");
    // Register before build: the initial SPA can request its context as soon
    // as it loads. No credential is interpolated into URL or JavaScript.
    with_open(|open| open.insert(label.clone(), context));
    let built = WebviewWindowBuilder::new(&app, &label, WebviewUrl::App(url.into()))
        .title("Otto room")
        .inner_size(1100.0, 760.0)
        .min_inner_size(480.0, 360.0)
        .disable_drag_drop_handler()
        .initialization_script(format!("window.__OTTO_WIN__='{label}';"))
        .on_navigation(move |destination| allows_navigation(&room_id, destination))
        .on_new_window(|_, _| NewWindowResponse::Deny)
        .on_download(|_, _| false)
        .build();
    match built {
        Ok(window) => {
            let _ = window.set_focus();
            Ok(label)
        }
        Err(_) => {
            destroyed(&label);
            Err("Otto could not open the hosted room window".into())
        }
    }
}

/// The caller chooses neither a label nor a room: lookup is bound to Tauri's
/// invoking webview, and the live URL must still be that window's own route.
#[tauri::command]
pub fn get_host_room_context(webview: tauri::Webview) -> Result<HostRoomContext, String> {
    let url = webview
        .url()
        .map_err(|_| "Hosted room context is unavailable".to_string())?;
    with_open(|open| context_for(open, webview.label(), &url))
        .ok_or_else(|| "Hosted room context is unavailable".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn credential() -> RoomCredential {
        RoomCredential {
            room_id: "room_123".into(),
            member_id: "member-456".into(),
            token: "0123456789abcdef0123456789abcdef".into(),
        }
    }

    #[test]
    fn context_accepts_canonical_http_loopback_and_https_origins() {
        for origin in [
            "https://rooms.example",
            "https://rooms.example:8443",
            "http://127.0.0.1:7700",
            "http://localhost:7700",
            "http://[::1]:7700",
        ] {
            let context = parse_context(&format!("{origin}/"), credential()).unwrap();
            assert_eq!(context.origin, origin);
            assert!(context.credential == credential());
        }
    }

    #[test]
    fn rejects_origin_confusion_and_secret_bearing_errors() {
        for origin in [
            "http://rooms.example",
            "tauri://localhost",
            "https://tauri.localhost",
            "https://tauri.localhost:8443",
            "https://user:secret@rooms.example",
            "https://rooms.example/api",
            "https://rooms.example?token=secret",
            "https://rooms.example#secret",
            " https://rooms.example",
            "file:///tmp/test",
        ] {
            let error = parse_context(origin, credential()).err().expect(origin);
            assert!(!error.contains(origin));
            assert!(!error.contains("secret"));
        }
        for invalid in ["", "a/b", "../room", "a b", "<script>"] {
            let mut value = credential();
            value.room_id = invalid.into();
            assert!(parse_context("https://rooms.example", value).is_err());
            let mut value = credential();
            value.member_id = invalid.into();
            assert!(parse_context("https://rooms.example", value).is_err());
            let mut value = credential();
            value.token = invalid.into();
            assert!(parse_context("https://rooms.example", value).is_err());
        }
        let mut value = credential();
        value.token = "x".repeat(513);
        assert!(parse_context("https://rooms.example", value).is_err());
    }

    #[test]
    fn only_bundled_workspace_views_can_open_host_rooms() {
        for label in ["main", "w2", "popout-1", "otto-pane-main", "otto-pane-w2"] {
            assert!(trusted_caller(
                label,
                &"tauri://localhost/index.html#/agents".parse().unwrap()
            ));
            assert!(trusted_caller(
                label,
                &"https://tauri.localhost/index.html#/agents"
                    .parse()
                    .unwrap()
            ));
        }
        for label in [
            "room-1",
            "host-room-1",
            "otto-browser-1",
            "unknown",
            "w-evil",
        ] {
            assert!(!trusted_caller(
                label,
                &"tauri://localhost/index.html#/agents".parse().unwrap()
            ));
        }
        for url in [
            "https://remote.example/index.html#/agents",
            "https://tauri.localhost.evil/index.html",
            "tauri://localhost:8443/index.html",
            "tauri://user@localhost/index.html",
            "tauri://localhost/remote.html",
        ] {
            assert!(!trusted_caller("main", &url.parse().unwrap()));
        }
    }

    #[test]
    fn hosted_window_stays_on_its_bundled_room_route() {
        for url in [
            "tauri://localhost/index.html#/room-host/room_123",
            "https://tauri.localhost/index.html#/room-host/room_123",
        ] {
            assert!(allows_navigation("room_123", &url.parse().unwrap()));
        }
        for url in [
            "tauri://localhost/index.html#/room-host/another",
            "tauri://localhost/index.html#/room/room_123",
            "tauri://localhost/index.html#/agents",
            "tauri://localhost/other.html#/room-host/room_123",
            "tauri://localhost/index.html?token=secret#/room-host/room_123",
            "https://remote.example/index.html#/room-host/room_123",
            "tauri://localhost/index.html#/room-host/room_123/secret",
        ] {
            assert!(!allows_navigation("room_123", &url.parse().unwrap()));
        }
    }

    #[test]
    fn credential_lookup_is_bound_to_label_and_exact_live_route() {
        let context = HostRoomContext {
            origin: "https://rooms.example".into(),
            credential: credential(),
        };
        let mut open = HashMap::from([("host-room-1".into(), context.clone())]);
        let url = "tauri://localhost/index.html#/room-host/room_123"
            .parse()
            .unwrap();
        assert!(context_for(&open, "host-room-1", &url).unwrap().credential == credential());
        assert!(context_for(&open, "host-room-2", &url).is_none());
        assert!(context_for(&open, "main", &url).is_none());
        assert!(context_for(
            &open,
            "host-room-1",
            &"tauri://localhost/index.html#/agents".parse().unwrap()
        )
        .is_none());
        assert!(context_for(
            &open,
            "host-room-1",
            &"https://remote.example/index.html#/room-host/room_123"
                .parse()
                .unwrap()
        )
        .is_none());
        assert_eq!(existing_label(&open, &context), Some("host-room-1".into()));
        let mut other = context.clone();
        other.origin = "https://another.example".into();
        assert!(existing_label(&open, &other).is_none());
        other = context.clone();
        other.credential.room_id = "another_room".into();
        assert!(existing_label(&open, &other).is_none());
        open.remove("host-room-1");
        assert!(context_for(&open, "host-room-1", &url).is_none());
        assert!(existing_label(&open, &context).is_none());
    }

    #[test]
    fn host_capability_is_local_only_and_never_matches_guest_windows() {
        let capability: serde_json::Value =
            serde_json::from_str(include_str!("../capabilities/default.json")).unwrap();
        assert!(capability.get("remote").is_none());
        assert!(capability.get("webviews").is_none());
        let patterns = capability["windows"].as_array().unwrap();
        assert!(patterns.iter().any(|pattern| pattern == "host-room-*"));
        for pattern in patterns {
            assert!(!"room-1".starts_with(pattern.as_str().unwrap().trim_end_matches('*')));
        }
    }
}
