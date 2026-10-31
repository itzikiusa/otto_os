//! Unprivileged, ephemeral windows for room invitations from another Otto.

use std::sync::atomic::{AtomicU64, Ordering};
use tauri::webview::NewWindowResponse;
use tauri::{Url, WebviewUrl, WebviewWindowBuilder};

const PREFIX: &str = "room-";
/// Returned instead of a window label when the room opened in the browser.
pub const BROWSER: &str = "browser";
static NEXT_WINDOW: AtomicU64 = AtomicU64::new(1);

#[derive(Debug)]
struct Invitation {
    url: Url,
    room_id: String,
    /// The host's pinned certificate (`…/<code>/<sha256>`, minted by an Otto
    /// whose invite points at its own self-signed network listener).
    fingerprint: Option<[u8; 32]>,
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
    let fingerprint = match parts.get(4) {
        None => None,
        Some(hex) => Some(crate::room_trust::parse_fingerprint(hex).ok_or_else(invalid)?),
    };
    if !(parts.len() == 4 || parts.len() == 5)
        || !parts[0].is_empty()
        || parts[1] != "room"
        || !opaque_segment(parts[2], 128)
        || !opaque_segment(parts[3], 512)
        || parts[3].len() < 16
    {
        return Err(invalid());
    }
    let room_id = parts[2].to_string();
    Ok(Invitation {
        url,
        room_id,
        fingerprint,
    })
}

/// A host on this network (private/link-local IP or a `.local` name) whose
/// invitation carries NO pinned certificate (an older Otto). Otto's
/// network listener serves those over a SELF-SIGNED certificate, and WKWebView
/// (via wry) has no certificate-challenge hook: the native window would load a
/// blank page with no prompt. The default browser lets the guest accept the
/// certificate once, and keeps camera, mic and screen sharing (secure context).
fn local_network_host(url: &Url) -> bool {
    let host = url.host_str().unwrap_or_default();
    match host.trim_matches(['[', ']']).parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(ip)) => ip.is_private() || ip.is_link_local(),
        Ok(std::net::IpAddr::V6(ip)) => {
            let first = ip.segments()[0];
            (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80
        }
        Err(_) => host.to_ascii_lowercase().ends_with(".local"),
    }
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
    if let Some(fingerprint) = invitation.fingerprint {
        // Teach wry's delegate the challenge handler BEFORE this window's
        // webview exists (WebKit caches delegate capabilities on assignment).
        let (tx, rx) = tokio::sync::oneshot::channel();
        webview
            .with_webview(move |platform| {
                crate::room_trust::install(platform.inner());
                let _ = tx.send(());
            })
            .map_err(|_| "Otto could not prepare the room window".to_string())?;
        let _ = rx.await;
        let host = invitation.url.host_str().unwrap_or_default();
        let port = invitation.url.port_or_known_default().unwrap_or(443);
        crate::room_trust::pin(host, port, fingerprint);
    } else if invitation.url.scheme() == "https" && local_network_host(&invitation.url) {
        use tauri_plugin_opener::OpenerExt;
        app.opener()
            .open_url(invitation.url.as_str(), None::<&str>)
            .map_err(|_| "Otto could not open the room in your browser".to_string())?;
        return Ok(BROWSER.into());
    }
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
    fn pinned_invites_carry_a_64_hex_certificate_fingerprint() {
        let fp = "ab".repeat(32);
        let parsed = parse_invitation(&format!(
            "https://192.168.60.96:7701/#/room/room_123/0123456789abcdef0123456789abcdef/{fp}"
        ))
        .unwrap();
        assert_eq!(parsed.room_id, "room_123");
        assert_eq!(parsed.fingerprint, Some([0xab; 32]));
        assert!(parse_invitation(INVITE).unwrap().fingerprint.is_none());
        for bad in ["abc", "zz".repeat(32).as_str(), "ab".repeat(33).as_str()] {
            assert!(parse_invitation(&format!(
                "https://192.168.60.96:7701/#/room/room_123/0123456789abcdef0123456789abcdef/{bad}"
            ))
            .is_err());
        }
        // The pinned invite may still navigate to its own room route.
        let pinned = parse_invitation(&format!(
            "https://192.168.60.96:7701/#/room/room_123/0123456789abcdef0123456789abcdef/{fp}"
        ))
        .unwrap();
        assert!(allows_navigation(
            &pinned,
            &"https://192.168.60.96:7701/#/room/room_123"
                .parse()
                .unwrap()
        ));
    }

    #[test]
    fn lan_hosts_go_to_the_browser_public_hosts_stay_native() {
        for url in [
            "https://192.168.60.96:7701/",
            "https://10.0.0.5:7701/",
            "https://172.16.4.2:7701/",
            "https://169.254.1.1:7701/",
            "https://[fd00::1]:7701/",
            "https://[fe80::1]:7701/",
            "https://otto.local:7701/",
            "https://MacBook.local:7701/",
        ] {
            assert!(local_network_host(&url.parse().unwrap()), "{url}");
        }
        for url in [
            "https://rooms.example/",
            "https://8.8.8.8/",
            "https://[2001:db8::1]/",
            "http://127.0.0.1:7700/",
            "https://mac.tail1234.ts.net/",
        ] {
            assert!(!local_network_host(&url.parse().unwrap()), "{url}");
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
