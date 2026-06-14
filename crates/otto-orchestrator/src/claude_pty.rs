//! Drive a REAL interactive `claude` CLI session inside a PTY and capture
//! the assistant's reply from claude's own session JSONL transcript —
//! loom's approach, replacing one-shot `claude -p`.
//!
//! Flow:
//!  1. Pick a fresh v4 UUID and spawn
//!     `claude --session-id <uuid> --dangerously-skip-permissions [--model m]`
//!     in a PTY rooted at the workspace cwd.
//!  2. Wait for the TUI to draw and settle, then "type" the prompt
//!     (bracketed paste, so multi-line prompts stay one message) and press
//!     Enter.
//!  3. Poll `~/.claude/projects/<enc(cwd)>/<uuid>.jsonl` until an assistant
//!     message with `stop_reason == "end_turn"` appears; return its
//!     concatenated text blocks. The encoding of `cwd` replaces EVERY
//!     non-alphanumeric character with '-' (claude's own convention, e.g.
//!     `/Users/x/My Dir` → `-Users-x-My-Dir`).
//!  4. Kill the PTY — the planner session is single-shot.
//!
//! Knowing the session id upfront (via `--session-id`) lets us locate the
//! exact JSONL file directly: no "most recently modified" guessing that
//! races with other claude sessions writing in the same project dir.

use std::path::PathBuf;
use std::time::Duration;

use otto_core::{Error, Result};
use otto_pty::{CommandSpec, PtyHandle};

/// Poll cadence for TUI readiness and JSONL appearance. 250ms gives
/// chat-style latency without burning CPU (same cadence loom uses).
const POLL: Duration = Duration::from_millis(250);
/// Max wait for the TUI to draw anything before we type the prompt anyway
/// (claude buffers typed input during startup, so typing early is safe).
const STARTUP_WAIT: Duration = Duration::from_secs(20);
/// Quiet window after the last output chunk we treat as "TUI settled".
const SETTLE: Duration = Duration::from_millis(600);
/// Pause between pasting the prompt and pressing Enter, so the TUI has
/// processed the paste before the submit keypress arrives.
const PASTE_TO_ENTER: Duration = Duration::from_millis(200);

/// A one-shot prompt runner backed by a real interactive claude session.
pub struct ClaudePty {
    bin: String,
}

impl ClaudePty {
    pub fn new(bin: impl Into<String>) -> Self {
        Self { bin: bin.into() }
    }

    /// Run one prompt through a fresh interactive claude session in `cwd`
    /// and return the assistant's reply text (from the session JSONL).
    ///
    /// The session always runs with `--dangerously-skip-permissions` so it
    /// never stalls on an approval prompt nobody can answer.
    pub async fn run_prompt(
        &self,
        prompt: &str,
        cwd: &str,
        model: Option<&str>,
        timeout: Duration,
    ) -> Result<String> {
        let sid = uuid::Uuid::new_v4().to_string();
        let mut args = vec![
            "--session-id".to_string(),
            sid.clone(),
            "--dangerously-skip-permissions".to_string(),
        ];
        if let Some(m) = model {
            if !m.trim().is_empty() {
                args.push("--model".to_string());
                args.push(m.trim().to_string());
            }
        }
        let spec = CommandSpec {
            program: self.bin.clone(),
            args,
            cwd: Some(cwd.to_string()),
            env: vec![],
        };
        let handle = PtyHandle::spawn(&spec)?;
        let result = drive(&handle, prompt, cwd, &sid, timeout).await;
        // Single-shot session: always tear the PTY down, success or not.
        let _ = handle.kill();
        result
    }
}

/// Type the prompt into a freshly-spawned claude TUI and wait for the
/// completed assistant turn to land in the session JSONL.
async fn drive(
    handle: &PtyHandle,
    prompt: &str,
    cwd: &str,
    sid: &str,
    timeout: Duration,
) -> Result<String> {
    let deadline = tokio::time::Instant::now() + timeout;
    let exit_rx = handle.on_exit();

    // 1. Wait for the TUI to draw and go quiet before typing.
    let settle_deadline = tokio::time::Instant::now() + STARTUP_WAIT;
    loop {
        if exit_rx.borrow().is_some() {
            return Err(Error::Upstream(
                "claude exited before accepting input (is the claude CLI installed and logged in?)"
                    .into(),
            ));
        }
        if !handle.scrollback(1).is_empty() && handle.last_output_at().elapsed() >= SETTLE {
            break;
        }
        if tokio::time::Instant::now() >= settle_deadline {
            break; // type anyway — claude buffers early input
        }
        tokio::time::sleep(POLL).await;
    }

    // 2. "Type" the prompt. Bracketed paste keeps multi-line prompts as a
    //    single message instead of submitting on the first newline.
    handle.write(format!("\x1b[200~{prompt}\x1b[201~").as_bytes())?;
    tokio::time::sleep(PASTE_TO_ENTER).await;
    handle.write(b"\r")?;

    // 3. Poll the session transcript until the turn completes. The JSONL
    //    can take tens of seconds to appear on cold spawn (hook init + MCP
    //    load happen first), so "file missing" is just "not yet".
    let path = session_jsonl_path(cwd, sid);
    loop {
        if let Ok(content) = tokio::fs::read_to_string(&path).await {
            if let Some(text) = completed_turn_text(&content) {
                return Ok(text);
            }
        }
        if exit_rx.borrow().is_some() {
            // Final lines may have landed right at exit — one last read.
            if let Ok(content) = tokio::fs::read_to_string(&path).await {
                if let Some(text) = completed_turn_text(&content) {
                    return Ok(text);
                }
            }
            return Err(Error::Upstream(
                "claude exited before completing a reply".into(),
            ));
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(Error::Upstream(format!(
                "claude session timed out after {}s waiting for a reply",
                timeout.as_secs()
            )));
        }
        tokio::time::sleep(POLL).await;
    }
}

/// `~/.claude/projects/<enc(cwd)>/<sid>.jsonl` for a given session.
pub fn session_jsonl_path(cwd: &str, sid: &str) -> PathBuf {
    project_dir(cwd).join(format!("{sid}.jsonl"))
}

/// The directory where claude stores session JSONL files for `cwd`. The
/// encoding replaces every non-alphanumeric character with '-'.
pub fn project_dir(cwd: &str) -> PathBuf {
    let enc: String = cwd
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let home = std::env::var("HOME").unwrap_or_else(|_| dirs_fallback().unwrap_or_default());
    PathBuf::from(home)
        .join(".claude")
        .join("projects")
        .join(enc)
}

/// Best-effort home lookup when $HOME is unset (rare on macOS/Linux).
fn dirs_fallback() -> Option<String> {
    std::env::var_os("USERPROFILE").map(|v| v.to_string_lossy().into_owned())
}

/// Text of the LAST assistant message in `jsonl` whose
/// `message.stop_reason == "end_turn"`: its `type == "text"` content blocks
/// concatenated with newlines. `None` while the turn is still in flight.
///
/// JSONL line shape (only the fields we care about):
/// `{"message":{"role":"assistant","content":[{"type":"text","text":"…"}],
///   "stop_reason":"end_turn"}}` — metadata lines with other shapes are
/// skipped, as are mid-turn entries (stop_reason "tool_use" / null).
pub fn completed_turn_text(jsonl: &str) -> Option<String> {
    let mut result = None;
    for line in jsonl.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue; // metadata lines with differing shapes — skip
        };
        let Some(msg) = v.get("message") else {
            continue;
        };
        if msg.get("role").and_then(|r| r.as_str()) != Some("assistant") {
            continue;
        }
        if msg.get("stop_reason").and_then(|r| r.as_str()) != Some("end_turn") {
            continue;
        }
        let mut text = String::new();
        if let Some(blocks) = msg.get("content").and_then(|c| c.as_array()) {
            for block in blocks {
                if block.get("type").and_then(|t| t.as_str()) != Some("text") {
                    continue;
                }
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
        let text = text.trim();
        if !text.is_empty() {
            result = Some(text.to_string());
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_every_non_alphanumeric_as_dash() {
        let p = session_jsonl_path("/tmp/otto ws", "abc");
        let s = p.to_string_lossy();
        assert!(s.contains("-tmp-otto-ws"), "got: {s}");
        assert!(s.ends_with("abc.jsonl"), "got: {s}");
    }

    #[test]
    fn end_turn_text_is_returned() {
        let jsonl = concat!(
            r#"{"type":"summary","summary":"meta line, no message"}"#,
            "\n",
            r#"{"message":{"role":"user","content":[{"type":"text","text":"plan this"}]}}"#,
            "\n",
            r#"{"message":{"role":"assistant","stop_reason":"end_turn","content":[{"type":"text","text":"[{\"action\":\"broadcast\",\"text\":\"hi\"}]"}]}}"#,
            "\n",
        );
        assert_eq!(
            completed_turn_text(jsonl).as_deref(),
            Some("[{\"action\":\"broadcast\",\"text\":\"hi\"}]")
        );
    }

    #[test]
    fn mid_turn_entries_do_not_complete_the_turn() {
        // tool_use / null stop_reason lines (even with preamble text) must
        // not be mistaken for the final reply.
        let jsonl = concat!(
            r#"{"message":{"role":"assistant","stop_reason":"tool_use","content":[{"type":"text","text":"Let me look around first."},{"type":"tool_use","name":"Read","input":{"file_path":"/x"}}]}}"#,
            "\n",
        );
        assert_eq!(completed_turn_text(jsonl), None);

        let jsonl = r#"{"message":{"role":"assistant","stop_reason":null,"content":[{"type":"text","text":"thinking..."}]}}"#;
        assert_eq!(completed_turn_text(jsonl), None);
    }

    #[test]
    fn last_end_turn_wins_and_partial_lines_are_skipped() {
        let jsonl = concat!(
            r#"{"message":{"role":"assistant","stop_reason":"end_turn","content":[{"type":"text","text":"first"}]}}"#,
            "\n",
            r#"{"message":{"role":"assistant","stop_reason":"end_turn","content":[{"type":"text","text":"second"}]}}"#,
            "\n",
            // partially-written trailing line — must be ignored, not crash
            r#"{"message":{"role":"assistant","stop_re"#,
        );
        assert_eq!(completed_turn_text(jsonl).as_deref(), Some("second"));
    }
}
