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
//!
//! DNS rebinding: a pre-flight [`check_url`] alone is a time-of-check — the
//! HTTP client would resolve the name AGAIN when it dials, and a 0-TTL record
//! can answer 127.0.0.1 the second time. Every guarded client therefore also
//! installs [`GuardedResolver`] (via [`guarded_client_builder`]), which applies
//! the same vetting to the addresses hyper actually dials; transports that dial
//! themselves (gRPC, WebSocket) connect to the addresses [`resolve_checked`]
//! returned instead of re-resolving the name.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs};

/// Max redirect hops we follow (matches reqwest's prior `Policy::limited(10)`).
pub const MAX_REDIRECTS: usize = 10;

/// Transport check for a CONFIGURED base URL that will carry credentials
/// (basic auth, API tokens): require TLS (`https`/`wss`/`grpcs`), allowing
/// plain `http`/`ws`/`grpc` only when the host is loopback — this is a local
/// dev tool and localhost flows must keep working. Sync and DNS-free on
/// purpose (config-time validation must not block); a hostname that RESOLVES
/// to loopback still needs `https`, which fails safe.
pub fn require_tls_or_loopback(url: &str) -> Result<(), String> {
    let parsed = reqwest::Url::parse(url).map_err(|e| format!("invalid url: {e}"))?;
    match parsed.scheme() {
        "https" | "wss" | "grpcs" => return Ok(()),
        "http" | "ws" | "grpc" => {}
        other => return Err(format!("blocked url scheme: {other}")),
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| "url has no host".to_string())?;
    // `host_str` keeps the brackets on an IPv6 literal — strip for parsing.
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    let loopback = host.eq_ignore_ascii_case("localhost")
        || bare
            .parse::<IpAddr>()
            .map(|ip| ip.is_loopback())
            .unwrap_or(false);
    if loopback {
        Ok(())
    } else {
        Err(format!(
            "{host} is reached over plain {} — credentials would travel unencrypted; use https \
             (plain http is allowed for localhost only)",
            parsed.scheme()
        ))
    }
}

/// True when `ip` must never be reachable by a user-supplied fetch: any
/// loopback, private (RFC1918 / ULA fc00::/7 / deprecated site-local
/// fec0::/10), link-local (incl. the 169.254.169.254 cloud-metadata address and
/// fe80::/10), CGNAT (100.64.0.0/10), "this network" (0.0.0.0/8), benchmarking
/// (198.18.0.0/15), IETF-protocol (192.0.0.0/24), reserved (240.0.0.0/4, incl.
/// broadcast), documentation, or multicast address.
///
/// IPv6 forms that EMBED an IPv4 address are unwrapped and the v4 rules applied
/// to the embedded address, so none of them reaches 127.0.0.1 /
/// 169.254.169.254 by the back door: IPv4-mapped (`::ffff:a.b.c.d`),
/// IPv4-compatible (`::a.b.c.d`), the NAT64 well-known prefix (`64:ff9b::/96`),
/// 6to4 (`2002:AABB:CCDD::/48`) and Teredo (`2001:0::/32`, client address XOR'd
/// into the low 32 bits). The local-use NAT64 prefix `64:ff9b:1::/48` is blocked
/// outright (its embedding offset is deployment-specific).
pub fn is_blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_blocked_v4(v4),
        IpAddr::V6(v6) => is_blocked_v6(v6),
    }
}

fn is_blocked_v4(v4: Ipv4Addr) -> bool {
    let o = v4.octets();
    v4.is_loopback()
        || v4.is_private()
        || v4.is_link_local()
        || v4.is_unspecified()
        || v4.is_broadcast()
        || v4.is_multicast()
        || v4.is_documentation()
        // Cloud metadata endpoint (also caught by link_local, kept explicit).
        || v4 == Ipv4Addr::new(169, 254, 169, 254)
        // "This network" 0.0.0.0/8 — 0.x dials the local host on macOS/Linux.
        || o[0] == 0
        // CGNAT 100.64.0.0/10 (not flagged by is_private).
        || (o[0] == 100 && (o[1] & 0xC0) == 0x40)
        // IETF protocol assignments 192.0.0.0/24.
        || (o[0] == 192 && o[1] == 0 && o[2] == 0)
        // Benchmarking 198.18.0.0/15.
        || (o[0] == 198 && (o[1] & 0xFE) == 18)
        // Reserved 240.0.0.0/4 (includes 255.255.255.255).
        || o[0] >= 240
}

/// The IPv4 address an IPv6 transition form carries, if any (see
/// [`is_blocked_ip`]). `::` and `::1` are NOT treated as IPv4-compatible — they
/// are the v6 unspecified/loopback addresses, handled by the v6 rules. (We
/// deliberately avoid the std `to_ipv4()`, which maps `::1` → `0.0.0.1`.)
fn embedded_v4(v6: Ipv6Addr) -> Option<Ipv4Addr> {
    let s = v6.segments();
    let low32 = (u32::from(s[6]) << 16) | u32::from(s[7]);
    if let Some(v4) = v6.to_ipv4_mapped() {
        return Some(v4);
    }
    // IPv4-compatible ::a.b.c.d (deprecated, but still routable on some stacks).
    if s[..6].iter().all(|x| *x == 0) && !v6.is_unspecified() && !v6.is_loopback() {
        return Some(Ipv4Addr::from(low32));
    }
    // NAT64 well-known prefix 64:ff9b::/96.
    if s[0] == 0x64 && s[1] == 0xff9b && s[2..6].iter().all(|x| *x == 0) {
        return Some(Ipv4Addr::from(low32));
    }
    // 6to4 2002:AABB:CCDD::/48 → AA.BB.CC.DD.
    if s[0] == 0x2002 {
        return Some(Ipv4Addr::from((u32::from(s[1]) << 16) | u32::from(s[2])));
    }
    // Teredo 2001:0000::/32 — the client address is the low 32 bits, XOR'd.
    if s[0] == 0x2001 && s[1] == 0 {
        return Some(Ipv4Addr::from(low32 ^ 0xffff_ffff));
    }
    None
}

fn is_blocked_v6(v6: Ipv6Addr) -> bool {
    if embedded_v4(v6).is_some_and(is_blocked_v4) {
        return true;
    }
    let s = v6.segments();
    v6.is_loopback()
        || v6.is_unspecified()
        || v6.is_multicast()
        // Unique-local fc00::/7.
        || (s[0] & 0xfe00) == 0xfc00
        // Link-local fe80::/10.
        || (s[0] & 0xffc0) == 0xfe80
        // Deprecated site-local fec0::/10.
        || (s[0] & 0xffc0) == 0xfec0
        // Local-use NAT64 64:ff9b:1::/48 (RFC 8215).
        || (s[0] == 0x64 && s[1] == 0xff9b && s[2] == 1)
        // Documentation 2001:db8::/32.
        || (s[0] == 0x2001 && s[1] == 0x0db8)
}

/// The URL's host with IPv6 brackets stripped: `http://[::1]/` → `::1`.
/// `Url::host_str` keeps the brackets, which used to make every IPv6 literal
/// fail `parse::<IpAddr>()` and then fail DNS with a misleading error.
/// (Decimal/octal/hex IPv4 forms such as `http://2130706433/` are already
/// normalised to dotted-quad by the WHATWG URL parser.)
fn bare_host(url: &reqwest::Url) -> Option<&str> {
    let host = url.host_str()?;
    Some(
        host.strip_prefix('[')
            .and_then(|h| h.strip_suffix(']'))
            .unwrap_or(host),
    )
}

fn scheme_allowed(scheme: &str) -> bool {
    matches!(scheme, "http" | "https" | "ws" | "wss" | "grpc" | "grpcs")
}

/// Parse `url`, returning `(bare host, port)` for DNS resolution. Rejects URLs
/// without a host (e.g. `file:`, `data:`) and any non-http(s)/ws/grpc scheme.
/// `port` is the explicit or scheme-default port, `80` when neither is known
/// (`grpc`/`grpcs` have no registered default).
fn host_port(url: &str) -> Result<(String, u16), String> {
    let parsed = reqwest::Url::parse(url).map_err(|e| format!("invalid url: {e}"))?;
    if !scheme_allowed(parsed.scheme()) {
        return Err(format!("blocked url scheme: {}", parsed.scheme()));
    }
    let host = bare_host(&parsed)
        .filter(|h| !h.is_empty())
        .ok_or_else(|| "url has no host".to_string())?
        .to_string();
    let port = parsed.port_or_known_default().unwrap_or(80);
    Ok((host, port))
}

/// Resolve `host` ONCE and vet every returned address: fails when the name
/// does not resolve or when ANY address is blocked (a name answering with both
/// a public and a private address is treated as hostile). The returned
/// addresses are the ones to dial — never re-resolve the name afterwards. An
/// IP literal (bare, no brackets) is vetted directly, without DNS.
pub async fn resolve_host_checked(host: &str, port: u16) -> Result<Vec<SocketAddr>, String> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return if is_blocked_ip(ip) {
            Err(format!("blocked address {ip} (SSRF guard)"))
        } else {
            Ok(vec![SocketAddr::new(ip, port)])
        };
    }
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host, port))
        .await
        .map_err(|e| format!("dns resolution failed for {host}: {e}"))?
        .collect();
    if addrs.is_empty() {
        return Err(format!("host {host} did not resolve"));
    }
    if let Some(bad) = addrs.iter().find(|a| is_blocked_ip(a.ip())) {
        return Err(format!(
            "blocked address {} for host {host} (SSRF guard)",
            bad.ip()
        ));
    }
    Ok(addrs)
}

/// Resolve + vet a target URL once (see [`resolve_host_checked`]) and return
/// `(bare host, vetted addresses)`. For transports that dial themselves (gRPC,
/// WebSocket): connect to one of the returned addresses — pinned — instead of
/// handing the hostname to a connector that would resolve it again.
pub async fn resolve_checked(url: &str) -> Result<(String, Vec<SocketAddr>), String> {
    let (host, port) = host_port(url)?;
    let addrs = resolve_host_checked(&host, port).await?;
    Ok((host, addrs))
}

/// Pre-flight async check of a target URL: resolve the host and reject if
/// ANY resolved address is blocked. A bare IP literal (incl. a bracketed IPv6
/// literal) is checked directly (no DNS). Returns a human-readable reason on
/// rejection.
///
/// On its own this is only a time-of-check (see the module docs): pair it with
/// a client from [`guarded_client_builder`], or dial the addresses from
/// [`resolve_checked`].
pub async fn check_url(url: &str) -> Result<(), String> {
    resolve_checked(url).await.map(|_| ())
}

/// DNS-free check of a URL: scheme allowed, has a host, and — when the host
/// is an IP literal — the address is not blocked. Hostnames pass here (they are
/// vetted at connect time by [`GuardedResolver`]). Used by
/// [`guarded_redirect_policy`] and by callers that must vet a navigation target
/// synchronously.
pub fn check_url_literal(url: &reqwest::Url) -> bool {
    if !scheme_allowed(url.scheme()) {
        return false;
    }
    match bare_host(url) {
        Some(h) if !h.is_empty() => match h.parse::<IpAddr>() {
            Ok(ip) => !is_blocked_ip(ip),
            Err(_) => true,
        },
        _ => false,
    }
}

/// Synchronous host check for use inside reqwest's redirect policy (which is
/// a sync callback). IP literals are checked directly; hostnames are
/// resolved via a bounded blocking lookup. On any resolution error we fail
/// closed (block), since a redirect we can't validate is not one we follow.
fn check_url_blocking(url: &reqwest::Url) -> bool {
    if !check_url_literal(url) {
        return false;
    }
    let Some(host) = bare_host(url) else {
        return false;
    };
    if host.parse::<IpAddr>().is_ok() {
        return true; // literal already vetted above
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
///
/// This variant resolves hostnames itself (blocking, inside reqwest's sync
/// callback) because the client it is installed on may still use the default
/// resolver. Prefer [`guarded_client_builder`], which vets at connect time and
/// uses the DNS-free [`guarded_redirect_policy`].
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

/// Redirect policy for a client that has [`GuardedResolver`] installed: caps
/// hops at [`MAX_REDIRECTS`] and re-checks each hop's scheme and IP-literal
/// host (hyper dials IP literals WITHOUT consulting the resolver, so those must
/// be vetted here); hop hostnames are vetted by the resolver when they connect.
/// No DNS on the callback thread.
pub fn guarded_redirect_policy() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= MAX_REDIRECTS {
            return attempt.error("too many redirects");
        }
        if check_url_literal(attempt.url()) {
            attempt.follow()
        } else {
            attempt.stop()
        }
    })
}

/// A `reqwest` DNS resolver that resolves each name once per connect and
/// refuses it when ANY address is blocked (see [`resolve_host_checked`]).
/// Because the vetted addresses are exactly the ones hyper dials, a 0-TTL
/// rebinding answer can't swap in 127.0.0.1 / 169.254.169.254 between check
/// and connect — and every redirect hop to a hostname is vetted the same way.
///
/// IP-literal hosts never reach a resolver (hyper parses them itself), so pair
/// this with [`check_url`] up front and [`guarded_redirect_policy`] for hops —
/// [`guarded_client_builder`] installs both.
#[derive(Debug, Clone, Copy, Default)]
pub struct GuardedResolver;

impl reqwest::dns::Resolve for GuardedResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_string();
        Box::pin(async move {
            // Port 0: reqwest substitutes the URL's (or the scheme-default) port.
            let addrs = resolve_host_checked(&host, 0)
                .await
                .map_err(|e: String| -> Box<dyn std::error::Error + Send + Sync> { e.into() })?;
            let iter: reqwest::dns::Addrs = Box::new(addrs.into_iter());
            Ok::<reqwest::dns::Addrs, Box<dyn std::error::Error + Send + Sync>>(iter)
        })
    }
}

/// A `reqwest::ClientBuilder` with the SSRF guard installed end to end:
/// [`GuardedResolver`] (connect-time vetting, DNS-rebinding safe) plus
/// [`guarded_redirect_policy`] (bounded, re-checked hops). Callers still run
/// [`check_url`] first for a clean error message and to vet IP literals.
///
/// Not for a client that must reach a local proxy BY HOSTNAME (the proxy's own
/// name would be resolved — and refused — here); tunnelled / explicitly
/// allow-local clients keep the plain builder.
pub fn guarded_client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .dns_resolver(std::sync::Arc::new(GuardedResolver))
        .redirect(guarded_redirect_policy())
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::dns::Resolve;

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
            "0.1.2.3",         // 0.0.0.0/8
            "198.18.0.1",      // benchmarking
            "198.19.255.254",  // benchmarking
            "192.0.0.8",       // IETF protocol assignments
            "240.0.0.1",       // reserved
            "255.255.255.255", // broadcast
        ] {
            assert!(is_blocked_ip(ip.parse().unwrap()), "{ip} must be blocked");
        }
        // IPv4-mapped loopback must not slip through.
        assert!(is_blocked_ip(IpAddr::V6(
            "::ffff:127.0.0.1".parse::<Ipv6Addr>().unwrap()
        )));
        assert!(is_blocked_ip(IpAddr::V6(Ipv6Addr::LOCALHOST)));
        assert!(is_blocked_ip(IpAddr::V6(Ipv6Addr::UNSPECIFIED)));
        // A normal public address is allowed.
        assert!(!is_blocked_ip("8.8.8.8".parse().unwrap()));
        assert!(!is_blocked_ip("1.1.1.1".parse().unwrap()));
        assert!(!is_blocked_ip("198.20.0.1".parse().unwrap()));
    }

    #[test]
    fn blocks_ipv6_forms_that_embed_a_blocked_ipv4() {
        for ip in [
            "64:ff9b::a9fe:a9fe",   // NAT64 → 169.254.169.254
            "64:ff9b::7f00:1",      // NAT64 → 127.0.0.1
            "64:ff9b:1::1",         // local-use NAT64, blocked outright
            "2002:7f00:1::",        // 6to4 → 127.0.0.1
            "2002:a9fe:a9fe::1",    // 6to4 → 169.254.169.254
            "2002:c0a8:101::",      // 6to4 → 192.168.1.1
            "::127.0.0.1",          // IPv4-compatible → 127.0.0.1
            "::a9fe:a9fe",          // IPv4-compatible → 169.254.169.254
            "2001:0:1:2:3:4:80ff:fffe", // Teredo client ^0xffffffff → 127.0.0.1
            "fec0::1",              // site-local
            "fe80::1",              // link-local
            "fd00::1",              // ULA
            "2001:db8::1",          // documentation
        ] {
            let v6: Ipv6Addr = ip.parse().unwrap();
            assert!(is_blocked_ip(IpAddr::V6(v6)), "{ip} must be blocked");
        }
        // Transition forms wrapping a PUBLIC v4 stay reachable.
        for ip in ["64:ff9b::808:808", "2002:808:808::1", "2606:4700:4700::1111"] {
            let v6: Ipv6Addr = ip.parse().unwrap();
            assert!(!is_blocked_ip(IpAddr::V6(v6)), "{ip} must be allowed");
        }
    }

    #[tokio::test]
    async fn check_url_rejects_loopback_and_bad_schemes() {
        assert!(check_url("http://127.0.0.1/").await.is_err());
        assert!(check_url("http://169.254.169.254/latest/meta-data/")
            .await
            .is_err());
        assert!(check_url("file:///etc/passwd").await.is_err());
        assert!(check_url("data:text/plain,hi").await.is_err());
        // A public host should pass scheme/host classification (DNS allowing).
        assert!(check_url("http://8.8.8.8/").await.is_ok());
    }

    #[tokio::test]
    async fn ipv6_literals_are_classified_not_dns_failed() {
        // Blocked for the RIGHT reason (the address), not a DNS failure.
        let err = check_url("http://[::1]/").await.unwrap_err();
        assert!(err.contains("blocked address"), "{err}");
        let err = check_url("http://[::ffff:169.254.169.254]/")
            .await
            .unwrap_err();
        assert!(err.contains("blocked address"), "{err}");
        let err = check_url("http://[64:ff9b::a9fe:a9fe]:8080/")
            .await
            .unwrap_err();
        assert!(err.contains("blocked address"), "{err}");
        // A public IPv6 literal is allowed (no DNS involved).
        assert!(check_url("http://[2606:4700:4700::1111]/").await.is_ok());
    }

    #[tokio::test]
    async fn decimal_and_octal_ipv4_hosts_are_normalised_and_blocked() {
        // WHATWG parsing turns these into 127.0.0.1 / 169.254.169.254.
        for url in [
            "http://2130706433/",
            "http://0x7f000001/",
            "http://0177.0.0.1/",
            "http://127.1/",
            "http://2852039166/",
        ] {
            let err = check_url(url).await.unwrap_err();
            assert!(err.contains("blocked address"), "{url}: {err}");
        }
    }

    #[tokio::test]
    async fn resolve_checked_pins_literal_with_default_port() {
        let (host, addrs) = resolve_checked("https://[2606:4700:4700::1111]/x")
            .await
            .unwrap();
        assert_eq!(host, "2606:4700:4700::1111");
        assert_eq!(addrs.len(), 1);
        assert_eq!(addrs[0].port(), 443);
        let (_, addrs) = resolve_checked("grpc://8.8.8.8:50051").await.unwrap();
        assert_eq!(addrs[0].port(), 50051);
    }

    #[tokio::test]
    async fn guarded_resolver_refuses_loopback_names() {
        // `localhost` resolves locally (no network) to loopback → refused.
        let name: reqwest::dns::Name = "localhost".parse().unwrap();
        let res = GuardedResolver.resolve(name).await;
        assert!(res.is_err());
    }

    #[test]
    fn literal_check_vets_scheme_and_ip_hosts_without_dns() {
        let ok = |u: &str| check_url_literal(&reqwest::Url::parse(u).unwrap());
        assert!(!ok("http://127.0.0.1/"));
        assert!(!ok("http://[::1]:7700/"));
        assert!(!ok("http://169.254.169.254/"));
        assert!(!ok("ftp://example.com/"));
        assert!(!ok("file:///etc/passwd"));
        assert!(ok("https://example.com/"));
        assert!(ok("https://8.8.8.8/"));
    }

    #[test]
    fn tls_or_loopback_gate() {
        // TLS anywhere is fine.
        assert!(require_tls_or_loopback("https://site.atlassian.net").is_ok());
        // Plain http only for loopback hosts (dev flows).
        assert!(require_tls_or_loopback("http://localhost:8081").is_ok());
        assert!(require_tls_or_loopback("http://127.0.0.1:8081/api").is_ok());
        assert!(require_tls_or_loopback("http://[::1]:8081").is_ok());
        // Plain http to anything else leaks credentials → rejected.
        assert!(require_tls_or_loopback("http://registry.internal:8081").is_err());
        assert!(require_tls_or_loopback("http://10.0.0.5").is_err());
        // Non-http(s) schemes stay blocked.
        assert!(require_tls_or_loopback("file:///etc/passwd").is_err());
    }
}
