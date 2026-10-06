//! Telegram long-poll adapter and listener.
//!
//! `TelegramAdapter` implements `Adapter` (send + edit + upload + typing).
//! `run` is a long-polling loop that forwards inbound messages to `Bridge`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use otto_core::domain::{Channel, Integration};
use serde::Deserialize;
use tracing::{debug, error, info};

use crate::adapter::{Adapter, Inbound};
use crate::bridge::Bridge;

const API_BASE: &str = "https://api.telegram.org";
const LONG_POLL_TIMEOUT: u64 = 25;

/// How long to wait for a TCP/TLS connection to the Telegram Bot API.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Overall per-request deadline for ordinary Bot API calls (sendMessage,
/// editMessageText, sendChatAction). A hung endpoint must not block indefinitely.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Overall deadline for `sendDocument` uploads, which can carry large files and
/// so get a more generous budget than ordinary API calls.
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(120);
/// Overall deadline for the long-poll `getUpdates` request. This MUST exceed
/// `LONG_POLL_TIMEOUT` (the server holds the connection open that long waiting
/// for updates) plus margin, or long-polling would be cut off mid-poll.
const LONG_POLL_REQUEST_TIMEOUT: Duration = Duration::from_secs(LONG_POLL_TIMEOUT + 15);

/// The process-wide HTTP client for ordinary Bot API calls (connect + overall
/// timeouts). Each timeout profile is built once and cloned (a cheap `Arc` bump
/// sharing one connection pool), so `TelegramAdapter::new` per message or
/// notification reuses warm connections. Falls back to a default client if the
/// builder fails.
fn build_http_client() -> reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .connect_timeout(CONNECT_TIMEOUT)
                .timeout(REQUEST_TIMEOUT)
                .build()
                .unwrap_or_default()
        })
        .clone()
}

/// Build an HTTP client for the long-poll listener. Its overall timeout is sized
/// to the long-poll interval plus margin so `getUpdates` is never cut short.
fn build_long_poll_client() -> reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .connect_timeout(CONNECT_TIMEOUT)
                .timeout(LONG_POLL_REQUEST_TIMEOUT)
                .build()
                .unwrap_or_default()
        })
        .clone()
}

/// Build an HTTP client for `sendDocument` uploads (larger overall budget).
fn build_upload_client() -> reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .connect_timeout(CONNECT_TIMEOUT)
                .timeout(UPLOAD_TIMEOUT)
                .build()
                .unwrap_or_default()
        })
        .clone()
}

// ---------------------------------------------------------------------------
// Telegram API response shapes
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct TgResponse<T> {
    ok: bool,
    result: Option<T>,
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TgMessage {
    message_id: i64,
    from: Option<TgUser>,
    chat: TgChat,
    text: Option<String>,
    /// A photo / document / video's caption — the question sent WITH a file.
    #[serde(default)]
    caption: Option<String>,
    message_thread_id: Option<i64>,
    // Attachment kinds (presence only): the bridge cannot fetch them, but a
    // message carrying one must not vanish without a reply.
    #[serde(default)]
    photo: Option<serde_json::Value>,
    #[serde(default)]
    document: Option<serde_json::Value>,
    #[serde(default)]
    video: Option<serde_json::Value>,
    #[serde(default)]
    voice: Option<serde_json::Value>,
    #[serde(default)]
    audio: Option<serde_json::Value>,
}

impl TgMessage {
    /// True when the message carries a file Otto does not download.
    fn has_attachment(&self) -> bool {
        self.photo.is_some()
            || self.document.is_some()
            || self.video.is_some()
            || self.voice.is_some()
            || self.audio.is_some()
    }
}

/// Note appended for the agent when a captioned message carried a file.
const ATTACHMENT_NOTE: &str =
    "[The user also attached a file, which Otto cannot receive over Telegram — ask them to paste its content as text if you need it.]";

/// Reply to a file sent with no text: say so instead of dropping it.
const ATTACHMENT_ONLY_REPLY: &str =
    "I can only read text over Telegram — files aren't supported. Please send your question (or the file's content) as a text message.";

/// What a message forwards to the bridge: its text, else its caption (plus a
/// note when a file came with it). `None` for a message with neither (a bare
/// file — answered with [`ATTACHMENT_ONLY_REPLY`] — or a service message).
/// Pure — unit-tested.
fn inbound_text(msg: &TgMessage) -> Option<String> {
    if let Some(t) = msg.text.as_ref().filter(|t| !t.trim().is_empty()) {
        return Some(t.clone());
    }
    let caption = msg.caption.as_ref().filter(|c| !c.trim().is_empty())?;
    Some(if msg.has_attachment() {
        format!("{caption}\n\n{ATTACHMENT_NOTE}")
    } else {
        caption.clone()
    })
}

#[derive(Debug, Deserialize)]
struct TgUser {
    id: i64,
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    first_name: Option<String>,
}

impl TgUser {
    /// `@handle`, else the first name — shown beside a rejected sender.
    fn display(&self) -> Option<String> {
        self.username
            .as_deref()
            .filter(|u| !u.is_empty())
            .map(|u| format!("@{u}"))
            .or_else(|| self.first_name.clone().filter(|n| !n.is_empty()))
    }
}

#[derive(Debug, Deserialize)]
struct TgChat {
    id: i64,
}

#[derive(Debug, Deserialize)]
struct TgUpdate {
    update_id: i64,
    message: Option<TgMessage>,
}

// ---------------------------------------------------------------------------
// Adapter implementation
// ---------------------------------------------------------------------------

/// Telegram bot adapter: post, edit, upload, and type via the Bot API.
pub struct TelegramAdapter {
    token: String,
    /// Bot API origin (`API_BASE` in production; a local fixture in tests).
    base: String,
    /// Client for ordinary calls (sendMessage / editMessageText / sendChatAction).
    http: reqwest::Client,
    /// Client for `sendDocument` uploads, with a more generous overall timeout.
    http_upload: reqwest::Client,
}

impl TelegramAdapter {
    pub fn new(token: impl Into<String>) -> Self {
        Self::with_base(token, API_BASE)
    }

    /// Adapter against a custom Bot API origin (tests point this at a local
    /// fixture server).
    fn with_base(token: impl Into<String>, base: impl Into<String>) -> Self {
        Self {
            token: token.into(),
            base: base.into(),
            http: build_http_client(),
            http_upload: build_upload_client(),
        }
    }

    fn api_url(&self, method: &str) -> String {
        format!("{}/bot{}/{method}", self.base, self.token)
    }

    /// Decode a Bot API reply for ANY HTTP status. Telegram reports request
    /// errors (400 "can't parse entities", 403 "bot was blocked", 429 "Too Many
    /// Requests: retry after N") as a non-2xx status WITH an `{ok:false,
    /// description}` envelope — the description is what callers branch on
    /// (Markdown → plain-text fallback, permanent-failure classification), so
    /// the body must be read rather than discarded by `error_for_status`. A
    /// non-2xx reply whose body isn't an envelope surfaces the HTTP status
    /// (e.g. `HTTP 429`) so rate limits stay recognisable.
    async fn decode<T: serde::de::DeserializeOwned>(
        &self,
        resp: reqwest::Response,
    ) -> anyhow::Result<TgResponse<T>> {
        let status = resp.status();
        let bytes = resp.bytes().await.map_err(|e| self.scrub(e))?;
        match serde_json::from_slice::<TgResponse<T>>(&bytes) {
            Ok(mut tg) => {
                if !status.is_success() && tg.ok {
                    // Defensive: a non-2xx can't be a success.
                    tg.ok = false;
                }
                if !tg.ok && status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                    let d = tg.description.take().unwrap_or_default();
                    tg.description = Some(if d.contains("429") || d.contains("Too Many") {
                        d
                    } else {
                        format!("HTTP 429 Too Many Requests: {d}")
                    });
                }
                Ok(tg)
            }
            Err(e) if status.is_success() => Err(anyhow::anyhow!(redact_token(
                format!("unreadable Telegram reply: {e}"),
                &self.token
            ))),
            Err(_) => Err(anyhow::anyhow!("Telegram API returned HTTP {status}")),
        }
    }

    /// Convert a reqwest error into an anyhow error with the bot token scrubbed.
    /// reqwest's Display embeds the request URL, which contains `bot<token>` — a
    /// bare `?` would propagate that secret into any error log downstream
    /// (e.g. `mirror.rs` logs failed sends).
    fn scrub(&self, e: reqwest::Error) -> anyhow::Error {
        anyhow::anyhow!(redact_token(e, &self.token))
    }

    /// POST a JSON body to a Bot API method and decode the envelope, scrubbing
    /// the token from any transport/decode error so it never reaches logs.
    async fn post_json<T: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        body: &serde_json::Value,
    ) -> anyhow::Result<TgResponse<T>> {
        let resp = self
            .http
            .post(self.api_url(method))
            .json(body)
            .send()
            .await
            .map_err(|e| self.scrub(e))?;
        self.decode(resp).await
    }
}

#[async_trait::async_trait]
impl Adapter for TelegramAdapter {
    async fn send(&self, chat: &str, thread: Option<&str>, text: &str) -> anyhow::Result<String> {
        let mut body = serde_json::json!({
            "chat_id": chat,
            "text": text,
        });
        if let Some(t) = thread {
            body["reply_to_message_id"] = serde_json::json!(t.parse::<i64>().unwrap_or(0));
            // The user may have deleted the message we answer: still deliver
            // (else "message to be replied not found" — permanent — loses
            // the final answer).
            body["allow_sending_without_reply"] = serde_json::json!(true);
        }
        let tg: TgResponse<TgMessage> = self.post_json("sendMessage", &body).await?;
        if !tg.ok {
            return Err(anyhow::anyhow!(
                "Telegram sendMessage failed: {}",
                tg.description.unwrap_or_default()
            ));
        }
        let msg_id = tg
            .result
            .as_ref()
            .map(|m| m.message_id.to_string())
            .unwrap_or_default();
        Ok(msg_id)
    }

    /// Send with Telegram Markdown parse mode enabled. Uses V1 legacy Markdown
    /// (less strict than MarkdownV2) so `*bold*`, `_italic_`, `` `code` ``,
    /// and `[text](url)` links render without requiring extensive escaping.
    /// Falls back to plain `send` if the formatted send fails (e.g. parse error).
    async fn send_formatted(
        &self,
        chat: &str,
        thread: Option<&str>,
        text: &str,
    ) -> anyhow::Result<String> {
        let mut body = serde_json::json!({
            "chat_id": chat,
            "text": text,
            "parse_mode": "Markdown",
        });
        if let Some(t) = thread {
            body["reply_to_message_id"] = serde_json::json!(t.parse::<i64>().unwrap_or(0));
            // The user may have deleted the message we answer: still deliver
            // (else "message to be replied not found" — permanent — loses
            // the final answer).
            body["allow_sending_without_reply"] = serde_json::json!(true);
        }
        let tg: TgResponse<TgMessage> = self.post_json("sendMessage", &body).await?;
        if !tg.ok {
            // Parse error from Markdown mode → fall back to plain text.
            let desc = tg.description.unwrap_or_default();
            if desc.contains("can't parse") || desc.contains("parse entities") {
                return self.send(chat, thread, text).await;
            }
            return Err(anyhow::anyhow!(
                "Telegram sendMessage (formatted) failed: {desc}"
            ));
        }
        Ok(tg
            .result
            .as_ref()
            .map(|m| m.message_id.to_string())
            .unwrap_or_default())
    }

    async fn edit(&self, chat: &str, message_id: &str, text: &str) -> anyhow::Result<()> {
        let body = serde_json::json!({
            "chat_id": chat,
            "message_id": message_id.parse::<i64>().unwrap_or(0),
            "text": text,
        });
        let tg: TgResponse<serde_json::Value> = self.post_json("editMessageText", &body).await?;
        if !tg.ok
            && tg
                .description
                .as_deref()
                .is_some_and(|d| d.contains("message is not modified"))
        {
            // Same text as already shown — nothing to change, not a failure.
            return Ok(());
        }
        if !tg.ok {
            return Err(anyhow::anyhow!(
                "Telegram editMessageText failed: {}",
                tg.description.unwrap_or_default()
            ));
        }
        Ok(())
    }

    fn channel(&self) -> Channel {
        Channel::Telegram
    }

    /// Upload `content` as a document file named `filename`.
    ///
    /// Uses `POST /bot{token}/sendDocument` with a multipart body.  The
    /// `reply_to_message_id` field is set to the thread message id if provided.
    async fn upload(
        &self,
        chat: &str,
        thread: Option<&str>,
        filename: &str,
        content: &[u8],
    ) -> anyhow::Result<()> {
        // Send the raw bytes verbatim so binary files are not corrupted. A
        // generic content type lets Telegram/clients infer the kind from the
        // filename extension rather than mislabelling everything as markdown.
        let file_part = reqwest::multipart::Part::bytes(content.to_vec())
            .file_name(filename.to_string())
            .mime_str("application/octet-stream")?;

        let mut form = reqwest::multipart::Form::new()
            .text("chat_id", chat.to_string())
            .part("document", file_part);

        if let Some(t) = thread {
            if let Ok(mid) = t.parse::<i64>() {
                form = form
                    .text("reply_to_message_id", mid.to_string())
                    .text("allow_sending_without_reply", "true");
            }
        }

        let resp = self
            .http_upload
            .post(self.api_url("sendDocument"))
            .multipart(form)
            .send()
            .await
            .map_err(|e| self.scrub(e))?;

        let tg: TgResponse<serde_json::Value> = self.decode(resp).await?;
        if !tg.ok {
            return Err(anyhow::anyhow!(
                "Telegram sendDocument failed: {}",
                tg.description.unwrap_or_default()
            ));
        }
        Ok(())
    }

    fn supports_typing(&self) -> bool {
        true
    }

    /// Send a "typing" chat action so Telegram shows "Bot is typing…".
    async fn typing(&self, chat: &str) -> anyhow::Result<()> {
        let body = serde_json::json!({
            "chat_id": chat,
            "action": "typing",
        });
        let tg: TgResponse<bool> = self.post_json("sendChatAction", &body).await?;
        if !tg.ok {
            return Err(anyhow::anyhow!(
                "Telegram sendChatAction failed: {}",
                tg.description.unwrap_or_default()
            ));
        }
        Ok(())
    }
}

/// Replace the bot token with a placeholder so it never lands in logs. reqwest's
/// error Display embeds the request URL, which contains `bot<token>`; this scrubs
/// the secret out of any string bound for a log or a propagated error.
fn redact_token(s: impl std::fmt::Display, token: &str) -> String {
    let s = s.to_string();
    if token.is_empty() {
        s
    } else {
        s.replace(token, "<redacted>")
    }
}

// ---------------------------------------------------------------------------
// Long-poll listener
// ---------------------------------------------------------------------------

/// Last confirmed `getUpdates` offset per bot token, shared across listener
/// generations. A config edit respawns the poller; seeding the new one from
/// here (instead of `0`) means it neither re-fetches nor re-dispatches the
/// updates the previous generation already handled.
fn offsets() -> &'static std::sync::Mutex<std::collections::HashMap<String, i64>> {
    static MAP: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, i64>>> =
        std::sync::OnceLock::new();
    MAP.get_or_init(Default::default)
}

fn load_offset(token: &str) -> i64 {
    offsets()
        .lock()
        .map(|m| m.get(token).copied().unwrap_or(0))
        .unwrap_or(0)
}

fn store_offset(token: &str, offset: i64) {
    if let Ok(mut m) = offsets().lock() {
        let e = m.entry(token.to_string()).or_insert(0);
        // Offsets only move forward.
        *e = (*e).max(offset);
    }
}

/// One poller per bot token in this process. A respawned generation waits for
/// the previous poller to notice its cancel flag and release this lock before
/// it issues its own `getUpdates` — two concurrent long-polls on one token get
/// a 409 "Conflict" from Telegram, which used to surface as a false failure.
fn poller_lock(token: &str) -> Arc<tokio::sync::Mutex<()>> {
    static MAP: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    > = std::sync::OnceLock::new();
    let map = MAP.get_or_init(Default::default);
    let mut m = map.lock().unwrap_or_else(|p| p.into_inner());
    Arc::clone(m.entry(token.to_string()).or_default())
}

/// Sleep `ms`, waking early (in 250 ms slices) once `cancel` is set so a
/// stopped poller in backoff releases its token promptly.
async fn sleep_unless_cancelled(ms: u64, cancel: &AtomicBool) {
    let mut left = ms;
    while left > 0 && !cancel.load(Ordering::Relaxed) {
        let step = left.min(250);
        tokio::time::sleep(Duration::from_millis(step)).await;
        left -= step;
    }
}

/// Outcome of one `getUpdates` round trip.
enum Poll {
    /// Updates to dispatch (possibly empty).
    Updates(Vec<TgUpdate>),
    /// The listener was cancelled while the request was in flight — the
    /// updates (if any) are NOT dispatched and the offset is NOT advanced, so
    /// the next generation receives them instead.
    Cancelled,
    /// Transport / decode / API failure; health already reported. Back off.
    Failed,
}

/// Issue one long-poll `getUpdates` at `offset`.
async fn poll_once(
    http: &reqwest::Client,
    base: &str,
    token: &str,
    offset: i64,
    cancel: &AtomicBool,
    health: &crate::health::Health,
) -> Poll {
    let url = format!("{base}/bot{token}/getUpdates?timeout={LONG_POLL_TIMEOUT}&offset={offset}");

    // The held-open request (up to LONG_POLL_TIMEOUT s) is abandoned as soon
    // as `cancel` is set: after a config edit the NEW poller waits for this
    // one's lock, so ignoring the flag answered messages up to 40 s late.
    // Nothing is lost — the dropped request's updates are re-fetched from
    // the persisted offset by the next generation.
    let send = http.get(&url).send();
    tokio::pin!(send);
    let sent = loop {
        tokio::select! {
            r = &mut send => break r,
            () = tokio::time::sleep(Duration::from_millis(250)) => {
                if cancel.load(Ordering::Relaxed) {
                    return Poll::Cancelled;
                }
            }
        }
    };
    let resp = match sent {
        Ok(r) => r,
        Err(e) => {
            if cancel.load(Ordering::Relaxed) {
                return Poll::Cancelled;
            }
            let why = redact_token(&e, token);
            error!("telegram getUpdates: {why}");
            health.failed(&format!("Couldn't reach Telegram: {why}"), false);
            return Poll::Failed;
        }
    };

    let tg: TgResponse<Vec<TgUpdate>> = match resp.json().await {
        Ok(v) => v,
        Err(e) => {
            if cancel.load(Ordering::Relaxed) {
                return Poll::Cancelled;
            }
            let why = redact_token(&e, token);
            error!("telegram getUpdates parse: {why}");
            health.failed(&format!("Unreadable reply from Telegram: {why}"), false);
            return Poll::Failed;
        }
    };

    // A respawn (config edit / disable) may have cancelled us while the
    // request was held open: hand these updates to the next generation
    // rather than dispatching them from a stale listener.
    if cancel.load(Ordering::Relaxed) {
        return Poll::Cancelled;
    }

    if !tg.ok {
        let desc = tg.description.unwrap_or_default();
        error!("telegram getUpdates not ok: {desc}");
        // 401 Unauthorized (revoked/wrong token) won't fix itself; a 409
        // Conflict (another poller / a webhook set on the bot) needs the
        // user too. Both are retried, but reported as failing.
        let permanent = desc.contains("Unauthorized") || desc.contains("Conflict");
        health.failed(&format!("Telegram rejected getUpdates: {desc}"), permanent);
        return Poll::Failed;
    }
    Poll::Updates(tg.result.unwrap_or_default())
}

/// Long-poll until `cancel` is set. Each incoming text message is forwarded
/// to `bridge`.
pub async fn run(
    integ: Integration,
    token: String,
    bridge: Arc<Bridge>,
    cancel: Arc<AtomicBool>,
    health: crate::health::Health,
) {
    // Wait out the previous generation's poller on this token (it releases
    // the lock once it sees its own cancel flag — at most one long-poll).
    let lock = poller_lock(&token);
    let _guard = lock.lock().await;
    if cancel.load(Ordering::Relaxed) {
        debug!("telegram listener cancelled before start");
        return;
    }

    let adapter = Arc::new(TelegramAdapter::new(token.clone()));
    // Long-poll client: its overall timeout exceeds LONG_POLL_TIMEOUT so the
    // held-open getUpdates request is not cut off mid-poll.
    let http = build_long_poll_client();
    // Resume where the previous generation stopped (0 on first start).
    let mut offset: i64 = load_offset(&token);
    let mut backoff_ms: u64 = 3_000;
    const BACKOFF_MAX_MS: u64 = 60_000;
    info!(workspace = %integ.workspace_id, "telegram: listener loop started");

    loop {
        if cancel.load(Ordering::Relaxed) {
            debug!("telegram listener stopping (cancel)");
            return;
        }

        let updates = match poll_once(&http, API_BASE, &token, offset, &cancel, &health).await {
            Poll::Updates(u) => u,
            Poll::Cancelled => {
                debug!("telegram listener stopping (cancel mid-poll)");
                return;
            }
            Poll::Failed => {
                sleep_unless_cancelled(backoff_ms, &cancel).await;
                backoff_ms = (backoff_ms * 2).min(BACKOFF_MAX_MS);
                continue;
            }
        };

        // Successful poll — reset backoff.
        backoff_ms = 3_000;
        health.connected();

        for update in &updates {
            if let Some(msg) = &update.message {
                if inbound_text(msg).is_none() && msg.has_attachment() {
                    // A bare file: tell an admitted sender instead of
                    // dropping it silently (strangers get nothing).
                    let user = msg
                        .from
                        .as_ref()
                        .map(|u| u.id.to_string())
                        .unwrap_or_default();
                    if crate::bridge::integration_admits(&integ, &user) {
                        let adapter = Arc::clone(&adapter);
                        let chat = msg.chat.id.to_string();
                        let reply_to = msg.message_id.to_string();
                        tokio::spawn(async move {
                            let _ = adapter
                                .send(&chat, Some(&reply_to), ATTACHMENT_ONLY_REPLY)
                                .await;
                        });
                    }
                }
                if let Some(text) = &inbound_text(msg) {
                    let user = msg
                        .from
                        .as_ref()
                        .map(|u| u.id.to_string())
                        .unwrap_or_default();
                    let chat = msg.chat.id.to_string();
                    let thread = msg.message_thread_id.map(|t| t.to_string());
                    health.event();

                    let inbound = Inbound {
                        workspace_id: integ.workspace_id.clone(),
                        chat,
                        thread,
                        user,
                        user_name: msg.from.as_ref().and_then(TgUser::display),
                        text: text.clone(),
                        edited: false,
                    };
                    info!(
                        workspace = %inbound.workspace_id,
                        chat = %inbound.chat,
                        thread = ?inbound.thread,
                        user = %inbound.user,
                        update_id = update.update_id,
                        "telegram: inbound text update received"
                    );
                    // Handle OFF the poll loop (as Slack does): a slow trigger
                    // (workflow / swarm / run launch, session spawn) must not
                    // stall polling for every other chat of this bot. Same-
                    // conversation routing stays serialized by the bridge's
                    // find-or-create lock.
                    let bridge = Arc::clone(&bridge);
                    let integ = integ.clone();
                    let adapter = Arc::clone(&adapter) as Arc<dyn Adapter>;
                    tokio::spawn(async move {
                        bridge.handle(&integ, adapter, inbound).await;
                    });
                }
            }
            // Advance offset past this update so we don't re-process it.
            offset = update.update_id + 1;
        }
        // Persist across generations (see `offsets`).
        store_offset(&token, offset);

        // If we got no updates, yield briefly to avoid a hot spin.
        if updates.is_empty() {
            tokio::task::yield_now().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_clients_build() {
        // Each timeout-configured builder must produce a usable client.
        let _ = build_http_client();
        let _ = build_long_poll_client();
        let _ = build_upload_client();
    }

    #[test]
    fn long_poll_request_timeout_exceeds_poll_interval() {
        // The held-open getUpdates request must outlive the long-poll window,
        // otherwise long-polling would be cut off mid-poll.
        assert!(
            LONG_POLL_REQUEST_TIMEOUT > Duration::from_secs(LONG_POLL_TIMEOUT),
            "long-poll request timeout must exceed the long-poll interval"
        );
    }

    /// Serve `router` on an ephemeral loopback port; returns its origin.
    async fn fixture(router: axum::Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn markdown_parse_error_400_falls_back_to_plain_text() {
        use axum::http::StatusCode;
        use axum::Json;
        use std::sync::atomic::AtomicUsize;
        // Telegram rejects bad Markdown with HTTP 400 + an `ok:false` envelope.
        // The body must be read so `send_formatted` retries as plain text
        // (it used to be discarded by `error_for_status` → reply lost).
        let calls = Arc::new(AtomicUsize::new(0));
        let c = Arc::clone(&calls);
        let router = axum::Router::new().route(
            "/botTOKEN/sendMessage",
            axum::routing::post(move |Json(body): Json<serde_json::Value>| {
                let c = Arc::clone(&c);
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    if body.get("parse_mode").is_some() {
                        (
                            StatusCode::BAD_REQUEST,
                            Json(serde_json::json!({
                                "ok": false,
                                "error_code": 400,
                                "description": "Bad Request: can't parse entities: Can't find end of the entity starting at byte offset 3"
                            })),
                        )
                    } else {
                        (
                            StatusCode::OK,
                            Json(serde_json::json!({
                                "ok": true,
                                "result": {"message_id": 7, "chat": {"id": 1}}
                            })),
                        )
                    }
                }
            }),
        );
        let base = fixture(router).await;
        let a = TelegramAdapter::with_base("TOKEN", base);
        let id = a.send_formatted("1", None, "a *b").await.unwrap();
        assert_eq!(id, "7");
        assert_eq!(calls.load(Ordering::SeqCst), 2, "formatted then plain");
    }

    #[tokio::test]
    async fn non_2xx_error_keeps_description_and_status() {
        use axum::http::StatusCode;
        use axum::Json;
        let router = axum::Router::new()
            .route(
                "/botTOKEN/sendMessage",
                axum::routing::post(|| async {
                    (
                        StatusCode::FORBIDDEN,
                        Json(serde_json::json!({
                            "ok": false,
                            "description": "Forbidden: bot was blocked by the user"
                        })),
                    )
                }),
            )
            .route(
                "/botTOKEN/editMessageText",
                axum::routing::post(|| async {
                    (
                        StatusCode::TOO_MANY_REQUESTS,
                        Json(serde_json::json!({
                            "ok": false,
                            "description": "retry after 5"
                        })),
                    )
                }),
            )
            .route(
                "/botTOKEN/sendChatAction",
                axum::routing::post(|| async { (StatusCode::BAD_GATEWAY, "upstream down") }),
            );
        let base = fixture(router).await;
        let a = TelegramAdapter::with_base("TOKEN", base);
        let e = a.send("1", None, "x").await.unwrap_err().to_string();
        assert!(e.contains("bot was blocked by the user"), "{e}");
        let e = a.edit("1", "2", "x").await.unwrap_err().to_string();
        assert!(e.contains("429"), "rate limit stays recognisable: {e}");
        let e = a.typing("1").await.unwrap_err().to_string();
        assert!(e.contains("502"), "{e}");
        assert!(!e.contains("TOKEN"), "token never in errors: {e}");
    }

    #[tokio::test]
    async fn poll_cancelled_mid_request_does_not_dispatch() {
        use axum::Json;
        // The respawn flips the old generation's cancel flag while its
        // getUpdates is held open; the updates it then receives must be left
        // for the next generation (no dispatch, no offset advance).
        let cancel = Arc::new(AtomicBool::new(false));
        let c = Arc::clone(&cancel);
        let router = axum::Router::new().route(
            "/botTOKEN/getUpdates",
            axum::routing::get(move || {
                let c = Arc::clone(&c);
                async move {
                    c.store(true, Ordering::SeqCst);
                    Json(serde_json::json!({
                        "ok": true,
                        "result": [{"update_id": 41, "message": {
                            "message_id": 1, "chat": {"id": 9}, "text": "hi"}}]
                    }))
                }
            }),
        );
        let base = fixture(router).await;
        let health = crate::health::Health::begin("tg-test-cancel", Channel::Telegram);
        let http = build_long_poll_client();
        let out = poll_once(&http, &base, "TOKEN", 0, &cancel, &health).await;
        assert!(matches!(out, Poll::Cancelled));

        // Not cancelled → the updates come through.
        let live = AtomicBool::new(false);
        let router = axum::Router::new().route(
            "/botTOKEN/getUpdates",
            axum::routing::get(|| async {
                Json(serde_json::json!({"ok": true, "result": [{"update_id": 41}]}))
            }),
        );
        let base = fixture(router).await;
        match poll_once(&http, &base, "TOKEN", 0, &live, &health).await {
            Poll::Updates(u) => assert_eq!(u[0].update_id, 41),
            _ => panic!("expected updates"),
        }
    }

    fn tg(v: serde_json::Value) -> TgMessage {
        serde_json::from_value(v).unwrap()
    }

    /// S5-17: a captioned file forwards its caption (noting the file); a bare
    /// file forwards nothing (it is answered instead); text wins.
    #[test]
    fn captions_are_read_and_bare_files_are_flagged() {
        let m = tg(serde_json::json!({"message_id": 1, "chat": {"id": 9},
            "caption": "why does this fail?", "document": {"file_id": "f"}}));
        let t = inbound_text(&m).unwrap();
        assert!(t.starts_with("why does this fail?"), "{t}");
        assert!(t.contains("cannot receive"), "{t}");
        let bare = tg(serde_json::json!({"message_id": 2, "chat": {"id": 9},
            "photo": [{"file_id": "p"}]}));
        assert!(inbound_text(&bare).is_none());
        assert!(bare.has_attachment());
        let plain = tg(serde_json::json!({"message_id": 3, "chat": {"id": 9}, "text": "hi"}));
        assert_eq!(inbound_text(&plain).as_deref(), Some("hi"));
    }

    /// S5-16: a reply survives the user deleting the message it answers.
    #[tokio::test]
    async fn replies_are_sent_even_without_the_replied_message() {
        use axum::Json;
        let seen = Arc::new(std::sync::Mutex::new(Vec::<serde_json::Value>::new()));
        let s2 = Arc::clone(&seen);
        let router = axum::Router::new().route(
            "/botTOKEN/sendMessage",
            axum::routing::post(move |Json(body): Json<serde_json::Value>| {
                let s2 = Arc::clone(&s2);
                async move {
                    s2.lock().unwrap().push(body);
                    Json(serde_json::json!({"ok": true, "result": {"message_id": 7, "chat": {"id": 1}}}))
                }
            }),
        );
        let base = fixture(router).await;
        let a = TelegramAdapter::with_base("TOKEN", base);
        a.send("1", Some("55"), "x").await.unwrap();
        a.send_formatted("1", Some("55"), "x").await.unwrap();
        for body in seen.lock().unwrap().iter() {
            assert_eq!(body["reply_to_message_id"], 55);
            assert_eq!(body["allow_sending_without_reply"], true, "{body}");
        }
    }

    /// S5-15: a cancel set while getUpdates is HELD OPEN ends the poll at
    /// once, not when Telegram finally answers.
    #[tokio::test]
    async fn a_held_open_poll_ends_on_cancel() {
        let router = axum::Router::new().route(
            "/botTOKEN/getUpdates",
            axum::routing::get(|| async {
                tokio::time::sleep(Duration::from_secs(30)).await;
                "{}"
            }),
        );
        let base = fixture(router).await;
        let cancel = Arc::new(AtomicBool::new(false));
        let c = Arc::clone(&cancel);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            c.store(true, Ordering::SeqCst);
        });
        let health = crate::health::Health::begin("tg-test-held", Channel::Telegram);
        let http = build_long_poll_client();
        let started = std::time::Instant::now();
        let out = poll_once(&http, &base, "TOKEN", 0, &cancel, &health).await;
        assert!(matches!(out, Poll::Cancelled));
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn offsets_persist_across_generations_and_only_advance() {
        let tok = "offset-test-token";
        assert_eq!(load_offset(tok), 0);
        store_offset(tok, 42);
        assert_eq!(load_offset(tok), 42, "a respawned poller resumes here");
        store_offset(tok, 10);
        assert_eq!(load_offset(tok), 42, "never rewinds");
    }

    #[tokio::test]
    async fn new_generation_waits_for_previous_poller() {
        let a = poller_lock("lock-test-token");
        let b = poller_lock("lock-test-token");
        assert!(Arc::ptr_eq(&a, &b), "one lock per token");
        let held = a.lock().await;
        assert!(b.try_lock().is_err(), "second poller must wait");
        drop(held);
        assert!(b.try_lock().is_ok());
    }
}
