//! SSRF defense shared by every outbound user-URL fetch in the daemon (the
//! API-client / streaming / gRPC / browser-proxy paths *and* the Message-Brokers
//! Prometheus-metrics / Schema-Registry fetches). Resolves the target host and
//! rejects requests that would reach loopback, private, link-local (incl. cloud
//! metadata), CGNAT, unspecified, or multicast/broadcast addresses, and bounds +
//! re-validates HTTP redirects so an upstream can't bounce us into the internal
//! network.
//!
//! Audit S1. This lives in its own leaf crate (depended on by `otto-server` and
//! `otto-brokers`) so the classifier is defined exactly once — a second,
//! drifting copy is how an SSRF hole sneaks back in. `std` + `tokio` only (URL
//! parsing via the already-vendored `reqwest::Url`).

use std::net::{IpAddr, Ipv4Addr, ToSocketAddrs};

/// Max redirect hops we follow (matches reqwest's prior `Policy::limited(10)`).
pub const MAX_REDIRECTS: usize = 10;

/// True when `ip` must never be reachable by a user-supplied fetch: any
/// loopback, private (RFC1918 / ULA fc00::/7), link-local (incl. the
/// 169.254.169.254 cloud-metadata address and fe80::/10), CGNAT
/// (100.64.0.0/10), unspecified (0.0.0.0 / ::), or multicast/broadcast
/// address. Also unwraps IPv4-mapped/compat IPv6 so `::ffff:127.0.0.1`
/// can't slip through.
pub fn is_blocked_ip(ip: IpAddr) -> bool {
    let ip = match ip {
        // Unwrap only the genuine IPv4-mapped form (`::ffff:a.b.c.d`) so the
        // v4 rules below apply to it. We must NOT use the deprecated
        // `to_ipv4()` here: it also maps IPv4-compatible addresses in `::/96`
        // (e.g. `::1` → `0.0.0.1`), which would dodge every v4 block check and
        // let IPv6 loopback slip through. `::1`/`fe80::`/`fc00::` are handled
        // by the V6 arm below instead.
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => IpAddr::V6(v6),
        },
        other => other,
    };
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_multicast()
                || v4.is_documentation()
                // Cloud metadata endpoint (also caught by link_local, kept explicit).
                || v4 == Ipv4Addr::new(169, 254, 169, 254)
                // CGNAT 100.64.0.0/10 (not flagged by is_private).
                || (v4.octets()[0] == 100 && (v4.octets()[1] & 0xC0) == 0x40)
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                // Unique-local fc00::/7.
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                // Link-local fe80::/10.
                || (v6.segments()[0] & 0xffc0) == 0xfe80
        }
    }
}

/// Parse `url`, returning `(host, port)` for DNS resolution. Rejects URLs
/// without a host (e.g. `file:`, `data:`) and any non-http(s)/ws/grpc scheme.
fn host_port(url: &str) -> Result<(String, u16), String> {
    let parsed = reqwest::Url::parse(url).map_err(|e| format!("invalid url: {e}"))?;
    match parsed.scheme() {
        "http" | "https" | "ws" | "wss" | "grpc" | "grpcs" => {}
        other => return Err(format!("blocked url scheme: {other}")),
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| "url has no host".to_string())?
        .to_string();
    let port = parsed.port_or_known_default().unwrap_or(0);
    Ok((host, port))
}

/// Pre-flight async check of a target URL: resolve the host and reject if
/// ANY resolved address is blocked. A bare IP literal is checked directly
/// (no DNS). Returns a human-readable reason on rejection.
pub async fn check_url(url: &str) -> Result<(), String> {
    let (host, port) = host_port(url)?;
    // IP literal → no DNS needed.
    if let Ok(ip) = host.parse::<IpAddr>() {
        return if is_blocked_ip(ip) {
            Err(format!("blocked address {ip} (SSRF guard)"))
        } else {
            Ok(())
        };
    }
    // Resolve on the blocking pool (lookup_host wants host:port).
    let probe_port = if port == 0 { 80 } else { port };
    let addrs = tokio::net::lookup_host((host.as_str(), probe_port))
        .await
        .map_err(|e| format!("dns resolution failed for {host}: {e}"))?;
    let mut saw = false;
    for sa in addrs {
        saw = true;
        if is_blocked_ip(sa.ip()) {
            return Err(format!(
                "blocked address {} for host {host} (SSRF guard)",
                sa.ip()
            ));
        }
    }
    if !saw {
        return Err(format!("host {host} did not resolve"));
    }
    Ok(())
}

/// Synchronous host check for use inside reqwest's redirect policy (which is
/// a sync callback). IP literals are checked directly; hostnames are
/// resolved via a bounded blocking lookup. On any resolution error we fail
/// closed (block), since a redirect we can't validate is not one we follow.
fn check_url_blocking(url: &reqwest::Url) -> bool {
    match url.scheme() {
        "http" | "https" | "ws" | "wss" | "grpc" | "grpcs" => {}
        _ => return false,
    }
    let host = match url.host_str() {
        Some(h) => h,
        None => return false,
    };
    if let Ok(ip) = host.parse::<IpAddr>() {
        return !is_blocked_ip(ip);
    }
    let port = url.port_or_known_default().unwrap_or(80);
    match (host, port).to_socket_addrs() {
        Ok(addrs) => {
            let mut saw = false;
            for sa in addrs {
                saw = true;
                if is_blocked_ip(sa.ip()) {
                    return false;
                }
            }
            saw
        }
        Err(_) => false,
    }
}

/// A `reqwest` redirect policy that caps hops at [`MAX_REDIRECTS`] and
/// re-validates each hop's target host against the SSRF rules, so an
/// upstream 30x can't bounce the fetch into a private/loopback address.
pub fn redirect_policy() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= MAX_REDIRECTS {
            return attempt.error("too many redirects");
        }
        if check_url_blocking(attempt.url()) {
            attempt.follow()
        } else {
            attempt.stop()
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv6Addr;

    #[test]
    fn blocks_internal_addresses() {
        for ip in [
            "127.0.0.1",
            "10.0.0.5",
            "192.168.1.1",
            "172.16.0.1",
            "169.254.169.254", // cloud metadata
            "100.64.0.1",      // CGNAT
            "0.0.0.0",
        ] {
            assert!(is_blocked_ip(ip.parse().unwrap()), "{ip} must be blocked");
        }
        // IPv4-mapped loopback must not slip through.
        assert!(is_blocked_ip(IpAddr::V6("::ffff:127.0.0.1".parse::<Ipv6Addr>().unwrap())));
        assert!(is_blocked_ip(IpAddr::V6(Ipv6Addr::LOCALHOST)));
        // A normal public address is allowed.
        assert!(!is_blocked_ip("8.8.8.8".parse().unwrap()));
        assert!(!is_blocked_ip("1.1.1.1".parse().unwrap()));
    }

    #[tokio::test]
    async fn check_url_rejects_loopback_and_bad_schemes() {
        assert!(check_url("http://127.0.0.1/").await.is_err());
        assert!(check_url("http://169.254.169.254/latest/meta-data/").await.is_err());
        assert!(check_url("file:///etc/passwd").await.is_err());
        assert!(check_url("data:text/plain,hi").await.is_err());
        assert!(check_url("http://[::1]/").await.is_err());
        // A public host should pass scheme/host classification (DNS allowing).
        assert!(check_url("http://8.8.8.8/").await.is_ok());
    }
}
