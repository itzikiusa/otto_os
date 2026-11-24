//! Long transcripts open on a window (`otto_transcript::window`): the newest
//! records before a cursor, folded with absolute record numbers, so a 60 MB
//! session's Chat opens on its recent turns instead of folding (and being
//! refused for) the whole file, and every turn paged in window by window is
//! exactly the turn a whole-file fold produces.
use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use otto_transcript::window::{fold_window, line_index, WINDOW_BYTES};
use otto_transcript::{fold_file, fold_file_limited, FoldOpts, Provider};
use serde_json::Value;

#[path = "support/long_session.rs"]
mod long_session;

/// Peak live heap, so the test can say what reading a window costs.
struct Counting;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let now = LIVE.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
        PEAK.fetch_max(now, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) }
    }
}
#[global_allocator]
static ALLOC: Counting = Counting;
/// Tests in this binary run one at a time so the peak is this test's own.
static SERIAL: Mutex<()> = Mutex::new(());

fn peak_of<T>(f: impl FnOnce() -> T) -> (T, usize) {
    let base = LIVE.load(Ordering::Relaxed);
    PEAK.store(base, Ordering::Relaxed);
    let out = f();
    (out, PEAK.load(Ordering::Relaxed).saturating_sub(base))
}

#[test]
fn a_long_session_opens_on_its_newest_window_within_a_bounded_peak() {
    let _serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let path = dir
        .path()
        .join("5e55e55e-0000-4000-8000-000000000001.jsonl");
    let long = long_session::write_long_claude_session(&path, 64 << 20, 20_000);
    // The cause: the whole-file read refuses this session outright.
    let err = fold_file_limited(
        Provider::Claude,
        &path,
        FoldOpts::default(),
        Default::default(),
    )
    .unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::OutOfMemory);

    let t0 = std::time::Instant::now();
    let (w, peak) = peak_of(|| {
        fold_window(
            Provider::Claude,
            &path,
            FoldOpts::default(),
            None,
            WINDOW_BYTES,
        )
        .unwrap()
    });
    let first_open = t0.elapsed();
    let t1 = std::time::Instant::now();
    fold_window(
        Provider::Claude,
        &path,
        FoldOpts::default(),
        None,
        WINDOW_BYTES,
    )
    .unwrap();
    let parsed = w.end - w.start;
    eprintln!(
        "file {} B / {} records; window parsed {parsed} B from record {}; peak heap {peak} B; \
         first open {first_open:?} (index scan + fold), reopen {:?}",
        long.bytes,
        long.records,
        w.first,
        t1.elapsed()
    );
    assert!(parsed <= WINDOW_BYTES, "only the window is parsed");
    assert!(
        peak < 24 << 20,
        "peak heap {peak} B for a {} B file",
        long.bytes
    );
    assert_eq!(
        w.folder.record_count(),
        long.records,
        "absolute record numbers"
    );
    let page = w.folder.page(None, 60, vec![]);
    assert!(page.has_earlier);
    let last = serde_json::to_string(page.turns.last().unwrap()).unwrap();
    assert!(last.contains(&format!("toolu-{}-r119", long.exchanges)));
    // Totals of a window are not the conversation's: none are claimed.
    assert_eq!(page.stats.cost_usd, None);

    // Reopening scans only what was appended since (nothing here).
    let index = line_index(&path).unwrap();
    assert_eq!(index.end, long.bytes);
    assert_eq!(index.starts.len(), long.records);
}

/// Page `path` window by window, newest first, and compare every turn with
/// the same turn of a whole-file fold: nothing lost, nothing twice, nothing
/// different.
fn windows_match_whole_file(provider: Provider, path: &Path) {
    let whole = fold_file(provider, path, FoldOpts::default()).unwrap();
    let want: std::collections::HashMap<String, Value> = whole
        .turns
        .iter()
        .map(|t| (t.turn.id.clone(), serde_json::to_value(&t.turn).unwrap()))
        .collect();
    let (mut before, mut seen, mut windows) = (None, HashSet::new(), 0);
    loop {
        let w = fold_window(provider, path, FoldOpts::default(), before, WINDOW_BYTES).unwrap();
        windows += 1;
        let mut page_before = None;
        loop {
            let page = w.folder.page(page_before, 60, vec![]);
            for t in &page.turns {
                assert!(seen.insert(t.id.clone()), "{} paged twice", t.id);
                assert_eq!(
                    &serde_json::to_value(t).unwrap(),
                    &want[&t.id],
                    "turn {}",
                    t.id
                );
            }
            let cursor: usize = page.cursor.parse().unwrap();
            if cursor <= w.base || !page.has_earlier {
                before = page.has_earlier.then_some(cursor);
                break;
            }
            page_before = Some(cursor);
        }
        if before.is_none() {
            break;
        }
    }
    assert!(windows > 2, "{}: several windows", path.display());
    assert_eq!(seen.len(), want.len(), "every turn paged once");
}

#[test]
fn claude_windows_page_exactly_the_whole_file_fold() {
    let _serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let path = dir
        .path()
        .join("5e55e55e-0000-4000-8000-000000000001.jsonl");
    long_session::write_long_claude_session(&path, 14 << 20, 4_000);
    windows_match_whole_file(Provider::Claude, &path);
}

#[test]
fn codex_windows_page_exactly_the_whole_file_fold() {
    let _serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let path = dir
        .path()
        .join("rollout-2026-11-24T10-00-00-5e55e55e-0000-4000-8000-000000000002.jsonl");
    let big = "x".repeat(200 * 1024);
    let mut s = String::from(
        r#"{"timestamp":"2026-11-24T10:00:00Z","type":"session_meta","payload":{"id":"5e55e55e-0000-4000-8000-000000000002","cwd":"/work/repo","cli_version":"0.150.0"}}"#,
    );
    s.push('\n');
    for n in 0..1_400 {
        let ts = format!("2026-11-24T10:{:02}:{:02}Z", n / 60 % 60, n % 60);
        let out = if n % 10 == 3 { big.as_str() } else { "ok" };
        for line in [
            format!(
                r#"{{"timestamp":"{ts}","type":"event_msg","payload":{{"type":"user_message","message":"prompt {n}"}}}}"#
            ),
            format!(
                r#"{{"timestamp":"{ts}","type":"response_item","payload":{{"type":"message","role":"user","content":[{{"type":"input_text","text":"prompt {n}"}}]}}}}"#
            ),
            format!(
                r#"{{"timestamp":"{ts}","type":"response_item","payload":{{"type":"function_call","name":"exec_command","call_id":"call_{n}","arguments":"{{\"cmd\":\"cargo test {n}\"}}"}}}}"#
            ),
            format!(
                r#"{{"timestamp":"{ts}","type":"response_item","payload":{{"type":"function_call_output","call_id":"call_{n}","output":"{out}"}}}}"#
            ),
            format!(
                r#"{{"timestamp":"{ts}","type":"event_msg","payload":{{"type":"agent_message","message":"done {n}"}}}}"#
            ),
            format!(
                r#"{{"timestamp":"{ts}","type":"response_item","payload":{{"type":"message","role":"assistant","content":[{{"type":"output_text","text":"done {n}"}}]}}}}"#
            ),
            format!(
                r#"{{"timestamp":"{ts}","type":"event_msg","payload":{{"type":"token_count","info":{{"total_token_usage":{{"input_tokens":{},"output_tokens":{}}}}}}}}}"#,
                1000 * (n + 1),
                10 * (n + 1)
            ),
        ] {
            s.push_str(&line);
            s.push('\n');
        }
    }
    std::fs::write(&path, s).unwrap();
    windows_match_whole_file(Provider::Codex, &path);
}

#[test]
fn agy_windows_page_exactly_the_whole_file_fold() {
    let _serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agy").join("transcript_full.jsonl");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let pad = "y".repeat(30 * 1024);
    let mut s = String::new();
    for n in 0..1_200 {
        let ts = format!("2026-11-24T11:{:02}:{:02}Z", n / 60 % 60, n % 60);
        s.push_str(&format!(
            r#"{{"type":"USER_INPUT","step_index":{},"created_at":"{ts}","content":"prompt {n}"}}"#,
            2 * n
        ));
        s.push('\n');
        s.push_str(&format!(r#"{{"type":"PLANNER_RESPONSE","step_index":{},"created_at":"{ts}","model":"gemini-3-pro","content":"answer {n} {pad}"}}"#, 2 * n + 1));
        s.push('\n');
    }
    std::fs::write(&path, s).unwrap();
    windows_match_whole_file(Provider::Agy, &path);
}


/// A window whose base is its first record (the `None => first` case: a
/// 64 MiB span with one turn) but whose only turn starts after a sidecar
/// record: paging before that turn must step the cursor back to the base,
/// not hand the client the same cursor again (a "Load earlier" loop).
#[test]
fn a_page_with_no_turn_before_the_cursor_steps_back_to_the_base() {
    let mut folder = otto_transcript::Folder::new(Provider::Claude, FoldOpts::default());
    folder.start_at(100);
    folder.seed_bytes(
        concat!(
            r#"{"type":"permission-mode","permissionMode":"default","sessionId":"s"}"#,
            "\n",
            r#"{"type":"user","uuid":"u1","parentUuid":null,"sessionId":"s","timestamp":"2026-11-01T00:00:00.000Z","message":{"role":"user","content":"hello"}}"#,
            "\n",
        )
        .as_bytes(),
    );
    folder.set_base(100);
    let first = folder.turn_first(0).unwrap();
    assert!(first > 100, "the turn starts after the sidecar record");
    let page = folder.page(Some(first), 20, vec![]);
    assert!(page.turns.is_empty() && page.has_earlier);
    assert_eq!(page.cursor, "100");
}
