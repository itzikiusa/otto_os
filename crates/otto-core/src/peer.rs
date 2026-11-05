//! Who is on the other end of a request, as far as the network can tell —
//! the input to the phone-access rule "a device token works only from the
//! tailnet or this Mac" (`otto_phone`, `otto_rbac::devices`).
//!
//! `tailscale serve` is a reverse proxy inside `tailscaled`: every tailnet
//! request reaches the daemon from `127.0.0.1` with the `Host` the phone used
//! (`<mac>.<tailnet>.ts.net`) and `X-Forwarded-For` SET (not appended) to the
//! tailnet source address; a Funnel (public) request also carries
//! `Tailscale-Funnel-Request`. So a loopback peer is the tailnet only when all
//! of: a `*.ts.net` Host, no Funnel marker, and a forwarded address inside the
//! Tailscale ranges. A direct peer (the optional network listener) is judged
//! by its own address. A process on this Mac can forge those headers — it is
//! on this Mac, which is what the rule admits anyway.

use std::net::IpAddr;

/// Where a request came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerKind {
    /// This Mac: a loopback peer that addressed the daemon by a loopback name
    /// (the desktop app, a local tool).
    Local,
    /// A device on the owner's tailnet (directly, or via `tailscale serve`).
    Tailnet,
    /// Anything else: the LAN, a Cloudflare tunnel, Tailscale Funnel, …
    Other,
}

/// The classified peer of the current request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestPeer {
    /// The best-known client address (the forwarded tailnet address behind
    /// `tailscale serve`, else the socket peer).
    pub ip: IpAddr,
    pub kind: PeerKind,
}

impl RequestPeer {
    /// The tailnet or this Mac — where a device token may be used from.
    pub fn trusted_for_devices(&self) -> bool {
        matches!(self.kind, PeerKind::Local | PeerKind::Tailnet)
    }
}

/// `100.64.0.0/10` (Tailscale's CGNAT range) or `fd7a:115c:a1e0::/48`.
pub fn is_tailnet_ip(ip: IpAddr) -> bool {
    match ip.to_canonical() {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            o[0] == 100 && (o[1] & 0xC0) == 64
        }
        IpAddr::V6(v6) => {
            let s = v6.segments();
            s[0] == 0xfd7a && s[1] == 0x115c && s[2] == 0xa1e0
        }
    }
}

fn is_loopback_host(host: &str) -> bool {
    let h = host.trim_start_matches('[').trim_end_matches(']');
    h == "localhost"
        || h.ends_with(".localhost")
        || h.parse::<IpAddr>()
            .map(|ip| ip.is_loopback())
            .unwrap_or(false)
}

/// Classify a request. `host` is the port-stripped, lower-cased `Host`;
/// `forwarded_for` the raw `X-Forwarded-For`; `funnel` whether a
/// `Tailscale-Funnel-Request` header is present.
pub fn classify_peer(
    peer: IpAddr,
    host: Option<&str>,
    forwarded_for: Option<&str>,
    funnel: bool,
) -> RequestPeer {
    let peer = peer.to_canonical();
    if is_tailnet_ip(peer) {
        return RequestPeer {
            ip: peer,
            kind: PeerKind::Tailnet,
        };
    }
    if !peer.is_loopback() {
        return RequestPeer {
            ip: peer,
            kind: PeerKind::Other,
        };
    }
    let host = host.unwrap_or("");
    if host.is_empty() || is_loopback_host(host) {
        return RequestPeer {
            ip: peer,
            kind: PeerKind::Local,
        };
    }
    // The LAST entry is the one the nearest proxy (tailscaled) wrote.
    let fwd = forwarded_for
        .and_then(|v| v.rsplit(',').next())
        .and_then(|v| v.trim().parse::<IpAddr>().ok())
        .map(|ip| ip.to_canonical());
    match fwd {
        Some(ip) if host.ends_with(".ts.net") && !funnel && is_tailnet_ip(ip) => RequestPeer {
            ip,
            kind: PeerKind::Tailnet,
        },
        Some(ip) => RequestPeer {
            ip,
            kind: PeerKind::Other,
        },
        None => RequestPeer {
            ip: peer,
            kind: PeerKind::Other,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn tailnet_ranges() {
        assert!(is_tailnet_ip(ip("100.64.0.1")));
        assert!(is_tailnet_ip(ip("100.111.7.125")));
        assert!(is_tailnet_ip(ip("100.127.255.255")));
        assert!(!is_tailnet_ip(ip("100.128.0.1")));
        assert!(!is_tailnet_ip(ip("100.63.255.255")));
        assert!(!is_tailnet_ip(ip("192.168.1.2")));
        assert!(is_tailnet_ip(ip("fd7a:115c:a1e0::932:77e")));
        assert!(!is_tailnet_ip(ip("fd7a:115c:a1e1::1")));
        assert!(is_tailnet_ip(ip("::ffff:100.100.1.1")));
    }

    #[test]
    fn serve_proxied_tailnet_request_is_tailnet() {
        let p = classify_peer(
            ip("127.0.0.1"),
            Some("mac.tail0b7b6b.ts.net"),
            Some("100.111.7.125"),
            false,
        );
        assert_eq!(p.kind, PeerKind::Tailnet);
        assert_eq!(p.ip, ip("100.111.7.125"));
    }

    #[test]
    fn funnel_lan_tunnel_and_spoofed_hosts_are_not_tailnet() {
        // Funnel: public client, even with a tailnet-looking forwarded value.
        let f = classify_peer(
            ip("127.0.0.1"),
            Some("mac.x.ts.net"),
            Some("100.1.2.3"),
            true,
        );
        assert_eq!(f.kind, PeerKind::Other);
        // A Cloudflare tunnel: loopback peer, foreign host.
        let cf = classify_peer(
            ip("127.0.0.1"),
            Some("otto.example.com"),
            Some("100.64.0.9"),
            false,
        );
        assert_eq!(cf.kind, PeerKind::Other);
        // A ts.net host with a public forwarded address.
        let pubf = classify_peer(
            ip("127.0.0.1"),
            Some("mac.x.ts.net"),
            Some("8.8.8.8"),
            false,
        );
        assert_eq!(pubf.kind, PeerKind::Other);
        // A ts.net host with no forwarded address at all.
        let none = classify_peer(ip("127.0.0.1"), Some("mac.x.ts.net"), None, false);
        assert_eq!(none.kind, PeerKind::Other);
        // A LAN peer.
        let lan = classify_peer(ip("192.168.1.9"), Some("192.168.1.2"), None, false);
        assert_eq!(lan.kind, PeerKind::Other);
        assert!(!lan.trusted_for_devices());
    }

    #[test]
    fn last_forwarded_entry_wins() {
        // A client-sent value first, the proxy's own last.
        let p = classify_peer(
            ip("127.0.0.1"),
            Some("mac.x.ts.net"),
            Some("100.64.0.1, 8.8.8.8"),
            false,
        );
        assert_eq!(p.kind, PeerKind::Other);
    }

    #[test]
    fn loopback_names_and_direct_tailnet_peers() {
        for h in [None, Some("localhost"), Some("127.0.0.1"), Some("[::1]")] {
            let p = classify_peer(ip("127.0.0.1"), h, None, false);
            assert_eq!(p.kind, PeerKind::Local, "{h:?}");
            assert!(p.trusted_for_devices());
        }
        let direct = classify_peer(ip("100.100.3.4"), Some("100.120.1.1"), None, false);
        assert_eq!(direct.kind, PeerKind::Tailnet);
    }
}
