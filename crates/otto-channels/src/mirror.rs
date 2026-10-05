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
use std::sync::Mutex as StdMutex;
use std::time::{Duration, Instant};

use tokio::io::AsyncReadExt;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

use otto_core::domain::{Channel, SessionStatus};
use otto_core::Id;
use otto_sessions::SessionManager;

use crate::adapter::Adapter;
use crate::attach_guard::{AttachmentPolicy, MAX_ATTACHMENT_BYTES};
use crate::secrets_redact::redact_secrets;
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
/// Maximum consecutive failed feed sends/edits (of any kind) before the feed
/// is disabled for the rest of the turn. A safety net on top of the permanent-
/// error detection: even an error we misclassify as transient can't retry
/// unboundedly.
const MAX_FEED_FAILURES: u32 = 5;
/// How long to pause feed sends after the channel rate-limits us (HTTP 429 /
/// Slack `ratelimited`). Well above [`EDIT_THROTTLE`] so a limited tailer backs
/// off instead of re-hitting the limit on its next tick.
const RATE_LIMIT_COOLDOWN: Duration = Duration::from_secs(60);
/// How many status ticks between session-liveness probes (~30s at
/// [`STATUS_TICK`]). A tailer whose session was deleted/archived/exited must
/// wind down instead of posting (and retrying) forever as a zombie.
const LIVENESS_EVERY_TICKS: u32 = 8;
/// Between turns the status ticker is paused; this slower timer keeps the
/// session-liveness probe running so a parked tailer still winds down when its
/// session is deleted/archived/exited.
const IDLE_LIVENESS: Duration = Duration::from_secs(30);
/// How often the rolling feed's header advances to the next liveness phrase
/// while a turn is in progress. Kept above [`EDIT_THROTTLE`] so a status tick is
/// always a legitimate (non-throttled) edit. Slack has no typing indicator, so
/// this rotating header is the only "still working" signal there.
const STATUS_TICK: Duration = Duration::from_millis(3500);

/// A turn with no transcript activity for this long is treated as over: an
/// interrupted or crashed turn never writes its `Final` line, which used to
/// leave the 300 ms transcript poll, the status ticker and the typing loop
/// running until the session exited (perf §15 N6). `begin_turn` re-arms them,
/// and so does any later `Tool` event (see [`TurnWatchdog`]).
const TURN_STALL_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// The stall watchdog for the tailer's live turn. A stall is a guess, not a
/// verdict: a legitimately silent tool (a 15-minute build or test run) looks
/// exactly like a crashed turn until it reports back. So a stall only parks
/// the turn — the next `Tool` event re-arms it (fast poll, typing, rotating
/// status header) instead of letting the turn finish with no liveness signal.
struct TurnWatchdog {
    timeout: Duration,
    /// Last transcript event (or turn start) — the watchdog's clock.
    last_activity: Instant,
    /// The live turn was parked by this watchdog (not ended by its `Final`).
    stalled: bool,
}

impl TurnWatchdog {
    fn new(timeout: Duration, now: Instant) -> Self {
        Self {
            timeout,
            last_activity: now,
            stalled: false,
        }
    }

    /// A fresh turn (`begin_turn`): restart the clock, forget any stall.
    fn begin_turn(&mut self, now: Instant) {
        self.last_activity = now;
        self.stalled = false;
    }

    /// A transcript event arrived. A `Tool` event on a stalled turn re-enters
    /// the active state; a `Final` just ends the turn (its arm idles it).
    fn on_activity(
        &mut self,
        now: Instant,
        is_final: bool,
        turn: &tokio::sync::watch::Sender<bool>,
    ) {
        self.last_activity = now;
        if std::mem::take(&mut self.stalled) && !is_final {
            turn.send_replace(true);
        }
    }

    /// A status wake during a live turn: has it gone quiet for `timeout`? If
    /// so, park it (drop to the idle cadence) and return `true`.
    fn check_stall(&mut self, now: Instant, turn: &tokio::sync::watch::Sender<bool>) -> bool {
        if now.saturating_duration_since(self.last_activity) < self.timeout {
            return false;
        }
        self.stalled = true;
        turn.send_replace(false);
        true
    }
}

/// The typing indicator loop. Persistent across turns: sends the typing
/// action every [`TYPING_INTERVAL`] while a turn is in flight, and between
/// turns parks on the `active` watch instead of waking every few seconds.
/// Adapters whose `typing` is the no-op default (Slack) never get a call —
/// the task just waits for the turn to end. Exits when `stop` is set or the
/// watch's senders are gone.
fn spawn_typing_loop(
    stop: Arc<AtomicBool>,
    mut active: tokio::sync::watch::Receiver<bool>,
    dest: Arc<StdMutex<Destination>>,
    every: Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            if active.wait_for(|on| *on).await.is_err() {
                return;
            }
            let d = current_dest(&dest);
            if !d.adapter.supports_typing() {
                if active.wait_for(|on| !*on).await.is_err() {
                    return;
                }
                continue;
            }
            let _ = d.adapter.typing(&d.chat).await;
            tokio::time::sleep(every).await;
        }
    })
}

/// The status loop's next wake: a [`STATUS_TICK`] while a turn is live
/// (`false` = refresh the header), only the [`IDLE_LIVENESS`] probe between
/// turns (`true` = idle — no feed edit).
async fn next_status_wake(
    turn_live: bool,
    ticker: &mut tokio::time::Interval,
    idle_probe: Duration,
) -> bool {
    if turn_live {
        ticker.tick().await;
        false
    } else {
        tokio::time::sleep(idle_probe).await;
        true
    }
}

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
    /// Whether a turn is in flight (on from `begin_turn`, off after its Final).
    /// Drives the typing indicator, the status ticker and the transcript poll
    /// cadence; a `watch` so all three sleep between turns and wake on the flip.
    typing_active: Arc<tokio::sync::watch::Sender<bool>>,
    /// Where the feed + reply go. Replaced by every `attach`, so the LATEST
    /// turn's destination wins: a webhook caller's own callback URL (a new
    /// `WebhookAdapter` per request — two automations sharing a conversation
    /// must not get each other's replies), and a Slack/Telegram adapter built
    /// with the current (possibly rotated) token instead of a revoked one.
    dest: Arc<StdMutex<Destination>>,
}

/// A session's channel destination (see [`SessionEntry::dest`]).
#[derive(Clone)]
struct Destination {
    adapter: Arc<dyn Adapter>,
    chat: String,
    thread: Option<String>,
    agent_reply: bool,
}

/// Snapshot the current destination (never held across an `.await`; a
/// poisoned lock still yields the last value written).
fn current_dest(dest: &StdMutex<Destination>) -> Destination {
    dest.lock().unwrap_or_else(|p| p.into_inner()).clone()
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
        let destination = Destination {
            adapter,
            chat,
            thread,
            agent_reply,
        };
        let mut guard = self.sessions.lock().await;

        // A live tailer already exists: just point it at this turn's
        // destination (it re-reads it when `begin_turn` starts the turn).
        if let Some(entry) = guard.get(&session_id) {
            *entry.dest.lock().unwrap_or_else(|p| p.into_inner()) = destination;
            return;
        }

        let cancel = Arc::new(AtomicBool::new(false));
        let new_turn = Arc::new(AtomicBool::new(false));
        let typing_active = Arc::new(tokio::sync::watch::Sender::new(true));
        let dest = Arc::new(StdMutex::new(destination));
        guard.insert(
            session_id.clone(),
            SessionEntry {
                cancel: Arc::clone(&cancel),
                new_turn: Arc::clone(&new_turn),
                typing_active: Arc::clone(&typing_active),
                dest: Arc::clone(&dest),
            },
        );
        drop(guard);

        // Transcript lines older than this attach belong to earlier turns — a
        // tailer re-attached to a live session's existing transcript must not
        // replay (and re-post) them. The bridge attaches BEFORE it submits the
        // turn's input, so every line of this turn is stamped at or after it.
        let since = chrono::Utc::now();
        let mirror = Arc::clone(self);
        tokio::spawn(async move {
            mirror
                .run_tailer(session_id, dest, since, cancel, new_turn, typing_active)
                .await;
        });
    }

    /// Signal that a new inbound comment started a fresh turn for `session_id`
    /// (reused sessions): the tailer posts a new activity feed and resumes the
    /// typing indicator. No-op if the session isn't tracked.
    pub async fn begin_turn(&self, session_id: &Id) {
        if let Some(e) = self.sessions.lock().await.get(session_id) {
            e.new_turn.store(true, Ordering::Relaxed);
            e.typing_active.send_replace(true);
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_tailer(
        &self,
        session_id: Id,
        dest: Arc<StdMutex<Destination>>,
        since: chrono::DateTime<chrono::Utc>,
        cancel: Arc<AtomicBool>,
        new_turn: Arc<AtomicBool>,
        typing_active: Arc<tokio::sync::watch::Sender<bool>>,
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
        let poll_active = typing_active.subscribe();

        tokio::spawn(async move {
            transcript::tail_adaptive(
                path,
                Some(since),
                move |evt| {
                    let _ = tx.send(evt);
                },
                cancel_clone,
                poll_active,
            )
            .await;
        });

        // --- Step 3: spawn the typing indicator task (parks between turns) ---
        let typing_stop = Arc::new(AtomicBool::new(false));
        spawn_typing_loop(
            Arc::clone(&typing_stop),
            typing_active.subscribe(),
            Arc::clone(&dest),
            TYPING_INTERVAL,
        );

        // This turn's destination; refreshed from `dest` whenever a new turn
        // starts (a later `attach` may have replaced it).
        let Destination {
            mut adapter,
            mut chat,
            mut thread,
            mut agent_reply,
        } = current_dest(&dest);

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
        // Feed health: a permanent send error (e.g. the thread can't be replied
        // to) or too many consecutive failures disables the feed for the rest
        // of the turn — without this, every status tick would retry the doomed
        // send forever, flooding the channel API into 429s.
        let mut feed = FeedHealth::new();
        let mut ticks_since_liveness: u32 = 0;
        // Slack renders mrkdwn (``` code fences) in chat.update text; Telegram's
        // in-place edit carries no parse mode, so fences would show literally —
        // there the command preview is rendered as plain indented lines instead.
        let mut code_blocks = matches!(adapter.channel(), Channel::Slack);

        // Liveness ticker: advances the header phrase while a turn is in flight.
        // Paused between turns (see the select below), where only the slower
        // IDLE_LIVENESS probe runs.
        let mut status_ticker = tokio::time::interval(STATUS_TICK);
        status_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut turn_rx = typing_active.subscribe();
        let mut watchdog = TurnWatchdog::new(TURN_STALL_TIMEOUT, Instant::now());

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
                feed = FeedHealth::new();
                typing_active.send_replace(true);
                watchdog.begin_turn(Instant::now());
                let d = current_dest(&dest);
                adapter = d.adapter;
                chat = d.chat;
                thread = d.thread;
                agent_reply = d.agent_reply;
                code_blocks = matches!(adapter.channel(), Channel::Slack);
            }

            // Seen-mark the current turn state so `changed()` below only fires
            // on a flip that happened after this point.
            let turn_live = *turn_rx.borrow_and_update();
            tokio::select! {
                maybe_evt = rx.recv() => {
                    let Some(evt) = maybe_evt else { break; };
                    if cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    // Re-arms a stalled turn on its next Tool event.
                    let is_final = matches!(evt, TranscriptEvent::Final { .. });
                    watchdog.on_activity(Instant::now(), is_final, &typing_active);
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
                            if feed.can_send() && last_edit.elapsed() >= EDIT_THROTTLE {
                                last_edit = Instant::now();
                                let body = render_feed(&status_header(status_idx), &activity_lines);
                                feed.apply(post_or_edit_feed(&adapter, &chat, thread.as_deref(), &mut rolling_msg_id, &body).await);
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
                            typing_active.send_replace(false);

                            // Freeze the rolling feed to a final "done — N steps".
                            let n = activity_lines.len();
                            let header = format!("🧠 done — {n} step{}", if n == 1 { "" } else { "s" });
                            let done_body = render_feed(&header, &activity_lines);
                            last_edit = Instant::now();
                            if feed.can_send() {
                                feed.apply(post_or_edit_feed(&adapter, &chat, thread.as_deref(), &mut rolling_msg_id, &done_body).await);
                            }

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
                                    // Scrub tokens/passwords the agent may have
                                    // echoed (emails stay — they're content).
                                    let cleaned = redact_secrets(&strip_file_directives(body), true);
                                    let cleaned = cleaned.trim();
                                    if !cleaned.is_empty() {
                                        post_reply(&adapter, &chat, thread.as_deref(), cleaned).await;
                                    }
                                }
                                for path in &file_paths {
                                    upload_file_path(&adapter, &chat, thread.as_deref(), path, &cwd).await;
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
                // A turn flip (begin_turn / our own Final) — loop round so a
                // new turn is picked up at once even while the ticker is paused.
                changed = turn_rx.changed() => {
                    if changed.is_err() {
                        break;
                    }
                }
                idle = next_status_wake(turn_live, &mut status_ticker, IDLE_LIVENESS) => {
                    if cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    // A turn that never wrote its Final (interrupt / crash)
                    // drops back to the idle cadence instead of polling at
                    // 300 ms and ticking the header until the session exits.
                    // The next Tool event re-arms it (see `TurnWatchdog`).
                    if !idle && watchdog.check_stall(Instant::now(), &typing_active) {
                        info!(session = %session_id, "mirror: turn stalled, pausing feed + typing");
                        continue;
                    }
                    // Liveness probe: a tailer must not outlive its session. The
                    // task is a detached spawn whose cancel flag is only set on
                    // shutdown, so without this check a deleted/archived/exited
                    // session leaves a zombie tailer posting (and retrying) into
                    // the channel forever. Every IDLE_LIVENESS tick between turns;
                    // every LIVENESS_EVERY_TICKS status ticks during one.
                    ticks_since_liveness += 1;
                    if idle || ticks_since_liveness >= LIVENESS_EVERY_TICKS {
                        ticks_since_liveness = 0;
                        match self.manager.get(&session_id).await {
                            Ok(s) if !s.archived && s.status != SessionStatus::Exited => {}
                            _ => {
                                info!(session = %session_id, "mirror: session gone, stopping tailer");
                                break;
                            }
                        }
                    }
                    // While the turn is live, refresh the header (creating the feed
                    // even before the first tool call so a long opening think still
                    // shows "Analyzing…"), then advance the phrase for next time.
                    // `interval`'s first tick fires immediately, so rendering before
                    // the increment makes "Analyzing…" (idx 0) the opening phrase.
                    if !idle {
                        if feed.can_send() && last_edit.elapsed() >= EDIT_THROTTLE {
                            last_edit = Instant::now();
                            let body = render_feed(&status_header(status_idx), &activity_lines);
                            feed.apply(post_or_edit_feed(&adapter, &chat, thread.as_deref(), &mut rolling_msg_id, &body).await);
                        }
                        status_idx = status_idx.wrapping_add(1);
                    }
                }
            }
        }

        // Tailer winding down — stop the typing task, and the transcript poller
        // (it only exits on `cancel`; a liveness-probe exit used to leave it
        // polling forever, one leaked poller per re-attach).
        typing_stop.store(true, Ordering::Relaxed);
        cancel.store(true, Ordering::Relaxed);

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

/// Outcome of a rolling-feed send/edit attempt, driving the caller's retry
/// policy: `Permanent` kills the feed for the rest of the turn, `RateLimited`
/// backs off for [`RATE_LIMIT_COOLDOWN`], `Transient` just waits for the next
/// throttled tick.
#[derive(Debug, PartialEq)]
enum FeedSend {
    Ok,
    Transient,
    RateLimited,
    Permanent,
}

/// Channel errors that will never succeed on retry for this feed — the thread
/// target refuses replies (Slack rejects threading onto join/system messages),
/// or the destination itself is gone. Matched on the adapter's error string,
/// which embeds the Slack API error code / the Telegram `description` verbatim.
fn classify_send_error(e: &anyhow::Error) -> FeedSend {
    let s = e.to_string();
    // Telegram refuses an edit whose text is unchanged — the feed already
    // shows exactly this, so it is a success, not a failure to count.
    if s.contains("message is not modified") {
        return FeedSend::Ok;
    }
    const PERMANENT: &[&str] = &[
        // Telegram Bot API descriptions.
        "chat not found",
        "bot was blocked by the user",
        "bot was kicked",
        "user is deactivated",
        "message to be replied not found",
        "message to edit not found",
        "not enough rights to send",
        "have no rights to send",
        "Unauthorized",
        // Slack Web API error codes.
        "cannot_reply_to_message",
        "thread_not_found",
        "message_not_found",
        "channel_not_found",
        "is_archived",
        "not_in_channel",
        "account_inactive",
        "token_revoked",
        "invalid_auth",
    ];
    if PERMANENT.iter().any(|p| s.contains(p)) {
        FeedSend::Permanent
    } else if s.contains("Too Many Requests") || s.contains("ratelimited") {
        FeedSend::RateLimited
    } else {
        FeedSend::Transient
    }
}

/// Per-turn feed retry policy. Trips permanently on a [`FeedSend::Permanent`]
/// error or [`MAX_FEED_FAILURES`] consecutive failures, and backs off for
/// [`RATE_LIMIT_COOLDOWN`] after a rate limit. Rebuilt each turn — a new turn
/// may target a different (repliable) thread.
struct FeedHealth {
    dead: bool,
    failures: u32,
    cooldown_until: Instant,
}

impl FeedHealth {
    fn new() -> Self {
        Self {
            dead: false,
            failures: 0,
            cooldown_until: Instant::now(),
        }
    }

    fn can_send(&self) -> bool {
        !self.dead && Instant::now() >= self.cooldown_until
    }

    fn apply(&mut self, outcome: FeedSend) {
        match outcome {
            FeedSend::Ok => self.failures = 0,
            FeedSend::Permanent => {
                self.dead = true;
                warn!("mirror feed: permanent send error — feed disabled for this turn");
            }
            FeedSend::RateLimited => {
                self.failures += 1;
                self.cooldown_until = Instant::now() + RATE_LIMIT_COOLDOWN;
            }
            FeedSend::Transient => self.failures += 1,
        }
        if !self.dead && self.failures >= MAX_FEED_FAILURES {
            self.dead = true;
            warn!(
                "mirror feed: {MAX_FEED_FAILURES} consecutive send failures — feed disabled for this turn"
            );
        }
    }
}

/// Post `body` as a new rolling-feed message, or edit the existing one in place.
/// Best-effort: send/edit failures are logged and swallowed, but the returned
/// [`FeedSend`] tells the caller whether retrying can ever work.
async fn post_or_edit_feed(
    adapter: &Arc<dyn Adapter>,
    chat: &str,
    thread: Option<&str>,
    rolling_msg_id: &mut Option<String>,
    body: &str,
) -> FeedSend {
    match rolling_msg_id.as_deref() {
        None => match adapter.send(chat, thread, body).await {
            Ok(mid) => {
                *rolling_msg_id = Some(mid);
                FeedSend::Ok
            }
            Err(e) => {
                warn!("mirror feed send: {e}");
                classify_send_error(&e)
            }
        },
        Some(mid) => match adapter.edit(chat, mid, body).await {
            Ok(()) => FeedSend::Ok,
            Err(e) => {
                warn!("mirror feed edit: {e}");
                classify_send_error(&e)
            }
        },
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
use otto_core::text::prefix_chars as truncate_to_char_boundary;

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
///
/// The path is agent output, i.e. attacker-influenced (prompt injection), and
/// the upload is the DAEMON's egress — so it is confined by
/// [`AttachmentPolicy`] (session cwd, /tmp, Otto artifact dirs; never secrets,
/// keys or Otto's own DB/credentials) before a single byte is read. A refusal
/// is logged with the resolved reason and noted in the thread.
async fn upload_file_path(
    adapter: &Arc<dyn Adapter>,
    chat: &str,
    thread: Option<&str>,
    path: &str,
    cwd: &str,
) {
    let filename = std::path::Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("attachment")
        .to_string();
    let policy = AttachmentPolicy::for_session(cwd);
    let canon = match policy.vet(path, cwd) {
        Ok((canon, _len)) => canon,
        Err(reason) => {
            warn!(
                path,
                cwd,
                reason = reason.as_str(),
                "mirror: refused ⟦otto-file⟧ attachment"
            );
            let note = format!("⚠️ Otto did not attach `{filename}`: {reason}.");
            if let Err(e) = adapter.send(chat, thread, &note).await {
                warn!("mirror: attachment-refusal note: {e}");
            }
            return;
        }
    };
    // Read the CANONICAL path (no symlinks left to swap) and re-apply the size
    // cap on the bytes actually read, so a file grown after the check can't
    // blow the buffer.
    let bytes = match tokio::fs::File::open(&canon).await {
        Ok(f) => {
            let mut buf = Vec::new();
            match f.take(MAX_ATTACHMENT_BYTES + 1).read_to_end(&mut buf).await {
                Ok(_) => buf,
                Err(e) => {
                    warn!(
                        "mirror: could not read file to attach {}: {e}",
                        canon.display()
                    );
                    return;
                }
            }
        }
        Err(e) => {
            warn!(
                "mirror: could not open file to attach {}: {e}",
                canon.display()
            );
            return;
        }
    };
    if bytes.len() as u64 > MAX_ATTACHMENT_BYTES {
        warn!(path = %canon.display(), "mirror: refused ⟦otto-file⟧ attachment: grew past the size cap");
        return;
    }
    // Upload the raw bytes verbatim — a UTF-8 round-trip would corrupt
    // binary attachments (images, PDFs, …).
    match adapter.upload(chat, thread, &filename, &bytes).await {
        Ok(()) => info!(
            file = filename.as_str(),
            resolved = %canon.display(),
            bytes = bytes.len(),
            "mirror: uploaded agent file attachment"
        ),
        Err(e) => warn!("mirror: file upload {}: {e}", canon.display()),
    }
}

/// Attempts for one final-reply post (the first try + retries).
const REPLY_ATTEMPTS: u32 = 3;
/// Wait before re-posting a rate-limited reply when the channel named no delay
/// (Telegram's 429 carries none through the adapter).
const REPLY_RETRY_DEFAULT: Duration = Duration::from_secs(5);
/// Longest single wait honoured before re-posting a rate-limited reply.
const REPLY_RETRY_MAX: Duration = Duration::from_secs(30);

/// The delay a rate-limit error asks for — `… (retry after Ns)`, as the Slack
/// adapter words an HTTP 429 with its `Retry-After`. `None` when absent.
fn retry_after_hint(e: &anyhow::Error) -> Option<Duration> {
    let s = e.to_string();
    let rest = &s[s.find("retry after ")? + "retry after ".len()..];
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse::<u64>().ok().map(Duration::from_secs)
}

/// Send one reply message, re-posting it after a rate limit. The agent's
/// final answer is the one message of the turn that matters, and it used to be
/// tried once: a 429 (likely right after a busy feed's edits) dropped it with
/// only a log line, leaving the thread on "done — N steps" and no answer. Only
/// a rate limit is retried — the platform refused the post, so a retry can't
/// duplicate it; a timeout might have landed and is not re-sent.
async fn send_reply_with_retry(
    adapter: &Arc<dyn Adapter>,
    chat: &str,
    thread: Option<&str>,
    text: &str,
) -> anyhow::Result<String> {
    let mut attempt = 1;
    loop {
        match adapter.send_formatted(chat, thread, text).await {
            Ok(id) => return Ok(id),
            Err(e)
                if attempt < REPLY_ATTEMPTS && classify_send_error(&e) == FeedSend::RateLimited =>
            {
                let wait = retry_after_hint(&e)
                    .unwrap_or(REPLY_RETRY_DEFAULT)
                    .min(REPLY_RETRY_MAX);
                warn!("mirror reply rate-limited (attempt {attempt}), re-posting in {wait:?}: {e}");
                tokio::time::sleep(wait).await;
                attempt += 1;
            }
            Err(e) => return Err(e),
        }
    }
}

/// Post one reply message to the channel via the adapter (the bot that received
/// the message). Long replies post a short head + an `investigation.md` upload.
/// Uses `send_formatted` so Slack mrkdwn and Telegram Markdown entities render
/// (bold, italic, code, links) in the relayed agent reply. Rate-limited posts
/// are re-sent ([`send_reply_with_retry`]).
async fn post_reply(adapter: &Arc<dyn Adapter>, chat: &str, thread: Option<&str>, text: &str) {
    if text.chars().count() > LONG_REPLY_THRESHOLD {
        let head = truncate_to_char_boundary(text, LONG_REPLY_HEAD_CHARS);
        let head_msg = format!("{head}\n\n📎 full reply attached as investigation.md");
        if let Err(e) = send_reply_with_retry(adapter, chat, thread, &head_msg).await {
            warn!("mirror final-head-send: {e}");
        }
        if let Err(e) = adapter
            .upload(chat, thread, "investigation.md", text.as_bytes())
            .await
        {
            warn!("mirror upload: {e}");
        }
    } else if let Err(e) = send_reply_with_retry(adapter, chat, thread, text).await {
        warn!("mirror final-send: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fails `send_formatted` with each queued error, then succeeds.
    struct FlakyAdapter {
        errors: StdMutex<Vec<String>>,
        calls: std::sync::atomic::AtomicU32,
    }

    #[async_trait::async_trait]
    impl Adapter for FlakyAdapter {
        async fn send(&self, _c: &str, _t: Option<&str>, _x: &str) -> anyhow::Result<String> {
            unreachable!("replies go through send_formatted")
        }
        async fn send_formatted(
            &self,
            _c: &str,
            _t: Option<&str>,
            _x: &str,
        ) -> anyhow::Result<String> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            let mut errs = self.errors.lock().unwrap();
            if errs.is_empty() {
                Ok("ts1".into())
            } else {
                Err(anyhow::anyhow!(errs.remove(0)))
            }
        }
        async fn edit(&self, _c: &str, _m: &str, _x: &str) -> anyhow::Result<()> {
            Ok(())
        }
        fn channel(&self) -> Channel {
            Channel::Slack
        }
    }

    fn flaky(errors: &[&str]) -> Arc<FlakyAdapter> {
        Arc::new(FlakyAdapter {
            errors: StdMutex::new(errors.iter().map(|s| s.to_string()).collect()),
            calls: std::sync::atomic::AtomicU32::new(0),
        })
    }

    /// Counts typing / edit calls; reports `supports_typing() == true` (the
    /// Telegram shape, the one with a live typing loop).
    #[derive(Default)]
    struct CountingAdapter {
        typing: std::sync::atomic::AtomicU32,
        edits: std::sync::atomic::AtomicU32,
    }

    #[async_trait::async_trait]
    impl Adapter for CountingAdapter {
        async fn send(&self, _c: &str, _t: Option<&str>, _x: &str) -> anyhow::Result<String> {
            Ok("m1".into())
        }
        async fn edit(&self, _c: &str, _m: &str, _x: &str) -> anyhow::Result<()> {
            self.edits.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
        fn channel(&self) -> Channel {
            Channel::Telegram
        }
        async fn typing(&self, _c: &str) -> anyhow::Result<()> {
            self.typing.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
        fn supports_typing(&self) -> bool {
            true
        }
    }

    /// perf §15 N2: between turns the typing loop is parked on the watch —
    /// zero typing calls while no turn is live — and a turn start
    /// (`begin_turn` flips the watch) resumes it at once, once per interval,
    /// until the turn ends again. Real clock, scaled cadence (40 ms for 4 s).
    #[tokio::test]
    async fn typing_loop_parks_between_turns_and_resumes_on_begin_turn() {
        let every = Duration::from_millis(40);
        let a = Arc::new(CountingAdapter::default());
        let dest = Arc::new(StdMutex::new(Destination {
            adapter: a.clone(),
            chat: "c".into(),
            thread: None,
            agent_reply: false,
        }));
        let active = tokio::sync::watch::Sender::new(false);
        let stop = Arc::new(AtomicBool::new(false));
        let task = spawn_typing_loop(stop.clone(), active.subscribe(), dest, every);
        // 15 intervals with no turn: parked.
        tokio::time::sleep(every * 15).await;
        assert_eq!(a.typing.load(Ordering::Relaxed), 0, "parked between turns");

        active.send_replace(true);
        tokio::time::sleep(every / 4).await;
        assert_eq!(a.typing.load(Ordering::Relaxed), 1, "resumes at once");
        tokio::time::sleep(every * 5).await;
        let live = a.typing.load(Ordering::Relaxed);
        assert!((4..=7).contains(&live), "about one per interval: {live}");

        active.send_replace(false);
        tokio::time::sleep(every * 2).await;
        let after = a.typing.load(Ordering::Relaxed);
        tokio::time::sleep(every * 15).await;
        assert_eq!(a.typing.load(Ordering::Relaxed), after, "parked again");

        stop.store(true, Ordering::Relaxed);
        drop(active);
        task.await.unwrap();
        assert_eq!(a.edits.load(Ordering::Relaxed), 0);
    }

    /// perf §15 N2: between turns the status loop wakes only for the slow
    /// liveness probe (an idle wake never refreshes the feed header); during
    /// a turn it ticks every STATUS_TICK. Real clock, scaled cadence.
    #[tokio::test]
    async fn status_wakes_park_between_turns() {
        let tick = Duration::from_millis(20);
        let probe = Duration::from_millis(150);
        let mut ticker = tokio::time::interval(tick);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let start = Instant::now();
        let (mut header_refreshes, mut wakes) = (0, 0);
        while start.elapsed() < probe * 3 {
            wakes += 1;
            if !next_status_wake(false, &mut ticker, probe).await {
                header_refreshes += 1;
            }
        }
        assert_eq!(header_refreshes, 0, "no feed edits between turns");
        assert_eq!(wakes, 3, "only the liveness probe");

        let start = Instant::now();
        let mut live = 0;
        while start.elapsed() < tick * 10 {
            if !next_status_wake(true, &mut ticker, probe).await {
                live += 1;
            }
        }
        assert!(live >= 8, "{live} header ticks in 10 intervals");
    }

    /// perf §15 N6: a turn that never writes its Final drops to idle after
    /// the stall timeout without transcript activity.
    #[test]
    fn a_silent_turn_stalls_after_the_timeout() {
        let t0 = Instant::now();
        let turn = tokio::sync::watch::Sender::new(true);
        let mut w = TurnWatchdog::new(TURN_STALL_TIMEOUT, t0);
        assert!(!w.check_stall(t0, &turn));
        assert!(!w.check_stall(t0 + TURN_STALL_TIMEOUT - Duration::from_secs(1), &turn));
        assert!(*turn.borrow(), "still live just under the timeout");
        // A clock that went "backwards" (activity stamped after `now`) is fresh.
        let mut ahead = TurnWatchdog::new(TURN_STALL_TIMEOUT, t0 + Duration::from_secs(5));
        assert!(!ahead.check_stall(t0, &turn));
        assert!(w.check_stall(t0 + TURN_STALL_TIMEOUT, &turn));
        assert!(!*turn.borrow(), "a stalled turn drops to idle");
    }

    /// perf §15 S3: a stall is not the end of the turn. A legitimately silent
    /// tool (a long build) reports back later; its Tool event must re-enter
    /// the active state (fast poll, typing, status header) — every watcher of
    /// the turn flag (tailer poll, typing loop, status ticker) sees the flip.
    /// A Final while stalled, or activity on a turn that never stalled, must
    /// not (re-)arm anything. Shortened stall interval.
    #[test]
    fn a_stalled_turn_re_arms_on_new_tool_activity() {
        let stall = Duration::from_millis(50);
        let t0 = Instant::now();
        let turn = tokio::sync::watch::Sender::new(true);
        let mut poll = turn.subscribe();
        let mut w = TurnWatchdog::new(stall, t0);

        // Live activity just keeps the clock fresh — no spurious flips.
        w.on_activity(t0 + stall / 2, false, &turn);
        assert!(!w.check_stall(t0 + stall, &turn));
        assert!(!poll.has_changed().unwrap(), "no flip while live");

        // Silent past the timeout → idle.
        let t1 = t0 + stall / 2 + stall;
        assert!(w.check_stall(t1, &turn));
        assert!(poll.has_changed().unwrap());
        assert!(!*poll.borrow_and_update(), "stalled → idle cadence");

        // The long tool finishes: its Tool event re-arms the turn.
        let t2 = t1 + stall * 4;
        w.on_activity(t2, false, &turn);
        assert!(poll.has_changed().unwrap(), "watchers woken");
        assert!(*poll.borrow_and_update(), "new activity → active again");
        // …with a fresh clock: not instantly re-stalled, but it can stall again.
        assert!(!w.check_stall(t2 + stall / 2, &turn));
        assert!(w.check_stall(t2 + stall, &turn));
        assert!(!*poll.borrow_and_update());

        // A Final that arrives while stalled ends the turn — it stays idle.
        w.on_activity(t2 + stall * 2, true, &turn);
        assert!(!poll.has_changed().unwrap(), "Final never re-arms");
        // And after that, stray activity doesn't resurrect the ended turn.
        w.on_activity(t2 + stall * 3, false, &turn);
        assert!(!*turn.borrow());

        // begin_turn forgets the stall state.
        w.check_stall(t2 + stall * 10, &turn);
        w.begin_turn(t2 + stall * 10);
        turn.send_replace(true);
        poll.borrow_and_update();
        w.on_activity(t2 + stall * 11, false, &turn);
        assert!(
            !poll.has_changed().unwrap(),
            "fresh turn — nothing to re-arm"
        );
    }

    #[tokio::test]
    async fn a_rate_limited_final_reply_is_re_posted() {
        let a = flaky(&[
            "slack chat.postMessage (formatted): ratelimited (retry after 0s)",
            "slack chat.postMessage (formatted): ratelimited (retry after 0s)",
        ]);
        let dyn_a: Arc<dyn Adapter> = a.clone();
        let id = send_reply_with_retry(&dyn_a, "C1", Some("1.2"), "the answer")
            .await
            .expect("delivered on the third attempt");
        assert_eq!(id, "ts1");
        assert_eq!(a.calls.load(Ordering::Relaxed), 3);
    }

    #[tokio::test]
    async fn retries_are_bounded_and_only_for_rate_limits() {
        // Three 429s in a row: gives up after REPLY_ATTEMPTS.
        let a = flaky(&["ratelimited (retry after 0s)"; 3]);
        let dyn_a: Arc<dyn Adapter> = a.clone();
        assert!(send_reply_with_retry(&dyn_a, "C1", None, "x")
            .await
            .is_err());
        assert_eq!(a.calls.load(Ordering::Relaxed), REPLY_ATTEMPTS);
        // A permanent error, or a timeout that may have landed, is not re-sent.
        for err in [
            "slack chat.postMessage: channel_not_found",
            "error sending request: operation timed out",
        ] {
            let a = flaky(&[err]);
            let dyn_a: Arc<dyn Adapter> = a.clone();
            assert!(send_reply_with_retry(&dyn_a, "C1", None, "x")
                .await
                .is_err());
            assert_eq!(a.calls.load(Ordering::Relaxed), 1, "{err}");
        }
    }

    #[test]
    fn retry_after_hint_reads_the_adapter_wording() {
        let e = anyhow::anyhow!("slack chat.postMessage: ratelimited (retry after 7s)");
        assert_eq!(retry_after_hint(&e), Some(Duration::from_secs(7)));
        let e = anyhow::anyhow!("HTTP status client error (429 Too Many Requests)");
        assert_eq!(retry_after_hint(&e), None);
        assert_eq!(classify_send_error(&e), FeedSend::RateLimited);
    }

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
        assert_eq!(
            render_tool_line("📖 read: ~/x.md", None, true),
            "📖 read: ~/x.md"
        );
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
    fn classify_send_error_matches_slack_error_codes() {
        // Slack API errors embedded by the adapter (verbatim error code).
        for code in [
            "cannot_reply_to_message",
            "thread_not_found",
            "channel_not_found",
            "is_archived",
        ] {
            let e = anyhow::anyhow!("slack chat.postMessage: {code}");
            assert_eq!(classify_send_error(&e), FeedSend::Permanent, "{code}");
        }
        // HTTP 429 (reqwest error_for_status Display) and Slack's own code.
        let e = anyhow::anyhow!(
            "HTTP status client error (429 Too Many Requests) for url (https://slack.com/api/chat.postMessage)"
        );
        assert_eq!(classify_send_error(&e), FeedSend::RateLimited);
        let e = anyhow::anyhow!("slack chat.postMessage: ratelimited");
        assert_eq!(classify_send_error(&e), FeedSend::RateLimited);
        // Anything else (network blips, timeouts) retries on the next tick.
        let e = anyhow::anyhow!("error sending request: connection reset by peer");
        assert_eq!(classify_send_error(&e), FeedSend::Transient);
    }

    #[test]
    fn classify_send_error_matches_telegram_descriptions() {
        for desc in [
            "Bad Request: chat not found",
            "Forbidden: bot was blocked by the user",
            "Forbidden: bot was kicked from the group chat",
            "Bad Request: message to be replied not found",
            "Unauthorized",
        ] {
            let e = anyhow::anyhow!("Telegram sendMessage failed: {desc}");
            assert_eq!(classify_send_error(&e), FeedSend::Permanent, "{desc}");
        }
        let e =
            anyhow::anyhow!("Telegram editMessageText failed: Too Many Requests: retry after 5");
        assert_eq!(classify_send_error(&e), FeedSend::RateLimited);
        // An unchanged edit is a no-op success, not a failure to count.
        let e = anyhow::anyhow!(
            "Telegram editMessageText failed: Bad Request: message is not modified: specified new message content and reply markup are exactly the same"
        );
        assert_eq!(classify_send_error(&e), FeedSend::Ok);
    }

    #[test]
    fn feed_health_trips_on_permanent_error() {
        let mut feed = FeedHealth::new();
        assert!(feed.can_send());
        feed.apply(FeedSend::Permanent);
        assert!(
            !feed.can_send(),
            "permanent error kills the feed for the turn"
        );
    }

    #[test]
    fn feed_health_trips_after_max_consecutive_failures() {
        let mut feed = FeedHealth::new();
        for _ in 0..MAX_FEED_FAILURES - 1 {
            feed.apply(FeedSend::Transient);
        }
        // A success in between resets the counter.
        feed.apply(FeedSend::Ok);
        assert!(feed.can_send());
        for _ in 0..MAX_FEED_FAILURES {
            feed.apply(FeedSend::Transient);
        }
        assert!(
            !feed.can_send(),
            "cap on consecutive failures trips the feed"
        );
    }

    #[test]
    fn feed_health_backs_off_on_rate_limit() {
        let mut feed = FeedHealth::new();
        feed.apply(FeedSend::RateLimited);
        assert!(!feed.can_send(), "rate limit starts a cooldown");
        assert!(!feed.dead, "rate limit alone does not kill the feed");
        // Cooldown elapsed → sending resumes.
        feed.cooldown_until = Instant::now() - Duration::from_secs(1);
        assert!(feed.can_send());
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
        assert!(
            out.contains("earlier step"),
            "notes how many steps were elided"
        );
        assert!(out.contains("step 199:"), "keeps the most recent step");
        assert!(!out.contains("step 0:"), "drops the oldest step");
    }
}
