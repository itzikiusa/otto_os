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
//!
//! Allow-local mode (the API client's per-workspace opt-in): [`Guard`],
//! [`allow_local_client_builder`] and friends let private/LAN/loopback targets
//! through but keep cloud-metadata addresses and the daemon's own listener
//! blocked always.

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

/// True when the URL names a local-network target BY LITERAL: the host is
/// `localhost`, or an IP literal that is loopback, private (RFC1918 / ULA
/// `fc00::/7`) or CGNAT (`100.64.0.0/10` — Tailscale/VPN addresses). Never
/// link-local (incl. the `169.254.169.254` metadata address), unspecified,
/// multicast or reserved space, and never a hostname that merely RESOLVES to
/// a private address (that is the DNS-rebinding shape the guard exists for).
///
/// For an owner-configured endpoint only — an MCP server URL a person typed,
/// whose registration already lets them spawn any command. "I asked for
/// `http://127.0.0.1:3845/mcp`" is not an SSRF; the guard keeps vetting every
/// other URL (and every hostname) exactly as before.
pub fn is_explicit_local_target(url: &reqwest::Url) -> bool {
    if !scheme_allowed(url.scheme()) {
        return false;
    }
    let Some(host) = bare_host(url) else {
        return false;
    };
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(v4)) => {
            let o = v4.octets();
            v4.is_loopback() || v4.is_private() || (o[0] == 100 && (o[1] & 0xC0) == 0x40)
        }
        Ok(IpAddr::V6(v6)) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return v4.is_loopback() || v4.is_private();
            }
            v6.is_loopback() || (v6.segments()[0] & 0xfe00) == 0xfc00
        }
        Err(_) => false,
    }
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

// ---------------------------------------------------------------------------
// Allow-local mode: the API client's per-workspace "Private addresses: allowed"
// opt-in. Loopback / RFC1918 / ULA / CGNAT targets are reachable (a dev server
// on localhost is the #1 thing an API client is pointed at), but two kinds of
// target stay refused whatever the setting says:
//   * cloud-metadata / link-local addresses — the credentials an instance
//     role hands out are one GET away, and no dev server lives there;
//   * Otto's own daemon listener — an agent driving the API client must not be
//     able to call the daemon's API with the client's cookies/headers.
// ---------------------------------------------------------------------------

/// The daemon's default port (`ottod`'s `DEFAULT_PORT`).
pub const DEFAULT_DAEMON_PORT: u16 = 7700;

fn extra_daemon_ports() -> &'static std::sync::Mutex<Vec<u16>> {
    static PORTS: std::sync::OnceLock<std::sync::Mutex<Vec<u16>>> = std::sync::OnceLock::new();
    PORTS.get_or_init(|| std::sync::Mutex::new(Vec::new()))
}

/// Record another port this daemon listens on (the opt-in network listener
/// may use its own port) — or one of its own loopback services listens on
/// (the embedded ClickHouse HTTP port, a plugin sidecar) — so allow-local
/// fetches refuse it too.
pub fn register_daemon_port(port: u16) {
    let mut ports = extra_daemon_ports()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if !ports.contains(&port) {
        ports.push(port);
    }
}

/// True when `port` is one the daemon listens on: the default 7700 (the
/// installed daemon, even when this process runs on another port),
/// `$OTTO_PORT`, or one passed to [`register_daemon_port`].
pub fn is_daemon_port(port: u16) -> bool {
    port == DEFAULT_DAEMON_PORT
        || std::env::var("OTTO_PORT")
            .ok()
            .and_then(|p| p.trim().parse::<u16>().ok())
            == Some(port)
        || extra_daemon_ports()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(&port)
}

fn is_metadata_v4(v4: Ipv4Addr) -> bool {
    // Link-local 169.254.0.0/16: AWS/GCP/Azure/OCI 169.254.169.254, ECS task
    // credentials 169.254.170.2, AWS DNS/NTP 169.254.169.253/.123.
    v4.is_link_local()
        // Alibaba Cloud metadata.
        || v4 == Ipv4Addr::new(100, 100, 100, 200)
        // Oracle Cloud classic metadata.
        || v4 == Ipv4Addr::new(192, 0, 0, 192)
}

/// Well-known names of cloud-metadata services. Locally they resolve to an
/// address [`is_metadata_ip`] refuses; through a tunnel the FAR end resolves
/// them, so a tunnelled fetch refuses the name itself ([`metadata_refusal`]).
pub const METADATA_HOSTNAMES: &[&str] = &[
    "metadata.google.internal",
    "metadata.goog",
    "metadata",
    "instance-data",
    "instance-data.ec2.internal",
    "metadata.tencentyun.com",
];

/// True when `host` (bare, any case, a trailing dot allowed) is one of
/// [`METADATA_HOSTNAMES`].
pub fn is_metadata_hostname(host: &str) -> bool {
    let host = host.trim_end_matches('.');
    METADATA_HOSTNAMES
        .iter()
        .any(|name| host.eq_ignore_ascii_case(name))
}

/// True for a cloud-metadata (or link-local) address, in any IPv6 form that
/// embeds one (see [`is_blocked_ip`]). Blocked even in allow-local mode.
pub fn is_metadata_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_metadata_v4(v4),
        IpAddr::V6(v6) => {
            embedded_v4(v6).is_some_and(is_metadata_v4)
                // Link-local fe80::/10.
                || (v6.segments()[0] & 0xffc0) == 0xfe80
                // AWS IMDS over IPv6.
                || v6 == Ipv6Addr::new(0xfd00, 0x0ec2, 0, 0, 0, 0, 0, 0x0254)
        }
    }
}

/// The allow-local classifier for a dial of `ip:port`: a metadata address
/// ([`is_metadata_ip`]), or a non-public address on a daemon port
/// ([`is_daemon_port`] — loopback, the unspecified address and the LAN
/// addresses the network listener answers on alike).
pub fn is_blocked_ip_allow_local(ip: IpAddr, port: u16) -> bool {
    is_metadata_ip(ip) || (is_daemon_port(port) && is_blocked_ip(ip))
}

/// Why an allow-local target is refused, `None` when it may be dialled. The
/// text says "always blocked" (the API client keys its hint on it: allowing
/// private addresses would not help) and keeps "SSRF guard".
fn allow_local_refusal(url: &reqwest::Url) -> Option<String> {
    if !scheme_allowed(url.scheme()) {
        return Some(format!("blocked url scheme: {}", url.scheme()));
    }
    let Some(host) = bare_host(url).filter(|h| !h.is_empty()) else {
        return Some("url has no host".to_string());
    };
    let port = url.port_or_known_default().unwrap_or(80);
    let ip = host.parse::<IpAddr>().ok();
    if is_daemon_port(port) && ip.is_none_or(is_blocked_ip) {
        // A HOSTNAME on the daemon's port is refused outright: the connect-time
        // resolver never sees the port, so a rebinding answer of 127.0.0.1
        // could not be caught there.
        return Some(format!(
            "blocked address {host}:{port}: port {port} is Otto's own daemon, always blocked \
             (SSRF guard)"
        ));
    }
    match ip {
        Some(ip) if is_metadata_ip(ip) => Some(format!(
            "blocked address {ip}: cloud-metadata and link-local addresses are always blocked \
             (SSRF guard)"
        )),
        _ => None,
    }
}

/// DNS-free allow-local check of a URL (the [`check_url_literal`] of
/// allow-local mode): scheme allowed, has a host, not a metadata literal, and
/// not the daemon's port on a non-public host.
pub fn check_url_literal_allow_local(url: &reqwest::Url) -> bool {
    allow_local_refusal(url).is_none()
}

/// The pre-send check of a request sent through [`allow_local_client_builder`]:
/// [`check_url_literal_allow_local`] with the reason, and no DNS. Resolution
/// adds nothing here — the client's [`AllowLocalResolver`] vets the addresses
/// it actually dials (a pre-send lookup would be re-done at connect time
/// anyway), and a hostname on the daemon's port is refused without one. What
/// the resolver never sees, an IP-literal host, is vetted here.
pub fn precheck_url_allow_local(url: &str) -> Result<(), String> {
    let parsed = reqwest::Url::parse(url).map_err(|e| format!("invalid url: {e}"))?;
    allow_local_refusal(&parsed).map_or(Ok(()), Err)
}

/// Resolve + vet a target URL once under the allow-local rules and return
/// `(bare host, vetted addresses)` — the allow-local [`resolve_checked`].
pub async fn resolve_checked_allow_local(url: &str) -> Result<(String, Vec<SocketAddr>), String> {
    let parsed = reqwest::Url::parse(url).map_err(|e| format!("invalid url: {e}"))?;
    if let Some(reason) = allow_local_refusal(&parsed) {
        return Err(reason);
    }
    let (host, port) = host_port(url)?;
    let addrs: Vec<SocketAddr> = match host.parse::<IpAddr>() {
        Ok(ip) => vec![SocketAddr::new(ip, port)],
        Err(_) => tokio::net::lookup_host((host.as_str(), port))
            .await
            .map_err(|e| format!("dns resolution failed for {host}: {e}"))?
            .collect(),
    };
    if addrs.is_empty() {
        return Err(format!("host {host} did not resolve"));
    }
    if let Some(bad) = addrs
        .iter()
        .find(|a| is_blocked_ip_allow_local(a.ip(), a.port()))
    {
        return Err(format!(
            "blocked address {} for host {host}: cloud-metadata addresses and Otto's own daemon \
             are always blocked (SSRF guard)",
            bad.ip()
        ));
    }
    Ok((host, addrs))
}

/// Pre-flight allow-local check of a target URL (see
/// [`resolve_checked_allow_local`]); pair it with
/// [`allow_local_client_builder`].
pub async fn check_url_allow_local(url: &str) -> Result<(), String> {
    resolve_checked_allow_local(url).await.map(|_| ())
}

/// The allow-local [`GuardedResolver`]: refuses a name when ANY address is a
/// metadata address, so a rebinding answer can't swap one in at connect time.
/// (The daemon-port rule needs the port, which a resolver never sees — the
/// URL-level checks refuse every hostname on a daemon port instead.)
#[derive(Debug, Clone, Copy, Default)]
pub struct AllowLocalResolver;

impl reqwest::dns::Resolve for AllowLocalResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_string();
        Box::pin(async move {
            let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), 0))
                .await
                .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
                    format!("dns resolution failed for {host}: {e}").into()
                })?
                .collect();
            if let Some(bad) = addrs.iter().find(|a| is_metadata_ip(a.ip())) {
                let msg = format!(
                    "blocked address {} for host {host}: cloud-metadata addresses are always \
                     blocked (SSRF guard)",
                    bad.ip()
                );
                return Err::<reqwest::dns::Addrs, Box<dyn std::error::Error + Send + Sync>>(
                    msg.into(),
                );
            }
            let iter: reqwest::dns::Addrs = Box::new(addrs.into_iter());
            Ok::<reqwest::dns::Addrs, Box<dyn std::error::Error + Send + Sync>>(iter)
        })
    }
}

/// Redirect policy of an allow-local client: caps hops at [`MAX_REDIRECTS`]
/// and refuses a hop [`check_url_literal_allow_local`] refuses.
pub fn allow_local_redirect_policy() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= MAX_REDIRECTS {
            return attempt.error("too many redirects");
        }
        if check_url_literal_allow_local(attempt.url()) {
            attempt.follow()
        } else {
            attempt.stop()
        }
    })
}

/// The allow-local [`guarded_client_builder`]: [`AllowLocalResolver`] plus
/// [`allow_local_redirect_policy`]. Callers run [`precheck_url_allow_local`]
/// first (it vets IP literals and the daemon port; no DNS).
pub fn allow_local_client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .dns_resolver(std::sync::Arc::new(AllowLocalResolver))
        .redirect(allow_local_redirect_policy())
}

/// A client builder for ONE credential-carrying fetch of `url` (an OAuth
/// token endpoint) under the workspace's rule: the host is resolved and
/// vetted ONCE ([`resolve_checked_for`]) and exactly those addresses are
/// pinned, on top of the matching guarded resolver and redirect policy — so
/// neither a second DNS answer nor a redirect hop reaches cloud metadata,
/// Otto's own daemon or (strict mode) a private address. The caller may still
/// replace the redirect policy with a stricter one.
pub async fn pinned_client_builder_for(
    url: &str,
    allow_local: bool,
) -> Result<reqwest::ClientBuilder, String> {
    let (host, addrs) = resolve_checked_for(url, allow_local).await?;
    let builder = if allow_local {
        allow_local_client_builder()
    } else {
        guarded_client_builder()
    };
    // An IP literal is dialled as written (no resolver involved).
    Ok(if host.parse::<IpAddr>().is_ok() {
        builder
    } else {
        builder.resolve_to_addrs(&host, &addrs)
    })
}

// ---------------------------------------------------------------------------
// Tunnelled mode: the request rides an SSH / SOCKS tunnel and the bastion
// resolves and dials. Private addresses behind it are what the tunnel is for,
// but the bastion's own instance credentials must not be one GET away: a
// metadata address literal or a well-known metadata name is still refused. (A
// custom name the bastion resolves to a metadata address can't be seen here.)
// ---------------------------------------------------------------------------

/// Why a tunnelled fetch of `url` is refused, `None` when it may go.
pub fn metadata_refusal(url: &reqwest::Url) -> Option<String> {
    if !scheme_allowed(url.scheme()) {
        return Some(format!("blocked url scheme: {}", url.scheme()));
    }
    let Some(host) = bare_host(url).filter(|h| !h.is_empty()) else {
        return Some("url has no host".to_string());
    };
    let metadata = match host.parse::<IpAddr>() {
        Ok(ip) => is_metadata_ip(ip),
        Err(_) => is_metadata_hostname(host),
    };
    metadata.then(|| {
        format!(
            "blocked address {host}: cloud-metadata addresses are always blocked, through a \
             tunnel too (SSRF guard)"
        )
    })
}

/// The pre-send check of a tunnelled request ([`metadata_refusal`], no DNS).
pub fn precheck_url_tunnelled(url: &str) -> Result<(), String> {
    let parsed = reqwest::Url::parse(url).map_err(|e| format!("invalid url: {e}"))?;
    metadata_refusal(&parsed).map_or(Ok(()), Err)
}

/// Redirect policy of a tunnelled client: caps hops at [`MAX_REDIRECTS`] and
/// refuses a hop [`metadata_refusal`] refuses.
pub fn tunnelled_redirect_policy() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= MAX_REDIRECTS {
            return attempt.error("too many redirects");
        }
        if metadata_refusal(attempt.url()).is_none() {
            attempt.follow()
        } else {
            attempt.stop()
        }
    })
}

/// Which outbound rules a fetch runs under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Guard {
    /// The fetch rides a tunnel whose FAR end resolves and dials (the local
    /// rules would only false-block bastion-only hosts): only cloud metadata
    /// is refused ([`metadata_refusal`]).
    Tunnelled,
    /// The full guard: public addresses only (the default).
    Strict,
    /// Private / LAN / loopback allowed; metadata and the daemon's own
    /// listener still refused.
    AllowLocal,
}

impl Guard {
    /// The guard of a fetch that is (or is not) `tunnelled`, in a workspace
    /// that has (or has not) opted in to private addresses.
    pub fn for_fetch(tunnelled: bool, allow_local: bool) -> Self {
        match (tunnelled, allow_local) {
            (true, _) => Guard::Tunnelled,
            (false, false) => Guard::Strict,
            (false, true) => Guard::AllowLocal,
        }
    }

    /// The DNS-free per-hop check of this guard (redirect targets).
    pub fn hop_allowed(self, url: &reqwest::Url) -> bool {
        match self {
            Guard::Tunnelled => metadata_refusal(url).is_none(),
            Guard::Strict => check_url_literal(url),
            Guard::AllowLocal => check_url_literal_allow_local(url),
        }
    }
}

/// [`resolve_checked`], or [`resolve_checked_allow_local`] in a workspace that
/// allows private addresses — one call for the callers that dial themselves.
pub async fn resolve_checked_for(
    url: &str,
    allow_local: bool,
) -> Result<(String, Vec<SocketAddr>), String> {
    if allow_local {
        resolve_checked_allow_local(url).await
    } else {
        resolve_checked(url).await
    }
}

/// [`check_url`], or [`check_url_allow_local`] when private addresses are allowed.
pub async fn check_url_for(url: &str, allow_local: bool) -> Result<(), String> {
    resolve_checked_for(url, allow_local).await.map(|_| ())
}

/// [`is_blocked_ip`], or [`is_blocked_ip_allow_local`] for a dial of `ip:port`
/// when private addresses are allowed (metadata and the daemon stay blocked).
pub fn is_blocked_ip_for(ip: IpAddr, port: u16, allow_local: bool) -> bool {
    if allow_local {
        is_blocked_ip_allow_local(ip, port)
    } else {
        is_blocked_ip(ip)
    }
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
            "64:ff9b::a9fe:a9fe",       // NAT64 → 169.254.169.254
            "64:ff9b::7f00:1",          // NAT64 → 127.0.0.1
            "64:ff9b:1::1",             // local-use NAT64, blocked outright
            "2002:7f00:1::",            // 6to4 → 127.0.0.1
            "2002:a9fe:a9fe::1",        // 6to4 → 169.254.169.254
            "2002:c0a8:101::",          // 6to4 → 192.168.1.1
            "::127.0.0.1",              // IPv4-compatible → 127.0.0.1
            "::a9fe:a9fe",              // IPv4-compatible → 169.254.169.254
            "2001:0:1:2:3:4:80ff:fffe", // Teredo client ^0xffffffff → 127.0.0.1
            "fec0::1",                  // site-local
            "fe80::1",                  // link-local
            "fd00::1",                  // ULA
            "2001:db8::1",              // documentation
        ] {
            let v6: Ipv6Addr = ip.parse().unwrap();
            assert!(is_blocked_ip(IpAddr::V6(v6)), "{ip} must be blocked");
        }
        // Transition forms wrapping a PUBLIC v4 stay reachable.
        for ip in [
            "64:ff9b::808:808",
            "2002:808:808::1",
            "2606:4700:4700::1111",
        ] {
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
    fn explicit_local_targets_are_literal_loopback_private_or_cgnat_only() {
        let local = |u: &str| is_explicit_local_target(&reqwest::Url::parse(u).unwrap());
        for u in [
            "http://127.0.0.1:3845/mcp",
            "http://localhost:8080/",
            "http://LOCALHOST/",
            "http://[::1]:9000/mcp",
            "http://10.0.0.5/",
            "https://192.168.1.20:8443/",
            "http://172.16.4.4/",
            "http://100.101.102.103/", // Tailscale
            "http://[fd00::1]/",
            "http://[::ffff:127.0.0.1]/",
        ] {
            assert!(local(u), "{u} is an explicit local target");
        }
        for u in [
            "http://169.254.169.254/latest/meta-data/", // metadata stays out
            "http://[fe80::1]/",
            "http://0.0.0.0/",
            "http://224.0.0.1/",
            "http://255.255.255.255/",
            "https://example.com/",
            "https://8.8.8.8/",
            "http://internal.corp/", // a NAME: still vetted at connect time
            "http://localhost.evil.example/",
            "file:///etc/passwd",
        ] {
            assert!(!local(u), "{u} is not an explicit local target");
        }
    }

    #[test]
    fn metadata_addresses_are_classified_in_every_form() {
        for ip in [
            "169.254.169.254",
            "169.254.170.2",
            "169.254.0.1",
            "100.100.100.200",
            "192.0.0.192",
            "::ffff:169.254.169.254",
            "64:ff9b::a9fe:a9fe",
            "2002:a9fe:a9fe::1",
            "fe80::1",
            "fd00:ec2::254",
        ] {
            assert!(is_metadata_ip(ip.parse().unwrap()), "{ip} is metadata");
        }
        for ip in [
            "127.0.0.1",
            "10.0.0.5",
            "192.168.1.1",
            "100.64.0.1",
            "fd00::1",
        ] {
            assert!(!is_metadata_ip(ip.parse().unwrap()), "{ip} is not metadata");
        }
    }

    #[test]
    fn allow_local_keeps_metadata_and_the_daemon_port_blocked() {
        let lit = |u: &str| check_url_literal_allow_local(&reqwest::Url::parse(u).unwrap());
        // Private / LAN / loopback dev targets are reachable.
        for u in [
            "http://127.0.0.1:3000/",
            "http://localhost:8080/",
            "http://[::1]:9000/",
            "http://10.0.0.5/",
            "https://192.168.1.20:8443/",
            "http://100.101.102.103/",
            "http://internal.corp/",
            "https://8.8.8.8:7700/", // a PUBLIC literal on 7700 is not the daemon
        ] {
            assert!(lit(u), "{u} is allowed with private addresses on");
        }
        // Metadata, any form, and the daemon's own listener stay refused.
        for u in [
            "http://169.254.169.254/latest/meta-data/",
            "http://[::ffff:169.254.169.254]/",
            "http://[fd00:ec2::254]/",
            "http://100.100.100.200/",
            "http://127.0.0.1:7700/sessions",
            "http://localhost:7700/",
            "http://[::1]:7700/",
            "http://0.0.0.0:7700/",
            "http://192.168.1.20:7700/",
            "http://rebind.example:7700/", // a hostname on the daemon port
            "file:///etc/passwd",
        ] {
            assert!(!lit(u), "{u} stays blocked with private addresses on");
        }
        assert!(is_blocked_ip_allow_local(
            "127.0.0.1".parse().unwrap(),
            7700
        ));
        assert!(!is_blocked_ip_allow_local(
            "127.0.0.1".parse().unwrap(),
            7701
        ));
        assert!(is_blocked_ip_allow_local(
            "169.254.169.254".parse().unwrap(),
            80
        ));
    }

    #[test]
    fn registered_daemon_ports_are_refused_too() {
        let lit = |u: &str| check_url_literal_allow_local(&reqwest::Url::parse(u).unwrap());
        assert!(lit("http://127.0.0.1:47123/"));
        register_daemon_port(47123);
        assert!(is_daemon_port(47123));
        assert!(!lit("http://127.0.0.1:47123/"));
    }

    #[tokio::test]
    async fn allow_local_preflight_says_why_and_lets_dev_servers_through() {
        let err = check_url_allow_local("http://169.254.169.254/")
            .await
            .unwrap_err();
        assert!(
            err.contains("always blocked") && err.contains("SSRF guard"),
            "{err}"
        );
        let err = check_url_allow_local("http://127.0.0.1:7700/sessions")
            .await
            .unwrap_err();
        assert!(err.contains("daemon"), "{err}");
        assert!(check_url_allow_local("http://127.0.0.1:3000/")
            .await
            .is_ok());
        let (_, addrs) = resolve_checked_allow_local("http://10.1.2.3:8080/x")
            .await
            .unwrap();
        assert_eq!(addrs, vec!["10.1.2.3:8080".parse::<SocketAddr>().unwrap()]);
    }

    #[tokio::test]
    async fn the_for_helpers_pick_the_workspace_rule() {
        let lo: IpAddr = "127.0.0.1".parse().unwrap();
        assert!(is_blocked_ip_for(lo, 3000, false));
        assert!(!is_blocked_ip_for(lo, 3000, true));
        assert!(is_blocked_ip_for(lo, DEFAULT_DAEMON_PORT, true));
        assert!(is_blocked_ip_for(
            "169.254.169.254".parse().unwrap(),
            80,
            true
        ));
        assert!(check_url_for("http://127.0.0.1:3000/", false)
            .await
            .is_err());
        assert!(check_url_for("http://127.0.0.1:3000/", true).await.is_ok());
        let err = check_url_for("http://169.254.169.254/", true)
            .await
            .expect_err("metadata");
        assert!(err.contains("always blocked"), "{err}");
    }

    #[test]
    fn allow_local_precheck_needs_no_dns() {
        // An unresolvable name passes: the connect-time resolver vets it.
        assert!(precheck_url_allow_local("https://dup.example.invalid/x").is_ok());
        assert!(precheck_url_allow_local("http://127.0.0.1:3000/").is_ok());
        // Literals and the daemon port are refused here, with the reason.
        let err = precheck_url_allow_local("http://169.254.169.254/").unwrap_err();
        assert!(err.contains("always blocked"), "{err}");
        let err = precheck_url_allow_local("http://dup.example.invalid:7700/").unwrap_err();
        assert!(err.contains("daemon"), "{err}");
        assert!(precheck_url_allow_local("file:///etc/passwd").is_err());
        assert!(precheck_url_allow_local("not a url").is_err());
    }

    #[test]
    fn guard_modes_pick_the_matching_hop_check() {
        let url = |u: &str| reqwest::Url::parse(u).unwrap();
        assert_eq!(Guard::for_fetch(true, false), Guard::Tunnelled);
        assert_eq!(Guard::for_fetch(false, false), Guard::Strict);
        assert_eq!(Guard::for_fetch(false, true), Guard::AllowLocal);
        assert!(Guard::Tunnelled.hop_allowed(&url("http://10.0.0.1/")));
        assert!(!Guard::Strict.hop_allowed(&url("http://10.0.0.1/")));
        assert!(Guard::AllowLocal.hop_allowed(&url("http://10.0.0.1/")));
        assert!(!Guard::AllowLocal.hop_allowed(&url("http://169.254.169.254/")));
    }

    /// F11: through a tunnel, private targets stay reachable but cloud
    /// metadata — by literal or by well-known name — does not, per hop too.
    #[test]
    fn tunnelled_fetches_still_refuse_cloud_metadata() {
        let url = |u: &str| reqwest::Url::parse(u).unwrap();
        for u in [
            "http://169.254.169.254/latest/meta-data/iam/",
            "http://[fd00:ec2::254]/latest/",
            "http://100.100.100.200/latest/meta-data/",
            "http://[::ffff:169.254.169.254]/",
            "http://metadata.google.internal/computeMetadata/v1/",
            "http://METADATA.google.internal./x",
            "http://metadata/computeMetadata/v1/",
            "http://instance-data/latest/",
            "file:///etc/passwd",
        ] {
            assert!(precheck_url_tunnelled(u).is_err(), "{u} must be refused");
            assert!(!Guard::Tunnelled.hop_allowed(&url(u)), "{u} hop refused");
        }
        let err = precheck_url_tunnelled("http://169.254.169.254/").unwrap_err();
        assert!(err.contains("always blocked"), "{err}");
        for u in [
            "http://10.0.0.5:8080/api",
            "http://127.0.0.1:7700/", // the BASTION's loopback, not this Mac's
            "http://internal.corp/",
            "https://example.com/",
        ] {
            assert!(precheck_url_tunnelled(u).is_ok(), "{u} must stay reachable");
            assert!(Guard::Tunnelled.hop_allowed(&url(u)), "{u} hop allowed");
        }
    }

    /// F6: the pinned builder of a credentialed fetch refuses metadata and
    /// the daemon's port with private addresses allowed, and every private
    /// target without.
    #[tokio::test]
    async fn pinned_builder_refuses_metadata_and_the_daemon_in_both_modes() {
        for allow_local in [false, true] {
            for u in [
                "http://169.254.169.254/oauth/token",
                "http://[fd00:ec2::254]/token",
                "http://127.0.0.1:7700/api/v1/auth/tokens",
                "http://localhost:7700/token",
            ] {
                let err = pinned_client_builder_for(u, allow_local)
                    .await
                    .expect_err("refused");
                assert!(err.contains("SSRF guard"), "{u} ({allow_local}): {err}");
            }
        }
        assert!(
            pinned_client_builder_for("http://127.0.0.1:3000/token", false)
                .await
                .is_err()
        );
        let builder = pinned_client_builder_for("http://127.0.0.1:3000/token", true)
            .await
            .expect("a dev Keycloak on localhost is allowed");
        assert!(builder.build().is_ok());
        assert!(pinned_client_builder_for("file:///etc/passwd", true)
            .await
            .is_err());
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
