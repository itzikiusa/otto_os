//! Total lag per consumer group for the Groups LIST (the detail page keeps
//! [`KafkaClient::describe_group`]). The list computes many groups, so the
//! cluster's partition list is fetched ONCE by the caller and shared, and a
//! group whose lag cannot be computed reads as unknown — never a made-up 0.

use super::{group_offset, is_internal, kerr, KafkaClient, GROUP_TIMEOUT, META_TIMEOUT};
use otto_core::Result;
use rdkafka::consumer::Consumer;
use rdkafka::topic_partition_list::{Offset, TopicPartitionList};
use std::collections::HashMap;

impl KafkaClient {
    /// Every non-internal `(topic, partition)`: the OffsetFetch request each
    /// group's lag needs (one metadata pass per list load, not per group).
    pub fn lag_partitions(&self) -> Result<Vec<(String, i32)>> {
        let md = self
            .consumer
            .fetch_metadata(None, META_TIMEOUT)
            .map_err(kerr)?;
        self.remember_partitions(&md);
        Ok(md
            .topics()
            .iter()
            .filter(|t| !is_internal(t.name()))
            .flat_map(|t| {
                t.partitions()
                    .iter()
                    .map(move |p| (t.name().to_string(), p.id()))
            })
            .collect())
    }

    /// `group`'s total lag over `parts` (committed offsets, then one batched
    /// watermark pass). `Ok(None)` when the group has no committed offset at
    /// all; an unreadable watermark is an error, so the caller shows "—".
    pub fn group_total_lag(&self, group: &str, parts: &[(String, i32)]) -> Result<Option<i64>> {
        if parts.is_empty() {
            return Ok(None);
        }
        let mut tpl = TopicPartitionList::new();
        for (t, p) in parts {
            let _ = tpl.add_partition(t, *p);
        }
        let committed = self.with_group_consumer(group, |c| {
            c.committed_offsets(tpl, GROUP_TIMEOUT).map_err(kerr)
        })?;
        let commits: Vec<(String, i32, i64)> = committed
            .elements()
            .iter()
            .filter_map(|e| match e.offset() {
                Offset::Offset(o) if o >= 0 => Some((e.topic().to_string(), e.partition(), o)),
                _ => None,
            })
            .collect();
        if commits.is_empty() {
            return Ok(None);
        }
        let targets: Vec<(&str, i32)> = commits.iter().map(|(t, p, _)| (t.as_str(), *p)).collect();
        let wm = self.batch_watermarks(&targets);
        sum_group_lag(&commits, &wm)
    }
}

/// Sum of per-partition lag (same clamp as the detail page's
/// [`group_offset`]); any partition without a watermark fails the whole sum.
pub(super) fn sum_group_lag(
    commits: &[(String, i32, i64)],
    wm: &HashMap<(String, i32), (i64, i64)>,
) -> Result<Option<i64>> {
    if commits.is_empty() {
        return Ok(None);
    }
    commits
        .iter()
        .try_fold(0i64, |sum, (t, p, cur)| {
            Ok(sum + group_offset(t, *p, *cur, wm)?.lag)
        })
        .map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wm(entries: &[(&str, i32, i64, i64)]) -> HashMap<(String, i32), (i64, i64)> {
        entries
            .iter()
            .map(|&(t, p, lo, hi)| ((t.to_string(), p), (lo, hi)))
            .collect()
    }

    #[test]
    fn group_lag_sums_partitions_and_clamps_negative() {
        let commits = vec![
            ("a".to_string(), 0, 90),
            ("a".to_string(), 1, 10),
            // Committed past the high watermark (truncated log): 0, not -5.
            ("b".to_string(), 0, 25),
        ];
        let w = wm(&[("a", 0, 0, 100), ("a", 1, 0, 10), ("b", 0, 0, 20)]);
        assert_eq!(sum_group_lag(&commits, &w).unwrap(), Some(10));
    }

    #[test]
    fn group_lag_missing_watermark_is_unknown_not_zero() {
        let commits = vec![("a".to_string(), 0, 5), ("a".to_string(), 1, 5)];
        let w = wm(&[("a", 0, 0, 5)]);
        assert!(sum_group_lag(&commits, &w).is_err());
    }

    #[test]
    fn group_lag_without_commits_is_none() {
        assert_eq!(sum_group_lag(&[], &HashMap::new()).unwrap(), None);
    }
}
