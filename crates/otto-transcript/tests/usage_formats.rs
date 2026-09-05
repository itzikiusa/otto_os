//! Frozen on-disk formats for the usage tailer's stores (design §4.1).
//!
//! `ottod/src/usage_tailer.rs` persists three JSON files under the daemon data
//! dir; this crate now owns their (de)serializers. Each fixture is a redacted
//! copy of a REAL file as written by the pre-extraction code — if a shape
//! changes, the daemon would silently restart from offset 0 (re-counting every
//! transcript) or lose its dedup memory, so these must keep loading verbatim.

use std::path::{Path, PathBuf};

use otto_transcript::usage::{CodexCounterStore, CursorStore, ParsedUsage, SeenKeys};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/usage")
        .join(name)
}

#[test]
fn cursor_store_loads_the_frozen_format() {
    // `{ "<abs path>": <byte offset>, … }`
    let store = CursorStore::load(fixture("usage_tailer.json"));
    assert_eq!(store.len(), 3);
    assert_eq!(
        store.get(Path::new(
            "/home/u/.claude/projects/-home-u-otto-os/cc7c6606-609d-4438-b8e2-02cb4e148651.jsonl"
        )),
        Some(65611)
    );
    // Round-trip through a temp copy keeps the exact shape.
    let dir = tempfile::tempdir().unwrap();
    let mut copy = CursorStore::load(dir.path().join("usage_tailer.json"));
    copy.set(Path::new("/a.jsonl"), 7);
    copy.save().unwrap();
    let raw: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.path().join("usage_tailer.json")).unwrap()).unwrap();
    assert_eq!(raw, serde_json::json!({ "/a.jsonl": 7 }));
}

#[test]
fn seen_keys_loads_the_frozen_format() {
    // `[ "<message.id>:<requestId>", … ]` in insertion order.
    let seen = SeenKeys::load(fixture("usage_tailer_seen.json"), 100_000);
    assert_eq!(seen.len(), 4);
    assert!(seen.contains("msg_01E4yhe7M5khxwmKQS61XomJ:req_011Cc8ZRRT49B3TGoQ17x1HR"));
    let dir = tempfile::tempdir().unwrap();
    let mut copy = SeenKeys::load(dir.path().join("seen.json"), 10);
    copy.insert("a:b");
    copy.save().unwrap();
    let raw: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.path().join("seen.json")).unwrap()).unwrap();
    assert_eq!(raw, serde_json::json!(["a:b"]));
}

#[test]
fn codex_counter_store_loads_the_frozen_format() {
    // `{ "counters": { "<session id>": {input, output, cache_read, cache_write} }, "order": [...] }`
    let store = CodexCounterStore::load(fixture("usage_tailer_codex_totals.json"), 20_000);
    assert_eq!(store.len(), 2);
    assert!(store.contains("01a06d82-5bea-7761-9449-919b19eb5da5"));
    // The persisted baseline is honoured: a snapshot equal to it emits nothing,
    // a larger one emits only the delta.
    let mut store = store;
    let same = ParsedUsage {
        model: "codex".into(),
        input: 388626,
        output: 29485,
        cache_read: 6709120,
        cache_write: 0,
    };
    assert_eq!(store.apply("01a06d82-5bea-7761-9449-919b19eb5da5", &same), None);
    let more = ParsedUsage {
        input: 388627,
        ..same.clone()
    };
    assert_eq!(
        store.apply("01a06d82-5bea-7761-9449-919b19eb5da5", &more).map(|d| d.input),
        Some(1)
    );
    let dir = tempfile::tempdir().unwrap();
    let mut copy = CodexCounterStore::load(dir.path().join("codex.json"), 10);
    copy.seed("s", &same);
    copy.save().unwrap();
    let raw: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.path().join("codex.json")).unwrap()).unwrap();
    assert_eq!(raw["order"], serde_json::json!(["s"]));
    assert_eq!(raw["counters"]["s"]["cache_read"], 6709120);
}
