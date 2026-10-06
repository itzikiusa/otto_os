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
//! the host guard's tunnel-aware `ClientIp`: the real socket peer, except for a
//! loopback peer that named the Cloudflare-tunnel host (`share_base_url`, the
//! recommended remote setup — every tunnelled client arrives as 127.0.0.1),
//! where `CF-Connecting-IP` identifies the client. `X-Forwarded-For` /
//! `X-Real-IP` are never honoured (an attacker would just rotate them to mint a
//! fresh `ip|username` key per request and never trip the lockout). Even with a
//! genuinely rotating source IP, the global per-username counter still trips
//! after [`FAILURE_THRESHOLD`] failures against any one account — for every
//! client except the desktop app itself (loopback peer + loopback Host), which
//! a remote party must not be able to lock out of its own Mac (S8-07).
//! account. See `tests/auth_security.rs` for the property test.
//!
//! A third, username-independent key, [`client_key`] (`"ip:<ip or /64>"`,
//! threshold [`CLIENT_FAILURE_THRESHOLD`]), stops one remote client from
//! spraying many usernames (S8-303). The map is capped at
//! [`MAX_TRACKED_KEYS`] and FAILS CLOSED when full: the handler refuses a
//! remote attempt whose keys it could not track
//! ([`AttemptStore::untracked_while_full`]), and the keys that matter most —
//! `user:<an existing username>` and the desktop's own key — are recorded with
//! [`AttemptStore::record_failure_pinned`], which ignores the cap (a bounded
//! set: real accounts). A flood of junk usernames can no longer stop `root`
//! from locking.

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
/// Failures tolerated per remote client across ALL usernames ([`client_key`]).
/// Higher than the per-account threshold: a person may mistype a few accounts.
pub const CLIENT_FAILURE_THRESHOLD: u32 = 20;
/// Cap the map so a flood of distinct keys can't grow memory unbounded. Expired
/// entries are pruned first; then unpinned new keys are not tracked and the
/// handler fails closed for them ([`AttemptStore::untracked_while_full`]).
pub const MAX_TRACKED_KEYS: usize = 10_000;
/// `Retry-After` for an attempt refused because the map is saturated.
pub const SATURATED_RETRY: Duration = Duration::from_secs(60);

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
    /// threshold inside the window. A new key is not tracked while the map is
    /// full of live entries (the handler fails closed for it — see
    /// [`Self::untracked_while_full`]).
    pub fn record_failure(&self, key: &str) {
        self.record(key, false);
    }

    /// [`Self::record_failure`] that ignores the size cap — for the bounded
    /// set of keys that must ALWAYS lock (`user:<existing username>`, the
    /// desktop's own key), so a junk-key flood can't switch their lockout off.
    pub fn record_failure_pinned(&self, key: &str) {
        self.record(key, true);
    }

    fn record(&self, key: &str, pinned: bool) {
        let mut store = self.inner.lock().unwrap();
        let now = Instant::now();
        if store.len() >= MAX_TRACKED_KEYS && !store.contains_key(key) {
            // Only a full map pays the O(n) prune.
            prune_expired(&mut store, now);
            if store.len() >= MAX_TRACKED_KEYS && !pinned {
                return;
            }
        }
        let entry = store.entry(key.to_string()).or_default();
        entry
            .failures
            .retain(|t| now.duration_since(*t) < FAILURE_WINDOW);
        entry.failures.push(now);
        if entry.failures.len() as u32 >= threshold_for(key) {
            entry.locked_until = Some(now + LOCKOUT_DURATION);
        }
    }

    /// True when the map is full of live entries and one of `keys` is not
    /// tracked: that attempt's failures could not be counted, so the handler
    /// refuses it (fail closed, S8-303).
    pub fn untracked_while_full(&self, keys: &[&str]) -> bool {
        let mut store = self.inner.lock().unwrap();
        if store.len() < MAX_TRACKED_KEYS || keys.iter().all(|k| store.contains_key(*k)) {
            return false;
        }
        prune_expired(&mut store, Instant::now());
        store.len() >= MAX_TRACKED_KEYS && keys.iter().any(|k| !store.contains_key(*k))
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
///
/// The IP part is the share throttle's client key: IPv6 is bucketed by /64.
pub fn ip_key(peer: Option<IpAddr>, username: &str) -> String {
    match peer {
        Some(ip) => format!(
            "{}|{}",
            otto_sessions::share_throttle::ip_key(ip),
            username.trim().to_lowercase()
        ),
        None => username_key(username),
    }
}

/// Per-client key independent of the username (S8-303): one remote client
/// spraying many usernames still locks after [`CLIENT_FAILURE_THRESHOLD`].
pub fn client_key(ip: IpAddr) -> String {
    format!("ip:{}", otto_sessions::share_throttle::ip_key(ip))
}

fn threshold_for(key: &str) -> u32 {
    if key.starts_with("ip:") {
        CLIENT_FAILURE_THRESHOLD
    } else {
        FAILURE_THRESHOLD
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

    #[test]
    fn a_flooded_map_still_locks_existing_accounts_and_fails_closed() {
        // S8-303: fill the map with junk-username keys, then guess root.
        let store = AttemptStore::default();
        for i in 0..MAX_TRACKED_KEYS {
            store.record_failure(&username_key(&format!("junk{i}")));
        }
        let root = username_key("root");
        let attacker = ip_key(ip(203, 0, 113, 5), "root");
        assert!(
            store.untracked_while_full(&[&attacker, &root]),
            "a remote attempt the full map can't track must be refused"
        );
        for _ in 0..FAILURE_THRESHOLD {
            store.record_failure_pinned(&root);
        }
        assert!(
            store.check_locked(&root).is_some(),
            "root must still lock with the map full"
        );
    }

    #[test]
    fn one_client_spraying_usernames_locks_its_client_key() {
        let store = AttemptStore::default();
        let client = client_key(ip(198, 51, 100, 3).unwrap());
        for i in 0..CLIENT_FAILURE_THRESHOLD {
            store.record_failure(&username_key(&format!("user{i}")));
            store.record_failure(&client);
        }
        assert!(store.check_locked(&client).is_some());
    }
}
