//! Screencast flow control. Pure (the caller passes `now`), so every rule is
//! unit-tested without a browser.
//!
//! - [`ViewerFlow`] — per-viewer ack-based backpressure: at most [`WINDOW`]
//!   frames in flight; beyond that only the NEWEST frame is held (older held
//!   frames are dropped, never queued), and it goes out on the next ack.
//! - [`Adaptive`] — one JPEG quality / every-Nth-frame level per session,
//!   stepped down when the slowest viewer's ack latency or drop ratio is bad
//!   and back up when it recovers, with hysteresis.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Frames a viewer may have un-acked at once.
pub const WINDOW: usize = 2;

/// `(jpeg quality, everyNthFrame)` from best to cheapest.
pub const LEVELS: [(u8, u32); 5] = [(80, 1), (65, 1), (50, 1), (40, 2), (35, 3)];

/// Level a new session starts at.
pub const START_LEVEL: usize = 1;

const STEP_DOWN_RTT_MS: f64 = 450.0;
const STEP_UP_RTT_MS: f64 = 150.0;
const STEP_DOWN_DROP_RATIO: f64 = 0.35;
const STEP_UP_DROP_RATIO: f64 = 0.05;
const MIN_CHANGE_GAP: Duration = Duration::from_secs(3);
const STEP_UP_GAP: Duration = Duration::from_secs(6);

#[derive(Debug)]
pub struct ViewerFlow<T> {
    in_flight: VecDeque<(u64, Instant)>,
    pending: Option<(u64, T)>,
    /// Smoothed ack latency (ms); `None` until the first ack.
    rtt_ms: Option<f64>,
    sent: u64,
    dropped: u64,
}

impl<T> Default for ViewerFlow<T> {
    fn default() -> Self {
        Self {
            in_flight: VecDeque::new(),
            pending: None,
            rtt_ms: None,
            sent: 0,
            dropped: 0,
        }
    }
}

impl<T> ViewerFlow<T> {
    pub fn has_capacity(&self) -> bool {
        self.in_flight.len() < WINDOW
    }

    /// A new frame `seq`. Returns it when it should be sent now; otherwise it
    /// is held (replacing — and counting as dropped — any older held frame).
    pub fn offer(&mut self, seq: u64, item: T, now: Instant) -> Option<T> {
        if self.has_capacity() {
            self.in_flight.push_back((seq, now));
            self.sent += 1;
            return Some(item);
        }
        if self.pending.replace((seq, item)).is_some() {
            self.dropped += 1;
        }
        None
    }

    /// The viewer drew `seq` (acks are cumulative). Returns the held frame
    /// when the window now has room for it.
    pub fn ack(&mut self, seq: u64, now: Instant) -> Option<T> {
        let mut sample = None;
        while let Some(&(s, at)) = self.in_flight.front() {
            if s > seq {
                break;
            }
            self.in_flight.pop_front();
            if s == seq {
                sample = Some(now.saturating_duration_since(at).as_secs_f64() * 1000.0);
            }
        }
        if let Some(ms) = sample {
            self.rtt_ms = Some(match self.rtt_ms {
                None => ms,
                Some(prev) => prev * 0.7 + ms * 0.3,
            });
        }
        if self.has_capacity() {
            if let Some((s, item)) = self.pending.take() {
                self.in_flight.push_back((s, now));
                self.sent += 1;
                return Some(item);
            }
        }
        None
    }

    pub fn rtt_ms(&self) -> Option<f64> {
        self.rtt_ms
    }

    /// `(sent, dropped)` since the last call — resets the counters.
    pub fn take_counters(&mut self) -> (u64, u64) {
        let c = (self.sent, self.dropped);
        self.sent = 0;
        self.dropped = 0;
        c
    }
}

#[derive(Debug)]
pub struct Adaptive {
    level: usize,
    last_change: Instant,
    sent: u64,
    dropped: u64,
    /// Worst smoothed RTT seen in the current window.
    worst_rtt_ms: f64,
}

impl Adaptive {
    pub fn new(now: Instant) -> Self {
        Self {
            level: START_LEVEL,
            last_change: now,
            sent: 0,
            dropped: 0,
            worst_rtt_ms: 0.0,
        }
    }

    pub fn params(&self) -> (u8, u32) {
        LEVELS[self.level]
    }

    pub fn level(&self) -> usize {
        self.level
    }

    /// Feed one viewer's sample (smoothed RTT + counters since last time).
    pub fn observe(&mut self, rtt_ms: Option<f64>, sent: u64, dropped: u64) {
        self.sent += sent;
        self.dropped += dropped;
        if let Some(r) = rtt_ms {
            if r > self.worst_rtt_ms {
                self.worst_rtt_ms = r;
            }
        }
    }

    /// Decide at `now`; `Some(new params)` when the level changed (the caller
    /// restarts the screencast with them). Resets the observation window
    /// whenever it evaluates.
    pub fn decide(&mut self, now: Instant) -> Option<(u8, u32)> {
        let since = now.saturating_duration_since(self.last_change);
        if since < MIN_CHANGE_GAP {
            return None;
        }
        let total = self.sent + self.dropped;
        if total == 0 {
            return None; // nothing observed — no evidence either way
        }
        let drop_ratio = self.dropped as f64 / total as f64;
        let rtt = self.worst_rtt_ms;
        self.sent = 0;
        self.dropped = 0;
        self.worst_rtt_ms = 0.0;
        if (rtt > STEP_DOWN_RTT_MS || drop_ratio > STEP_DOWN_DROP_RATIO)
            && self.level + 1 < LEVELS.len()
        {
            self.level += 1;
            self.last_change = now;
            return Some(self.params());
        }
        if rtt < STEP_UP_RTT_MS
            && drop_ratio < STEP_UP_DROP_RATIO
            && since >= STEP_UP_GAP
            && self.level > 0
        {
            self.level -= 1;
            self.last_change = now;
            return Some(self.params());
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_limits_in_flight_and_keeps_only_the_newest_held_frame() {
        let t0 = Instant::now();
        let mut f: ViewerFlow<&str> = ViewerFlow::default();
        assert_eq!(f.offer(1, "f1", t0), Some("f1"));
        assert_eq!(f.offer(2, "f2", t0), Some("f2"));
        // Window full: held, then replaced by a newer frame (one drop).
        assert_eq!(f.offer(3, "f3", t0), None);
        assert_eq!(f.offer(4, "f4", t0), None);
        // Ack 1 → room for one → the NEWEST held frame goes out.
        assert_eq!(f.ack(1, t0 + Duration::from_millis(50)), Some("f4"));
        assert_eq!(f.ack(1, t0), None, "duplicate ack is a no-op");
        let (sent, dropped) = f.take_counters();
        assert_eq!((sent, dropped), (3, 1));
    }

    #[test]
    fn a_viewer_that_never_acks_gets_nothing_more() {
        let t0 = Instant::now();
        let mut f: ViewerFlow<u64> = ViewerFlow::default();
        let mut sent = 0;
        for s in 1..=100 {
            if f.offer(s, s, t0).is_some() {
                sent += 1;
            }
        }
        assert_eq!(sent, WINDOW);
    }

    #[test]
    fn acks_are_cumulative_and_measure_rtt() {
        let t0 = Instant::now();
        let mut f: ViewerFlow<u64> = ViewerFlow::default();
        f.offer(1, 1, t0);
        f.offer(2, 2, t0);
        // Ack of 2 retires 1 as well.
        assert_eq!(f.ack(2, t0 + Duration::from_millis(100)), None);
        assert!(f.has_capacity());
        let rtt = f.rtt_ms().unwrap();
        assert!((rtt - 100.0).abs() < 1.0, "rtt {rtt}");
        // An ack for a frame never sent changes nothing.
        assert_eq!(f.ack(99, t0), None);
    }

    #[test]
    fn adaptive_steps_down_on_latency_and_up_after_recovery() {
        let t0 = Instant::now();
        let mut a = Adaptive::new(t0);
        assert_eq!(a.params(), LEVELS[START_LEVEL]);
        // Too early to change.
        a.observe(Some(900.0), 10, 0);
        assert_eq!(a.decide(t0 + Duration::from_secs(1)), None);
        // Slow viewer → step down.
        a.observe(Some(900.0), 10, 0);
        let t1 = t0 + Duration::from_secs(4);
        assert_eq!(a.decide(t1), Some(LEVELS[START_LEVEL + 1]));
        // Healthy, but the step-up gap has not elapsed.
        a.observe(Some(40.0), 30, 0);
        assert_eq!(a.decide(t1 + Duration::from_secs(4)), None);
        a.observe(Some(40.0), 30, 0);
        assert_eq!(
            a.decide(t1 + Duration::from_secs(7)),
            Some(LEVELS[START_LEVEL])
        );
    }

    #[test]
    fn adaptive_steps_down_on_drops_and_saturates() {
        let mut now = Instant::now();
        let mut a = Adaptive::new(now);
        for _ in 0..10 {
            now += Duration::from_secs(4);
            a.observe(Some(50.0), 2, 8);
            a.decide(now);
        }
        assert_eq!(a.level(), LEVELS.len() - 1);
        // No evidence → no change.
        now += Duration::from_secs(10);
        assert_eq!(a.decide(now), None);
    }
}
