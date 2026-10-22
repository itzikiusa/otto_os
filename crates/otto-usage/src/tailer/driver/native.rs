//! Accounting for provider records whose usage can be updated in place.
//! The checkpoint commits with the owed events through the tailer's journal.

use crate::ParsedUsage;

#[path = "native/agy.rs"]
mod agy;
use std::collections::{HashMap, HashSet};

/// Backstop cap on `totals` (like `SEEN_KEYS_CAP`): the per-source pruning
/// below (a deleted Grok file, a vanished Antigravity DB) keeps the live set
/// small; past this the oldest baselines go first. An evicted baseline is
/// only owed again if its source record is re-read — Grok lines are read
/// once past the cursor, Antigravity rows once past the high-water mark.
const NATIVE_TOTALS_CAP: usize = 100_000;

#[derive(Default, serde::Serialize, serde::Deserialize)]
pub(super) struct NativeState {
    totals: HashMap<String, NativeTotals>,
    #[serde(default)]
    aliases: HashMap<String, String>,
    /// Source of [`NativeTotals::seq`]. Older checkpoints have none: their
    /// baselines read as seq 0, i.e. the oldest.
    #[serde(default)]
    next_seq: u64,
    #[serde(skip)]
    pub(super) dirty: bool,
    /// Per Antigravity DB (keyed by its lossy path): the fingerprint it was
    /// last read at and the incremental read mark. Persisted so a restart
    /// does not re-read every DB; older checkpoints lack it, which costs one
    /// full read per DB (deduped against `totals`, so nothing is re-billed).
    #[serde(default)]
    agy_files: HashMap<String, agy::FileState>,
}

#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
struct NativeTotals {
    usage: ParsedUsage,
    cost_ticks: Option<u64>,
    /// When this baseline last grew (FIFO order for [`NATIVE_TOTALS_CAP`]).
    #[serde(default, skip_serializing_if = "is_zero")]
    seq: u64,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

impl NativeState {
    /// Drop every baseline (and alias) whose key starts with one of
    /// `prefixes` — the records of a source that no longer exists.
    fn forget_prefixes(&mut self, prefixes: &[String]) {
        if prefixes.is_empty() {
            return;
        }
        let gone = |k: &str| prefixes.iter().any(|p| k.starts_with(p.as_str()));
        let before = self.totals.len() + self.aliases.len();
        self.totals.retain(|k, _| !gone(k));
        self.aliases.retain(|k, v| !gone(k) && !gone(v));
        if self.totals.len() + self.aliases.len() != before {
            self.dirty = true;
        }
    }

    /// Forget the per-turn baselines of Grok `updates.jsonl` files whose
    /// cursor was evicted (the file is gone, so its turns never recur).
    pub(super) fn forget_grok_files<'a>(&mut self, files: impl IntoIterator<Item = &'a String>) {
        let prefixes: Vec<String> = files.into_iter().map(|f| format!("grok:{f}:")).collect();
        self.forget_prefixes(&prefixes);
    }

    /// FIFO backstop: past the cap, drop the oldest tenth of the baselines
    /// (and the aliases resolving to them) in one O(n) pass.
    fn enforce_cap(&mut self) {
        self.enforce_cap_at(NATIVE_TOTALS_CAP);
    }

    fn enforce_cap_at(&mut self, cap: usize) {
        if self.totals.len() <= cap {
            return;
        }
        let excess = self.totals.len() - cap + cap / 10;
        let mut seqs: Vec<u64> = self.totals.values().map(|t| t.seq).collect();
        let cutoff = *seqs.select_nth_unstable(excess - 1).1;
        // Everything below the cutoff goes, then ties AT it until `excess`.
        let mut ties = excess - seqs.iter().filter(|s| **s < cutoff).count();
        let mut removed = HashSet::new();
        self.totals.retain(|k, t| {
            let drop = t.seq < cutoff
                || (t.seq == cutoff && ties > 0 && {
                    ties -= 1;
                    true
                });
            if drop {
                removed.insert(k.clone());
            }
            !drop
        });
        self.aliases
            .retain(|k, v| !removed.contains(k) && !removed.contains(v));
        self.dirty = true;
        tracing::debug!(
            "usage tailer: evicted {} native baseline(s) past the cap",
            removed.len()
        );
    }

    /// A provider can add response ids after its first usage snapshot. Retain
    /// the original ledger identity through any shared durable source alias.
    fn delta_aliased(
        &mut self,
        key: &str,
        aliases: &[String],
        usage: &ParsedUsage,
        cost: Option<f64>,
    ) -> Option<(ParsedUsage, Option<f64>)> {
        let canonical = std::iter::once(key)
            .chain(aliases.iter().map(String::as_str))
            .find_map(|alias| {
                self.aliases
                    .get(alias)
                    .cloned()
                    .or_else(|| self.totals.contains_key(alias).then(|| alias.to_owned()))
            })
            .unwrap_or_else(|| key.to_owned());
        for alias in std::iter::once(key).chain(aliases.iter().map(String::as_str)) {
            if self.aliases.get(alias) != Some(&canonical) {
                self.aliases.insert(alias.to_owned(), canonical.clone());
                self.dirty = true;
            }
        }
        self.delta(&canonical, usage, cost)
    }

    pub(super) fn delta(
        &mut self,
        key: &str,
        usage: &ParsedUsage,
        cost: Option<f64>,
    ) -> Option<(ParsedUsage, Option<f64>)> {
        if !self.totals.contains_key(key) {
            self.enforce_cap();
        }
        let old = self.totals.entry(key.to_owned()).or_default();
        let delta = ParsedUsage {
            model: usage.model.clone(),
            input: usage.input.saturating_sub(old.usage.input),
            output: usage.output.saturating_sub(old.usage.output),
            cache_read: usage.cache_read.saturating_sub(old.usage.cache_read),
            cache_write: usage.cache_write.saturating_sub(old.usage.cache_write),
        };
        // Match Grok's 1e-10 USD precision. Integer checkpoints avoid phantom
        // positive corrections caused by JSON float round-off after restart.
        const TICKS_PER_USD: f64 = 10_000_000_000.0;
        let cost = cost
            .filter(|v| v.is_finite() && *v >= 0.0)
            .map(|v| (v * TICKS_PER_USD).round() as u64);
        let cost_delta =
            cost.map(|v| v.saturating_sub(old.cost_ticks.unwrap_or(0)) as f64 / TICKS_PER_USD);
        old.usage.input = old.usage.input.max(usage.input);
        old.usage.output = old.usage.output.max(usage.output);
        old.usage.cache_read = old.usage.cache_read.max(usage.cache_read);
        old.usage.cache_write = old.usage.cache_write.max(usage.cache_write);
        old.usage.model.clone_from(&usage.model);
        if let Some(cost) = cost {
            old.cost_ticks = Some(old.cost_ticks.unwrap_or(0).max(cost));
        }
        if delta.input == 0
            && delta.output == 0
            && delta.cache_read == 0
            && delta.cache_write == 0
            && cost_delta.unwrap_or(0.0) == 0.0
        {
            return None;
        }
        // Only a baseline that grew moves to the back of the FIFO (it is
        // committed with the owed event anyway, so no extra checkpoint churn).
        self.next_seq += 1;
        old.seq = self.next_seq;
        Some((delta, cost_delta))
    }
}

impl super::UsageTailer {
    /// New provider history starts at byte zero; its baselines commit alongside
    /// the emitted events, so a restart cannot replay the historical backfill.
    pub(super) async fn scan_native(&mut self) -> Result<(), String> {
        self.scan_native_parts(true, true).await
    }

    /// [`Self::scan_native`] limited to the providers whose trees changed
    /// (an event-driven pass); the reconcile pass scans both.
    pub(super) async fn scan_native_parts(&mut self, grok: bool, agy: bool) -> Result<(), String> {
        if grok {
            self.scan_grok().await?;
        }
        if agy {
            self.scan_agy().await;
        }
        Ok(())
    }

    async fn scan_grok(&mut self) -> Result<(), String> {
        use otto_transcript::grok_usage::{
            discover_update_files, fallback_for_updates, parse_turn_completed_line,
        };
        let root = self.grok_root.clone();
        let files = match tokio::task::spawn_blocking(move || discover_update_files(&root)).await {
            Ok(Ok(files)) => files,
            failure => {
                tracing::warn!("usage tailer: Grok discovery unavailable: {failure:?}");
                Vec::new()
            }
        };
        for path in files {
            let size = match tokio::fs::metadata(&path).await {
                Ok(meta) => meta.len(),
                Err(_) => continue,
            };
            if self.cursors.get(&path) == Some(size) {
                continue;
            }
            let summary = tokio::fs::read_to_string(path.with_file_name("summary.json"))
                .await
                .ok();
            let fallback = fallback_for_updates(&path, summary.as_deref());
            loop {
                let fallback = fallback.clone();
                let chunk = self
                    .read_new_lines(&path, size, move |line| {
                        Some(parse_turn_completed_line(line, &fallback))
                    })
                    .await;
                let Some((rows, offset, more)) = (match chunk {
                    Ok(chunk) => chunk,
                    Err(error) => {
                        tracing::debug!("usage tailer: Grok {}: {error}", path.display());
                        break;
                    }
                }) else {
                    break;
                };
                for (_, rows) in rows {
                    for row in rows {
                        let Some(timestamp) = row.timestamp_rfc3339() else {
                            continue;
                        };
                        let usage = row.parsed_usage();
                        let cost = row.cost_usd().unwrap_or_else(|| {
                            crate::estimate_cost(
                                &usage.model,
                                usage.input,
                                usage.output,
                                usage.cache_read,
                                usage.cache_write,
                            )
                        });
                        let key = format!("grok:{}:{}", path.display(), row.dedupe_key);
                        let Some((delta, cost)) = self.native.delta(&key, &usage, Some(cost))
                        else {
                            continue;
                        };
                        let session = self.lookup_native("grok", &row.session_id).await;
                        self.pending_events.push(crate::UsageEvent {
                            ts: Some(timestamp),
                            workspace_id: session
                                .as_ref()
                                .map(|s| s.workspace_id.clone())
                                .unwrap_or_else(|| crate::EXTERNAL_WORKSPACE.into()),
                            session_id: session
                                .map(|s| s.otto_session_id)
                                .unwrap_or(row.session_id),
                            provider: "grok".into(),
                            model: delta.model,
                            kind: "completion".into(),
                            input_tokens: delta.input,
                            output_tokens: delta.output,
                            cache_read_tokens: delta.cache_read,
                            cache_write_tokens: delta.cache_write,
                            cost_usd: cost.unwrap_or(0.0),
                            duration_ms: 0,
                            ..Default::default()
                        });
                    }
                }
                self.set_cursor(&path, offset);
                if !more {
                    break;
                }
                self.persist().await?;
            }
        }
        Ok(())
    }

    async fn scan_agy(&mut self) {
        let home = self.home.clone();
        let extra = self.agy_extra_dir.clone();
        let files =
            match tokio::task::spawn_blocking(move || agy::discover(&home, extra.as_deref())).await
            {
                Ok(files) => files,
                Err(error) => {
                    tracing::warn!("usage tailer: Antigravity discovery failed: {error}");
                    return;
                }
            };
        self.forget_vanished_agy(&files);
        for (path, fingerprint) in files {
            let key = path.to_string_lossy().into_owned();
            let mark = match self.native.agy_files.get(&key) {
                Some(seen) if seen.fp == fingerprint => continue,
                Some(seen) => seen.mark.clone(),
                None => agy::Mark::default(),
            };
            let snapshot = match agy::read(&path, &mark).await {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    tracing::debug!("usage tailer: Antigravity {}: {error}", path.display());
                    continue;
                }
            };
            let mut session = self.lookup_native("agy", &snapshot.conversation).await;
            for ancestor in [snapshot.parent.as_deref(), snapshot.root.as_deref()]
                .into_iter()
                .flatten()
            {
                if session.is_none() {
                    session = self.lookup_native("agy", ancestor).await;
                }
            }
            for row in snapshot.rows {
                let cost = crate::estimate_cost(
                    &row.usage.model,
                    row.usage.input,
                    row.usage.output,
                    row.usage.cache_read,
                    row.usage.cache_write,
                );
                let Some((delta, cost)) =
                    self.native
                        .delta_aliased(&row.dedup_key, &row.aliases, &row.usage, Some(cost))
                else {
                    continue;
                };
                self.pending_events.push(crate::UsageEvent {
                    ts: Some(row.timestamp),
                    workspace_id: session
                        .as_ref()
                        .map(|s| s.workspace_id.clone())
                        .unwrap_or_else(|| crate::EXTERNAL_WORKSPACE.into()),
                    session_id: session
                        .as_ref()
                        .map(|s| s.otto_session_id.clone())
                        .unwrap_or_else(|| snapshot.conversation.clone()),
                    provider: "agy".into(),
                    model: delta.model,
                    kind: "completion".into(),
                    input_tokens: delta.input,
                    output_tokens: delta.output,
                    cache_read_tokens: delta.cache_read,
                    cache_write_tokens: delta.cache_write,
                    cost_usd: cost.unwrap_or(0.0),
                    duration_ms: 0,
                    ..Default::default()
                });
            }
            // A moved mark must commit (a restart resumes from it); a bare
            // fingerprint change rides along with the next commit — losing it
            // costs one re-read from the persisted mark, never a re-bill.
            if snapshot.mark != mark {
                self.native.dirty = true;
            }
            self.native.agy_files.insert(
                key,
                agy::FileState {
                    fp: fingerprint,
                    mark: snapshot.mark,
                },
            );
        }
        self.native.enforce_cap();
    }

    /// Drop the state of Antigravity DBs that no longer exist: their file
    /// entry, and — unless the same conversation is still discovered under
    /// another root — its baselines and aliases. A DB missing from this
    /// listing is only dropped after an explicit existence check, so a
    /// transient `read_dir` failure never resets a live conversation (which
    /// would re-bill its whole history).
    fn forget_vanished_agy(&mut self, files: &[(std::path::PathBuf, agy::Fingerprint)]) {
        let listed: HashSet<String> = files
            .iter()
            .map(|(p, _)| p.to_string_lossy().into_owned())
            .collect();
        let candidates: Vec<String> = self
            .native
            .agy_files
            .keys()
            .filter(|k| !listed.contains(*k))
            .cloned()
            .collect();
        if candidates.is_empty() {
            return;
        }
        let gone: Vec<String> = super::blocking_io(|| {
            candidates
                .into_iter()
                .filter(|k| !std::path::Path::new(k).exists())
                .collect()
        });
        let live_stems: HashSet<_> = files.iter().filter_map(|(p, _)| p.file_stem()).collect();
        let mut prefixes = Vec::new();
        for key in gone {
            self.native.agy_files.remove(&key);
            self.native.dirty = true;
            if let Some(stem) = std::path::Path::new(&key).file_stem() {
                if !live_stems.contains(stem) {
                    prefixes.push(format!("agy:{}:", stem.to_string_lossy()));
                }
            }
        }
        self.native.forget_prefixes(&prefixes);
    }

    async fn lookup_native(&mut self, provider: &str, id: &str) -> Option<super::SessionRef> {
        if let Some(session) = self
            .attribution()
            .await
            .by_provider_identity
            .get(&(provider.to_owned(), id.to_owned()))
        {
            return Some(session.clone());
        }
        if !self.refresh_after_miss().await {
            return None;
        }
        self.attr
            .as_ref()?
            .by_provider_identity
            .get(&(provider.to_owned(), id.to_owned()))
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(output: u64) -> ParsedUsage {
        ParsedUsage {
            model: "m".into(),
            input: 1,
            output,
            ..Default::default()
        }
    }

    #[test]
    fn the_cap_evicts_the_oldest_baselines_and_their_aliases() {
        // A pre-upgrade checkpoint: no `seq`, no `next_seq` — reads as oldest.
        let mut state: NativeState = serde_json::from_value(serde_json::json!({
            "totals": {"legacy": {"usage": usage(5), "cost_ticks": null}},
            "aliases": {"legacy-alias": "legacy"},
        }))
        .unwrap();
        for i in 0..10 {
            state
                .delta_aliased(&format!("k{i}"), &[format!("a{i}")], &usage(1), None)
                .unwrap();
        }
        // k0 grows again: it moves behind k1..k9 in the FIFO.
        state.delta("k0", &usage(2), None).unwrap();
        state.enforce_cap_at(8);
        // 11 entries, cap 8 → drop 3 + 0 (8 / 10): legacy, k1, k2.
        assert_eq!(state.totals.len(), 8);
        for gone in ["legacy", "k1", "k2"] {
            assert!(!state.totals.contains_key(gone), "{gone}");
        }
        assert!(state.totals.contains_key("k0"), "a grown baseline is young");
        assert!(!state.aliases.contains_key("legacy-alias"));
        assert!(!state.aliases.contains_key("a1"));
        assert_eq!(state.aliases.get("a3").map(String::as_str), Some("k3"));
        // The restored order survives a checkpoint round trip.
        let mut restored: NativeState =
            serde_json::from_value(serde_json::to_value(state).unwrap()).unwrap();
        restored.delta("k3", &usage(9), None).unwrap();
        restored.enforce_cap_at(6);
        assert!(restored.totals.contains_key("k3"));
        assert!(!restored.totals.contains_key("k4"));
    }

    #[test]
    fn a_deleted_grok_file_forgets_only_its_own_turns() {
        let mut state = NativeState::default();
        state.delta("grok:/s/a/updates.jsonl:t1", &usage(1), None);
        state.delta("grok:/s/a/updates.jsonl:t2", &usage(1), None);
        state.delta("grok:/s/ab/updates.jsonl:t1", &usage(1), None);
        state.delta("agy:c:step:1:f9", &usage(1), None);
        state.dirty = false;
        state.forget_grok_files(&["/s/a/updates.jsonl".to_string()]);
        let mut left: Vec<_> = state.totals.keys().cloned().collect();
        left.sort();
        assert_eq!(left, ["agy:c:step:1:f9", "grok:/s/ab/updates.jsonl:t1"]);
        assert!(state.dirty, "the pruned state must commit");
    }

    #[test]
    fn late_response_identity_does_not_rebill_a_source_step() {
        let mut state = NativeState::default();
        let usage = ParsedUsage {
            model: "m".into(),
            input: 100,
            output: 10,
            ..Default::default()
        };
        let source = "agy:c:step:1:f9".to_string();
        state
            .delta_aliased(&source, std::slice::from_ref(&source), &usage, Some(0.1))
            .unwrap();
        let response = "agy:c:resp:new";
        assert!(state
            .delta_aliased(response, std::slice::from_ref(&source), &usage, Some(0.1))
            .is_none());
        let mut restored: NativeState =
            serde_json::from_value(serde_json::to_value(state).unwrap()).unwrap();
        assert!(restored
            .delta_aliased(response, &[], &usage, Some(0.1))
            .is_none());
        let updated = ParsedUsage {
            output: 20,
            ..usage
        };
        let (delta, _) = restored
            .delta_aliased(response, &[source], &updated, Some(0.2))
            .unwrap();
        assert_eq!((delta.input, delta.output), (0, 10));
    }

    #[test]
    fn generation_first_step_later_retains_the_committed_baseline() {
        let mut state = NativeState::default();
        let usage = ParsedUsage {
            model: "m".into(),
            input: 100,
            output: 10,
            ..Default::default()
        };
        let generation = "agy:c:gen:0:f4".to_string();
        let response = "agy:c:resp:r1".to_string();
        state
            .delta_aliased(
                &generation,
                std::slice::from_ref(&response),
                &usage,
                Some(0.1),
            )
            .unwrap();
        let mut state: NativeState =
            serde_json::from_value(serde_json::to_value(state).unwrap()).unwrap();
        let step = "agy:c:step:1:f9";
        assert!(state
            .delta_aliased(step, &[generation, response], &usage, Some(0.1))
            .is_none());
        assert!(
            state.dirty,
            "the new alias must commit even when no new usage is owed"
        );
        let mut state: NativeState =
            serde_json::from_value(serde_json::to_value(state).unwrap()).unwrap();
        assert!(state.delta_aliased(step, &[], &usage, Some(0.1)).is_none());
    }

    #[test]
    fn native_history_replay_and_late_growth_only_bill_the_difference() {
        let mut state = NativeState::default();
        let first = ParsedUsage {
            model: "model".into(),
            input: 100,
            output: 10,
            cache_read: 50,
            cache_write: 0,
        };
        let (got, cost) = state.delta("grok:turn:model", &first, Some(0.25)).unwrap();
        assert_eq!(
            (got.input, got.output, got.cache_read, cost),
            (100, 10, 50, Some(0.25))
        );
        assert!(state.delta("grok:turn:model", &first, Some(0.25)).is_none());
        let encoded = serde_json::to_value(&state).unwrap();
        let mut restored: NativeState = serde_json::from_value(encoded).unwrap();
        assert!(restored
            .delta("grok:turn:model", &first, Some(0.25))
            .is_none());
        let final_usage = ParsedUsage {
            output: 25,
            ..first.clone()
        };
        let (got, cost) = restored
            .delta("grok:turn:model", &final_usage, Some(0.5))
            .unwrap();
        assert_eq!(
            (got.input, got.output, got.cache_read, cost),
            (0, 15, 0, Some(0.25))
        );
        assert!(
            restored
                .delta("grok:turn:model", &first, Some(0.25))
                .is_none(),
            "an older source copy must not reduce the durable baseline"
        );
        assert_eq!(
            restored
                .delta("agy:conversation:1", &first, None)
                .unwrap()
                .0
                .input,
            100,
            "different source records remain independent"
        );
    }
}
