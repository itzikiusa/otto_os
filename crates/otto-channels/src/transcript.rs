//! Tail a claude session JSONL transcript and emit structured events.
//!
//! Polls the file every ~300 ms from a saved byte offset. For each new
//! complete line it emits either a `Tool` event (assistant tool_use block) or
//! a `Final` event (assistant end_turn reply).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use tokio::io::{AsyncReadExt, AsyncSeekExt};

use crate::secrets_redact::redact_secrets;

/// An event emitted by the tailer.
#[derive(Debug, Clone, PartialEq)]
pub enum TranscriptEvent {
    /// The agent invoked a tool. `display` is the formatted label line:
    /// `"<emoji> <label>: <summary>"` (summary already truncated to ~70 chars),
    /// or just `"<emoji> <label>"` when the call carries a `code` preview.
    /// `code`, when set, is a multi-line command/payload the mirror renders as a
    /// formatted code block under the label (used for terminal calls). Any home
    /// directory in either field is abbreviated to `~`.
    Tool {
        name: String,
        display: String,
        code: Option<String>,
    },
    /// The agent finished a turn (stop_reason == "end_turn"). `text` is the
    /// concatenated text blocks.
    Final { text: String },
}

/// Poll interval while a turn is in flight (or no activity signal is wired).
const ACTIVE_POLL: Duration = Duration::from_millis(300);
/// Ceiling of the idle back-off: between turns the poll doubles from
/// [`ACTIVE_POLL`] up to this, so a parked channel session costs one `fstat`
/// every few seconds instead of an open+seek+read every 300 ms.
const IDLE_POLL_MAX: Duration = Duration::from_secs(3);

/// The tailer's poll timings (production: [`PROD_CADENCE`]; tests scale it
/// down to run in real time).
#[derive(Clone, Copy)]
struct Cadence {
    active: Duration,
    idle_max: Duration,
}

const PROD_CADENCE: Cadence = Cadence {
    active: ACTIVE_POLL,
    idle_max: IDLE_POLL_MAX,
};

impl Cadence {
    /// Delay before the next poll. `active` (a turn is in flight) keeps the
    /// fast cadence; otherwise each consecutive empty poll doubles the delay
    /// up to `idle_max` (`idle_polls` resets whenever new bytes arrive).
    fn delay(self, active: bool, idle_polls: u32) -> Duration {
        if active {
            return self.active;
        }
        self.active
            .saturating_mul(1u32 << idle_polls.min(8))
            .min(self.idle_max)
    }
}

/// Tail `path`, emitting events to `on_event` until `cancel` is set to `true`.
///
/// Polls every 300 ms from a saved byte offset; lines that do not match the
/// expected shapes are silently skipped. Only COMPLETE lines are consumed: a
/// line claude is still writing (no trailing `\n` yet) is left for the next
/// poll instead of being parsed half-written and lost for good.
///
/// `since` guards against replaying history: a tailer (re)attached to an
/// existing transcript — after a daemon restart, a liveness-probe stop, or a
/// map-miss recovery — would otherwise re-read it from byte 0 and re-emit
/// every earlier turn's `Final`, re-posting old answers into the thread. Lines
/// that already existed when the tailer started are emitted only when their
/// `timestamp` is at or after `since` (the moment this turn was attached);
/// lines appended afterwards are always emitted. `None` replays everything.
pub async fn tail(
    path: PathBuf,
    since: Option<DateTime<Utc>>,
    on_event: impl FnMut(TranscriptEvent),
    cancel: Arc<AtomicBool>,
) {
    tail_inner(path, since, on_event, cancel, None, None, PROD_CADENCE).await
}

/// [`tail`] with an activity signal: while `active` reads `true` (a turn is in
/// flight) the file is polled every 300 ms; between turns the poll backs off
/// to [`IDLE_POLL_MAX`], and a flip back to `true` (`Mirror::begin_turn`)
/// wakes the poller at once.
pub async fn tail_adaptive(
    path: PathBuf,
    since: Option<DateTime<Utc>>,
    on_event: impl FnMut(TranscriptEvent),
    cancel: Arc<AtomicBool>,
    active: tokio::sync::watch::Receiver<bool>,
) {
    tail_inner(
        path,
        since,
        on_event,
        cancel,
        Some(active),
        None,
        PROD_CADENCE,
    )
    .await
}

/// The tail loop. The file handle stays open between polls: each poll is one
/// `fstat` comparing the length against the consumed offset, and bytes are
/// read only when the file grew. The handle is reopened only when the file
/// shrank (truncated/replaced) or could not be stat'ed. `polls`, when set,
/// counts poll iterations (the idle-wakeup test's probe).
async fn tail_inner(
    path: PathBuf,
    since: Option<DateTime<Utc>>,
    mut on_event: impl FnMut(TranscriptEvent),
    cancel: Arc<AtomicBool>,
    mut active: Option<tokio::sync::watch::Receiver<bool>>,
    polls: Option<Arc<std::sync::atomic::AtomicU64>>,
    cadence: Cadence,
) {
    // Everything before this byte offset predates the tailer.
    let history_end: u64 = match since {
        Some(_) => tokio::fs::metadata(&path)
            .await
            .map(|m| m.len())
            .unwrap_or(0),
        None => 0,
    };
    // A (re)attach to a long-lived session used to read the WHOLE history into
    // memory and JSON-parse every line just to drop it by timestamp. Start at
    // the first line that can still be at/after `since` instead (found by a
    // bounded backwards scan, off the runtime).
    let mut offset: u64 = match since {
        Some(since) if history_end > 0 => {
            let p = path.clone();
            tokio::task::spawn_blocking(move || history_start(&p, history_end, since))
                .await
                .unwrap_or(0)
        }
        _ => 0,
    };

    let mut file: Option<tokio::fs::File> = None;
    let mut idle_polls: u32 = 0;
    // The activity sender was dropped: no more turns are coming.
    let mut orphaned = false;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        if let Some(p) = &polls {
            p.fetch_add(1, Ordering::Relaxed);
        }

        let mut got_bytes = false;
        if file.is_none() {
            file = tokio::fs::File::open(&path).await.ok();
        }
        if let Some(f) = file.as_mut() {
            match f.metadata().await.map(|m| m.len()) {
                Ok(len) if len > offset => {
                    let mut buf: Vec<u8> = Vec::new();
                    let read = f.seek(std::io::SeekFrom::Start(offset)).await.is_ok()
                        && f.read_to_end(&mut buf).await.is_ok();
                    if !read {
                        file = None;
                    } else if !buf.is_empty() {
                        got_bytes = true;
                        let (consumed, lines) = complete_lines(&buf, offset);
                        offset += consumed;
                        for (line_start, line) in lines {
                            if line_start < history_end && !line_at_or_after(line, since) {
                                continue;
                            }
                            if let Some(evt) = parse_line(line) {
                                on_event(evt);
                            }
                        }
                    }
                }
                Ok(len) if len < offset => {
                    // Truncated or replaced: start over on a fresh handle.
                    offset = 0;
                    file = None;
                }
                Ok(_) => {}
                Err(_) => file = None,
            }
        }

        idle_polls = if got_bytes {
            0
        } else {
            idle_polls.saturating_add(1)
        };
        match active.as_mut() {
            None if orphaned => tokio::time::sleep(cadence.idle_max).await,
            None => tokio::time::sleep(cadence.active).await,
            Some(rx) => {
                let is_active = *rx.borrow_and_update();
                let delay = cadence.delay(is_active, idle_polls);
                tokio::select! {
                    _ = tokio::time::sleep(delay) => {}
                    changed = rx.changed() => {
                        if changed.is_err() {
                            // The mirror dropped its side: poll at the idle
                            // ceiling until cancelled.
                            active = None;
                            orphaned = true;
                        } else {
                            idle_polls = 0;
                        }
                    }
                }
            }
        }
    }
}

/// Split `buf` (bytes read from file offset `start`) into its complete,
/// non-empty lines. Returns the number of bytes consumed (through the last
/// `\n`; a trailing partial line is NOT consumed) and each line with the file
/// offset it starts at. Invalid UTF-8 lines are skipped (their bytes still
/// count as consumed).
fn complete_lines(buf: &[u8], start: u64) -> (u64, Vec<(u64, &str)>) {
    let Some(last_nl) = buf.iter().rposition(|&b| b == b'\n') else {
        return (0, Vec::new());
    };
    let mut out = Vec::new();
    let mut pos = start;
    for raw in buf[..last_nl].split(|&b| b == b'\n') {
        let line_start = pos;
        pos += raw.len() as u64 + 1;
        if let Ok(line) = std::str::from_utf8(raw) {
            let line = line.trim();
            if !line.is_empty() {
                out.push((line_start, line));
            }
        }
    }
    ((last_nl + 1) as u64, out)
}

/// A line whose timestamp is this far before `since` proves that everything
/// before it predates `since` too (transcript timestamps only move forward;
/// the margin absorbs clock jitter between writers).
const HISTORY_MARGIN_SECS: i64 = 120;
const HISTORY_CHUNK: u64 = 256 * 1024;

/// Byte offset of a line start at or before the first history line (in
/// `[0, end)`) that could be at/after `since`: walk back from `end` one chunk
/// at a time until a chunk holds a line stamped more than
/// [`HISTORY_MARGIN_SECS`] before `since`, and start at that chunk's first
/// complete line. Lines from there on still go through [`line_at_or_after`],
/// so the emitted set is exactly the whole-file one; 0 (the whole file) when no
/// such line exists. Blocking IO.
fn history_start(path: &std::path::Path, end: u64, since: DateTime<Utc>) -> u64 {
    use std::io::{Read, Seek, SeekFrom};
    let cutoff = since - chrono::Duration::seconds(HISTORY_MARGIN_SECS);
    let Ok(mut f) = std::fs::File::open(path) else {
        return 0;
    };
    let mut hi = end;
    while hi > 0 {
        let lo = hi.saturating_sub(HISTORY_CHUNK);
        let mut buf = Vec::with_capacity((hi - lo) as usize);
        if f.seek(SeekFrom::Start(lo)).is_err()
            || (&mut f).take(hi - lo).read_to_end(&mut buf).is_err()
        {
            return 0;
        }
        // The first piece may be the tail of a line that started earlier.
        let first = if lo == 0 {
            0
        } else {
            match buf.iter().position(|&b| b == b'\n') {
                Some(nl) => nl + 1,
                None => {
                    hi = lo;
                    continue;
                }
            }
        };
        let older = buf[first..]
            .split(|&b| b == b'\n')
            .filter_map(|raw| std::str::from_utf8(raw).ok())
            .filter_map(line_timestamp)
            .any(|ts| ts < cutoff);
        if older {
            return lo + first as u64;
        }
        if lo == 0 {
            break;
        }
        // Next: the chunk ending where this one's first complete line starts
        // (so the straddling line is read whole) — always strictly earlier.
        hi = (lo + first as u64).min(hi - 1).max(lo);
    }
    0
}

/// The JSONL `line`'s top-level `timestamp`, if any.
fn line_timestamp(line: &str) -> Option<DateTime<Utc>> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let v = serde_json::from_str::<serde_json::Value>(line).ok()?;
    let t = v.get("timestamp")?.as_str()?;
    DateTime::parse_from_rfc3339(t)
        .ok()
        .map(|ts| ts.with_timezone(&Utc))
}

/// True when the JSONL `line`'s `timestamp` is at or after `since` (or there
/// is no cutoff). A line without a parseable timestamp counts as history.
fn line_at_or_after(line: &str, since: Option<DateTime<Utc>>) -> bool {
    let Some(since) = since else { return true };
    serde_json::from_str::<serde_json::Value>(line)
        .ok()
        .and_then(|v| {
            v.get("timestamp")
                .and_then(|t| t.as_str())
                .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
        })
        .is_some_and(|ts| ts.with_timezone(&Utc) >= since)
}

/// Parse one JSONL line into a `TranscriptEvent`, if it matches. Reads the
/// caller's home directory once so file paths/commands display as `~/…`.
fn parse_line(line: &str) -> Option<TranscriptEvent> {
    parse_line_with_home(line, home_dir().as_deref().unwrap_or(""))
}

/// The home-injectable core of [`parse_line`] (kept separate so it can be tested
/// deterministically without depending on the process `$HOME`).
fn parse_line_with_home(line: &str, home: &str) -> Option<TranscriptEvent> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    let msg = v.get("message")?;

    if msg.get("role").and_then(|r| r.as_str()) != Some("assistant") {
        return None;
    }

    let content = msg.get("content")?.as_array()?;
    let stop_reason = msg.get("stop_reason").and_then(|r| r.as_str());

    // --- end_turn → Final event ---
    if stop_reason == Some("end_turn") {
        let mut text = String::new();
        for block in content {
            if block.get("type").and_then(|t| t.as_str()) == Some("text") {
                if let Some(t) = block.get("text").and_then(|t| t.as_str()) {
                    if !t.is_empty() {
                        if !text.is_empty() {
                            text.push('\n');
                        }
                        text.push_str(t);
                    }
                }
            }
        }
        let text = text.trim().to_string();
        if text.is_empty() {
            return None;
        }
        return Some(TranscriptEvent::Final { text });
    }

    // --- tool_use blocks → Tool events ---
    // Emit one Tool event per tool_use block found.
    for block in content {
        if block.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
            let name = block
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or("unknown_tool")
                .to_string();
            let input = block
                .get("input")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            let (emoji, label) = emoji_label(&name);

            // A shell command renders as a formatted code block (`code`) under a
            // bare `<emoji> <label>` line — a readable preview of the call rather
            // than a one-line truncation. Everything else stays on one line:
            // `<emoji> <label>: <summary>` with the home dir abbreviated.
            if name == "Bash" {
                if let Some(cmd) = input.get("command").and_then(|v| v.as_str()) {
                    // Redact BEFORE truncating, so a cut can't leave half a
                    // secret that no longer matches a pattern.
                    let code = terminal_preview(&redact_secrets(cmd, false), home);
                    return Some(TranscriptEvent::Tool {
                        name,
                        display: format!("{emoji} {label}"),
                        code: Some(code),
                    });
                }
            }

            // The feed is posted to a shared channel: scrub tokens/passwords
            // (URLs with `?token=`, MCP args, …) before it leaves the machine.
            let summary = redact_secrets(&summarize_input(&name, &input, home), false);
            // Truncate summary to ~70 chars.
            let truncated_summary = truncate_chars(&summary, 70);
            let display = format!("{emoji} {label}: {truncated_summary}");
            // Only emit the first tool_use from a mid-turn line (the others
            // will arrive in subsequent lines or are batched — mirror loom).
            return Some(TranscriptEvent::Tool {
                name,
                display,
                code: None,
            });
        }
    }

    None
}

/// The caller's home directory (`$HOME`), or `None` when unset/empty. Used to
/// abbreviate paths to `~` in the activity feed.
fn home_dir() -> Option<String> {
    std::env::var("HOME").ok().filter(|h| !h.is_empty())
}

/// Replace the home directory prefix in `s` with `~` so the activity feed never
/// leaks the user's username. Handles both `"<home>/…"` → `"~/…"` (anywhere in
/// the string, e.g. inside a shell command) and a bare `"<home>"` equal to the
/// whole string → `"~"`. A trailing-slash match is required so an unrelated path
/// like `/Users/bobby` is never mangled by a `home` of `/Users/bob`.
fn abbreviate_home(s: &str, home: &str) -> String {
    if home.is_empty() {
        return s.to_string();
    }
    if s == home {
        return "~".to_string();
    }
    s.replace(&format!("{home}/"), "~/")
}

/// Render a shell command as a code-block preview: abbreviate the home dir and
/// cap it to a sane number of lines/chars so a giant heredoc can't blow the
/// feed's message budget. Appends `…` when truncated.
fn terminal_preview(cmd: &str, home: &str) -> String {
    const MAX_LINES: usize = 8;
    const MAX_CHARS: usize = 500;
    let abbreviated = abbreviate_home(cmd.trim(), home);

    // Cap lines first (a heredoc can be hundreds of lines), then total chars.
    let mut truncated = abbreviated.lines().count() > MAX_LINES;
    let mut out: String = abbreviated
        .lines()
        .take(MAX_LINES)
        .collect::<Vec<_>>()
        .join("\n");
    if out.chars().count() > MAX_CHARS {
        out = out.chars().take(MAX_CHARS).collect();
        truncated = true;
    }
    if truncated {
        out.push('…');
    }
    out
}

/// Return `(emoji, label)` for a tool name.
fn emoji_label(name: &str) -> (&'static str, String) {
    match name {
        "Read" | "Glob" => ("📖", "read".to_string()),
        "Write" => ("✍️", "write".to_string()),
        "Edit" | "MultiEdit" => ("✏️", "edit".to_string()),
        "Bash" | "KillShell" => ("💻", "terminal".to_string()),
        "Grep" => ("🔍", "search".to_string()),
        "WebSearch" | "WebFetch" => ("🌐", "web".to_string()),
        "Task" => ("🤖", "agent".to_string()),
        "TodoWrite" => ("📝", "plan".to_string()),
        other => {
            if let Some(rest) = other.strip_prefix("mcp__") {
                // e.g. "mcp__slack__send_message" → last segment = "send_message"
                let segment = rest.rsplit("__").next().unwrap_or(rest);
                ("⚙️", format!("{segment} (mcp)"))
            } else {
                ("🔧", other.to_string())
            }
        }
    }
}

/// Truncate a string to at most `max_chars` Unicode scalar values, appending
/// `…` when it is actually truncated.
use otto_core::text::clip_chars as truncate_chars;

/// Build a short human-readable summary of a tool call's input. Any home
/// directory in a path is abbreviated to `~` via `home`.
fn summarize_input(name: &str, input: &serde_json::Value, home: &str) -> String {
    match name {
        "Bash" | "KillShell" => {
            let cmd = input
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            format!("$ {}", abbreviate_home(cmd, home))
        }
        "Read" => {
            let path = input
                .get("file_path")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            abbreviate_home(path, home)
        }
        "Glob" => {
            let pattern = input.get("pattern").and_then(|v| v.as_str()).unwrap_or("?");
            abbreviate_home(pattern, home)
        }
        "Write" | "Edit" | "MultiEdit" => {
            let path = input
                .get("file_path")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            abbreviate_home(path, home)
        }
        "Grep" => {
            let pattern = input.get("pattern").and_then(|v| v.as_str()).unwrap_or("?");
            let path = input.get("path").and_then(|v| v.as_str()).unwrap_or("");
            if path.is_empty() {
                pattern.to_string()
            } else {
                format!("{pattern} in {}", abbreviate_home(path, home))
            }
        }
        "WebSearch" => input
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or("?")
            .to_string(),
        "WebFetch" => input
            .get("url")
            .and_then(|v| v.as_str())
            .unwrap_or("?")
            .to_string(),
        "Task" => input
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("sub-agent")
            .to_string(),
        "TodoWrite" => "update todos".to_string(),
        other => {
            // For mcp__ tools try to grab a sensible first param.
            if other.starts_with("mcp__") {
                if let Some(obj) = input.as_object() {
                    if let Some((_, v)) = obj.iter().next() {
                        if let Some(s) = v.as_str() {
                            return s.to_string();
                        }
                    }
                }
            }
            other.to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const HOME: &str = "/Users/itziklavon";

    fn tool_line(name: &str, input: serde_json::Value, home: &str) -> TranscriptEvent {
        let line = json!({
            "message": {
                "role": "assistant",
                "content": [{ "type": "tool_use", "name": name, "input": input }],
            }
        })
        .to_string();
        parse_line_with_home(&line, home).expect("tool_use line parses")
    }

    #[test]
    fn abbreviate_home_replaces_prefix_with_tilde() {
        assert_eq!(
            abbreviate_home("/Users/itziklavon/.hermes/cache/doc.md", HOME),
            "~/.hermes/cache/doc.md"
        );
    }

    #[test]
    fn abbreviate_home_handles_exact_home_and_no_match_and_empty() {
        // Whole string equal to home → "~".
        assert_eq!(abbreviate_home(HOME, HOME), "~");
        // Unrelated path is untouched.
        assert_eq!(abbreviate_home("/etc/hosts", HOME), "/etc/hosts");
        // Empty home disables abbreviation (no `$HOME` set).
        assert_eq!(
            abbreviate_home("/Users/itziklavon/x", ""),
            "/Users/itziklavon/x"
        );
    }

    #[test]
    fn abbreviate_home_does_not_mangle_sibling_prefix() {
        // A different user whose name starts with ours must NOT be abbreviated:
        // the trailing-slash guard prevents `/Users/itziklavon` matching
        // `/Users/itziklavon2`.
        assert_eq!(
            abbreviate_home("/Users/itziklavon2/secret", HOME),
            "/Users/itziklavon2/secret"
        );
    }

    #[test]
    fn abbreviate_home_replaces_every_occurrence_in_a_command() {
        let cmd = "cp /Users/itziklavon/a.txt /Users/itziklavon/b.txt";
        assert_eq!(abbreviate_home(cmd, HOME), "cp ~/a.txt ~/b.txt");
    }

    #[test]
    fn read_summary_abbreviates_home() {
        let evt = tool_line(
            "Read",
            json!({ "file_path": "/Users/itziklavon/.hermes/x.md" }),
            HOME,
        );
        assert_eq!(
            evt,
            TranscriptEvent::Tool {
                name: "Read".into(),
                display: "📖 read: ~/.hermes/x.md".into(),
                code: None,
            }
        );
    }

    #[test]
    fn bash_call_becomes_terminal_label_plus_code_block_preview() {
        let evt = tool_line(
            "Bash",
            json!({ "command": "python /Users/itziklavon/.hermes/support.py --flag" }),
            HOME,
        );
        match evt {
            TranscriptEvent::Tool {
                name,
                display,
                code,
            } => {
                assert_eq!(name, "Bash");
                // The label line carries no inline command — just the heading.
                assert_eq!(display, "💻 terminal");
                // The command is the code preview, home-abbreviated.
                assert_eq!(code.as_deref(), Some("python ~/.hermes/support.py --flag"));
            }
            other => panic!("expected a Tool event, got {other:?}"),
        }
    }

    #[test]
    fn terminal_preview_caps_long_multiline_commands() {
        let cmd = (0..40)
            .map(|i| format!("echo line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let preview = terminal_preview(&cmd, HOME);
        assert!(preview.ends_with('…'), "truncation marker appended");
        assert!(preview.lines().count() <= 9, "capped to ~8 lines (+ the …)");
        assert!(preview.contains("echo line 0"), "keeps the first lines");
        assert!(!preview.contains("echo line 39"), "drops the tail");
    }

    #[test]
    fn non_path_tools_are_unchanged() {
        // WebSearch has no path to abbreviate.
        let evt = tool_line("WebSearch", json!({ "query": "rust tokio select" }), HOME);
        assert_eq!(
            evt,
            TranscriptEvent::Tool {
                name: "WebSearch".into(),
                display: "🌐 web: rust tokio select".into(),
                code: None,
            }
        );
    }

    #[test]
    fn complete_lines_leaves_a_partial_trailing_line_unconsumed() {
        let buf = b"{\"a\":1}\n\n{\"b\":2}\n{\"c\":";
        let (consumed, lines) = complete_lines(buf, 100);
        // Consumed through the last newline only; `{"c":` waits for the next poll.
        assert_eq!(consumed, 17);
        assert_eq!(lines, vec![(100, "{\"a\":1}"), (109, "{\"b\":2}")]);
        // No newline at all → nothing consumed.
        assert_eq!(complete_lines(b"{\"half", 0), (0, Vec::new()));
    }

    #[test]
    fn history_start_skips_only_lines_that_cannot_be_at_or_after_since() {
        let dir = std::env::temp_dir().join(format!(
            "otto-history-start-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("t.jsonl");
        let since: DateTime<Utc> = "2026-09-12T10:00:00Z".parse().unwrap();
        // 4 000 old lines (~1 MB) an hour before `since`, then a few recent.
        let mut body = String::new();
        let pad = "x".repeat(200);
        for i in 0..4000 {
            body.push_str(&format!(
                "{{\"type\":\"assistant\",\"timestamp\":\"2026-09-12T09:00:{:02}Z\",\"pad\":\"{pad}\"}}\n",
                i % 60
            ));
        }
        let recent_at = body.len() as u64;
        body.push_str("{\"type\":\"assistant\",\"timestamp\":\"2026-09-12T09:59:30Z\"}\n");
        body.push_str("{\"type\":\"assistant\",\"timestamp\":\"2026-09-12T10:00:05Z\"}\n");
        std::fs::write(&p, &body).unwrap();
        let end = body.len() as u64;
        let start = history_start(&p, end, since);
        // Well past the start of the file, at a line boundary, and before
        // every line that could still count.
        assert!(start > 0 && start <= recent_at, "start {start}");
        assert!(start == 0 || body.as_bytes()[start as usize - 1] == b'\n');
        // Everything skipped is older than `since`.
        for line in body[..start as usize].lines() {
            assert!(!line_at_or_after(line, Some(since)));
        }
        // No timestamps at all → the whole file (the old behavior).
        std::fs::write(&p, "{\"a\":1}\n".repeat(50_000)).unwrap();
        assert_eq!(history_start(&p, 50_000 * 8, since), 0);
        // One huge line with no newline inside a chunk terminates too.
        let huge = format!("{{\"pad\":\"{}\"}}\n", "y".repeat(600_000));
        std::fs::write(&p, &huge).unwrap();
        assert_eq!(history_start(&p, huge.len() as u64, since), 0);
        // Missing file → 0.
        assert_eq!(history_start(&dir.join("none"), 10, since), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn line_at_or_after_compares_the_jsonl_timestamp() {
        let since = DateTime::parse_from_rfc3339("2026-09-07T10:59:22Z")
            .unwrap()
            .with_timezone(&Utc);
        let old = r#"{"timestamp":"2026-09-07T10:51:00.123Z","message":{}}"#;
        let new = r#"{"timestamp":"2026-09-07T10:59:25.000Z","message":{}}"#;
        let none = r#"{"message":{}}"#;
        assert!(!line_at_or_after(old, Some(since)));
        assert!(line_at_or_after(new, Some(since)));
        assert!(
            !line_at_or_after(none, Some(since)),
            "no timestamp = history"
        );
        assert!(line_at_or_after(old, None), "no cutoff replays everything");
    }

    /// A re-attached tailer must not re-post earlier turns' answers (CH-1),
    /// and must pick up a line that was half-written on the previous poll.
    #[tokio::test]
    async fn reattached_tail_skips_history_and_waits_for_whole_lines() {
        use std::io::Write;
        let path = std::env::temp_dir().join(format!(
            "otto-tail-test-{}-{}.jsonl",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        let final_line = |ts: &str, text: &str| {
            json!({
                "timestamp": ts,
                "message": {
                    "role": "assistant",
                    "stop_reason": "end_turn",
                    "content": [{ "type": "text", "text": text }],
                }
            })
            .to_string()
        };
        std::fs::write(
            &path,
            format!("{}\n", final_line("2026-01-01T00:00:00Z", "old answer")),
        )
        .unwrap();

        let since = chrono::Utc::now();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let task = tokio::spawn(tail(
            path.clone(),
            Some(since),
            move |e| {
                let _ = tx.send(e);
            },
            Arc::clone(&cancel),
        ));
        tokio::time::sleep(Duration::from_millis(400)).await;

        // Write the new turn's final in two halves across a poll.
        let new = final_line(&chrono::Utc::now().to_rfc3339(), "new answer");
        let (a, b) = new.split_at(new.len() / 2);
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        f.write_all(a.as_bytes()).unwrap();
        f.flush().unwrap();
        tokio::time::sleep(Duration::from_millis(500)).await;
        f.write_all(b.as_bytes()).unwrap();
        f.write_all(b"\n").unwrap();
        f.flush().unwrap();

        let got = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("new final emitted")
            .expect("channel open");
        assert_eq!(
            got,
            TranscriptEvent::Final {
                text: "new answer".into()
            }
        );
        cancel.store(true, Ordering::Relaxed);
        let _ = task.await;
        assert!(rx.try_recv().is_err(), "the old answer was never replayed");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn poll_delay_backs_off_only_between_turns() {
        assert_eq!(PROD_CADENCE.delay(true, 50), ACTIVE_POLL);
        assert_eq!(PROD_CADENCE.delay(false, 0), ACTIVE_POLL);
        assert_eq!(PROD_CADENCE.delay(false, 1), ACTIVE_POLL * 2);
        assert_eq!(PROD_CADENCE.delay(false, 3), Duration::from_millis(2400));
        assert_eq!(PROD_CADENCE.delay(false, 4), IDLE_POLL_MAX);
        assert_eq!(PROD_CADENCE.delay(false, u32::MAX), IDLE_POLL_MAX);
    }

    /// M1 idle-wakeup guard: a parked tailer (no turn in flight, file static)
    /// backs off to the idle ceiling — at production timings ≤1 poll per 3 s,
    /// inside the ≤1-per-2 s budget — and a new turn restores the fast cadence
    /// at once (no waiting out the back-off). Runs the production loop with
    /// the cadence scaled down 30× so it finishes in real time.
    #[tokio::test]
    async fn idle_tailer_backs_off_and_wakes_on_a_new_turn() {
        use std::sync::atomic::AtomicU64;
        let cadence = Cadence {
            active: Duration::from_millis(10),
            idle_max: Duration::from_millis(100),
        };
        let path = std::env::temp_dir().join(format!(
            "otto-tail-idle-{}-{}.jsonl",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        std::fs::write(&path, "{}\n").unwrap();
        let (active_tx, active_rx) = tokio::sync::watch::channel(false);
        let polls = Arc::new(AtomicU64::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let task = tokio::spawn(tail_inner(
            path.clone(),
            None,
            |_| {},
            Arc::clone(&cancel),
            Some(active_rx),
            Some(Arc::clone(&polls)),
            cadence,
        ));
        // Let the back-off ramp up, then measure a 1 s idle window: at the
        // 100 ms ceiling that is ~10 polls (the fast cadence would be ~100).
        tokio::time::sleep(Duration::from_millis(400)).await;
        let before = polls.load(Ordering::Relaxed);
        tokio::time::sleep(Duration::from_secs(1)).await;
        let idle = polls.load(Ordering::Relaxed) - before;
        assert!(
            idle <= 12,
            "idle tailer polled {idle} times in 1 s (ceiling 100 ms)"
        );

        // A new turn: the poller wakes immediately and runs at the fast pace.
        active_tx.send_replace(true);
        let before = polls.load(Ordering::Relaxed);
        tokio::time::sleep(Duration::from_millis(300)).await;
        let busy = polls.load(Ordering::Relaxed) - before;
        assert!(
            busy >= 10,
            "active tailer polled only {busy} times in 300 ms"
        );

        cancel.store(true, Ordering::Relaxed);
        drop(active_tx);
        let _ = task.await;
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn final_event_still_parses() {
        let line = json!({
            "message": {
                "role": "assistant",
                "stop_reason": "end_turn",
                "content": [{ "type": "text", "text": "all done" }],
            }
        })
        .to_string();
        assert_eq!(
            parse_line_with_home(&line, HOME),
            Some(TranscriptEvent::Final {
                text: "all done".into()
            })
        );
    }
}
