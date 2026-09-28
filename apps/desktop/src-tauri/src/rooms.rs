//! Unprivileged, ephemeral windows for room invitations from another Otto.

use std::sync::atomic::{AtomicU64, Ordering};
use tauri::webview::NewWindowResponse;
use tauri::{Url, WebviewUrl, WebviewWindowBuilder};

const PREFIX: &str = "room-";
static NEXT_WINDOW: AtomicU64 = AtomicU64::new(1);

#[derive(Debug)]
struct Invitation {
    url: Url,
    room_id: String,
}

fn opaque_segment(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn parse_invitation(raw: &str) -> Result<Invitation, String> {
    // Error strings deliberately never interpolate the secret-bearing URL.
    let invalid = || "Use an HTTPS Otto invitation with a room ID and invitation code".to_string();
    if raw.len() > 2048 || raw.chars().any(char::is_whitespace) {
        return Err(invalid());
    }
    let url = Url::parse(raw).map_err(|_| invalid())?;
    let host = url.host_str().unwrap_or_default();
    let loopback = host == "localhost"
        || host
            .trim_matches(['[', ']'])
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback());
    // Tauri treats this HTTPS alias as a privileged local app origin. Never
    // permit an invitation to reclassify the remote room as bundled content.
    if url.host_str() == Some("tauri.localhost")
        || url.host().is_none()
        || !(url.scheme() == "https" || (url.scheme() == "http" && loopback))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.path() != "/"
    {
        return Err(invalid());
    }
    let parts: Vec<_> = url.fragment().unwrap_or_default().split('/').collect();
    if parts.len() != 4
        || !parts[0].is_empty()
        || parts[1] != "room"
        || !opaque_segment(parts[2], 128)
        || !opaque_segment(parts[3], 512)
        || parts[3].len() < 16
    {
        return Err(invalid());
    }
    let room_id = parts[2].to_string();
    Ok(Invitation { url, room_id })
}

fn allows_navigation(invitation: &Invitation, url: &Url) -> bool {
    url.origin() == invitation.url.origin()
        && url.path() == "/"
        && url.query().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && (url.fragment() == invitation.url.fragment()
            || url.fragment() == Some(format!("/room/{}", invitation.room_id).as_str()))
}

fn trusted_caller(label: &str, url: &Url) -> bool {
    // Defense in depth in addition to Tauri's remote-origin ACL. A room window
    // must never create windows even if its page changes to a local app URL.
    !label.starts_with(PREFIX)
        && ((url.scheme() == "tauri" && url.host_str() == Some("localhost"))
            || (url.scheme() == "https" && url.host_str() == Some("tauri.localhost")))
}

/// Called only by the bundled Join room sheet, after it shows the destination.
/// No room page is granted this command or any other native capability.
#[tauri::command]
pub async fn open_room_window(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    url: String,
) -> Result<String, String> {
    let caller = webview
        .url()
        .map_err(|_| "Unable to validate the joining window".to_string())?;
    if !trusted_caller(webview.label(), &caller) {
        return Err("Join rooms from Otto's local Join room screen".into());
    }
    let invitation = parse_invitation(&url)?;
    let label = format!("{PREFIX}{}", NEXT_WINDOW.fetch_add(1, Ordering::Relaxed));
    let title = format!(
        "Otto room — {}",
        invitation.url.origin().ascii_serialization()
    );
    WebviewWindowBuilder::new(&app, &label, WebviewUrl::External(invitation.url.clone()))
        .title(title)
        .inner_size(1100.0, 760.0)
        .min_inner_size(480.0, 360.0)
        .incognito(true)
        .disable_drag_drop_handler()
        .on_navigation(move |destination| allows_navigation(&invitation, destination))
        .on_new_window(|_, _| NewWindowResponse::Deny)
        .on_download(|_, _| false)
        .build()
        // Framework errors can include the URL; do not echo an invitation.
        .map_err(|_| "Otto could not open the room window".to_string())?;
    Ok(label)
}

#[cfg(test)]
mod tests {
    use super::*;

    const INVITE: &str = "https://rooms.example/#/room/room_123/0123456789abcdef0123456789abcdef";

    #[test]
    fn accepts_https_and_loopback_development_invites() {
        for origin in [
            "https://rooms.example",
            "https://host.example:8443",
            "http://127.0.0.1:7700",
            "http://[::1]:7700",
            "http://localhost:7700",
        ] {
            let parsed = parse_invitation(&format!(
                "{origin}/#/room/room_123/0123456789abcdef0123456789abcdef"
            ))
            .unwrap();
            assert_eq!(parsed.room_id, "room_123");
        }
    }

    #[test]
    fn rejects_privileged_origins_and_credential_leaks() {
        for url in [
            "http://rooms.example/#/room/id/0123456789abcdef",
            "tauri://localhost/#/room/id/0123456789abcdef",
            "https://tauri.localhost/#/room/id/0123456789abcdef",
            "https://tauri.localhost:8443/#/room/id/0123456789abcdef",
            "https://user:password@rooms.example/#/room/id/0123456789abcdef",
            "https://rooms.example/?token=secret#/room/id/0123456789abcdef",
            "https://rooms.example/api/v1/#/room/id/0123456789abcdef",
            "https://rooms.example/#/agents/id",
            "https://rooms.example/#/room/id",
            "https://rooms.example/#/room/id/invite/extra",
            "https://rooms.example/#/room/id/%2Fsecret",
            "https://rooms.example/#/room/id/has space",
            "https://rooms.example/#/room/../0123456789abcdef",
        ] {
            let error = parse_invitation(url).expect_err(url);
            assert!(
                !error.contains(url),
                "errors must not echo secret-bearing URLs"
            );
        }
    }

    #[test]
    fn navigation_stays_on_the_original_room() {
        let invitation = parse_invitation(INVITE).unwrap();
        for allowed in [INVITE, "https://rooms.example/#/room/room_123"] {
            assert!(allows_navigation(&invitation, &allowed.parse().unwrap()));
        }
        for denied in [
            "https://other.example/#/room/room_123",
            "https://rooms.example:8443/#/room/room_123",
            "http://rooms.example/#/room/room_123",
            "https://rooms.example/#/room/other",
            "https://rooms.example/#/room/room_123/replacement_invite",
            "https://rooms.example/#/settings",
            "https://rooms.example/",
            "https://rooms.example/api/v1/sessions#/room/room_123",
            "https://rooms.example/?token=secret#/room/room_123",
            "https://user@rooms.example/#/room/room_123",
            "tauri://localhost/#/room/room_123",
        ] {
            assert!(
                !allows_navigation(&invitation, &denied.parse().unwrap()),
                "{denied}"
            );
        }
    }

    #[test]
    fn remote_room_cannot_invoke_window_creation() {
        assert!(trusted_caller(
            "main",
            &"tauri://localhost/".parse().unwrap()
        ));
        assert!(!trusted_caller(
            "main",
            &"https://rooms.example/".parse().unwrap()
        ));
        assert!(!trusted_caller(
            "room-1",
            &"tauri://localhost/".parse().unwrap()
        ));
        assert!(!trusted_caller(
            "room-1",
            &"https://rooms.example/".parse().unwrap()
        ));
        let capability: serde_json::Value =
            serde_json::from_str(include_str!("../capabilities/default.json")).unwrap();
        assert!(capability.get("remote").is_none());
        assert!(capability.get("webviews").is_none());
        for pattern in capability["windows"].as_array().unwrap() {
            let pattern = pattern.as_str().unwrap();
            assert!(!"room-1".starts_with(pattern.trim_end_matches('*')));
        }
    }
}
