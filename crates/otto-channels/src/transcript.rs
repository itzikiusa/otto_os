//! Tail a claude session JSONL transcript and emit structured events.
//!
//! Polls the file every ~300 ms from a saved byte offset. For each new
//! complete line it emits either a `Tool` event (assistant tool_use block) or
//! a `Final` event (assistant end_turn reply).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncSeekExt};

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

/// Tail `path`, emitting events to `on_event` until `cancel` is set to `true`.
///
/// Polls every 300 ms from a saved byte offset; lines that do not match the
/// expected shapes are silently skipped.
pub async fn tail(
    path: PathBuf,
    mut on_event: impl FnMut(TranscriptEvent),
    cancel: Arc<AtomicBool>,
) {
    let mut offset: u64 = 0;
    let poll = Duration::from_millis(300);

    loop {
        if cancel.load(Ordering::Relaxed) {
            return;
        }

        // Try to read any new bytes appended since the last poll.
        if let Ok(mut f) = tokio::fs::File::open(&path).await {
            if f.seek(std::io::SeekFrom::Start(offset)).await.is_ok() {
                let mut buf = String::new();
                if f.read_to_string(&mut buf).await.is_ok() && !buf.is_empty() {
                    offset += buf.len() as u64;
                    for line in buf.lines() {
                        let line = line.trim();
                        if line.is_empty() {
                            continue;
                        }
                        if let Some(evt) = parse_line(line) {
                            on_event(evt);
                        }
                    }
                }
            }
        }

        tokio::time::sleep(poll).await;
    }
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
                    let code = terminal_preview(cmd, home);
                    return Some(TranscriptEvent::Tool {
                        name,
                        display: format!("{emoji} {label}"),
                        code: Some(code),
                    });
                }
            }

            let summary = summarize_input(&name, &input, home);
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
fn truncate_chars(s: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (count, c) in s.chars().enumerate() {
        if count >= max_chars {
            out.push('…');
            return out;
        }
        out.push(c);
    }
    out
}

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
        assert_eq!(abbreviate_home("/Users/itziklavon/x", ""), "/Users/itziklavon/x");
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
        let evt = tool_line("Read", json!({ "file_path": "/Users/itziklavon/.hermes/x.md" }), HOME);
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
            TranscriptEvent::Tool { name, display, code } => {
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
        let cmd = (0..40).map(|i| format!("echo line {i}")).collect::<Vec<_>>().join("\n");
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
            Some(TranscriptEvent::Final { text: "all done".into() })
        );
    }
}
