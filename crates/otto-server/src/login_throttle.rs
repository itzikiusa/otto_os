//! Login brute-force throttle / lockout (audit S5).
//!
//! `POST /auth/login` is the one unauthenticated, password-checking endpoint, so
//! it is the brute-force surface. We keep a small in-memory tally of recent
//! failures and lock a key out for [`LOCKOUT_DURATION`] (the handler returns
//! 429) once it crosses [`FAILURE_THRESHOLD`] failures inside [`FAILURE_WINDOW`].
//! State is per-process and resets on restart — adequate for a single-node
//! daemon and intentionally light (no new crate, no DB writes on the hot path).
//!
//! Two keys are tracked for every attempt and EITHER can lock the request:
//!   * [`ip_key`] — `"<ip>|<username>"`, the per-client tally; and
//!   * [`username_key`] — `"user:<username>"`, a GLOBAL per-username tally.
//!
//! The per-username tally is what closes the original bypass. The client IP is
//! taken from the real socket peer (`ConnectInfo<SocketAddr>`); Tailscale and the
//! Tauri shell connect directly with no trusted proxy in front, so the handler
//! must NOT honor `X-Forwarded-For` / `X-Real-IP` (an attacker would just rotate
//! them to mint a fresh `ip|username` key per request and never trip the
//! lockout). Even with a genuinely rotating source IP, the global per-username
//! counter still trips after [`FAILURE_THRESHOLD`] failures against any one
//! account. See `tests/auth_security.rs` for the property test.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Failed attempts (per key) tolerated before lockout kicks in.
pub const FAILURE_THRESHOLD: u32 = 5;
/// Failures older than this are forgotten (sliding window).
pub const FAILURE_WINDOW: Duration = Duration::from_secs(15 * 60);
/// How long a key stays locked once the threshold is crossed.
pub const LOCKOUT_DURATION: Duration = Duration::from_secs(15 * 60);
/// Cap the map so a flood of distinct keys can't grow memory unbounded; oldest
/// expired entries are pruned first, then we stop tracking new keys.
const MAX_TRACKED_KEYS: usize = 10_000;

/// One key's recent failure history.
#[derive(Default)]
struct Attempts {
    /// Timestamps of failures still inside the window.
    failures: Vec<Instant>,
    /// When set and in the future, the key is locked until this instant.
    locked_until: Option<Instant>,
}

/// In-memory failed-attempt tally with sliding-window lockout. The production
/// instance is a process-global [`global`]; tests build their own with
/// [`AttemptStore::default`] so they don't share (and contaminate) that state.
#[derive(Default)]
pub struct AttemptStore {
    inner: Mutex<HashMap<String, Attempts>>,
}

impl AttemptStore {
    /// If `key` is currently locked out, return the remaining lock duration.
    pub fn check_locked(&self, key: &str) -> Option<Duration> {
        let mut store = self.inner.lock().unwrap();
        let now = Instant::now();
        if let Some(entry) = store.get_mut(key) {
            match entry.locked_until {
                Some(until) if until > now => return Some(until - now),
                Some(_) => {
                    // Lock expired: clear it and the stale failure list.
                    entry.locked_until = None;
                    entry.failures.clear();
                }
                None => {}
            }
        }
        None
    }

    /// Record one failed attempt for `key`; lock the key once it crosses the
    /// threshold inside the window.
    pub fn record_failure(&self, key: &str) {
        let mut store = self.inner.lock().unwrap();
        let now = Instant::now();
        prune_expired(&mut store, now);
        if store.len() >= MAX_TRACKED_KEYS && !store.contains_key(key) {
            // Map is full of live entries; skip tracking rather than grow.
            return;
        }
        let entry = store.entry(key.to_string()).or_default();
        entry.failures.retain(|t| now.duration_since(*t) < FAILURE_WINDOW);
        entry.failures.push(now);
        if entry.failures.len() as u32 >= FAILURE_THRESHOLD {
            entry.locked_until = Some(now + LOCKOUT_DURATION);
        }
    }

    /// Clear a key's failure history (called on a successful login).
    pub fn clear(&self, key: &str) {
        self.inner.lock().unwrap().remove(key);
    }

    /// Longest remaining lock across `keys`, if any is locked. Used by the
    /// handler to gate on EITHER the per-client or the per-username key.
    pub fn max_locked(&self, keys: &[&str]) -> Option<Duration> {
        keys.iter().filter_map(|k| self.check_locked(k)).max()
    }
}

/// Process-global attempt store used by the live `login` handler.
pub fn global() -> &'static AttemptStore {
    static STORE: OnceLock<AttemptStore> = OnceLock::new();
    STORE.get_or_init(AttemptStore::default)
}

/// Per-client throttle key: real socket-peer IP + the username being tried
/// (lowercased so casing can't be used to dodge the tally). A `None` IP — which
/// should not happen once connect-info is wired in — collapses to the global
/// per-username key so an attempt is never left untracked.
pub fn ip_key(peer: Option<IpAddr>, username: &str) -> String {
    match peer {
        Some(ip) => format!("{ip}|{}", username.trim().to_lowercase()),
        None => username_key(username),
    }
}

/// Global per-username throttle key. Independent of the source IP, so rotating
/// the IP (or a spoofed forwarding header, which we don't honor anyway) can't
/// reset it — this is the anti-rotation guarantee.
pub fn username_key(username: &str) -> String {
    format!("user:{}", username.trim().to_lowercase())
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

    fn ip(a: u8, b: u8, c: u8, d: u8) -> Option<IpAddr> {
        Some(IpAddr::V4(Ipv4Addr::new(a, b, c, d)))
    }

    #[test]
    fn locks_after_threshold_then_clears_on_success() {
        let store = AttemptStore::default();
        let key = ip_key(ip(10, 0, 0, 1), "alice");

        for _ in 0..FAILURE_THRESHOLD - 1 {
            store.record_failure(&key);
            assert!(store.check_locked(&key).is_none(), "must not lock early");
        }
        // The threshold-crossing failure locks the key.
        store.record_failure(&key);
        assert!(store.check_locked(&key).is_some(), "must lock at threshold");

        // A successful login clears the key (legit-user happy path).
        store.clear(&key);
        assert!(store.check_locked(&key).is_none(), "success must reset");
    }

    #[test]
    fn username_lockout_survives_ip_rotation() {
        // The core S5 property: an attacker rotating the source IP every request
        // mints a fresh per-client key each time, so no `ip|username` key ever
        // trips — but the global per-username key still locks the account.
        let store = AttemptStore::default();
        let uname = "victim";
        let user_key = username_key(uname);

        for i in 0..FAILURE_THRESHOLD {
            let rotating = ip_key(ip(203, 0, 113, i as u8), uname);
            // No single per-client key reaches the threshold.
            store.record_failure(&rotating);
            assert!(
                store.check_locked(&rotating).is_none(),
                "per-client key must never lock under rotation"
            );
            // But every attempt also tallies against the username.
            store.record_failure(&user_key);
        }

        assert!(
            store.check_locked(&user_key).is_some(),
            "username key must lock despite IP rotation"
        );
    }
}
