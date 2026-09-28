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
/// How long typing the prompt may wait for claude to drain it from its tty.
/// Generous (a cold TUI reads late; the old blocking write waited forever);
/// past it the TUI is wedged and the attempt is retried in a fresh PTY.
const PROMPT_WRITE_TIMEOUT: Duration = Duration::from_secs(120);
/// Absolute backstop so a truly wedged session can't run forever. There is
/// otherwise NO wall-clock limit — a healthy turn may run as long as it keeps
/// making progress (planning/recruiting are one-time, quality-sensitive turns
/// the operator is happy to let run long for a better result).
const HARD_CAP: Duration = Duration::from_secs(3600);
/// Attempts before giving up, mirroring the review engine's recovery loop: a
/// stuck/failed turn is killed and re-run from scratch.
const MAX_ATTEMPTS: u32 = 3;
/// Pause between a stuck/failed attempt and the respawn.
const RETRY_BACKOFF: Duration = Duration::from_secs(3);

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
    /// `no_progress` is NOT a wall-clock cap: a healthy turn may run as long as
    /// it keeps making progress (the session transcript grows or the TUI stays
    /// active). It is the "stuck" window — if NOTHING advances for that long the
    /// turn is considered wedged, killed, and re-run from scratch (up to
    /// [`MAX_ATTEMPTS`]). A 1h [`HARD_CAP`] is the only absolute backstop.
    ///
    /// The session always runs with `--dangerously-skip-permissions` so it
    /// never stalls on an approval prompt nobody can answer.
    pub async fn run_prompt(
        &self,
        prompt: &str,
        cwd: &str,
        model: Option<&str>,
        no_progress: Duration,
    ) -> Result<String> {
        // Canonicalize the cwd: claude resolves symlinks (macOS /var →
        // /private/var) when computing its transcript dir, so the spawn cwd and
        // the JSONL path we poll must be the SAME resolved path — otherwise the
        // completed turn lands in a dir we never read and we false-time-out.
        let cwd_canon = std::fs::canonicalize(cwd)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| cwd.to_string());
        let cwd: &str = &cwd_canon;
        let mut last_err: Option<Error> = None;
        for attempt in 1..=MAX_ATTEMPTS {
            // Fresh session id (→ fresh JSONL transcript) per attempt.
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
            // fork/exec of the agent CLI is a synchronous syscall path (tens to
            // hundreds of ms, growing with daemon RSS since portable-pty's
            // `pre_exec` forces a real fork) — keep it off the async workers,
            // exactly like `SessionManager::create` does (r3-06-03).
            let handle = tokio::task::spawn_blocking(move || PtyHandle::spawn(&spec))
                .await
                .map_err(|e| Error::Internal(format!("spawn claude: {e}")))??;
            let result = drive(&handle, prompt, cwd, &sid, no_progress).await;
            // Single-shot session: always tear the PTY down, success or not.
            let _ = handle.kill();
            match result {
                Ok(text) => return Ok(text),
                Err(e) => {
                    tracing::warn!("claude turn attempt {attempt}/{MAX_ATTEMPTS} failed: {e}");
                    // A claude API error (wrong model, auth, rate-limit) is terminal
                    // — retrying just re-hits the same error. Surface it now.
                    let is_api_error =
                        matches!(&e, Error::Upstream(m) if m.starts_with("agent error:"));
                    last_err = Some(e);
                    if is_api_error {
                        break;
                    }
                    if attempt < MAX_ATTEMPTS {
                        tokio::time::sleep(RETRY_BACKOFF).await;
                    }
                }
            }
        }
        Err(last_err.unwrap_or_else(|| Error::Upstream("claude turn failed with no detail".into())))
    }
}

/// Type the prompt into a freshly-spawned claude TUI and wait for the
/// completed assistant turn to land in the session JSONL.
async fn drive(
    handle: &PtyHandle,
    prompt: &str,
    cwd: &str,
    sid: &str,
    no_progress: Duration,
) -> Result<String> {
    let start = tokio::time::Instant::now();
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
    //    Written through the PTY's own writer thread and awaited without
    //    parking a tokio worker: the blocking `write` waited on a sync channel
    //    until claude drained the (often tens of KB) prompt — seconds on a
    //    cold start (r3-06-03).
    handle
        .write_async(
            format!("\x1b[200~{prompt}\x1b[201~").as_bytes(),
            PROMPT_WRITE_TIMEOUT,
        )
        .await?;
    tokio::time::sleep(PASTE_TO_ENTER).await;
    handle.write_async(b"\r", PROMPT_WRITE_TIMEOUT).await?;

    // 3. Poll the session transcript until the turn completes. There is NO
    //    wall-clock deadline: instead we track *progress* and only give up when
    //    the turn looks wedged. "Progress" is the JSONL transcript growing (the
    //    model writing messages/tool calls) OR the PTY staying active (TUI
    //    redraws / "thinking…" — keeps cold-start and long reasoning from being
    //    mistaken for a stall). If neither advances for `no_progress`, the turn
    //    is stuck → the caller respawns it. `HARD_CAP` is the only absolute cap.
    //    The transcript is read INCREMENTALLY ([`ReplyTail`]): each tick reads
    //    only the bytes appended since the last one, off the runtime — a long
    //    planning turn's JSONL reaches tens of MB, and re-reading + re-parsing
    //    all of it every 250 ms on a runtime worker was the old cost (A5).
    let path = session_jsonl_path(cwd, sid);
    let mut tail = ReplyTail::default();
    let mut last_pty = handle.last_output_at();
    let mut last_progress = tokio::time::Instant::now();
    loop {
        let mut progressed = false;
        let (t, read) = poll_reply_tail(tail, path.clone()).await?;
        tail = t;
        if let Ok(read) = read {
            if let Some(text) = tail.reply() {
                return Ok(text);
            }
            // FAIL FAST on a claude API error (wrong model, auth, rate-limit): it
            // carries stop_reason "stop_sequence", so completed_turn_text never
            // accepts it — without this we'd wait out the whole no-progress window
            // on an instant, terminal error and surface a misleading "stuck".
            if let Some(apierr) = tail.api_error() {
                return Err(Error::Upstream(format!("agent error: {apierr}")));
            }
            if read > 0 {
                progressed = true;
            }
        }
        // PTY liveness: a fresh output timestamp means the session is still
        // doing something (drawing, streaming, thinking) even if the transcript
        // hasn't flushed a new line yet.
        let pty_at = handle.last_output_at();
        if pty_at > last_pty {
            last_pty = pty_at;
            progressed = true;
        }
        if progressed {
            last_progress = tokio::time::Instant::now();
        }
        if exit_rx.borrow().is_some() {
            // Final lines may have landed right at exit — one last read.
            let (t, read) = poll_reply_tail(tail, path.clone()).await?;
            tail = t;
            if read.is_ok() {
                if let Some(text) = tail.reply() {
                    return Ok(text);
                }
            }
            return Err(Error::Upstream(
                "claude exited before completing a reply".into(),
            ));
        }
        if last_progress.elapsed() >= no_progress {
            return Err(Error::Upstream(format!(
                "claude session stuck — no transcript progress for {}s",
                no_progress.as_secs()
            )));
        }
        if start.elapsed() >= HARD_CAP {
            return Err(Error::Upstream(format!(
                "claude session exceeded the {}h hard cap",
                HARD_CAP.as_secs() / 3600
            )));
        }
        tokio::time::sleep(POLL).await;
    }
}

/// One [`ReplyTail::poll`] on the blocking pool (the tail moves in and out).
async fn poll_reply_tail(
    mut tail: ReplyTail,
    path: PathBuf,
) -> Result<(ReplyTail, std::io::Result<u64>)> {
    tokio::task::spawn_blocking(move || {
        let read = tail.poll(&path);
        (tail, read)
    })
    .await
    .map_err(|e| Error::Internal(format!("transcript poll task failed: {e}")))
}

/// Incremental [`completed_turn_text`] + [`transcript_api_error`] over a
/// growing session JSONL. [`ReplyTail::poll`] reads only the bytes appended
/// since the previous poll and folds the complete lines; an unterminated last
/// line is kept and — exactly like `str::lines` on the whole file — still
/// counts once it parses. A file that shrank (replaced/truncated) restarts
/// from byte 0. Invalid UTF-8 is decoded lossily per line (the whole-file
/// `read_to_string` gave no answer at all for such a file).
#[derive(Debug, Default)]
pub struct ReplyTail {
    offset: u64,
    partial: Vec<u8>,
    reply: Option<String>,
    api_error: Option<String>,
}

impl ReplyTail {
    /// Fold newly appended bytes.
    pub fn feed(&mut self, bytes: &[u8]) {
        self.offset += bytes.len() as u64;
        self.partial.extend_from_slice(bytes);
        let Some(nl) = self.partial.iter().rposition(|b| *b == b'\n') else {
            return;
        };
        let rest = self.partial.split_off(nl + 1);
        let done = std::mem::replace(&mut self.partial, rest);
        for line in String::from_utf8_lossy(&done).lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            if self.api_error.is_none() {
                self.api_error = line_api_error(&v);
            }
            if let Some(t) = line_end_turn_text(&v) {
                self.reply = Some(t);
            }
        }
    }

    /// The unterminated last line, when it already parses.
    fn partial_value(&self) -> Option<serde_json::Value> {
        let t = std::str::from_utf8(&self.partial).ok()?.trim();
        if t.is_empty() {
            return None;
        }
        serde_json::from_str(t).ok()
    }

    /// [`completed_turn_text`] of everything read so far.
    pub fn reply(&self) -> Option<String> {
        self.partial_value()
            .and_then(|v| line_end_turn_text(&v))
            .or_else(|| self.reply.clone())
    }

    /// [`transcript_api_error`] of everything read so far (first one wins).
    pub fn api_error(&self) -> Option<String> {
        self.api_error
            .clone()
            .or_else(|| self.partial_value().and_then(|v| line_api_error(&v)))
    }

    /// Read what was appended to `path` since the last poll (blocking IO).
    /// `Ok(bytes read)`; `Err` when the file is missing/unreadable.
    pub fn poll(&mut self, path: &std::path::Path) -> std::io::Result<u64> {
        use std::io::{Read, Seek, SeekFrom};
        let mut f = std::fs::File::open(path)?;
        let len = f.metadata()?.len();
        if len < self.offset {
            *self = Self::default();
        }
        if len == self.offset {
            return Ok(0);
        }
        f.seek(SeekFrom::Start(self.offset))?;
        let mut buf = Vec::with_capacity((len - self.offset) as usize);
        f.take(len - self.offset).read_to_end(&mut buf)?;
        self.feed(&buf);
        Ok(buf.len() as u64)
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
        if let Some(t) = line_end_turn_text(&v) {
            result = Some(t);
        }
    }
    result
}

/// One parsed JSONL line's contribution to [`completed_turn_text`]: the
/// trimmed text of a non-sidechain assistant `end_turn` message, `None` for
/// anything else (or an end_turn with no text).
fn line_end_turn_text(v: &serde_json::Value) -> Option<String> {
    let msg = v.get("message")?;
    // Legacy in-file sub-agent lines (older Claude Code wrote them into the
    // MAIN transcript with a top-level `isSidechain:true`). Their end_turns
    // are the CHILD's, never the parent's — counting them completes a step
    // the moment its first sub-agent finishes.
    if v.get("isSidechain").and_then(|b| b.as_bool()) == Some(true) {
        return None;
    }
    if msg.get("role").and_then(|r| r.as_str()) != Some("assistant") {
        return None;
    }
    if msg.get("stop_reason").and_then(|r| r.as_str()) != Some("end_turn") {
        return None;
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
    (!text.is_empty()).then(|| text.to_string())
}

/// Count assistant messages whose `stop_reason == "end_turn"` AND that carry
/// non-empty text — i.e. completed turns, mirroring [`completed_turn_text`]'s
/// filter. Used to BASELINE a transcript before submitting a new turn to a
/// *resumed* session, so the prior turn's reply isn't mistaken for the new one.
pub fn completed_turn_count(jsonl: &str) -> usize {
    let mut n = 0;
    for line in jsonl.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
            continue;
        };
        let Some(msg) = v.get("message") else {
            continue;
        };
        // Legacy in-file sub-agent lines — see `completed_turn_text`.
        if v.get("isSidechain").and_then(|b| b.as_bool()) == Some(true) {
            continue;
        }
        if msg.get("role").and_then(|r| r.as_str()) != Some("assistant") {
            continue;
        }
        if msg.get("stop_reason").and_then(|r| r.as_str()) != Some("end_turn") {
            continue;
        }
        let has_text = msg
            .get("content")
            .and_then(|c| c.as_array())
            .is_some_and(|bs| {
                bs.iter().any(|b| {
                    b.get("type").and_then(|t| t.as_str()) == Some("text")
                        && b.get("text")
                            .and_then(|t| t.as_str())
                            .is_some_and(|t| !t.trim().is_empty())
                })
            });
        if has_text {
            n += 1;
        }
    }
    n
}

/// Text of the LAST `role:"user"` message in `jsonl`. claude records the user's
/// submitted prompt here, so a caller can confirm its prompt was actually ENTERED
/// (vs. lost to a startup/promo screen that swallowed the paste). `content` is a
/// plain string for a typed/pasted prompt, or an array of `{type:"text",text}`
/// blocks; tool-result user turns (arrays of `{type:"tool_result",…}`) carry no
/// text and are skipped, so this returns the last *typed* prompt, never a tool
/// result. `None` while no user text is present yet.
pub fn last_user_text(jsonl: &str) -> Option<String> {
    let mut result = None;
    for line in jsonl.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
            continue;
        };
        let Some(msg) = v.get("message") else {
            continue;
        };
        if msg.get("role").and_then(|r| r.as_str()) != Some("user") {
            continue;
        }
        let text = match msg.get("content") {
            Some(serde_json::Value::String(s)) => s.trim().to_string(),
            Some(serde_json::Value::Array(blocks)) => {
                let mut t = String::new();
                for block in blocks {
                    if block.get("type").and_then(|k| k.as_str()) != Some("text") {
                        continue;
                    }
                    if let Some(s) = block.get("text").and_then(|s| s.as_str()) {
                        if !s.is_empty() {
                            if !t.is_empty() {
                                t.push('\n');
                            }
                            t.push_str(s);
                        }
                    }
                }
                t.trim().to_string()
            }
            _ => continue,
        };
        if !text.is_empty() {
            result = Some(text);
        }
    }
    result
}

/// The text of an assistant message claude flagged with `isApiErrorMessage:
/// true` — claude's own error surface (wrong/unknown `--model`, auth, rate-limit,
/// overload). These carry `stop_reason: "stop_sequence"` (NOT `end_turn`), so
/// [`completed_turn_text`] never accepts them; a watcher that only waits for
/// `end_turn` would burn the whole no-progress window on what is actually an
/// instant, terminal error. Detecting it lets callers FAIL FAST with the real
/// message instead of a generic "stuck" timeout.
pub fn transcript_api_error(jsonl: &str) -> Option<String> {
    jsonl.lines().find_map(|line| {
        serde_json::from_str::<serde_json::Value>(line.trim())
            .ok()
            .and_then(|v| line_api_error(&v))
    })
}

/// One parsed JSONL line's [`transcript_api_error`] text, if it is one.
fn line_api_error(v: &serde_json::Value) -> Option<String> {
    if v.get("isApiErrorMessage").and_then(|b| b.as_bool()) != Some(true) {
        return None;
    }
    let msg = v.get("message")?;
    if msg.get("role").and_then(|r| r.as_str()) != Some("assistant") {
        return None;
    }
    msg.get("content")?.as_array()?.iter().find_map(|b| {
        if b.get("type").and_then(|t| t.as_str()) != Some("text") {
            return None;
        }
        let t = b.get("text")?.as_str()?.trim();
        (!t.is_empty()).then(|| t.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// r3-06-03: typing a large prompt into a TUI that is not reading its tty
    /// yet must not park the runtime. On a single-threaded runtime the old
    /// blocking `write` froze every other task until the child drained it;
    /// with the spawn on the blocking pool and `write_async` a ticker keeps
    /// running through the child's one-second nap.
    #[tokio::test(flavor = "current_thread")]
    async fn prompt_write_does_not_park_the_runtime() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        let spec = CommandSpec {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), "sleep 1; exec cat >/dev/null".into()],
            cwd: None,
            env: vec![],
        };
        let handle = tokio::task::spawn_blocking(move || PtyHandle::spawn(&spec))
            .await
            .unwrap()
            .unwrap();
        let ticks = Arc::new(AtomicUsize::new(0));
        let t = Arc::clone(&ticks);
        let ticker = tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(10)).await;
                t.fetch_add(1, Ordering::Relaxed);
            }
        });
        let prompt: String = (0..800)
            .map(|i| format!("line {i:05} of the prompt\n"))
            .collect();
        handle
            .write_async(prompt.as_bytes(), PROMPT_WRITE_TIMEOUT)
            .await
            .unwrap();
        ticker.abort();
        let _ = handle.kill();
        assert!(
            ticks.load(Ordering::Relaxed) >= 20,
            "runtime stalled during the prompt write: {} ticks",
            ticks.load(Ordering::Relaxed)
        );
    }

    #[test]
    fn encodes_every_non_alphanumeric_as_dash() {
        let p = session_jsonl_path("/tmp/otto ws", "abc");
        let s = p.to_string_lossy();
        assert!(s.contains("-tmp-otto-ws"), "got: {s}");
        assert!(s.ends_with("abc.jsonl"), "got: {s}");
    }

    /// A5: the incremental tail equals the whole-file scan after EVERY
    /// append, for appends split anywhere (mid-line, mid-UTF-8 char), with
    /// sidechain end_turns, a tool_use turn, an api error and an
    /// unterminated last line; a shrunk file restarts from byte 0.
    #[test]
    fn reply_tail_matches_the_whole_file_scan_at_every_split() {
        let jsonl = concat!(
            r#"{"type":"summary","summary":"meta"}"#,
            "\n\n",
            r#"{"message":{"role":"user","content":"plan é this"}}"#,
            "\n",
            r#"{"isSidechain":true,"message":{"role":"assistant","stop_reason":"end_turn","content":[{"type":"text","text":"child"}]}}"#,
            "\n",
            r#"{"message":{"role":"assistant","stop_reason":"tool_use","content":[{"type":"text","text":"working"}]}}"#,
            "\n",
            r#"{"message":{"role":"assistant","stop_reason":"end_turn","content":[{"type":"text","text":"first ✓"},{"type":"text","text":"second"}]}}"#,
            "\n",
            "not json at all\n",
            r#"{"isApiErrorMessage":true,"message":{"role":"assistant","stop_reason":"stop_sequence","content":[{"type":"text","text":" model not found "}]}}"#,
            "\n",
            r#"{"isApiErrorMessage":true,"message":{"role":"assistant","content":[{"type":"text","text":"later error"}]}}"#,
            "\n",
            r#"{"message":{"role":"assistant","stop_reason":"end_turn","content":[{"type":"text","text":"final λ"}]}}"#,
        )
        .as_bytes();
        for step in [1usize, 3, 7, 64, 1000] {
            let dir =
                std::env::temp_dir().join(format!("otto-reply-tail-{}-{step}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("s.jsonl");
            std::fs::write(&path, b"").unwrap();
            let mut tail = ReplyTail::default();
            let mut at = 0;
            while at < jsonl.len() {
                let end = (at + step).min(jsonl.len());
                let prev = at;
                use std::io::Write;
                std::fs::OpenOptions::new()
                    .append(true)
                    .open(&path)
                    .unwrap()
                    .write_all(&jsonl[at..end])
                    .unwrap();
                at = end;
                assert_eq!(tail.poll(&path).unwrap(), (end - prev) as u64);
                // Whole-file semantics (lossy only matters mid-char, where
                // read_to_string would have failed outright — compare on the
                // valid prefix's lossy view, like the tail sees it).
                let whole = String::from_utf8_lossy(&jsonl[..end]).into_owned();
                assert_eq!(
                    tail.reply(),
                    completed_turn_text(&whole),
                    "step {step} at {end}"
                );
                assert_eq!(
                    tail.api_error(),
                    transcript_api_error(&whole),
                    "step {step} at {end}"
                );
            }
            assert_eq!(tail.reply().as_deref(), Some("final λ"));
            assert_eq!(tail.api_error().as_deref(), Some("model not found"));
            assert_eq!(tail.poll(&path).unwrap(), 0, "idle poll reads nothing");
            // Replaced by a shorter file: start over.
            std::fs::write(&path, concat!(r#"{"message":{"role":"assistant","stop_reason":"end_turn","content":[{"type":"text","text":"new"}]}}"#, "\n")).unwrap();
            tail.poll(&path).unwrap();
            assert_eq!(tail.reply().as_deref(), Some("new"));
            assert_eq!(tail.api_error(), None);
            let _ = std::fs::remove_dir_all(&dir);
        }
        assert!(ReplyTail::default()
            .poll(std::path::Path::new("/nonexistent/otto.jsonl"))
            .is_err());
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

    #[test]
    fn completed_turn_count_baselines_resumed_transcripts() {
        let one = r#"{"message":{"role":"assistant","stop_reason":"end_turn","content":[{"type":"text","text":"a"}]}}"#;
        let mid = r#"{"message":{"role":"assistant","stop_reason":"tool_use","content":[{"type":"text","text":"x"}]}}"#;
        assert_eq!(completed_turn_count(""), 0);
        assert_eq!(completed_turn_count(one), 1);
        assert_eq!(completed_turn_count(&format!("{one}\n{mid}\n{one}")), 2); // mid_turn not counted
    }

    #[test]
    fn sidechain_lines_never_count() {
        // Older Claude Code recorded sub-agent turns in the MAIN transcript with
        // a top-level `isSidechain:true`. A child's end_turn is not the parent's.
        let parent = r#"{"message":{"role":"assistant","stop_reason":"end_turn","content":[{"type":"text","text":"parent reply"}]}}"#;
        let child = r#"{"isSidechain":true,"message":{"role":"assistant","stop_reason":"end_turn","content":[{"type":"text","text":"child reply"}]}}"#;
        let jsonl = format!("{parent}\n{child}\n");
        assert_eq!(completed_turn_count(&jsonl), 1);
        assert_eq!(completed_turn_text(&jsonl).as_deref(), Some("parent reply"));
    }

    #[test]
    fn last_user_text_returns_typed_prompt_and_skips_tool_results() {
        // string content (the common shape for a typed/pasted prompt)
        let s = r#"{"type":"user","message":{"role":"user","content":"do the thing"}}"#;
        assert_eq!(last_user_text(s).as_deref(), Some("do the thing"));
        // array-of-text-blocks content
        let a = r#"{"message":{"role":"user","content":[{"type":"text","text":"hello"}]}}"#;
        assert_eq!(last_user_text(a).as_deref(), Some("hello"));
        // a later tool_result user turn carries no text → the typed prompt still wins
        let tool =
            r#"{"message":{"role":"user","content":[{"type":"tool_result","content":"42"}]}}"#;
        assert_eq!(
            last_user_text(&format!("{s}\n{tool}")).as_deref(),
            Some("do the thing")
        );
        // the LAST typed prompt wins across turns
        let s2 = r#"{"message":{"role":"user","content":"second prompt"}}"#;
        assert_eq!(
            last_user_text(&format!("{s}\n{s2}")).as_deref(),
            Some("second prompt")
        );
        // assistant turns and empty transcripts contribute nothing
        let asst = r#"{"message":{"role":"assistant","stop_reason":"end_turn","content":[{"type":"text","text":"hi"}]}}"#;
        assert_eq!(last_user_text(asst), None);
        assert_eq!(last_user_text(""), None);
    }

    #[test]
    fn transcript_api_error_is_detected() {
        // The real shape from the live "wrong model" hang: isApiErrorMessage +
        // stop_reason "stop_sequence" (so completed_turn_text wouldn't see it).
        let jsonl = r#"{"isApiErrorMessage":true,"message":{"role":"assistant","stop_reason":"stop_sequence","content":[{"type":"text","text":"There's an issue with the selected model (claude)."}]}}"#;
        assert_eq!(
            transcript_api_error(jsonl).as_deref(),
            Some("There's an issue with the selected model (claude).")
        );
        // A normal completed turn is NOT an api error.
        let ok = r#"{"message":{"role":"assistant","stop_reason":"end_turn","content":[{"type":"text","text":"hi"}]}}"#;
        assert_eq!(transcript_api_error(ok), None);
    }
}
