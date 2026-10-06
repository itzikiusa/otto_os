//! Share-redemption brute-force throttle / lockout (mobile plan Task 1.8).
//!
//! `/ws/term/{session_id}` (when accessed via a scoped share-link token) is the
//! public share-redemption surface — unlike `/auth/login` it requires no
//! password, but an attacker can still hammer it with guessed tokens. We mirror
//! [`crate::ws`]'s sibling structure from `otto-server`'s `login_throttle`:
//!
//! * Key: the **client IP** only (no username axis — share tokens are random
//!   32-byte handles, not usernames, so a per-username tally is not
//!   meaningful). That is the socket peer, except behind the documented
//!   Cloudflare tunnel, where every client arrives as `127.0.0.1` and the
//!   `CF-Connecting-IP` header is used instead ([`resolve_client_ip`]: only for
//!   a loopback peer that named the tunnel host — never a spoofable header from
//!   a direct client).
//! * The `/ws/term` gate authenticates FIRST and never refuses a token that
//!   verifies; only failures are counted and only failures are refused while
//!   locked (S8-02). The OTP routes (`/share/verify`, `/share/extend`) still
//!   check the lock up front: a 6-digit code IS guessable.
//! * Threshold: [`FAILURE_THRESHOLD`] failures inside [`FAILURE_WINDOW`] →
//!   lockout for [`LOCKOUT_DURATION`].
//! * Map size is capped at [`MAX_TRACKED_KEYS`]; a flood of distinct IPs is
//!   pruned aggressively before we drop new entries.
//! * State is in-memory and per-process; it resets on daemon restart.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Failed token attempts (per IP) tolerated before lockout kicks in.
pub const FAILURE_THRESHOLD: u32 = 10;
/// Failures older than this are forgotten (sliding window).
pub const FAILURE_WINDOW: Duration = Duration::from_secs(15 * 60);
/// How long an IP stays locked once the threshold is crossed.
pub const LOCKOUT_DURATION: Duration = Duration::from_secs(15 * 60);
/// Cap the map so a flood of distinct IPs can't grow memory unbounded; oldest
/// expired entries are pruned first, then we stop tracking new keys.
const MAX_TRACKED_KEYS: usize = 10_000;

/// One IP's recent failure history.
#[derive(Default)]
struct Attempts {
    /// Timestamps of failures still inside the window.
    failures: Vec<Instant>,
    /// When set and in the future, the IP is locked until this instant.
    locked_until: Option<Instant>,
}

/// In-memory failed-attempt tally with sliding-window lockout. The production
/// instance is the process-global [`global`]; tests build their own with
/// [`ShareThrottle::default`] so they don't contaminate that state.
#[derive(Default)]
pub struct ShareThrottle {
    inner: Mutex<HashMap<String, Attempts>>,
}

impl ShareThrottle {
    /// If `ip` is currently locked out, return the remaining lock duration.
    pub fn check(&self, ip: IpAddr) -> Result<(), LockedOut> {
        let key = ip_key(ip);
        let mut store = self.inner.lock().unwrap();
        let now = Instant::now();
        if let Some(entry) = store.get_mut(&key) {
            match entry.locked_until {
                Some(until) if until > now => {
                    return Err(LockedOut {
                        retry_after: until - now,
                    });
                }
                Some(_) => {
                    // Lock expired: clear it and stale failures.
                    entry.locked_until = None;
                    entry.failures.clear();
                }
                None => {}
            }
        }
        Ok(())
    }

    /// Record one failed token attempt for `ip`; lock the IP once it crosses
    /// the threshold inside the window.
    pub fn record_failure(&self, ip: IpAddr) {
        let key = ip_key(ip);
        let mut store = self.inner.lock().unwrap();
        let now = Instant::now();
        prune_expired(&mut store, now);
        if store.len() >= MAX_TRACKED_KEYS && !store.contains_key(&key) {
            // Map is full of live entries; skip tracking rather than grow.
            return;
        }
        let entry = store.entry(key).or_default();
        entry
            .failures
            .retain(|t| now.duration_since(*t) < FAILURE_WINDOW);
        entry.failures.push(now);
        if entry.failures.len() as u32 >= FAILURE_THRESHOLD {
            entry.locked_until = Some(now + LOCKOUT_DURATION);
        }
    }

    /// Clear an IP's failure history (called on a successful auth).
    pub fn clear(&self, ip: IpAddr) {
        self.inner.lock().unwrap().remove(&ip_key(ip));
    }
}

/// The IP is currently locked out.
#[derive(Debug)]
pub struct LockedOut {
    /// How long until the lockout expires.
    pub retry_after: Duration,
}

/// The caller's client address as the daemon's `Host` guard resolved it
/// (`otto-server` `host_guard`), inserted as a request extension on every
/// request it lets through. Throttles and audit rows key on [`ClientIp::ip`]
/// instead of the raw socket peer (S8-02 / S8-07).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClientIp {
    /// Tunnel-aware client IP: the `CF-Connecting-IP` of a request that came
    /// through the documented Cloudflare tunnel, else the socket peer.
    pub ip: IpAddr,
    /// A loopback peer that addressed the daemon by a loopback name (or sent
    /// no `Host`): the desktop app / a local tool, never a tunnelled client.
    pub local: bool,
}

/// Resolve the tunnel-aware client IP (pure; the host guard feeds it).
///
/// The documented remote setup is a Cloudflare tunnel (`cloudflared` →
/// `http://127.0.0.1:7700`), so EVERY internet client reaches the listener as
/// `127.0.0.1` — keying a lockout on the peer made one anonymous visitor lock
/// out every guest and the desktop app at once. `CF-Connecting-IP` is trusted
/// ONLY when both hold: the peer is loopback (cloudflared runs on this Mac) and
/// the request named the tunnel host (`share_base_url`) — a LAN/tailnet peer
/// or a loopback request for `127.0.0.1` can never pick its own key with it.
pub fn resolve_client_ip(
    peer: IpAddr,
    via_tunnel_host: bool,
    cf_connecting_ip: Option<&str>,
) -> IpAddr {
    if peer.is_loopback() && via_tunnel_host {
        if let Some(ip) = cf_connecting_ip.and_then(|v| v.trim().parse::<IpAddr>().ok()) {
            return ip;
        }
    }
    peer
}

/// The client IP of a request: the guard-resolved [`ClientIp`] when present,
/// else the raw `ConnectInfo` socket peer (routers mounted without the guard).
pub fn client_ip(ext: &axum::http::Extensions) -> Option<IpAddr> {
    ext.get::<ClientIp>().map(|c| c.ip).or_else(|| {
        ext.get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
            .map(|ci| ci.0.ip())
    })
}

/// Process-global share throttle used by the live [`ws_auth_gate`].
pub fn global() -> &'static ShareThrottle {
    static STORE: OnceLock<ShareThrottle> = OnceLock::new();
    STORE.get_or_init(ShareThrottle::default)
}

/// Per-IP throttle key: real socket-peer IP. A `None` IP (should not happen
/// with `into_make_service_with_connect_info`) becomes a sentinel so attempts
/// are never left untracked.
pub fn ip_key(ip: IpAddr) -> String {
    ip.to_string()
}

/// Drop keys whose window and lock have both elapsed, to keep the map bounded.
fn prune_expired(store: &mut HashMap<String, Attempts>, now: Instant) {
    store.retain(|_, e| {
        let locked = matches!(e.locked_until, Some(until) if until > now);
        let has_recent = e
            .failures
            .iter()
            .any(|t| now.duration_since(*t) < FAILURE_WINDOW);
        locked || has_recent
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn ip(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(a, b, c, d))
    }

    #[test]
    fn locks_after_threshold_then_clears_on_success() {
        let store = ShareThrottle::default();
        let addr = ip(10, 0, 0, 1);

        // FAILURE_THRESHOLD - 1 attempts must not lock.
        for _ in 0..FAILURE_THRESHOLD - 1 {
            store.record_failure(addr);
            assert!(store.check(addr).is_ok(), "must not lock before threshold");
        }

        // The threshold-crossing failure locks the IP.
        store.record_failure(addr);
        assert!(
            store.check(addr).is_err(),
            "must lock at/after threshold ({FAILURE_THRESHOLD})"
        );

        // A successful redemption clears the lockout.
        store.clear(addr);
        assert!(store.check(addr).is_ok(), "clear() must reset the lockout");
    }

    #[test]
    fn cf_connecting_ip_is_trusted_only_via_the_tunnel_host_on_loopback() {
        let lo = ip(127, 0, 0, 1);
        let lan = ip(192, 168, 1, 9);
        // Tunnelled: loopback peer + tunnel host → the forwarded client.
        assert_eq!(
            resolve_client_ip(lo, true, Some("203.0.113.7")),
            ip(203, 0, 113, 7)
        );
        // Desktop (loopback Host): the header is ignored.
        assert_eq!(resolve_client_ip(lo, false, Some("203.0.113.7")), lo);
        // A non-loopback peer can never choose its key with the header.
        assert_eq!(resolve_client_ip(lan, true, Some("203.0.113.7")), lan);
        // Garbage / missing header falls back to the peer.
        assert_eq!(resolve_client_ip(lo, true, Some("nope")), lo);
        assert_eq!(resolve_client_ip(lo, true, None), lo);
    }

    #[test]
    fn different_ips_are_independent() {
        let store = ShareThrottle::default();
        let attacker = ip(203, 0, 113, 1);
        let victim = ip(198, 51, 100, 1);

        // Lock out the attacker.
        for _ in 0..FAILURE_THRESHOLD {
            store.record_failure(attacker);
        }
        assert!(store.check(attacker).is_err(), "attacker must be locked");

        // Victim (different IP) is not affected.
        assert!(
            store.check(victim).is_ok(),
            "a different IP must not be locked"
        );
    }

    #[test]
    fn lockout_carries_retry_after() {
        let store = ShareThrottle::default();
        let addr = ip(10, 0, 0, 2);

        for _ in 0..FAILURE_THRESHOLD {
            store.record_failure(addr);
        }

        match store.check(addr) {
            Err(LockedOut { retry_after }) => {
                assert!(
                    retry_after <= LOCKOUT_DURATION,
                    "retry_after must be ≤ LOCKOUT_DURATION"
                );
                assert!(
                    retry_after > Duration::from_secs(0),
                    "retry_after must be positive"
                );
            }
            Ok(()) => panic!("expected LockedOut"),
        }
    }
}
