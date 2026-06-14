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
#[derive(Debug, Clone)]
pub enum TranscriptEvent {
    /// The agent invoked a tool. `display` is the fully formatted line:
    /// `"<emoji> <label>: <summary>"` (summary already truncated to ~70 chars).
    Tool { name: String, display: String },
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

/// Parse one JSONL line into a `TranscriptEvent`, if it matches.
fn parse_line(line: &str) -> Option<TranscriptEvent> {
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
            let summary = summarize_input(&name, &input);
            let (emoji, label) = emoji_label(&name);
            // Truncate summary to ~70 chars.
            let truncated_summary = truncate_chars(&summary, 70);
            let display = format!("{emoji} {label}: {truncated_summary}");
            // Only emit the first tool_use from a mid-turn line (the others
            // will arrive in subsequent lines or are batched — mirror loom).
            return Some(TranscriptEvent::Tool { name, display });
        }
    }

    None
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
    let mut chars = s.chars();
    let mut out = String::new();
    let mut count = 0;
    while let Some(c) = chars.next() {
        if count >= max_chars {
            out.push('…');
            return out;
        }
        out.push(c);
        count += 1;
    }
    out
}

/// Build a short human-readable summary of a tool call's input.
fn summarize_input(name: &str, input: &serde_json::Value) -> String {
    match name {
        "Bash" | "KillShell" => {
            let cmd = input
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            format!("$ {cmd}")
        }
        "Read" => {
            let path = input
                .get("file_path")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            path.to_string()
        }
        "Glob" => {
            let pattern = input.get("pattern").and_then(|v| v.as_str()).unwrap_or("?");
            pattern.to_string()
        }
        "Write" | "Edit" | "MultiEdit" => {
            let path = input
                .get("file_path")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            path.to_string()
        }
        "Grep" => {
            let pattern = input.get("pattern").and_then(|v| v.as_str()).unwrap_or("?");
            let path = input.get("path").and_then(|v| v.as_str()).unwrap_or("");
            if path.is_empty() {
                pattern.to_string()
            } else {
                format!("{pattern} in {path}")
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
