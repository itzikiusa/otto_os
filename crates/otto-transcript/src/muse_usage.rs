//! Muse Code token usage, read from its session logs (verified on 1.4.20).
//!
//! Muse writes one append-only `session.jsonl` per session under
//! `~/.local/share/muse/sessions/YYYY/MM/DD/<session-id>/`; a subagent's log
//! sits under its parent's `subagent/<child-id>/` and bills to the parent.
//! The directory name is the session id Muse reports over ACP (`sessionId`,
//! a Threads session's native thread id) and in `.session.lock`.
//!
//! Every model call is one durable `runtime.session` record whose
//! `payload.event` is `model_completed`, with OpenAI-style usage:
//! `input_tokens` includes the cached prompt (`cache_read_tokens`, which
//! `cached_tokens` repeats). The `goal_usage_attribution` record beside it
//! repeats the same numbers and is not counted. Muse reports no cost and its
//! model catalog no price (`"cost": null`), so calls are tokens-only.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::usage::ParsedUsage;
use crate::util::format_rfc3339_millis;

/// One model call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MuseCall {
    /// RFC 3339 UTC (`recorded_at` is in microseconds).
    pub timestamp: String,
    pub usage: ParsedUsage,
}

/// Where Muse keeps its data: `~/.local/share/muse`.
pub fn default_muse_root(home: &Path) -> PathBuf {
    home.join(".local").join("share").join("muse")
}

/// Every `session.jsonl` under `root/sessions`, each with the top-level
/// session id it bills to. Muse's own bookkeeping dirs (`.msp-view-*`, …)
/// are dot-named and skipped.
pub fn discover_session_files(root: &Path) -> Vec<(PathBuf, String)> {
    let mut days = vec![root.join("sessions")];
    for _ in 0..3 {
        days = days.iter().flat_map(|d| subdirs(d)).collect();
    }
    let mut out = Vec::new();
    for day in days {
        for session in subdirs(&day) {
            if let Some(id) = session.file_name().and_then(|n| n.to_str()) {
                collect(&session, id, &mut out);
            }
        }
    }
    out.sort();
    out
}

fn collect(dir: &Path, owner: &str, out: &mut Vec<(PathBuf, String)>) {
    let log = dir.join("session.jsonl");
    if log.is_file() {
        out.push((log, owner.to_string()));
    }
    for child in subdirs(&dir.join("subagent")) {
        collect(&child, owner, out);
    }
}

fn subdirs(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .map(|e| e.path())
        .collect()
}

/// The usage of a `model_completed` line; `None` for any other line and for
/// a call that used no tokens (the `echo` test provider).
pub fn parse_model_completed_line(line: &str) -> Option<MuseCall> {
    if !line.contains("\"model_completed\"") {
        return None;
    }
    let record: Value = serde_json::from_str(line).ok()?;
    let event = record.pointer("/payload/event")?;
    if event.get("kind")?.as_str()? != "model_completed" {
        return None;
    }
    let usage = event.get("usage")?;
    let n = |key: &str| usage.get(key).and_then(Value::as_u64).unwrap_or(0);
    let cache_read = n("cache_read_tokens").max(n("cached_tokens"));
    let usage = ParsedUsage {
        model: event
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("muse")
            .to_string(),
        input: n("input_tokens").saturating_sub(cache_read),
        output: n("output_tokens"),
        cache_read,
        cache_write: n("cache_write_tokens"),
    };
    if usage.input + usage.output + usage.cache_read + usage.cache_write == 0 {
        return None;
    }
    let micros = record.get("recorded_at")?.as_i64()?;
    Some(MuseCall {
        timestamp: format_rfc3339_millis(micros.div_euclid(1000)),
        usage,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real Muse 1.4.20 lines (ids shortened).
    const CACHED: &str = r#"{"schema_version":1,"id":"01a1e401-b0f1","stream":{"kind":"session","id":"01a1e400-5934"},"sequence":123,"recorded_at":1794826678513891,"record_type":"event","durability":"durable","causation_id":null,"payload_type":"runtime.session","payload_schema_version":1,"payload":{"kind":"run","run_id":"01a1e401-7acb","event":{"kind":"model_completed","usage":{"input_tokens":48050,"output_tokens":206,"cached_tokens":44148,"cache_write_tokens":0,"cache_read_tokens":44148,"reasoning_tokens":0},"duration_ms":3203,"finish_reason":"tool_calls","model":"muse-wonder-full-1.0","estimated_prompt_tokens":48050},"source_run_record_id":"c3b6806b","source_run_record_sequence":42}}"#;
    const ATTRIBUTION: &str = r#"{"schema_version":1,"id":"01a1e401-a32a","stream":{"kind":"session","id":"01a1e400-5934"},"sequence":78,"recorded_at":1794826674986702,"record_type":"event","durability":"durable","causation_id":null,"payload_type":"runtime.session","payload_schema_version":1,"payload":{"kind":"run","run_id":"01a1e401-7acb","event":{"kind":"goal_usage_attribution","record":{"schema_version":1,"usage_id":"usage-622c","usage_family":"provider","quantity":{"unit":"tokens","reported":true,"input_tokens":44216,"output_tokens":271,"cached_tokens":0,"reasoning_tokens":78,"main_llm_steps":1},"owner":{"requester_kind":"main","session_id":"01a1e400-5934","run_id":"01a1e401-7acb","owner_id":"main-root","owner_type":"main_root"},"goal_attribution":{"mode":"none"}}},"source_run_record_id":"622c2590","source_run_record_sequence":25}}"#;
    const ECHO: &str = r#"{"schema_version":1,"id":"01a1e335-c0de","stream":{"kind":"session","id":"01a1e335-bc38"},"sequence":40,"recorded_at":1794813312138008,"record_type":"event","durability":"durable","causation_id":null,"payload_type":"runtime.session","payload_schema_version":1,"payload":{"kind":"run","run_id":"01a1e335-aaaa","event":{"kind":"model_completed","usage":{"input_tokens":0,"output_tokens":0,"cached_tokens":0,"reasoning_tokens":0},"duration_ms":6,"estimated_prompt_tokens":9},"source_run_record_id":"1cdab8f3","source_run_record_sequence":24}}"#;

    #[test]
    fn a_model_call_splits_the_cached_prompt_out_of_input() {
        let call = parse_model_completed_line(CACHED).unwrap();
        assert_eq!(call.timestamp, "2026-11-16T10:57:58.513Z");
        assert_eq!(
            call.usage,
            ParsedUsage {
                model: "muse-wonder-full-1.0".into(),
                input: 48050 - 44148,
                output: 206,
                cache_read: 44148,
                cache_write: 0,
            }
        );
    }

    #[test]
    fn the_attribution_echo_and_other_lines_count_nothing() {
        assert_eq!(parse_model_completed_line(ATTRIBUTION), None);
        assert_eq!(parse_model_completed_line(ECHO), None);
        assert_eq!(
            parse_model_completed_line("not json \"model_completed\""),
            None
        );
        assert_eq!(
            parse_model_completed_line(r#"{"retained_marker":"x"}"#),
            None
        );
    }

    #[test]
    fn subagent_logs_bill_to_their_top_level_session() {
        let root = tempfile::tempdir().unwrap();
        let day = root.path().join("sessions/2026/11/16");
        let parent = day.join("parent-1");
        let child = parent.join("subagent/child-1");
        let grandchild = child.join("subagent/child-2");
        for dir in [&parent, &child, &grandchild] {
            std::fs::create_dir_all(dir).unwrap();
            std::fs::write(dir.join("session.jsonl"), "").unwrap();
        }
        // A session dir with no log yet, and Muse's own view dir.
        std::fs::create_dir_all(day.join("starting")).unwrap();
        let view = root.path().join("sessions/.msp-view-g12/parent-1");
        std::fs::create_dir_all(&view).unwrap();
        std::fs::write(view.join("session.jsonl"), "").unwrap();

        let got = discover_session_files(root.path());
        let owners: Vec<&str> = got.iter().map(|(_, o)| o.as_str()).collect();
        assert_eq!(owners, ["parent-1", "parent-1", "parent-1"]);
        assert!(got.iter().all(|(p, _)| p.starts_with(&parent)));
    }
}
