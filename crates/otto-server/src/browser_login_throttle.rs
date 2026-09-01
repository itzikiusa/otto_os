//! Per-domain rate limit for `POST /workspaces/{wid}/browser/login`.
//!
//! Mirrors [`crate::login_throttle`]'s in-memory sliding-window shape, but
//! throttles CALL RATE rather than failures: `browser_login` drives a real
//! CDP navigation + form submit against a third-party site on the caller's
//! behalf, so a governed agent tool calling it in a loop (or a genuine
//! brute-force attempt routed through it) is capped at
//! [`MAX_ATTEMPTS_PER_WINDOW`] per domain per [`WINDOW`], independent of
//! whether any given attempt actually logs in.
//!
//! State is per-process, in-memory, and resets on daemon restart — same
//! trade-off `login_throttle` makes, appropriate for a single-node daemon.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Calls tolerated per domain inside [`WINDOW`] before further attempts are
/// rejected with 429.
pub const MAX_ATTEMPTS_PER_WINDOW: usize = 3;
/// Sliding window over which attempts are counted.
pub const WINDOW: Duration = Duration::from_secs(60);
/// Cap the map so a flood of distinct domains can't grow memory unbounded.
const MAX_TRACKED_DOMAINS: usize = 10_000;

#[derive(Default)]
pub struct BrowserLoginThrottle {
    inner: Mutex<HashMap<String, Vec<Instant>>>,
}

impl BrowserLoginThrottle {
    /// Records this attempt against `domain` and returns whether it is
    /// allowed (`true`) or the domain is currently over the rate (`false`).
    /// A rejected attempt is NOT itself recorded — it must not extend the
    /// window past what real attempts earned.
    pub fn try_acquire(&self, domain: &str) -> bool {
        let mut store = self.inner.lock().unwrap();
        let now = Instant::now();
        prune_expired(&mut store, now);
        let already_tracked = store.contains_key(domain);
        if store.len() >= MAX_TRACKED_DOMAINS && !already_tracked {
            // Map is full of live entries; fail closed rather than grow
            // unbounded — an untracked domain is treated as rate-limited.
            return false;
        }
        let entry = store.entry(domain.to_string()).or_default();
        entry.retain(|t| now.duration_since(*t) < WINDOW);
        if entry.len() >= MAX_ATTEMPTS_PER_WINDOW {
            return false;
        }
        entry.push(now);
        true
    }
}

fn prune_expired(store: &mut HashMap<String, Vec<Instant>>, now: Instant) {
    store.retain(|_, attempts| {
        attempts.retain(|t| now.duration_since(*t) < WINDOW);
        !attempts.is_empty()
    });
}

/// Process-global throttle used by the live `browser_login` handler.
pub fn global() -> &'static BrowserLoginThrottle {
    static STORE: OnceLock<BrowserLoginThrottle> = OnceLock::new();
    STORE.get_or_init(BrowserLoginThrottle::default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_up_to_the_cap_then_rejects() {
        let store = BrowserLoginThrottle::default();
        for _ in 0..MAX_ATTEMPTS_PER_WINDOW {
            assert!(store.try_acquire("example.com"));
        }
        assert!(!store.try_acquire("example.com"), "must reject the attempt over the cap");
    }

    #[test]
    fn domains_are_independent() {
        let store = BrowserLoginThrottle::default();
        for _ in 0..MAX_ATTEMPTS_PER_WINDOW {
            assert!(store.try_acquire("a.com"));
        }
        assert!(!store.try_acquire("a.com"));
        // A different domain has its own budget.
        assert!(store.try_acquire("b.com"));
    }

    #[test]
    fn a_rejected_attempt_is_not_itself_recorded() {
        let store = BrowserLoginThrottle::default();
        for _ in 0..MAX_ATTEMPTS_PER_WINDOW {
            assert!(store.try_acquire("example.com"));
        }
        // Several rejected calls in a row must not further extend/refill the
        // window with phantom entries.
        assert!(!store.try_acquire("example.com"));
        assert!(!store.try_acquire("example.com"));
        assert!(!store.try_acquire("example.com"));
    }
}
