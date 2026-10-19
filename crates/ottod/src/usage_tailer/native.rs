//! Accounting for provider records whose usage can be updated in place.
//! The checkpoint commits with the owed events through the tailer's journal.

use otto_usage::ParsedUsage;

#[path = "native/agy.rs"]
mod agy;
use std::collections::HashMap;

#[derive(Default, serde::Serialize, serde::Deserialize)]
pub(super) struct NativeState {
    totals: HashMap<String, NativeTotals>,
    #[serde(default)]
    aliases: HashMap<String, String>,
    #[serde(skip)]
    pub(super) dirty: bool,
    #[serde(skip)]
    agy_files: HashMap<std::path::PathBuf, agy::Fingerprint>,
}

#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
struct NativeTotals {
    usage: ParsedUsage,
    cost_ticks: Option<u64>,
}

impl NativeState {
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
        Some((delta, cost_delta))
    }
}

impl super::UsageTailer {
    /// New provider history starts at byte zero; its baselines commit alongside
    /// the emitted events, so a restart cannot replay the historical backfill.
    pub(super) async fn scan_native(&mut self) -> Result<(), String> {
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
                            otto_usage::estimate_cost(
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
                        self.pending_events.push(otto_usage::UsageEvent {
                            ts: Some(timestamp),
                            workspace_id: session
                                .as_ref()
                                .map(|s| s.workspace_id.clone())
                                .unwrap_or_else(|| otto_usage::EXTERNAL_WORKSPACE.into()),
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
        self.scan_agy().await;
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
        for (path, fingerprint) in files {
            if self.native.agy_files.get(&path) == Some(&fingerprint) {
                continue;
            }
            let snapshot = match agy::read(&path).await {
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
                let cost = otto_usage::estimate_cost(
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
                self.pending_events.push(otto_usage::UsageEvent {
                    ts: Some(row.timestamp),
                    workspace_id: session
                        .as_ref()
                        .map(|s| s.workspace_id.clone())
                        .unwrap_or_else(|| otto_usage::EXTERNAL_WORKSPACE.into()),
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
            self.native.agy_files.insert(path, fingerprint);
        }
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
