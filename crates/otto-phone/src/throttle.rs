//! A small in-process rate limiter for the public pairing routes, keyed by
//! the client address. The codes are 256-bit (unguessable), so this is about
//! abuse and noise, not brute force: a peer that keeps presenting bad codes is
//! locked out for a while, and every peer's poll rate is bounded.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Bad claims tolerated per window before the peer is locked out.
pub const CLAIM_FAILURES: u32 = 5;
/// Requests (claims + polls) per window per peer.
pub const REQUESTS_PER_WINDOW: u32 = 300;
/// Window / lockout length.
pub const WINDOW: Duration = Duration::from_secs(10 * 60);
/// Tracked peers before the stalest are dropped.
const MAX_KEYS: usize = 4096;

#[derive(Default)]
struct Slot {
    start: Option<Instant>,
    requests: u32,
    failures: u32,
}

/// Per-peer counters.
#[derive(Default)]
pub struct Throttle {
    slots: Mutex<HashMap<IpAddr, Slot>>,
}

impl Throttle {
    /// The process-wide throttle for the pairing routes.
    pub fn global() -> &'static Throttle {
        static T: std::sync::OnceLock<Throttle> = std::sync::OnceLock::new();
        T.get_or_init(Throttle::default)
    }

    /// Count one request; `false` = refuse it (429).
    pub fn allow(&self, ip: IpAddr) -> bool {
        let mut slots = self.slots.lock().unwrap_or_else(|p| p.into_inner());
        if slots.len() >= MAX_KEYS && !slots.contains_key(&ip) {
            slots.retain(|_, s| s.start.is_some_and(|t| t.elapsed() < WINDOW));
            if slots.len() >= MAX_KEYS {
                return false;
            }
        }
        let s = slots.entry(ip).or_default();
        if s.start.is_none_or(|t| t.elapsed() >= WINDOW) {
            *s = Slot {
                start: Some(Instant::now()),
                ..Slot::default()
            };
        }
        if s.failures >= CLAIM_FAILURES || s.requests >= REQUESTS_PER_WINDOW {
            return false;
        }
        s.requests += 1;
        true
    }

    /// Record a bad code from `ip`.
    pub fn fail(&self, ip: IpAddr) {
        let mut slots = self.slots.lock().unwrap_or_else(|p| p.into_inner());
        slots.entry(ip).or_default().failures += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failures_lock_a_peer_out_but_not_others() {
        let t = Throttle::default();
        let a: IpAddr = "100.1.1.1".parse().unwrap();
        let b: IpAddr = "100.1.1.2".parse().unwrap();
        for _ in 0..CLAIM_FAILURES {
            assert!(t.allow(a));
            t.fail(a);
        }
        assert!(!t.allow(a));
        assert!(t.allow(b));
    }

    #[test]
    fn request_rate_is_bounded() {
        let t = Throttle::default();
        let a: IpAddr = "100.1.1.1".parse().unwrap();
        for _ in 0..REQUESTS_PER_WINDOW {
            assert!(t.allow(a));
        }
        assert!(!t.allow(a));
    }
}
