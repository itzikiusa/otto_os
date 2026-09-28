//! Pure ownership and geometry rules for local movable panes.
pub fn host_label(label: &str) -> bool {
    label == "main"
        || label.strip_prefix('w').is_some_and(|n| {
            !n.is_empty()
                && n.bytes().all(|b| b.is_ascii_digit())
                && n.parse::<u32>().is_ok_and(|n| n >= 2)
        })
}
pub fn local_document(
    scheme: &str,
    host: Option<&str>,
    path: &str,
    fragment: Option<&str>,
) -> bool {
    let origin = (scheme == "tauri" && host == Some("localhost"))
        || (scheme == "https" && host == Some("tauri.localhost"));
    let route = fragment.unwrap_or("").trim_start_matches('/');
    let module = route.split(['/', '?']).next().unwrap_or("");
    origin
        && matches!(path, "" | "/" | "/index.html")
        && !module.contains('%')
        && !matches!(module, "room" | "s" | "bar" | "tray")
}
pub fn valid_bounds(x: f64, y: f64, width: f64, height: f64) -> bool {
    [x, y, width, height].iter().all(|n| n.is_finite())
        && x >= 0.0
        && y >= 0.0
        && width >= 1.0
        && height >= 1.0
        && x + width <= 32768.0
        && y + height <= 32768.0
}
pub fn owns(host: &str, child: &str, caller: &str) -> bool {
    host == caller || child == caller
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn host_identity_excludes_auxiliary_remote_and_child_views() {
        assert!(host_label("main"));
        assert!(host_label("w23"));
        for name in [
            "w",
            "w0",
            "w1",
            "wfoo",
            "otto-pane-main",
            "otto-browser-1",
            "otto-room-1",
            "popout-1",
            "otto-bar",
        ] {
            assert!(!host_label(name), "{name}");
        }
    }
    #[test]
    fn local_document_excludes_remote_and_guest_routes() {
        assert!(local_document(
            "tauri",
            Some("localhost"),
            "/index.html",
            Some("/agents")
        ));
        assert!(local_document("tauri", Some("localhost"), "", None));
        assert!(local_document(
            "https",
            Some("tauri.localhost"),
            "/",
            Some("/database")
        ));
        assert!(!local_document("https", Some("example.com"), "/", None));
        for route in [
            "/room/a/b",
            "/s/abc",
            "/bar",
            "/tray",
            "/%72oom/a",
            "/room?guest=1",
        ] {
            assert!(
                !local_document("tauri", Some("localhost"), "/index.html", Some(route)),
                "{route}"
            );
        }
    }
    #[test]
    fn bounds_reject_nonfinite_negative_and_unbounded_values() {
        assert!(valid_bounds(0.0, 0.0, 360.0, 400.0));
        for bounds in [
            (f64::NAN, 0.0, 1.0, 1.0),
            (0.0, f64::INFINITY, 1.0, 1.0),
            (-1.0, 0.0, 1.0, 1.0),
            (0.0, 0.0, 0.0, 10.0),
            (10000.0, 0.0, 30000.0, 100.0),
        ] {
            assert!(!valid_bounds(bounds.0, bounds.1, bounds.2, bounds.3));
        }
    }
    #[test]
    fn unrelated_hosts_cannot_control_or_relay_owned_pair() {
        assert!(owns("main", "otto-pane-main", "main"));
        assert!(owns("main", "otto-pane-main", "otto-pane-main"));
        assert!(!owns("main", "otto-pane-main", "w2"));
        assert!(!owns("main", "otto-pane-main", "otto-pane-w2"));
    }
}
