//! Share-redemption brute-force throttle / lockout (mobile plan Task 1.8).
//!
//! `/ws/term/{session_id}` (when accessed via a scoped share-link token) is the
//! public share-redemption surface — unlike `/auth/login` it requires no
//! password, but an attacker can still hammer it with guessed tokens. We mirror
//! [`crate::ws`]'s sibling structure from `otto-server`'s `login_throttle`:
//!
//! * Key: **real socket-peer IP** only (no username axis — share tokens are
//!   random 32-byte handles, not usernames, so a per-username tally is not
//!   meaningful). The peer IP comes from `ConnectInfo<SocketAddr>`, which is
//!   always the real wire address (never a spoofable forwarding header).
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
        entry.failures.retain(|t| now.duration_since(*t) < FAILURE_WINDOW);
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
        assert!(
            store.check(addr).is_ok(),
            "clear() must reset the lockout"
        );
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
