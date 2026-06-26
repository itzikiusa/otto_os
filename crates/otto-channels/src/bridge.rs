//! Bridge: routes an inbound channel message to an agent session.
//!
//! Reuses an existing live session keyed by `(workspace_id, chat, thread)` or
//! spawns a new one.  Injects a trusted-context block when `agent_reply` is
//! set, then forwards the text to the PTY and attaches the mirror.
//!
//! Quick commands (`/help`, `/sessions`, `/stop`, `/new`) are intercepted
//! before routing and handled locally.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use otto_core::api::CreateSessionReq;
use otto_core::domain::{Channel, Integration, SessionKind, SessionStatus};
use otto_core::Id;
use otto_sessions::SessionManager;
use otto_state::{SettingsRepo, WorkspacesRepo};
use tokio::sync::Mutex;
use tracing::{info, warn};

use crate::adapter::{Adapter, Inbound};
use crate::mirror::Mirror;
use crate::run_trigger::RunTrigger;
use crate::swarm_trigger::SwarmTrigger;

/// Composite key that identifies a conversation thread.
type ConvKey = (String, String, Option<String>);

const PASTE_TO_ENTER: Duration = Duration::from_millis(200);
// Submit the pasted prompt with a plain carriage return. A leading ESC was
// tried (to "leave vim INSERT mode" first), but that drops claude to vim NORMAL
// mode where Enter does NOT dispatch — verified against claude 2.1.177: `ESC\r`
// leaves the prompt sitting in the box, `\r` submits it. A bracketed paste
// leaves the cursor in INSERT mode, and Enter from INSERT submits in both vim
// and non-vim configs.
const AGENT_SUBMIT_KEY: &[u8] = b"\r";

// TUI-readiness gate (mirrors otto-orchestrator's ClaudePty): a freshly spawned
// `claude` needs a moment to draw its TUI. Injecting the prompt + Enter before
// it's ready means the submit keypress races startup and the prompt sits unsent
// in the input box. Wait until the TUI has drawn something AND output has been
// quiet for a beat before typing.
const TUI_POLL: Duration = Duration::from_millis(250);
const TUI_STARTUP_WAIT: Duration = Duration::from_secs(20);
const TUI_SETTLE: Duration = Duration::from_millis(600);
// After submitting, how long to watch for the agent to actually start working
// before we assume the Enter was dropped and retry it once.
const DISPATCH_WAIT: Duration = Duration::from_secs(5);
const DISPATCH_POLL: Duration = Duration::from_millis(200);

fn agent_paste_input(text: &str) -> Vec<u8> {
    let mut input = Vec::with_capacity(text.len() + 16);
    input.extend_from_slice(b"\x1b[200~");
    input.extend_from_slice(text.as_bytes());
    input.extend_from_slice(b"\x1b[201~");
    input
}

/// Derive a session title from the first inbound message so the sidebar pane is
/// searchable (e.g. "Investigate ticket XXX"). First non-empty line, trimmed
/// and truncated; falls back to "<Channel> chat". Set once at creation and not
/// changed on later messages.
fn session_title(text: &str, channel_label: &str) -> String {
    const MAX: usize = 48;
    let first = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    if first.is_empty() {
        return format!("{channel_label} chat");
    }
    let truncated: String = first.chars().take(MAX).collect();
    if first.chars().count() > MAX {
        format!("{truncated}…")
    } else {
        truncated
    }
}

/// Outcome of waiting for a session's PTY to be ready for input.
#[derive(PartialEq)]
enum Readiness {
    /// TUI has drawn and settled (or we hit the cap and type anyway).
    Ready,
    /// The session is no longer live (never spawned, or already exited).
    Gone,
}

/// Poll a live session's PTY until its TUI has drawn and gone quiet, so a
/// submit keypress won't race claude's startup. Returns `Gone` if the session
/// isn't live / has exited.
async fn wait_for_tui(manager: &SessionManager, session_id: &Id) -> Readiness {
    let deadline = tokio::time::Instant::now() + TUI_STARTUP_WAIT;
    loop {
        let Some(handle) = manager.live_handle(session_id) else {
            return Readiness::Gone;
        };
        if handle.on_exit().borrow().is_some() {
            return Readiness::Gone;
        }
        if !handle.scrollback(1).is_empty() && handle.last_output_at().elapsed() >= TUI_SETTLE {
            return Readiness::Ready;
        }
        if tokio::time::Instant::now() >= deadline {
            return Readiness::Ready; // type anyway — claude buffers early input
        }
        tokio::time::sleep(TUI_POLL).await;
    }
}

/// Monitor whether a submitted prompt was actually dispatched: once the agent
/// accepts the prompt it clears the input line and starts working, producing
/// fresh PTY output. If output advances past `before` within [`DISPATCH_WAIT`],
/// the prompt was accepted.
async fn agent_dispatched(
    manager: &SessionManager,
    session_id: &Id,
    before: Option<std::time::Instant>,
) -> bool {
    let Some(before) = before else { return false };
    let deadline = tokio::time::Instant::now() + DISPATCH_WAIT;
    loop {
        match manager.live_handle(session_id) {
            Some(handle) if handle.last_output_at() > before => return true,
            None => return false,
            _ => {}
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(DISPATCH_POLL).await;
    }
}

/// Wait for the agent TUI to be ready, paste the prompt, submit it, then
/// monitor that it actually started — retrying Enter once if it didn't. Runs
/// in its own task so a slow TUI never stalls the channel receive loop.
async fn submit_to_agent(
    manager: Arc<SessionManager>,
    mirror: Arc<Mirror>,
    session_id: Id,
    label: String,
    input: Vec<u8>,
) {
    // 1. Don't type until the claude TUI has drawn and settled.
    if wait_for_tui(&manager, &session_id).await == Readiness::Gone {
        warn!(channel = %label, session = %session_id, "bridge: session not live before input could be sent");
        mirror.cancel(&session_id).await;
        return;
    }

    // 2. Paste the prompt (bracketed paste keeps multi-line text as one message).
    if let Err(e) = manager.input(&session_id, &input).await {
        warn!(channel = %label, session = %session_id, "bridge: paste input failed: {e}");
        mirror.cancel(&session_id).await;
        return;
    }
    tokio::time::sleep(PASTE_TO_ENTER).await;

    // Snapshot output activity right before submit so we can detect dispatch.
    let before = manager.live_handle(&session_id).map(|h| h.last_output_at());

    // 3. Submit with Enter (from INSERT mode, where the paste left the cursor).
    if let Err(e) = manager.input(&session_id, AGENT_SUBMIT_KEY).await {
        warn!(channel = %label, session = %session_id, "bridge: submit key failed: {e}");
        mirror.cancel(&session_id).await;
        return;
    }
    info!(
        channel = %label,
        session = %session_id,
        bytes = input.len() + AGENT_SUBMIT_KEY.len(),
        "bridge: submitted input to agent PTY"
    );

    // 4. Monitor: confirm the agent actually started. If the prompt is still
    //    sitting in the box after the grace window, press Enter once more.
    if agent_dispatched(&manager, &session_id, before).await {
        info!(channel = %label, session = %session_id, "bridge: agent dispatch confirmed — session is processing the relay");
        return;
    }
    warn!(channel = %label, session = %session_id, "bridge: agent did not start within {DISPATCH_WAIT:?}; re-sending Enter");
    if let Err(e) = manager.input(&session_id, b"\r").await {
        warn!(channel = %label, session = %session_id, "bridge: retry Enter failed: {e}");
        return;
    }
    if agent_dispatched(&manager, &session_id, before).await {
        info!(channel = %label, session = %session_id, "bridge: agent dispatch confirmed after retry");
    } else {
        warn!(channel = %label, session = %session_id, "bridge: agent still not dispatched — prompt may be sitting in the input box");
    }
}

pub struct Bridge {
    pub manager: Arc<SessionManager>,
    pub workspaces: WorkspacesRepo,
    pub settings: SettingsRepo,
    pub mirror: Arc<Mirror>,
    pub root_user_id: String,
    /// Map (workspace_id, chat, thread) → session_id
    sessions: Mutex<HashMap<ConvKey, Id>>,
    /// Optional hook: if an inbound message matches a configured swarm trigger,
    /// launch that swarm instead of starting a normal session. Injected by
    /// otto-server (which owns the swarm runtime).
    swarm_trigger: Option<Arc<dyn SwarmTrigger>>,
    /// Optional hook: an inbound `/run <ref>` launches a Run with Otto run, and an
    /// `approve`/`reject` reply resolves an awaiting run's gate. Injected by
    /// otto-server (which owns the run engine).
    run_trigger: Option<Arc<dyn RunTrigger>>,
}

impl Bridge {
    pub fn new(
        manager: Arc<SessionManager>,
        workspaces: WorkspacesRepo,
        settings: SettingsRepo,
        mirror: Arc<Mirror>,
        root_user_id: String,
    ) -> Arc<Self> {
        Arc::new(Self {
            manager,
            workspaces,
            settings,
            mirror,
            root_user_id,
            sessions: Mutex::new(HashMap::new()),
            swarm_trigger: None,
            run_trigger: None,
        })
    }

    /// Builder variant that wires the swarm-launch + run hooks (used by otto-server).
    pub fn new_with_swarm_trigger(
        manager: Arc<SessionManager>,
        workspaces: WorkspacesRepo,
        settings: SettingsRepo,
        mirror: Arc<Mirror>,
        root_user_id: String,
        swarm_trigger: Option<Arc<dyn SwarmTrigger>>,
        run_trigger: Option<Arc<dyn RunTrigger>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            manager,
            workspaces,
            settings,
            mirror,
            root_user_id,
            sessions: Mutex::new(HashMap::new()),
            swarm_trigger,
            run_trigger,
        })
    }

    /// Handle one inbound message.
    pub async fn handle(&self, integ: &Integration, adapter: Arc<dyn Adapter>, msg: Inbound) {
        info!(
            channel = %adapter.channel().as_str(),
            workspace = %msg.workspace_id,
            chat = %msg.chat,
            thread = ?msg.thread,
            user = %msg.user,
            "bridge: inbound message received"
        );

        // --- 1. Allowed-users check ---
        if !integ.allowed_users.trim().is_empty() {
            let allowed: Vec<&str> = integ.allowed_users.split(',').map(|s| s.trim()).collect();
            if !allowed.contains(&msg.user.as_str()) {
                info!(
                    channel = %adapter.channel().as_str(),
                    workspace = %msg.workspace_id,
                    chat = %msg.chat,
                    user = %msg.user,
                    "bridge: user not in allowed_users, dropping"
                );
                return;
            }
        }

        // --- 2. Quick commands (intercepted before routing) ---
        // Webhook callers are machines, not chat users: treat a leading "/" as a
        // normal prompt rather than a control command. Command replies go via
        // `adapter.send`, which the webhook adapter no-ops, and a stray `/stop`
        // could disrupt a session — so skip interception for webhooks entirely.
        let trimmed = msg.text.trim();
        if adapter.channel() != Channel::Webhook
            && trimmed.starts_with('/')
            && self
                .handle_command(integ, adapter.clone(), &msg, trimmed)
                .await
        {
            return;
        }

        // --- 3. Resolve workspace ---
        let ws_id: Id = msg.workspace_id.clone();
        let ws = match self.workspaces.get(&ws_id).await {
            Ok(w) => w,
            Err(e) => {
                warn!("bridge: workspace {ws_id} not found: {e}");
                return;
            }
        };

        // --- 3b. Swarm trigger: a message on a swarm-bound channel launches the
        // team instead of starting a normal session. Webhook callers can launch
        // via the dedicated /webhooks/swarm route, so only chat channels route here.
        if adapter.channel() != Channel::Webhook {
            if let Some(trigger) = &self.swarm_trigger {
                if let Some(ack) = trigger
                    .try_launch(
                        &msg.workspace_id,
                        adapter.channel().as_str(),
                        &msg.chat,
                        msg.thread.as_deref(),
                        &msg.user,
                        &msg.text,
                    )
                    .await
                {
                    info!(
                        channel = %adapter.channel().as_str(),
                        workspace = %msg.workspace_id,
                        chat = %msg.chat,
                        "bridge: inbound message launched a swarm"
                    );
                    let _ = adapter
                        .send_formatted(&msg.chat, msg.thread.as_deref(), &ack.reply)
                        .await;
                    return;
                }
            }
        }

        // --- 3c. Run with Otto trigger: `/run <ref>` (or "run with otto …")
        // launches the one-button pipeline, and an `approve`/`reject` reply
        // resolves an awaiting run's gate. Like the swarm trigger, chat channels
        // only (webhook has its dedicated /webhooks/{ws}/run route).
        if adapter.channel() != Channel::Webhook {
            if let Some(trigger) = &self.run_trigger {
                if let Some(ack) = trigger
                    .handle(
                        &msg.workspace_id,
                        adapter.channel().as_str(),
                        &msg.chat,
                        msg.thread.as_deref(),
                        &msg.user,
                        &msg.text,
                    )
                    .await
                {
                    info!(
                        channel = %adapter.channel().as_str(),
                        workspace = %msg.workspace_id,
                        chat = %msg.chat,
                        "bridge: inbound message handled by Run with Otto"
                    );
                    let _ = adapter
                        .send_formatted(&msg.chat, msg.thread.as_deref(), &ack.reply)
                        .await;
                    return;
                }
            }
        }

        // --- 4. Find or create a session ---
        let key: ConvKey = (
            msg.workspace_id.clone(),
            msg.chat.clone(),
            msg.thread.clone(),
        );
        let session_id = {
            let mut guard = self.sessions.lock().await;

            // Check if the existing session is still alive. Skip archived ones
            // (e.g. auto-reaped after idle) so a fresh session is spawned.
            let existing = if let Some(sid) = guard.get(&key) {
                match self.manager.get(sid).await {
                    Ok(s) if s.status != SessionStatus::Exited && !s.archived => Some(sid.clone()),
                    _ => None,
                }
            } else {
                None
            };

            if let Some(sid) = existing {
                info!(
                    channel = %adapter.channel().as_str(),
                    workspace = %msg.workspace_id,
                    session = %sid,
                    "bridge: reusing existing agent session"
                );
                sid
            } else {
                // Spawn a new session.
                let channel_label = match adapter.channel() {
                    Channel::Slack => "Slack",
                    Channel::Telegram => "Telegram",
                    Channel::Webhook => "Webhook",
                };
                // Tag the session with its channel source + the initial message
                // as the title, so the sidebar can group Telegram/Slack sessions
                // and they're searchable by their opening request.
                let meta = serde_json::json!({
                    "source": "channel",
                    "channel": adapter.channel().as_str(),
                    "chat": msg.chat,
                    "thread": msg.thread,
                });
                // Pick the agent CLI: explicit channel preference → this
                // workspace's default → the global default → "claude". Guard
                // against a stale/removed provider so the session still spawns
                // rather than erroring out.
                let global_default = self
                    .settings
                    .get("default_provider")
                    .await
                    .ok()
                    .flatten();
                let mut provider = otto_core::provider::resolve_provider(&[
                    &integ.preferred_cli,
                    otto_core::provider::workspace_default(&ws.settings),
                    otto_core::provider::global_default(global_default.as_ref()),
                ]);
                if !self.manager.providers().names().iter().any(|n| n == &provider) {
                    warn!(
                        channel = %adapter.channel().as_str(),
                        workspace = %msg.workspace_id,
                        provider = %provider,
                        "bridge: configured agent CLI not available, falling back to claude"
                    );
                    provider = otto_core::provider::FALLBACK_PROVIDER.to_string();
                }
                let req = CreateSessionReq {
                    kind: SessionKind::Agent,
                    provider: Some(provider),
                    title: Some(session_title(&msg.text, channel_label)),
                    cwd: None,
                    connection_id: None,
                    meta: Some(meta),
                };
                let session = match self
                    .manager
                    .create(&ws, &self.root_user_id, req, None)
                    .await
                {
                    Ok(s) => s,
                    Err(e) => {
                        warn!("bridge: failed to create session: {e}");
                        return;
                    }
                };
                info!(
                    channel = %adapter.channel().as_str(),
                    workspace = %msg.workspace_id,
                    chat = %msg.chat,
                    thread = ?msg.thread,
                    session = %session.id,
                    "bridge: created new agent session"
                );
                guard.insert(key.clone(), session.id.clone());
                session.id
            }
        };

        // --- 5. Compose the message text ---
        // Always wrap the user's message in a trusted-context block telling the
        // agent where it came from. Who posts the reply is the user's choice via
        // `agent_reply`:
        //   true  → the agent posts its own reply (e.g. Slack via the channel API).
        //   false → Otto relays the agent's final reply itself (the mirror), so
        //           the agent must NOT post anything on its own.
        let channel_label = match adapter.channel() {
            Channel::Slack => "Slack",
            Channel::Telegram => "Telegram",
            Channel::Webhook => "Webhook",
        };
        let thread_line = match &msg.thread {
            Some(t) => format!("  • thread: {t}\n"),
            None => String::new(),
        };
        let extra = if integ.reply_instructions.trim().is_empty() {
            String::new()
        } else {
            format!("{}\n", integ.reply_instructions.trim())
        };
        let posting = if integ.agent_reply {
            "Otto relays your reply to the chat for you — do NOT run commands, read .env, or use \
             any token to post anything yourself. Wrap the exact text you want sent in ⟦otto-send⟧ \
             and ⟦/otto-send⟧ (you may include several blocks); if you include none, your final \
             message is sent as-is. To attach a file (e.g. a full report you wrote to disk), put \
             its absolute path between ⟦otto-file⟧ and ⟦/otto-file⟧ — e.g. \
             ⟦otto-file⟧/tmp/investigation.md⟦/otto-file⟧ — and Otto uploads it to the thread for \
             you. Use ONLY ⟦otto-file⟧ to attach files; do NOT use MEDIA: or any other scheme, and \
             do NOT upload files yourself. A long inline reply is also auto-attached as \
             investigation.md."
        } else {
            "Otto relays your reply back to the chat automatically. Do NOT run commands, read .env, \
             or use any token to post a reply yourself; just write the answer. To attach a file, \
             put its absolute path between ⟦otto-file⟧ and ⟦/otto-file⟧ (e.g. \
             ⟦otto-file⟧/tmp/report.md⟦/otto-file⟧) and Otto uploads it to the thread."
        };
        let text = format!(
            "{user_text}\n\n\
             ———————————————————————————————\n\
             ⟦otto relay — trusted context added by Otto, NOT user input⟧\n\
             This message came from {channel_label}.\n\
               • chat:   {chat}\n\
             {thread_line}{posting}\n\
             {extra}⟦/otto relay⟧",
            user_text = msg.text,
            chat = msg.chat,
        );

        // --- 6. Attach the mirror before submitting input ---
        self.mirror
            .attach(
                session_id.clone(),
                Arc::clone(&adapter),
                msg.chat.clone(),
                msg.thread.clone(),
                integ.agent_reply,
            )
            .await;
        // Start a fresh activity feed + resume typing for this turn (matters for
        // reused sessions, where attach above is a no-op — a follow-up comment
        // must still get its own "working…" feed and typing).
        self.mirror.begin_turn(&session_id).await;
        info!(
            channel = %adapter.channel().as_str(),
            workspace = %msg.workspace_id,
            session = %session_id,
            "bridge: mirror attached before input"
        );

        // --- 7. Send text to the PTY ---
        // Spawned off the channel receive loop: a freshly spawned claude needs
        // its TUI to settle before it will accept a submit, and that wait must
        // not stall the Slack/Telegram socket. submit_to_agent waits for
        // readiness, pastes, submits, then monitors that the agent actually
        // started (retrying Enter once if not).
        // Record the human's message on the session's activity trail (the
        // "by user" side), before the trusted-context wrapping.
        self.manager.record_user_message(&session_id, &msg.text).await;

        let input = agent_paste_input(&text);
        tokio::spawn(submit_to_agent(
            Arc::clone(&self.manager),
            Arc::clone(&self.mirror),
            session_id.clone(),
            adapter.channel().as_str().to_string(),
            input,
        ));
    }

    /// Handle a `/command`. Returns `true` if the command was consumed (caller
    /// should return without further processing), `false` if it was not a
    /// recognised command.
    async fn handle_command(
        &self,
        _integ: &Integration,
        adapter: Arc<dyn Adapter>,
        msg: &Inbound,
        trimmed: &str,
    ) -> bool {
        // Extract the command word (everything up to the first space or end).
        let cmd = trimmed.split_whitespace().next().unwrap_or(trimmed);

        match cmd {
            "/help" => {
                let help = "\
                    Otto quick commands:\n\
                    /help     — show this message\n\
                    /sessions — list active agent sessions for this workspace\n\
                    /stop     — stop the session bound to this chat/thread\n\
                    /new      — drop the current session mapping so the next message starts fresh\n\
                    /restart  — restart the current session (equivalent to /new)\n\
                    /who      — show which session this conversation is mapped to";
                let _ = adapter.send(&msg.chat, msg.thread.as_deref(), help).await;
                true
            }
            "/sessions" => {
                let ws_id: Id = msg.workspace_id.clone();
                let sessions = match self.manager.list_by_workspace(&ws_id).await {
                    Ok(list) => list,
                    Err(e) => {
                        warn!("bridge /sessions: {e}");
                        let _ = adapter
                            .send(&msg.chat, msg.thread.as_deref(), "Error listing sessions.")
                            .await;
                        return true;
                    }
                };
                let lines: Vec<String> = sessions
                    .iter()
                    .map(|s| {
                        let title = if s.title.is_empty() {
                            s.id.as_str()
                        } else {
                            s.title.as_str()
                        };
                        format!("• {} — {:?}", title, s.status)
                    })
                    .collect();
                let reply = if lines.is_empty() {
                    "No active sessions.".to_string()
                } else {
                    lines.join("\n")
                };
                let _ = adapter.send(&msg.chat, msg.thread.as_deref(), &reply).await;
                true
            }
            "/stop" => {
                let key: ConvKey = (
                    msg.workspace_id.clone(),
                    msg.chat.clone(),
                    msg.thread.clone(),
                );
                let sid = self.sessions.lock().await.remove(&key);
                match sid {
                    None => {
                        let _ = adapter
                            .send(
                                &msg.chat,
                                msg.thread.as_deref(),
                                "No session mapped to this conversation.",
                            )
                            .await;
                    }
                    Some(sid) => {
                        if let Err(e) = self.manager.kill_session(&sid).await {
                            warn!("bridge /stop kill: {e}");
                        }
                        self.mirror.cancel(&sid).await;
                        let _ = adapter
                            .send(&msg.chat, msg.thread.as_deref(), "stopped")
                            .await;
                        info!(session = %sid, "bridge: /stop killed session");
                    }
                }
                true
            }
            "/new" => {
                let key: ConvKey = (
                    msg.workspace_id.clone(),
                    msg.chat.clone(),
                    msg.thread.clone(),
                );
                self.sessions.lock().await.remove(&key);
                let _ = adapter
                    .send(
                        &msg.chat,
                        msg.thread.as_deref(),
                        "new session will start on your next message",
                    )
                    .await;
                true
            }
            "/restart" => {
                let key: ConvKey = (
                    msg.workspace_id.clone(),
                    msg.chat.clone(),
                    msg.thread.clone(),
                );
                self.sessions.lock().await.remove(&key);
                let _ = adapter
                    .send(
                        &msg.chat,
                        msg.thread.as_deref(),
                        "session restarted — next message starts a new session",
                    )
                    .await;
                true
            }
            "/who" => {
                let ws_id: Id = msg.workspace_id.clone();
                let sessions = match self.manager.list_by_workspace(&ws_id).await {
                    Ok(list) => list,
                    Err(e) => {
                        warn!("bridge /who: {e}");
                        let _ = adapter
                            .send(&msg.chat, msg.thread.as_deref(), "Error listing sessions.")
                            .await;
                        return true;
                    }
                };
                let key: ConvKey = (
                    msg.workspace_id.clone(),
                    msg.chat.clone(),
                    msg.thread.clone(),
                );
                let bound_id = self.sessions.lock().await.get(&key).cloned();
                let reply = match bound_id {
                    None => "No session is mapped to this conversation.".to_string(),
                    Some(sid) => match sessions.iter().find(|s| s.id == sid) {
                        Some(s) => {
                            let title = if s.title.is_empty() { s.id.as_str() } else { s.title.as_str() };
                            format!("This conversation is mapped to: {} — {:?}", title, s.status)
                        }
                        None => format!("Mapped to session {} (not found in listing)", sid.as_str()),
                    },
                };
                let _ = adapter.send(&msg.chat, msg.thread.as_deref(), &reply).await;
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_paste_input_uses_bracketed_paste_without_submit_key() {
        let bytes = agent_paste_input("line one\nline two");

        assert_eq!(bytes, b"\x1b[200~line one\nline two\x1b[201~".to_vec());
        assert_eq!(AGENT_SUBMIT_KEY, b"\r");
    }

    #[test]
    fn session_title_uses_first_line_trimmed_and_truncated() {
        // First non-empty line becomes the title (sidebar pane name).
        assert_eq!(
            session_title("Investigate ticket XYZ\n\nmore details", "Telegram"),
            "Investigate ticket XYZ"
        );
        // Leading blank lines / whitespace are skipped + trimmed.
        assert_eq!(session_title("\n   \n  hello there  ", "Slack"), "hello there");
        // Empty / whitespace-only falls back to "<Channel> chat".
        assert_eq!(session_title("   \n  ", "Telegram"), "Telegram chat");
        assert_eq!(session_title("", "Slack"), "Slack chat");
        // Over 48 chars is truncated with an ellipsis.
        let long = "a".repeat(60);
        let title = session_title(&long, "Telegram");
        assert_eq!(title.chars().count(), 49); // 48 chars + '…'
        assert!(title.ends_with('…'));
    }
}
