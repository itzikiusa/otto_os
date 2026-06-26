//! Per-session activity feed + typing indicator + final reply poster.
//!
//! `Mirror` keeps one tailer task per session. It watches the claude JSONL
//! transcript, maintains a rolling "🧠 working…" feed of the agent's steps
//! (edited in place, capped at [`MAX_ACTIVITY_LINES`] and trimmed to fit the
//! channel's message-length limit) and sends a periodic "typing…" chat action.
//! On `Final` it rewrites the feed to "done — N steps" and, unless `agent_reply`
//! is set, posts the reply text itself via the adapter (the bot that received
//! the message) — inline, or a short head + `investigation.md` upload when long.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;
use tracing::{debug, info, warn};

use otto_core::domain::Channel;
use otto_core::Id;
use otto_sessions::SessionManager;

use crate::adapter::Adapter;
use crate::transcript::{self, TranscriptEvent};

/// Hook that runs Otto's self-improvement on a just-finished channel
/// interaction and returns a short human-readable summary to post back in the
/// thread (e.g. "🛠️ Self-improvement: updated skill `frb-grant-failure`").
///
/// Injected by otto-server (which owns the improvement engine), mirroring the
/// `SwarmTrigger` pattern — so otto-channels needs no dependency on otto-improve.
/// Returns `None` when self-improvement is disabled for the workspace, nothing
/// changed, or the interaction was too trivial to learn from.
#[async_trait::async_trait]
pub trait InteractionImprover: Send + Sync {
    async fn evolve_interaction(&self, session_id: &Id) -> Option<String>;
}

/// Maximum number of activity lines to retain in the rolling feed. High so a
/// long investigation (100+ tool calls) shows its full trail of steps.
const MAX_ACTIVITY_LINES: usize = 250;
/// Minimum gap between edits of the rolling feed message. Kept well above ~1/s
/// so a long investigation's frequent tool events don't trip Telegram/Slack
/// message-edit rate limits (429s).
const EDIT_THROTTLE: Duration = Duration::from_millis(2500);
/// Hard char budget for the rolling feed body, kept under the channels' single
/// message limits (Telegram 4096). When the feed is longer we keep the most
/// recent lines that fit and note how many earlier steps were elided.
const FEED_CHAR_BUDGET: usize = 3500;
/// How long to poll for `provider_session_id` to appear.
const PSID_TIMEOUT: Duration = Duration::from_secs(20);
/// Poll cadence for `provider_session_id` polling.
const PSID_POLL: Duration = Duration::from_millis(500);
/// Character count above which we post a short head and attach the full reply
/// as an `investigation.md` file instead of inlining it.
const LONG_REPLY_THRESHOLD: usize = 1800;
/// How many characters of the full reply to include in the inline head.
const LONG_REPLY_HEAD_CHARS: usize = 1500;
/// How often to send the typing indicator while the agent is working.
const TYPING_INTERVAL: Duration = Duration::from_secs(4);
/// How often the rolling feed's header advances to the next liveness phrase
/// while a turn is in progress. Kept above [`EDIT_THROTTLE`] so a status tick is
/// always a legitimate (non-throttled) edit. Slack has no typing indicator, so
/// this rotating header is the only "still working" signal there.
const STATUS_TICK: Duration = Duration::from_millis(3500);

/// Rotating "still working" phrases shown in the feed header (cycled on each
/// [`STATUS_TICK`]). Generic by design — they signal liveness without claiming
/// progress the mirror can't actually observe.
const STATUS_PHRASES: &[&str] = &[
    "Analyzing…",
    "Looking into it…",
    "Working through it…",
    "Gathering context…",
    "Reviewing the details…",
    "Summarizing findings…",
    "Finding answers…",
    "Putting it together…",
];

struct SessionEntry {
    cancel: Arc<AtomicBool>,
    /// Set by `begin_turn` when a new inbound comment arrives for this session;
    /// the tailer then resets the feed (a fresh "working…" message) for the new
    /// turn instead of editing the previous turn's (now scrolled-up) message.
    new_turn: Arc<AtomicBool>,
    /// Whether the typing indicator should be sent right now (on while a turn is
    /// in progress, off after its Final).
    typing_active: Arc<AtomicBool>,
}

/// Shared mirror state — holds one entry per tracked session.
pub struct Mirror {
    sessions: Mutex<HashMap<Id, SessionEntry>>,
    manager: Arc<SessionManager>,
    /// Optional self-improvement hook: when set, a finished channel turn is fed
    /// to Otto's improvement engine and the result posted back in the thread.
    improver: Option<Arc<dyn InteractionImprover>>,
}

impl Mirror {
    pub fn new(manager: Arc<SessionManager>) -> Arc<Self> {
        Arc::new(Self {
            sessions: Mutex::new(HashMap::new()),
            manager,
            improver: None,
        })
    }

    /// Builder variant that wires the self-improvement hook (otto-server provides
    /// the implementation). `None` leaves the mirror's behaviour unchanged.
    pub fn new_with_improver(
        manager: Arc<SessionManager>,
        improver: Option<Arc<dyn InteractionImprover>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            sessions: Mutex::new(HashMap::new()),
            manager,
            improver,
        })
    }

    /// Attach (or re-attach) a channel destination to `session_id` and ensure
    /// a tailer task is running for it.
    ///
    /// `agent_reply = true` means the agent will post its own final reply;
    /// we only post the activity feed, not the final text.
    pub async fn attach(
        self: &Arc<Self>,
        session_id: Id,
        adapter: Arc<dyn Adapter>,
        chat: String,
        thread: Option<String>,
        agent_reply: bool,
    ) {
        let mut guard = self.sessions.lock().await;

        // If we already have a live tailer for this session, do nothing.
        if guard.contains_key(&session_id) {
            return;
        }

        let cancel = Arc::new(AtomicBool::new(false));
        let new_turn = Arc::new(AtomicBool::new(false));
        let typing_active = Arc::new(AtomicBool::new(true));
        guard.insert(
            session_id.clone(),
            SessionEntry {
                cancel: Arc::clone(&cancel),
                new_turn: Arc::clone(&new_turn),
                typing_active: Arc::clone(&typing_active),
            },
        );
        drop(guard);

        let mirror = Arc::clone(self);
        tokio::spawn(async move {
            mirror
                .run_tailer(
                    session_id,
                    adapter,
                    chat,
                    thread,
                    agent_reply,
                    cancel,
                    new_turn,
                    typing_active,
                )
                .await;
        });
    }

    /// Signal that a new inbound comment started a fresh turn for `session_id`
    /// (reused sessions): the tailer posts a new activity feed and resumes the
    /// typing indicator. No-op if the session isn't tracked.
    pub async fn begin_turn(&self, session_id: &Id) {
        if let Some(e) = self.sessions.lock().await.get(session_id) {
            e.new_turn.store(true, Ordering::Relaxed);
            e.typing_active.store(true, Ordering::Relaxed);
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_tailer(
        &self,
        session_id: Id,
        adapter: Arc<dyn Adapter>,
        chat: String,
        thread: Option<String>,
        agent_reply: bool,
        cancel: Arc<AtomicBool>,
        new_turn: Arc<AtomicBool>,
        typing_active: Arc<AtomicBool>,
    ) {
        // --- Step 1: wait for provider_session_id and cwd ---
        let (cwd, psid) = match self.wait_for_psid(&session_id, &cancel).await {
            Some(v) => v,
            None => {
                debug!(session = %session_id, "mirror: cancelled or timed-out waiting for psid");
                self.sessions.lock().await.remove(&session_id);
                return;
            }
        };

        let path: PathBuf = otto_orchestrator::claude_pty::session_jsonl_path(&cwd, &psid);

        info!(session = %session_id, ?path, "mirror: starting transcript tailer");

        // --- Step 2: run the tailer ---
        // Shared state for the closure (can't async in sync FnMut, so we use
        // a channel to pass events to an async task).
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<TranscriptEvent>();
        let cancel_clone = Arc::clone(&cancel);

        tokio::spawn(async move {
            transcript::tail(
                path,
                move |evt| {
                    let _ = tx.send(evt);
                },
                cancel_clone,
            )
            .await;
        });

        // --- Step 3: spawn the typing indicator task ---
        // Persistent across turns: sends the typing action only while
        // `typing_active` is set (on during a turn, off after its Final), and
        // exits when `typing_stop` is set as the tailer winds down.
        let typing_stop = Arc::new(AtomicBool::new(false));
        {
            let stop = Arc::clone(&typing_stop);
            let active = Arc::clone(&typing_active);
            let adapter_clone = Arc::clone(&adapter);
            let chat_clone = chat.clone();
            tokio::spawn(async move {
                loop {
                    if stop.load(Ordering::Relaxed) {
                        return;
                    }
                    if active.load(Ordering::Relaxed) {
                        let _ = adapter_clone.typing(&chat_clone).await;
                    }
                    tokio::time::sleep(TYPING_INTERVAL).await;
                }
            });
        }

        // Process events: maintain a rolling feed of the agent's steps (edited in
        // place) whose header rotates through "still working" phrases on a timer
        // so the user sees liveness even during a long think with no tool calls
        // (Slack has no typing indicator). On Final, freeze the header to
        // "done — N steps", post the reply, and run self-improvement if wired.
        let mut activity_lines: Vec<String> = Vec::new();
        let mut rolling_msg_id: Option<String> = None;
        let mut last_edit = Instant::now() - EDIT_THROTTLE * 2; // allow first edit immediately
        let mut last_posted_final: Option<String> = None;
        let mut status_idx: usize = 0;
        let thread_ref = thread.as_deref();
        // Slack renders mrkdwn (``` code fences) in chat.update text; Telegram's
        // in-place edit carries no parse mode, so fences would show literally —
        // there the command preview is rendered as plain indented lines instead.
        let code_blocks = matches!(adapter.channel(), Channel::Slack);

        // Liveness ticker: advances the header phrase while a turn is in flight.
        let mut status_ticker = tokio::time::interval(STATUS_TICK);
        status_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            // A new inbound comment starts a fresh turn: reset the feed so the
            // next activity posts a NEW "working…" message (rather than editing
            // the previous turn's), resume the rotating status, and resume typing.
            if new_turn.swap(false, Ordering::Relaxed) {
                rolling_msg_id = None;
                activity_lines.clear();
                last_posted_final = None;
                status_idx = 0;
                last_edit = Instant::now() - EDIT_THROTTLE * 2; // post the new turn's first update at once
                typing_active.store(true, Ordering::Relaxed);
            }

            tokio::select! {
                maybe_evt = rx.recv() => {
                    let Some(evt) = maybe_evt else { break; };
                    if cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    match evt {
                        TranscriptEvent::Tool { name: _, display: display_line, code } => {
                            let line = render_tool_line(&display_line, code.as_deref(), code_blocks);
                            debug!(
                                session = %session_id,
                                activity = display_line.as_str(),
                                "mirror: transcript tool event"
                            );
                            activity_lines.push(line);
                            if activity_lines.len() > MAX_ACTIVITY_LINES {
                                activity_lines.remove(0);
                            }
                            if last_edit.elapsed() >= EDIT_THROTTLE {
                                last_edit = Instant::now();
                                let body = render_feed(&status_header(status_idx), &activity_lines);
                                post_or_edit_feed(&adapter, &chat, thread_ref, &mut rolling_msg_id, &body).await;
                            }
                        }
                        TranscriptEvent::Final { text } => {
                            info!(
                                session = %session_id,
                                chars = text.chars().count(),
                                steps = activity_lines.len(),
                                agent_reply,
                                "mirror: transcript final event"
                            );
                            // Turn finished — pause typing + status rotation until
                            // the next comment resumes it (via begin_turn).
                            typing_active.store(false, Ordering::Relaxed);

                            // Freeze the rolling feed to a final "done — N steps".
                            let n = activity_lines.len();
                            let header = format!("🧠 done — {n} step{}", if n == 1 { "" } else { "s" });
                            let done_body = render_feed(&header, &activity_lines);
                            last_edit = Instant::now();
                            post_or_edit_feed(&adapter, &chat, thread_ref, &mut rolling_msg_id, &done_body).await;

                            // Otto posts the reply itself via the adapter (the bot that
                            // received the message) — the agent never uses .env/tokens.
                            // With agent_reply on, send only the agent's ⟦otto-send⟧
                            // blocks if it marked any; otherwise (and when off) send the
                            // whole final message. Dedup repeated Final events.
                            let messages: Vec<String> = {
                                let blocks = if agent_reply {
                                    extract_send_blocks(&text)
                                } else {
                                    Vec::new()
                                };
                                if blocks.is_empty() {
                                    vec![text.clone()]
                                } else {
                                    blocks
                                }
                            };
                            // Explicit file attachments the agent requested via
                            // ⟦otto-file⟧<abs path>⟦/otto-file⟧ — extracted from the full
                            // text so it works whether or not it sits inside a send
                            // block, and the markup is stripped from the posted text.
                            let file_paths = extract_file_paths(&text);
                            let joined = messages.join("\u{1e}");
                            if last_posted_final.as_deref() != Some(joined.as_str()) {
                                for body in &messages {
                                    let cleaned = strip_file_directives(body);
                                    let cleaned = cleaned.trim();
                                    if !cleaned.is_empty() {
                                        post_reply(&adapter, &chat, thread_ref, cleaned).await;
                                    }
                                }
                                for path in &file_paths {
                                    upload_file_path(&adapter, &chat, thread_ref, path).await;
                                }
                                last_posted_final = Some(joined);

                                // Self-improvement: learn from this just-finished
                                // interaction and reply in-thread (spawned so the
                                // slow evolve never stalls the tailer). No-op when
                                // no improver is wired / self-improvement is off.
                                if let Some(improver) = self.improver.clone() {
                                    let adapter2 = Arc::clone(&adapter);
                                    let chat2 = chat.clone();
                                    let thread2 = thread.clone();
                                    let sid2 = session_id.clone();
                                    tokio::spawn(async move {
                                        if let Some(summary) = improver.evolve_interaction(&sid2).await {
                                            post_reply(&adapter2, &chat2, thread2.as_deref(), &summary).await;
                                        }
                                    });
                                }
                            }
                        }
                    }
                }
                _ = status_ticker.tick() => {
                    if cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    // While the turn is live, refresh the header (creating the feed
                    // even before the first tool call so a long opening think still
                    // shows "Analyzing…"), then advance the phrase for next time.
                    // `interval`'s first tick fires immediately, so rendering before
                    // the increment makes "Analyzing…" (idx 0) the opening phrase.
                    if typing_active.load(Ordering::Relaxed) {
                        if last_edit.elapsed() >= EDIT_THROTTLE {
                            last_edit = Instant::now();
                            let body = render_feed(&status_header(status_idx), &activity_lines);
                            post_or_edit_feed(&adapter, &chat, thread_ref, &mut rolling_msg_id, &body).await;
                        }
                        status_idx = status_idx.wrapping_add(1);
                    }
                }
            }
        }

        // Tailer winding down — stop the typing task.
        typing_stop.store(true, Ordering::Relaxed);

        self.sessions.lock().await.remove(&session_id);
        debug!(session = %session_id, "mirror: tailer finished");
    }

    /// Poll `SessionManager::get` until `provider_session_id` is Some or we
    /// time out / are cancelled. Returns `(cwd, provider_session_id)`.
    async fn wait_for_psid(
        &self,
        session_id: &Id,
        cancel: &Arc<AtomicBool>,
    ) -> Option<(String, String)> {
        let deadline = Instant::now() + PSID_TIMEOUT;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return None;
            }
            if let Ok(session) = self.manager.get(session_id).await {
                if let Some(psid) = session.provider_session_id {
                    return Some((session.cwd, psid));
                }
            }
            if Instant::now() >= deadline {
                return None;
            }
            tokio::time::sleep(PSID_POLL).await;
        }
    }

    /// Cancel a tailer (best-effort, used on shutdown).
    pub async fn cancel(&self, session_id: &Id) {
        if let Some(entry) = self.sessions.lock().await.get(session_id) {
            entry.cancel.store(true, Ordering::Relaxed);
        }
    }
}

/// The rolling-feed header for liveness phrase `idx` (cycled through
/// [`STATUS_PHRASES`]). Prefixed with 🧠 to match the "done" header's style.
fn status_header(idx: usize) -> String {
    let phrase = STATUS_PHRASES[idx % STATUS_PHRASES.len()];
    format!("🧠 {phrase}")
}

/// Render one tool step for the feed. A plain step is its `display` line; a step
/// with a `code` preview (a terminal command) renders the command beneath the
/// label — as a ``` fenced block when the channel renders mrkdwn (`code_blocks`)
/// or as indented plain lines otherwise (Telegram's in-place edit has no parse
/// mode, so a literal fence would just be noise).
fn render_tool_line(display: &str, code: Option<&str>, code_blocks: bool) -> String {
    match code {
        None => display.to_string(),
        Some(cmd) if code_blocks => format!("{display}\n```\n{cmd}\n```"),
        Some(cmd) => {
            let indented = cmd
                .lines()
                .map(|l| format!("    {l}"))
                .collect::<Vec<_>>()
                .join("\n");
            format!("{display}\n{indented}")
        }
    }
}

/// Post `body` as a new rolling-feed message, or edit the existing one in place.
/// Best-effort: send/edit failures are logged and swallowed.
async fn post_or_edit_feed(
    adapter: &Arc<dyn Adapter>,
    chat: &str,
    thread: Option<&str>,
    rolling_msg_id: &mut Option<String>,
    body: &str,
) {
    match rolling_msg_id.as_deref() {
        None => match adapter.send(chat, thread, body).await {
            Ok(mid) => *rolling_msg_id = Some(mid),
            Err(e) => warn!("mirror feed send: {e}"),
        },
        Some(mid) => {
            if let Err(e) = adapter.edit(chat, mid, body).await {
                warn!("mirror feed edit: {e}");
            }
        }
    }
}

/// Render the rolling feed: a header line plus the activity lines, trimmed from
/// the oldest end so the whole body stays under [`FEED_CHAR_BUDGET`] (channels
/// reject over-long messages). Whole lines are dropped and a note records how
/// many earlier steps were elided.
fn render_feed(header: &str, lines: &[String]) -> String {
    let full = format!("{header}\n{}", lines.join("\n"));
    if full.chars().count() <= FEED_CHAR_BUDGET {
        return full;
    }
    // Keep the most recent lines that fit, counting from the end.
    let mut kept: Vec<&str> = Vec::new();
    let mut used = header.chars().count() + 1; // header + newline
    for line in lines.iter().rev() {
        let cost = line.chars().count() + 1;
        if used + cost > FEED_CHAR_BUDGET {
            break;
        }
        used += cost;
        kept.push(line.as_str());
    }
    kept.reverse();
    let hidden = lines.len() - kept.len();
    format!(
        "{header}\n…({hidden} earlier step{} hidden)\n{}",
        if hidden == 1 { "" } else { "s" },
        kept.join("\n")
    )
}

/// Truncate `s` to at most `max_chars` Unicode scalar values.  Does NOT append
/// `…` — the caller adds a continuation note instead.
fn truncate_to_char_boundary(s: &str, max_chars: usize) -> &str {
    for (char_count, (byte_idx, _)) in s.char_indices().enumerate() {
        if char_count == max_chars {
            return &s[..byte_idx];
        }
    }
    s
}

/// Extract the agent's explicit reply blocks marked with ⟦otto-send⟧ … ⟦/otto-send⟧.
/// Empty blocks are skipped; unterminated markers are ignored.
fn extract_send_blocks(text: &str) -> Vec<String> {
    const OPEN: &str = "⟦otto-send⟧";
    const CLOSE: &str = "⟦/otto-send⟧";
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(o) = rest.find(OPEN) {
        let after = &rest[o + OPEN.len()..];
        let Some(c) = after.find(CLOSE) else { break };
        let block = after[..c].trim();
        if !block.is_empty() {
            out.push(block.to_string());
        }
        rest = &after[c + CLOSE.len()..];
    }
    out
}

/// Extract absolute file paths the agent asked to attach, marked with
/// ⟦otto-file⟧ … ⟦/otto-file⟧. Each path is trimmed; empty/unterminated markers
/// are ignored (mirrors [`extract_send_blocks`]).
fn extract_file_paths(text: &str) -> Vec<String> {
    const OPEN: &str = "⟦otto-file⟧";
    const CLOSE: &str = "⟦/otto-file⟧";
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(o) = rest.find(OPEN) {
        let after = &rest[o + OPEN.len()..];
        let Some(c) = after.find(CLOSE) else { break };
        let path = after[..c].trim();
        if !path.is_empty() {
            out.push(path.to_string());
        }
        rest = &after[c + CLOSE.len()..];
    }
    out
}

/// Remove every ⟦otto-file⟧ … ⟦/otto-file⟧ directive from `text` so the marker
/// never appears in the posted chat message. Unterminated markers are left as-is.
fn strip_file_directives(text: &str) -> String {
    const OPEN: &str = "⟦otto-file⟧";
    const CLOSE: &str = "⟦/otto-file⟧";
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(o) = rest.find(OPEN) {
        out.push_str(&rest[..o]);
        let after = &rest[o + OPEN.len()..];
        match after.find(CLOSE) {
            Some(c) => rest = &after[c + CLOSE.len()..],
            None => {
                out.push_str(OPEN);
                rest = after;
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Read a local file the agent asked to attach (via ⟦otto-file⟧) and upload it
/// to the chat. Best-effort: a missing/unreadable path is logged, not fatal.
async fn upload_file_path(
    adapter: &Arc<dyn Adapter>,
    chat: &str,
    thread: Option<&str>,
    path: &str,
) {
    match tokio::fs::read(path).await {
        Ok(bytes) => {
            let filename = std::path::Path::new(path)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("attachment");
            // Upload the raw bytes verbatim — a UTF-8 round-trip would corrupt
            // binary attachments (images, PDFs, …).
            match adapter.upload(chat, thread, filename, &bytes).await {
                Ok(()) => info!(file = filename, "mirror: uploaded agent file attachment"),
                Err(e) => warn!("mirror: file upload {path}: {e}"),
            }
        }
        Err(e) => warn!("mirror: could not read file to attach {path}: {e}"),
    }
}

/// Post one reply message to the channel via the adapter (the bot that received
/// the message). Long replies post a short head + an `investigation.md` upload.
/// Uses `send_formatted` so Slack mrkdwn and Telegram Markdown entities render
/// (bold, italic, code, links) in the relayed agent reply.
async fn post_reply(adapter: &Arc<dyn Adapter>, chat: &str, thread: Option<&str>, text: &str) {
    if text.chars().count() > LONG_REPLY_THRESHOLD {
        let head = truncate_to_char_boundary(text, LONG_REPLY_HEAD_CHARS);
        let head_msg = format!("{head}\n\n📎 full reply attached as investigation.md");
        if let Err(e) = adapter.send_formatted(chat, thread, &head_msg).await {
            warn!("mirror final-head-send: {e}");
        }
        if let Err(e) = adapter
            .upload(chat, thread, "investigation.md", text.as_bytes())
            .await
        {
            warn!("mirror upload: {e}");
        }
    } else if let Err(e) = adapter.send_formatted(chat, thread, text).await {
        warn!("mirror final-send: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_feed_short_is_verbatim() {
        let lines = vec!["one".to_string(), "two".to_string()];
        assert_eq!(render_feed("🧠 working…", &lines), "🧠 working…\none\ntwo");
    }

    #[test]
    fn status_header_cycles_through_phrases() {
        // First phrase, and wraps around after the last.
        assert_eq!(status_header(0), format!("🧠 {}", STATUS_PHRASES[0]));
        assert_eq!(
            status_header(STATUS_PHRASES.len()),
            format!("🧠 {}", STATUS_PHRASES[0])
        );
        assert_eq!(
            status_header(STATUS_PHRASES.len() + 1),
            format!("🧠 {}", STATUS_PHRASES[1])
        );
    }

    #[test]
    fn render_tool_line_plain_step_is_just_the_display() {
        assert_eq!(render_tool_line("📖 read: ~/x.md", None, true), "📖 read: ~/x.md");
    }

    #[test]
    fn render_tool_line_terminal_uses_fenced_block_on_slack() {
        // Slack (code_blocks=true) → command rendered as a ``` fenced block so it
        // shows as a bordered code preview, matching the reference design.
        assert_eq!(
            render_tool_line("💻 terminal", Some("python ~/app.py"), true),
            "💻 terminal\n```\npython ~/app.py\n```"
        );
    }

    #[test]
    fn render_tool_line_terminal_indents_on_plain_channel() {
        // Telegram (code_blocks=false) → indented plain lines, no literal fences.
        assert_eq!(
            render_tool_line("💻 terminal", Some("cd ~/app\nmake build"), false),
            "💻 terminal\n    cd ~/app\n    make build"
        );
    }

    #[test]
    fn extract_send_blocks_parses_marked_replies() {
        // Multiple blocks, trimmed; prose around them ignored.
        let text = "thinking…\n⟦otto-send⟧ Hello! ⟦/otto-send⟧ more\n⟦otto-send⟧line one\nline two⟦/otto-send⟧";
        assert_eq!(
            extract_send_blocks(text),
            vec!["Hello!".to_string(), "line one\nline two".to_string()]
        );
        // No markers → no blocks (caller falls back to the whole message).
        assert!(extract_send_blocks("just a normal reply").is_empty());
        // Unterminated marker is ignored.
        assert!(extract_send_blocks("⟦otto-send⟧ oops no close").is_empty());
        // Empty block skipped.
        assert!(extract_send_blocks("⟦otto-send⟧   ⟦/otto-send⟧").is_empty());
    }

    #[test]
    fn extract_file_paths_parses_marked_attachments() {
        // Multiple directives, trimmed; works alongside send blocks and prose.
        let text = "summary ⟦otto-send⟧hi⟦/otto-send⟧\n⟦otto-file⟧ /tmp/a.md ⟦/otto-file⟧ and \
                    ⟦otto-file⟧/tmp/b.md⟦/otto-file⟧";
        assert_eq!(
            extract_file_paths(text),
            vec!["/tmp/a.md".to_string(), "/tmp/b.md".to_string()]
        );
        // No markers / empty / unterminated → nothing to upload.
        assert!(extract_file_paths("no attachments here").is_empty());
        assert!(extract_file_paths("⟦otto-file⟧   ⟦/otto-file⟧").is_empty());
        assert!(extract_file_paths("⟦otto-file⟧/tmp/x.md no close").is_empty());
    }

    #[test]
    fn strip_file_directives_removes_markup_keeps_prose() {
        assert_eq!(
            strip_file_directives("Done. ⟦otto-file⟧/tmp/r.md⟦/otto-file⟧ See report."),
            "Done.  See report."
        );
        // Plain text is untouched.
        assert_eq!(strip_file_directives("just text"), "just text");
        // Unterminated marker is left verbatim (so it's visible, not silently eaten).
        assert_eq!(
            strip_file_directives("oops ⟦otto-file⟧/tmp/x"),
            "oops ⟦otto-file⟧/tmp/x"
        );
    }

    #[test]
    fn render_feed_trims_oldest_to_fit_budget() {
        // ~95 chars/line × 200 lines ≫ FEED_CHAR_BUDGET, forcing a trim.
        let lines: Vec<String> = (0..200)
            .map(|i| format!("step {i}: {}", "x".repeat(88)))
            .collect();
        let out = render_feed("🧠 done — 200 steps", &lines);

        assert!(
            out.chars().count() <= FEED_CHAR_BUDGET + 64,
            "stays within the channel char budget (plus the elision note)"
        );
        assert!(out.contains("earlier step"), "notes how many steps were elided");
        assert!(out.contains("step 199:"), "keeps the most recent step");
        assert!(!out.contains("step 0:"), "drops the oldest step");
    }
}
