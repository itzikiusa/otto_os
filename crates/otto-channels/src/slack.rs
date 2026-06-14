//! Slack Socket Mode adapter and listener.
//!
//! `SlackAdapter` implements `Adapter` (send + edit + upload messages via Web API).
//! `run` opens a Socket Mode WebSocket connection and forwards inbound messages
//! to `Bridge`.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use otto_core::domain::{Channel, Integration};
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, error, info, warn};

use crate::adapter::{Adapter, Inbound};
use crate::bridge::Bridge;

const RETRY_SLEEP: Duration = Duration::from_secs(3);
const RECONNECT_SLEEP: Duration = Duration::from_secs(5);
const CANCEL_CHECK_INTERVAL: Duration = Duration::from_secs(1);
/// Cap on the dedup set size; cleared when exceeded to avoid unbounded growth.
const DEDUP_CAP: usize = 2000;

// Regex-free mention strip: strips leading `<@Uxxxxxxx>` (and trailing space)
// from Slack message text when the bot is mentioned.
fn strip_mention(text: &str) -> &str {
    let t = text.trim_start();
    if let Some(rest) = t.strip_prefix("<@") {
        if let Some(end) = rest.find('>') {
            return rest[end + 1..].trim_start();
        }
    }
    t
}

// ---------------------------------------------------------------------------
// Adapter implementation
// ---------------------------------------------------------------------------

/// Slack bot adapter: post, edit and upload messages via the Web API.
pub struct SlackAdapter {
    bot_token: String,
    http: reqwest::Client,
}

impl SlackAdapter {
    pub fn new(bot_token: impl Into<String>) -> Self {
        Self {
            bot_token: bot_token.into(),
            http: reqwest::Client::new(),
        }
    }
}

#[async_trait::async_trait]
impl Adapter for SlackAdapter {
    async fn send(&self, chat: &str, thread: Option<&str>, text: &str) -> anyhow::Result<String> {
        let mut body = serde_json::json!({
            "channel": chat,
            "text": text,
        });
        if let Some(ts) = thread {
            body["thread_ts"] = serde_json::json!(ts);
        }

        let resp = self
            .http
            .post("https://slack.com/api/chat.postMessage")
            .header("Authorization", format!("Bearer {}", self.bot_token))
            .json(&body)
            .send()
            .await?
            .error_for_status()?;

        let val: serde_json::Value = resp.json().await?;
        if !val["ok"].as_bool().unwrap_or(false) {
            let err = val["error"].as_str().unwrap_or("unknown").to_string();
            return Err(anyhow::anyhow!("slack chat.postMessage: {err}"));
        }
        let ts = val["ts"].as_str().unwrap_or("").to_string();
        Ok(ts)
    }

    async fn edit(&self, chat: &str, message_id: &str, text: &str) -> anyhow::Result<()> {
        let body = serde_json::json!({
            "channel": chat,
            "ts": message_id,
            "text": text,
        });

        let resp = self
            .http
            .post("https://slack.com/api/chat.update")
            .header("Authorization", format!("Bearer {}", self.bot_token))
            .json(&body)
            .send()
            .await?
            .error_for_status()?;

        let val: serde_json::Value = resp.json().await?;
        if !val["ok"].as_bool().unwrap_or(false) {
            let err = val["error"].as_str().unwrap_or("unknown").to_string();
            return Err(anyhow::anyhow!("slack chat.update: {err}"));
        }
        Ok(())
    }

    fn channel(&self) -> Channel {
        Channel::Slack
    }

    /// Upload `content` as a file named `filename` to the Slack conversation.
    ///
    /// Uses the legacy `files.upload` multipart endpoint which accepts a `content`
    /// field directly — no external URL round-trip needed.
    async fn upload(
        &self,
        chat: &str,
        thread: Option<&str>,
        filename: &str,
        content: &str,
    ) -> anyhow::Result<()> {
        let mut form = reqwest::multipart::Form::new()
            .text("channels", chat.to_string())
            .text("filename", filename.to_string())
            .text("content", content.to_string());

        if let Some(ts) = thread {
            form = form.text("thread_ts", ts.to_string());
        }

        let resp = self
            .http
            .post("https://slack.com/api/files.upload")
            .header("Authorization", format!("Bearer {}", self.bot_token))
            .multipart(form)
            .send()
            .await?
            .error_for_status()?;

        let val: serde_json::Value = resp.json().await?;
        if !val["ok"].as_bool().unwrap_or(false) {
            let err = val["error"].as_str().unwrap_or("unknown").to_string();
            return Err(anyhow::anyhow!("slack files.upload: {err}"));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Socket Mode listener
// ---------------------------------------------------------------------------

/// Open and maintain a Slack Socket Mode connection until `cancel` is set.
/// Each inbound `message` event is forwarded to `bridge`.
pub async fn run(
    integ: Integration,
    bot_token: String,
    app_token: String,
    bridge: Arc<Bridge>,
    cancel: Arc<AtomicBool>,
) {
    let http = reqwest::Client::new();
    // In-memory dedup set: keyed by "channel:ts".
    let seen: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));

    'outer: loop {
        if cancel.load(Ordering::Relaxed) {
            debug!("slack listener stopping (cancel)");
            return;
        }

        // --- Step 1: request a fresh WSS URL from apps.connections.open ---
        let wss_url = match open_socket_mode_connection(&http, &app_token).await {
            Some(url) => url,
            None => {
                error!("slack: failed to open socket mode connection, retrying in 5s");
                tokio::time::sleep(RECONNECT_SLEEP).await;
                continue 'outer;
            }
        };

        // --- Step 2: connect to the WSS URL ---
        info!("slack: connecting to socket mode");
        let ws_stream = match tokio_tungstenite::connect_async(&wss_url).await {
            Ok((stream, _)) => stream,
            Err(e) => {
                error!("slack: websocket connect failed: {e}");
                tokio::time::sleep(RETRY_SLEEP).await;
                continue 'outer;
            }
        };

        let (mut sink, mut stream) = ws_stream.split();

        // --- Step 3: read frames ---
        'inner: loop {
            if cancel.load(Ordering::Relaxed) {
                debug!("slack listener stopping (cancel) in inner loop");
                return;
            }

            // Use a select with a timeout so we can check `cancel` periodically.
            let maybe_msg = tokio::select! {
                msg = stream.next() => msg,
                _ = tokio::time::sleep(CANCEL_CHECK_INTERVAL) => {
                    continue 'inner;
                }
            };

            let raw = match maybe_msg {
                Some(Ok(Message::Text(text))) => text,
                Some(Ok(Message::Ping(data))) => {
                    // Respond to WebSocket-level pings.
                    let _ = sink.send(Message::Pong(data)).await;
                    continue 'inner;
                }
                Some(Ok(Message::Close(_))) => {
                    info!("slack: server closed websocket, reconnecting");
                    break 'inner;
                }
                Some(Ok(_)) => continue 'inner, // binary / pong frames
                Some(Err(e)) => {
                    error!("slack: websocket error: {e}, reconnecting");
                    tokio::time::sleep(RETRY_SLEEP).await;
                    break 'inner;
                }
                None => {
                    info!("slack: stream ended, reconnecting");
                    tokio::time::sleep(RETRY_SLEEP).await;
                    break 'inner;
                }
            };

            let val: serde_json::Value = match serde_json::from_str(&raw) {
                Ok(v) => v,
                Err(e) => {
                    warn!("slack: could not parse frame: {e}");
                    continue 'inner;
                }
            };

            let msg_type = val["type"].as_str().unwrap_or("");

            match msg_type {
                "hello" => {
                    info!("slack: socket mode connected (hello received)");
                }
                "disconnect" => {
                    info!("slack: disconnect requested by server, reconnecting");
                    break 'inner;
                }
                "events_api" => {
                    // Always ack immediately.
                    let envelope_id = val["envelope_id"].as_str().unwrap_or("").to_string();
                    if !envelope_id.is_empty() {
                        let ack = format!(r#"{{"envelope_id":"{envelope_id}"}}"#);
                        if let Err(e) = sink.send(Message::Text(ack.into())).await {
                            error!("slack: failed to send ack: {e}");
                            break 'inner;
                        }
                    }

                    // Dedup: build key from (channel, ts).
                    let event = &val["payload"]["event"];
                    let dedup_key = {
                        let ch = event["channel"].as_str().unwrap_or("");
                        let ts = event["ts"].as_str().unwrap_or("");
                        format!("{ch}:{ts}")
                    };
                    {
                        let mut guard = seen.lock().await;
                        if guard.contains(&dedup_key) {
                            debug!("slack: duplicate event {dedup_key}, skipping");
                            continue 'inner;
                        }
                        if guard.len() >= DEDUP_CAP {
                            guard.clear();
                        }
                        guard.insert(dedup_key);
                    }

                    // Process the event payload.
                    handle_event(event, &integ, &bot_token, Arc::clone(&bridge)).await;
                }
                other => {
                    // Ack anything that carries an envelope_id (slash commands, interactive, etc.)
                    if let Some(eid) = val["envelope_id"].as_str() {
                        if !eid.is_empty() {
                            let ack = format!(r#"{{"envelope_id":"{eid}"}}"#);
                            let _ = sink.send(Message::Text(ack.into())).await;
                        }
                    }
                    debug!("slack: unhandled envelope type '{other}', ignored");
                }
            }
        }

        // Brief pause before reconnecting.
        tokio::time::sleep(RETRY_SLEEP).await;
    }
}

/// POST to `apps.connections.open` and return the WSS URL, or `None` on error.
async fn open_socket_mode_connection(http: &reqwest::Client, app_token: &str) -> Option<String> {
    let resp = match http
        .post("https://slack.com/api/apps.connections.open")
        .header("Authorization", format!("Bearer {app_token}"))
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body("")
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            error!("slack: apps.connections.open request failed: {e}");
            return None;
        }
    };

    let val: serde_json::Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => {
            error!("slack: apps.connections.open parse failed: {e}");
            return None;
        }
    };

    if !val["ok"].as_bool().unwrap_or(false) {
        let err = val["error"].as_str().unwrap_or("unknown");
        error!("slack: apps.connections.open not ok: {err}");
        return None;
    }

    val["url"].as_str().map(|s| s.to_string())
}

/// Inspect a single `events_api` payload event and, if it is a user message,
/// build an `Inbound` and forward it to the bridge.
async fn handle_event(
    event: &serde_json::Value,
    integ: &Integration,
    bot_token: &str,
    bridge: Arc<Bridge>,
) {
    let event_type = event["type"].as_str().unwrap_or("");

    // Only handle message + app_mention events.
    if event_type != "message" && event_type != "app_mention" {
        debug!(event_type, "slack: ignored non-message event");
        return;
    }
    info!(event_type, "slack: message-like event received");

    // Loop prevention — the ONLY thing we ever drop. Never forward the bot's
    // own messages, including the nested message of an edit (`message_changed`):
    // the mirror edits its "working…" feed every couple of seconds, and each
    // edit emits a message_changed event authored by the bot. Without this the
    // relay would feed its own output back to the agent in a tight loop.
    if event["bot_id"].is_string() || event["message"]["bot_id"].is_string() {
        info!(event_type, "slack: bot message skipped (loop prevention)");
        return;
    }

    // Policy: accept ALL user messages. We never drop a message for having an
    // unknown subtype — when in doubt, forward it and let the agent read/act.
    // The two exceptions carry no new user content: a deletion is a tombstone,
    // and an edit (`message_changed`) carries its content under `message`.
    let subtype = event["subtype"].as_str();
    if subtype == Some("message_deleted") {
        info!(event_type, "slack: deletion skipped (nothing to forward)");
        return;
    }
    let content = if subtype == Some("message_changed") {
        &event["message"]
    } else {
        event
    };

    let user = content["user"]
        .as_str()
        .or_else(|| event["user"].as_str())
        .unwrap_or("")
        .to_string();

    let raw_text = content["text"].as_str().unwrap_or("");
    let text = strip_mention(raw_text).to_string();

    // Download any attached files to local paths so the agent can read them
    // (Slack `url_private` needs the bot token, so the agent can't fetch them
    // itself). Returns a note listing the saved paths, appended to the message.
    let files_note = collect_attachments(content, bot_token).await;

    // Only skip when there is genuinely nothing to act on (no text AND no
    // attachments). Everything else is forwarded.
    if text.is_empty() && files_note.is_empty() {
        info!(event_type, user = %user, "slack: no text or attachments, skipping");
        return;
    }

    let combined = match (text.is_empty(), files_note.is_empty()) {
        (false, true) => text,
        (true, false) => format!(
            "[The user sent attachment(s) with no message text — read them and act:]\n{files_note}"
        ),
        (false, false) => format!("{text}\n\n{files_note}"),
        (true, true) => unreachable!(),
    };

    let channel = match content["channel"]
        .as_str()
        .or_else(|| event["channel"].as_str())
    {
        Some(c) if !c.is_empty() => c.to_string(),
        _ => {
            warn!("slack: message event missing channel, skipping");
            return;
        }
    };

    // Use thread_ts if present, otherwise fall back to ts (the message itself).
    let thread = content["thread_ts"]
        .as_str()
        .or_else(|| event["thread_ts"].as_str())
        .or_else(|| content["ts"].as_str())
        .or_else(|| event["ts"].as_str())
        .map(|s| s.to_string());

    let inbound = Inbound {
        workspace_id: integ.workspace_id.clone(),
        chat: channel,
        thread,
        user,
        text: combined,
    };
    info!(
        workspace = %inbound.workspace_id,
        chat = %inbound.chat,
        thread = ?inbound.thread,
        user = %inbound.user,
        event_type,
        "slack: forwarding inbound event to bridge"
    );

    let adapter = Arc::new(SlackAdapter::new(bot_token.to_string())) as Arc<dyn Adapter>;
    bridge.handle(integ, adapter, inbound).await;
}

/// Download every file attached to a message to a local temp path and return a
/// note (for the agent) listing the saved paths. Empty when there are no files.
/// A file that can't be downloaded is still listed (with its permalink/URL) so
/// the agent at least knows it exists — we never drop the message over it.
async fn collect_attachments(content: &serde_json::Value, bot_token: &str) -> String {
    let files = match content["files"].as_array() {
        Some(f) if !f.is_empty() => f,
        _ => return String::new(),
    };
    let client = reqwest::Client::new();
    let mut notes = Vec::new();
    for f in files {
        let name = f["name"].as_str().unwrap_or("file");
        let id = f["id"].as_str().unwrap_or("nofileid");
        let mimetype = f["mimetype"].as_str().unwrap_or("application/octet-stream");
        let url = f["url_private_download"]
            .as_str()
            .or_else(|| f["url_private"].as_str());
        match url {
            Some(u) => match download_slack_file(&client, u, bot_token, id, name).await {
                Ok(path) => notes.push(format!("• {name} ({mimetype}) — saved to: {path}")),
                Err(e) => {
                    warn!("slack: failed to download attachment {name}: {e}");
                    let link = f["permalink"].as_str().unwrap_or(u);
                    notes.push(format!(
                        "• {name} ({mimetype}) — could not download automatically; URL: {link}"
                    ));
                }
            },
            None => notes.push(format!("• {name} ({mimetype}) — no download URL available")),
        }
    }
    if notes.is_empty() {
        return String::new();
    }
    format!(
        "[Attachment(s) from the user — read them from these local paths and act on them:]\n{}",
        notes.join("\n")
    )
}

/// GET a Slack `url_private` file (auth via the bot token) and save it under the
/// temp dir, returning the absolute path. Filenames are sanitised so a crafted
/// name can't escape the temp dir.
async fn download_slack_file(
    client: &reqwest::Client,
    url: &str,
    bot_token: &str,
    id: &str,
    name: &str,
) -> anyhow::Result<String> {
    let resp = client
        .get(url)
        .header("Authorization", format!("Bearer {bot_token}"))
        .send()
        .await?;
    if !resp.status().is_success() {
        anyhow::bail!("http {}", resp.status());
    }
    let bytes = resp.bytes().await?;
    let safe_name: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let path = std::env::temp_dir().join(format!("otto-slack-{id}-{safe_name}"));
    tokio::fs::write(&path, &bytes).await?;
    Ok(path.to_string_lossy().to_string())
}
