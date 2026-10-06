//! `Host`-header allowlist (DNS-rebinding guard) + the host predicates the CORS
//! layer shares.
//!
//! The daemon's auth is a bearer header, but a few routes are public (`/meta`,
//! first-run onboarding, the share/OTP flow, ingest). A DNS-rebinding page
//! (`evil.example` re-resolved to 127.0.0.1) talks to the loopback listener
//! *same-origin* with `Host: evil.example:7700` — so every request whose `Host`
//! is a DNS name we don't own is refused before routing. Accepted:
//!   - any IP literal (v4 or `[v6]`): an IP in `Host` means the browser was
//!     pointed at that address, never re-resolved — not a rebinding vector;
//!   - `localhost` / `*.localhost` (and Tauri's `tauri.localhost`);
//!   - mDNS `*.local` (the TLS listener's `otto.local` + the Mac's Bonjour name);
//!   - Tailscale MagicDNS `*.ts.net` — narrowed to the names listed in
//!     `OTTO_ALLOWED_HOSTS` when that lists any `.ts.net` name;
//!   - the host of the **Public link domain** (`share_base_url` setting) — the
//!     Cloudflare-tunnel hostname in the recommended remote setup, which
//!     reaches loopback with `Host: otto.<your-domain>`;
//!   - anything listed in `OTTO_ALLOWED_HOSTS` (comma-separated host names,
//!     for any other DNS name in front of the daemon).
//!
//! A request with NO `Host` (in-process tests, HTTP/1.0 tools) passes: browsers
//! always send one, so its absence is not a rebinding signal.

use axum::extract::Request;
use axum::http::{header, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

/// Extra host names from `OTTO_ALLOWED_HOSTS` (lower-cased, trimmed). Read once.
fn configured_hosts() -> &'static [String] {
    static HOSTS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    HOSTS.get_or_init(|| parse_host_list(&std::env::var("OTTO_ALLOWED_HOSTS").unwrap_or_default()))
}

fn parse_host_list(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|h| h.trim().trim_end_matches('.').to_ascii_lowercase())
        .filter(|h| !h.is_empty())
        .collect()
}

/// The host part of a `Host` header / origin authority (port stripped,
/// lower-cased; a bracketed IPv6 literal keeps its brackets).
pub(crate) fn host_of(authority: &str) -> String {
    let a = authority.trim();
    let host = if a.starts_with('[') {
        match a.find(']') {
            Some(end) => &a[..=end],
            None => a,
        }
    } else {
        a.rsplit_once(':')
            .filter(|(_, port)| port.bytes().all(|b| b.is_ascii_digit()))
            .map(|(h, _)| h)
            .unwrap_or(a)
    };
    host.trim_end_matches('.').to_ascii_lowercase()
}

pub(crate) fn is_ip_literal(host: &str) -> bool {
    if let Some(inner) = host.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
        return inner.parse::<std::net::Ipv6Addr>().is_ok();
    }
    host.parse::<std::net::Ipv4Addr>().is_ok()
}

pub(crate) fn is_loopback_name(host: &str) -> bool {
    host == "localhost"
        || host.ends_with(".localhost")
        || host == "[::1]"
        || host
            .parse::<std::net::Ipv4Addr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// Whether a `*.ts.net` name is trusted: any, unless `extra` pins specific ones.
pub(crate) fn tailscale_name_allowed(host: &str, extra: &[String]) -> bool {
    if !host.ends_with(".ts.net") {
        return false;
    }
    let pinned: Vec<&String> = extra.iter().filter(|h| h.ends_with(".ts.net")).collect();
    pinned.is_empty() || pinned.iter().any(|p| p.as_str() == host)
}

/// Pure verdict for a (port-stripped, lower-cased) host.
pub(crate) fn host_allowed_with(host: &str, extra: &[String]) -> bool {
    is_ip_literal(host)
        || is_loopback_name(host)
        || host.ends_with(".local")
        || tailscale_name_allowed(host, extra)
        || extra.iter().any(|h| h == host)
}

pub(crate) fn host_allowed(host: &str) -> bool {
    host_allowed_with(host, configured_hosts())
}

pub(crate) fn tailscale_allowed(host: &str) -> bool {
    tailscale_name_allowed(host, configured_hosts())
}

fn misdirected() -> Response {
    (
        StatusCode::MISDIRECTED_REQUEST,
        axum::Json(otto_core::api::Problem {
            code: "bad_host".into(),
            message: "this daemon does not serve that Host name — set it as the Public link \
                      domain (share_base_url) or list it in OTTO_ALLOWED_HOSTS"
                .into(),
        }),
    )
        .into_response()
}

/// Static-only middleware (no settings lookup) — for routers without a DB.
pub async fn host_guard(req: Request, next: Next) -> Response {
    if let Some(h) = req.headers().get(header::HOST) {
        let ok = h
            .to_str()
            .map(|v| host_allowed(&host_of(v)))
            .unwrap_or(false);
        if !ok {
            return misdirected();
        }
    }
    next.run(req).await
}

/// When the `share_base_url` host was read, and its value.
type CachedShareHost = (std::time::Instant, Option<String>);

/// State for [`host_guard_with_settings`]: the settings DB + a short cache of
/// the `share_base_url` host (only consulted for a host the static rules
/// refuse, so the loopback hot path never touches the DB).
#[derive(Clone)]
pub struct HostGuardState {
    pool: otto_state::DbPool,
    cache: std::sync::Arc<std::sync::Mutex<Option<CachedShareHost>>>,
}

impl HostGuardState {
    pub fn new(pool: otto_state::DbPool) -> Self {
        Self {
            pool,
            cache: Default::default(),
        }
    }

    async fn share_host(&self) -> Option<String> {
        const TTL: std::time::Duration = std::time::Duration::from_secs(15);
        if let Some((at, v)) = self.cache.lock().unwrap_or_else(|p| p.into_inner()).clone() {
            if at.elapsed() < TTL {
                return v;
            }
        }
        let v = otto_state::SettingsRepo::new(self.pool.clone())
            .get("share_base_url")
            .await
            .ok()
            .flatten()
            .and_then(|v| v.as_str().map(str::to_string))
            .and_then(|u| reqwest::Url::parse(u.trim()).ok())
            .and_then(|u| u.host_str().map(|h| h.to_ascii_lowercase()));
        *self.cache.lock().unwrap_or_else(|p| p.into_inner()) =
            Some((std::time::Instant::now(), v.clone()));
        v
    }
}

/// Production middleware: the static rules, plus the Public link domain host.
pub async fn host_guard_with_settings(
    axum::extract::State(st): axum::extract::State<HostGuardState>,
    req: Request,
    next: Next,
) -> Response {
    if let Some(h) = req.headers().get(header::HOST) {
        let Ok(raw) = h.to_str() else {
            return misdirected();
        };
        let host = host_of(raw);
        if !host_allowed(&host) && st.share_host().await.as_deref() != Some(host.as_str()) {
            return misdirected();
        }
    }
    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_parsing_strips_port_and_case() {
        assert_eq!(host_of("LocalHost:7700"), "localhost");
        assert_eq!(host_of("[::1]:7700"), "[::1]");
        assert_eq!(host_of("mac.tail1234.ts.net."), "mac.tail1234.ts.net");
        assert_eq!(host_of("192.168.1.5"), "192.168.1.5");
    }

    #[test]
    fn rebinding_names_are_refused_ip_literals_and_ours_allowed() {
        let none: Vec<String> = vec![];
        for ok in [
            "127.0.0.1",
            "localhost",
            "tauri.localhost",
            "[::1]",
            "192.168.1.20",
            "100.101.102.103",
            "[fd7a:115c:a1e0::1]",
            "otto.local",
            "my-mac.local",
            "my-mac.tail1234.ts.net",
        ] {
            assert!(host_allowed_with(ok, &none), "{ok}");
        }
        for bad in [
            "evil.example",
            "127.0.0.1.nip.io",
            "localhost.evil.example",
            "ts.net.evil",
        ] {
            assert!(!host_allowed_with(bad, &none), "{bad}");
        }
    }

    #[test]
    fn configured_hosts_add_names_and_pin_tailscale() {
        let extra = parse_host_list(" Otto.Example.com , my-mac.tail1234.ts.net ");
        assert!(host_allowed_with("otto.example.com", &extra));
        assert!(host_allowed_with("my-mac.tail1234.ts.net", &extra));
        // Pinned: another tailnet name is no longer trusted.
        assert!(!host_allowed_with("other.tail9999.ts.net", &extra));
    }

    #[tokio::test]
    async fn middleware_rejects_foreign_host_and_passes_ours() {
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt;
        let app = axum::Router::new()
            .fallback(|| async { "ok" })
            .layer(axum::middleware::from_fn(host_guard));
        let req = |host: Option<&str>| {
            let mut b = Request::builder().uri("/meta");
            if let Some(h) = host {
                b = b.header(header::HOST, h);
            }
            b.body(Body::empty()).unwrap()
        };
        let bad = app
            .clone()
            .oneshot(req(Some("evil.example:7700")))
            .await
            .unwrap();
        assert_eq!(bad.status(), StatusCode::MISDIRECTED_REQUEST);
        for ok in [Some("127.0.0.1:7700"), Some("localhost:5173"), None] {
            let r = app.clone().oneshot(req(ok)).await.unwrap();
            assert_eq!(r.status(), StatusCode::OK, "{ok:?}");
        }
    }
}
