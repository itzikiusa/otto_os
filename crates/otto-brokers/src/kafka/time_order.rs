//! Output order of a multi-partition peek. The poll loop receives partitions
//! interleaved by fetch timing and used to return them grouped by partition,
//! so "latest 50" over 6 partitions read as six unrelated runs. The peek now
//! returns one timeline: a k-way merge by timestamp that never reorders a
//! partition's own offsets (CreateTime can step backwards within a partition,
//! and replay relies on per-partition order).

use super::RawMessage;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, VecDeque};

/// Merge per-partition runs into one timestamp-ordered list. Ties (and
/// heads without a timestamp, which sort first) break by partition, then
/// offset, so the order is deterministic. Each partition stays in offset order.
pub(super) fn merge_by_timestamp(mut msgs: Vec<RawMessage>) -> Vec<RawMessage> {
    msgs.sort_by_key(|m| (m.partition, m.offset));
    let mut runs: Vec<VecDeque<RawMessage>> = Vec::new();
    for m in msgs.drain(..) {
        match runs.last_mut() {
            Some(run) if run.back().is_some_and(|b| b.partition == m.partition) => run.push_back(m),
            _ => runs.push(VecDeque::from([m])),
        }
    }
    let head_key = |m: &RawMessage| (m.timestamp_ms.unwrap_or(i64::MIN), m.partition, m.offset);
    let mut heap = BinaryHeap::with_capacity(runs.len());
    for (i, run) in runs.iter().enumerate() {
        if let Some(m) = run.front() {
            heap.push(Reverse((head_key(m), i)));
        }
    }
    while let Some(Reverse((_, i))) = heap.pop() {
        if let Some(m) = runs[i].pop_front() {
            msgs.push(m);
        }
        if let Some(next) = runs[i].front() {
            heap.push(Reverse((head_key(next), i)));
        }
    }
    msgs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(partition: i32, offset: i64, ts: Option<i64>) -> RawMessage {
        RawMessage {
            partition,
            offset,
            timestamp_ms: ts,
            key: None,
            value: None,
            headers: Vec::new(),
            size: 0,
        }
    }

    fn order(v: &[RawMessage]) -> Vec<(i32, i64)> {
        v.iter().map(|m| (m.partition, m.offset)).collect()
    }

    #[test]
    fn multi_partition_peek_is_time_ordered() {
        // Arrival order as the poll loop saw it: partition 1 first.
        let got = merge_by_timestamp(vec![
            msg(1, 7, Some(150)),
            msg(1, 8, Some(400)),
            msg(0, 3, Some(100)),
            msg(0, 4, Some(300)),
            msg(2, 0, Some(200)),
        ]);
        assert_eq!(order(&got), vec![(0, 3), (1, 7), (2, 0), (0, 4), (1, 8)]);
    }

    #[test]
    fn equal_timestamps_break_by_partition_then_offset() {
        let got = merge_by_timestamp(vec![
            msg(2, 5, Some(100)),
            msg(0, 9, Some(100)),
            msg(0, 8, Some(100)),
        ]);
        assert_eq!(order(&got), vec![(0, 8), (0, 9), (2, 5)]);
    }

    #[test]
    fn a_partition_never_reorders_its_own_offsets() {
        // CreateTime stepped backwards inside partition 0 (offset 2 is older
        // than offset 1): offsets stay in order, the merge only interleaves.
        let got = merge_by_timestamp(vec![
            msg(0, 1, Some(500)),
            msg(0, 2, Some(100)),
            msg(1, 0, Some(300)),
        ]);
        assert_eq!(order(&got), vec![(1, 0), (0, 1), (0, 2)]);
    }

    #[test]
    fn merge_keeps_every_message_and_handles_empty() {
        assert!(merge_by_timestamp(Vec::new()).is_empty());
        let got = merge_by_timestamp(vec![msg(0, 0, None), msg(1, 0, Some(1))]);
        assert_eq!(order(&got), vec![(0, 0), (1, 0)]);
    }
}
