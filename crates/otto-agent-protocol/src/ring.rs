//! The per-session replay ring: the most recent sequenced events, bounded by
//! count AND encoded bytes, so a reconnecting client resumes exactly after
//! the last `sequence` it saw. Past the ring — or past T3 Code's replay
//! limits (`THREAD_RESUME_MAX_REPLAY_EVENTS` = 128,
//! `THREAD_RESUME_MAX_REPLAY_ENCODED_BYTES` = 1 MiB in
//! `apps/server/src/orchestration-v2/ThreadStream.ts`, MIT, T3 Tools Inc.) —
//! the client gets a snapshot instead ([`crate::runtime`]).

use std::collections::VecDeque;
use std::sync::Arc;

use otto_core::agent_runtime::RuntimeEvent;

/// Ring bounds. Defaults: 4096 events / 8 MiB.
#[derive(Debug, Clone, Copy)]
pub struct RingLimits {
    pub max_events: usize,
    pub max_bytes: usize,
}

impl Default for RingLimits {
    fn default() -> Self {
        Self {
            max_events: 4096,
            max_bytes: 8 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Default)]
pub struct EventRing {
    events: VecDeque<(Arc<RuntimeEvent>, usize)>,
    bytes: usize,
    limits: RingLimits,
}

impl EventRing {
    pub fn new(limits: RingLimits) -> Self {
        Self {
            events: VecDeque::new(),
            bytes: 0,
            limits,
        }
    }

    /// Append (seqs must increase), evicting the oldest past either bound.
    /// The newest event is always kept, however large.
    pub fn push(&mut self, ev: Arc<RuntimeEvent>) {
        debug_assert!(self.last_seq().is_none_or(|last| ev.sequence > last));
        let size = encoded_len(&ev);
        self.bytes += size;
        self.events.push_back((ev, size));
        while self.events.len() > 1
            && (self.events.len() > self.limits.max_events || self.bytes > self.limits.max_bytes)
        {
            if let Some((_, s)) = self.events.pop_front() {
                self.bytes -= s;
            }
        }
    }

    /// The events after `after_seq` if the ring holds all of them and they
    /// fit `max_events` / `max_bytes`; `None` → serve a snapshot.
    pub fn replayable(
        &self,
        after_seq: u64,
        max_events: usize,
        max_bytes: usize,
    ) -> Option<Vec<RuntimeEvent>> {
        // An empty ring covers nothing; the caller answers "already up to
        // date" before asking.
        if !self.covers(after_seq) {
            return None;
        }
        let start = self
            .events
            .partition_point(|(e, _)| e.sequence <= after_seq);
        let tail = self.events.range(start..);
        if tail.len() > max_events || tail.clone().map(|(_, s)| s).sum::<usize>() > max_bytes {
            return None;
        }
        Some(tail.map(|(e, _)| (**e).clone()).collect())
    }

    pub fn first_seq(&self) -> Option<u64> {
        self.events.front().map(|(e, _)| e.sequence)
    }

    pub fn last_seq(&self) -> Option<u64> {
        self.events.back().map(|(e, _)| e.sequence)
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Can the ring alone answer "everything after `after_seq`"? True when
    /// nothing after it was evicted.
    pub fn covers(&self, after_seq: u64) -> bool {
        match self.first_seq() {
            Some(first) => after_seq.saturating_add(1) >= first,
            None => false,
        }
    }

    /// Events with `seq > after_seq`, oldest first.
    pub fn after(&self, after_seq: u64) -> Vec<RuntimeEvent> {
        let start = self
            .events
            .partition_point(|(e, _)| e.sequence <= after_seq);
        self.events
            .iter()
            .skip(start)
            .map(|(e, _)| (**e).clone())
            .collect()
    }
}

/// Encoded JSON size of one event (the ring's byte budget unit).
pub fn encoded_len(ev: &RuntimeEvent) -> usize {
    serde_json::to_vec(ev).map_or(0, |v| v.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use otto_core::agent_runtime::RuntimeEventBody;

    fn ev(seq: u64, chunk: &str) -> Arc<RuntimeEvent> {
        Arc::new(RuntimeEvent {
            session_id: "s".into(),
            sequence: seq,
            ts: Utc::now(),
            body: RuntimeEventBody::CommandOutputDelta {
                item_id: "i".into(),
                chunk: chunk.into(),
            },
        })
    }

    #[test]
    fn ring_replays_after_seq_and_reports_coverage() {
        let mut ring = EventRing::new(RingLimits {
            max_events: 3,
            max_bytes: usize::MAX,
        });
        assert!(!ring.covers(0));
        for s in 1..=5 {
            ring.push(ev(s, "x"));
        }
        assert_eq!((ring.len(), ring.first_seq()), (3, Some(3)));
        assert!(ring.covers(2) && ring.covers(4) && !ring.covers(1));
        let seqs: Vec<u64> = ring.after(3).iter().map(|e| e.sequence).collect();
        assert_eq!(seqs, vec![4, 5]);
        assert!(ring.after(5).is_empty());
        assert!(ring.replayable(1, 128, usize::MAX).is_none(), "evicted");
        assert_eq!(ring.replayable(2, 128, usize::MAX).unwrap().len(), 3);
        assert!(
            ring.replayable(2, 2, usize::MAX).is_none(),
            "over the event cap"
        );
        assert!(ring.replayable(2, 128, 10).is_none(), "over the byte cap");
        assert!(ring.replayable(5, 0, 0).unwrap().is_empty());
    }

    #[test]
    fn ring_evicts_by_bytes_but_keeps_the_newest() {
        let one = encoded_len(&ev(1, &"a".repeat(100)));
        let mut ring = EventRing::new(RingLimits {
            max_events: 100,
            max_bytes: one * 2 - 1,
        });
        ring.push(ev(1, &"a".repeat(100)));
        ring.push(ev(2, &"b".repeat(100)));
        assert_eq!(ring.first_seq(), Some(2), "byte budget evicted seq 1");
        ring.push(ev(3, &"c".repeat(10_000)));
        assert_eq!((ring.len(), ring.last_seq()), (1, Some(3)));
    }
}
