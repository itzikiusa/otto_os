//! Keystroke-echo clock (latency diagnostics, review 01 L1).
//!
//! The writer thread stamps the moment an input job reached the PTY; the
//! output side ([`crate::Mirror::feed`]) turns the first chunk after it into
//! one "child echo" sample. That is the time the CLI (and the launchd QoS it
//! inherits) took to answer a keystroke, with no websocket or webview in the
//! path — the segment the terminal latency HUD cannot see from the client.
//!
//! Lock-free: two atomics on the hot paths, a few more for the stats. The
//! sample is "first output after input", so a spinner frame landing between
//! the write and the real echo under-reads; the max and the EWMA still show a
//! slow child plainly.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use serde::Serialize;

/// Weight of a new sample in the moving average (1/8, like TCP's SRTT).
const EWMA_SHIFT: u32 = 3;

/// Echo statistics since the PTY was spawned, in milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct EchoStats {
    /// The most recent sample.
    pub last_ms: f64,
    /// Exponentially weighted moving average (α = 1/8).
    pub avg_ms: f64,
    /// The worst sample seen.
    pub max_ms: f64,
    /// How many samples were taken.
    pub samples: u64,
}

#[derive(Debug)]
pub(crate) struct EchoClock {
    epoch: Instant,
    /// µs since `epoch` + 1 of the last input write not yet answered; 0 = none.
    pending_us: AtomicU64,
    last_us: AtomicU64,
    /// EWMA in µs, scaled by 2^EWMA_SHIFT to keep integer precision.
    avg_scaled: AtomicU64,
    max_us: AtomicU64,
    samples: AtomicU64,
}

impl EchoClock {
    pub(crate) fn new() -> Self {
        Self {
            epoch: Instant::now(),
            pending_us: AtomicU64::new(0),
            last_us: AtomicU64::new(0),
            avg_scaled: AtomicU64::new(0),
            max_us: AtomicU64::new(0),
            samples: AtomicU64::new(0),
        }
    }

    fn now_us(&self) -> u64 {
        self.epoch.elapsed().as_micros() as u64
    }

    /// An input job reached the PTY. Keeps the EARLIEST unanswered write, so a
    /// burst of keystrokes measures from the first one the child has not
    /// answered yet.
    pub(crate) fn input_written(&self) {
        let stamp = self.now_us() + 1;
        let _ = self
            .pending_us
            .compare_exchange(0, stamp, Ordering::AcqRel, Ordering::Relaxed);
    }

    /// Child output arrived: close the pending sample, if any.
    pub(crate) fn output(&self) {
        // Cheap common case (output with no keystroke pending): one load.
        if self.pending_us.load(Ordering::Relaxed) == 0 {
            return;
        }
        let pending = self.pending_us.swap(0, Ordering::AcqRel);
        if pending == 0 {
            return;
        }
        let sample = self.now_us().saturating_sub(pending - 1);
        self.record(sample);
    }

    fn record(&self, sample_us: u64) {
        self.last_us.store(sample_us, Ordering::Relaxed);
        self.max_us.fetch_max(sample_us, Ordering::Relaxed);
        let n = self.samples.fetch_add(1, Ordering::Relaxed);
        // Single writer in practice (the output reader thread); a lost update
        // under a race only nudges a diagnostic average.
        let prev = self.avg_scaled.load(Ordering::Relaxed);
        let next = if n == 0 {
            sample_us << EWMA_SHIFT
        } else {
            prev - (prev >> EWMA_SHIFT) + sample_us
        };
        self.avg_scaled.store(next, Ordering::Relaxed);
    }

    pub(crate) fn stats(&self) -> EchoStats {
        let ms = |us: u64| us as f64 / 1000.0;
        EchoStats {
            last_ms: ms(self.last_us.load(Ordering::Relaxed)),
            avg_ms: ms(self.avg_scaled.load(Ordering::Relaxed) >> EWMA_SHIFT),
            max_ms: ms(self.max_us.load(Ordering::Relaxed)),
            samples: self.samples.load(Ordering::Relaxed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_without_input_takes_no_sample() {
        let c = EchoClock::new();
        c.output();
        c.output();
        assert_eq!(c.stats().samples, 0);
    }

    #[test]
    fn first_output_after_input_is_one_sample() {
        let c = EchoClock::new();
        c.input_written();
        std::thread::sleep(std::time::Duration::from_millis(5));
        c.output();
        c.output(); // a second chunk is not another sample
        let s = c.stats();
        assert_eq!(s.samples, 1);
        assert!(s.last_ms >= 5.0, "{s:?}");
        assert_eq!(s.max_ms, s.last_ms);
        assert!((s.avg_ms - s.last_ms).abs() < 0.01, "{s:?}");
    }

    #[test]
    fn burst_measures_from_the_first_unanswered_write() {
        let c = EchoClock::new();
        c.input_written();
        std::thread::sleep(std::time::Duration::from_millis(5));
        c.input_written(); // does not move the stamp forward
        c.output();
        assert!(c.stats().last_ms >= 5.0);
    }

    #[test]
    fn average_and_max_track_samples() {
        let c = EchoClock::new();
        c.record(8_000);
        c.record(0);
        let s = c.stats();
        assert_eq!(s.samples, 2);
        assert_eq!(s.max_ms, 8.0);
        assert_eq!(s.last_ms, 0.0);
        // 8 ms, then a 0 sample at α = 1/8 → 7 ms.
        assert!((s.avg_ms - 7.0).abs() < 0.01, "{s:?}");
    }
}
