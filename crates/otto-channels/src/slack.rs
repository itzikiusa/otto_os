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
use crate::health::Health;

/// Strip a secret-bearing URL out of an error/log string. Slack's external-upload
/// URL and the Socket Mode WSS URL carry single-use tickets in their query
/// string; reqwest/tungstenite `Error` Display can include the full URL, so scrub
/// it (and its raw query, in case the error re-serialized the URL) before logging.
fn redact_url(s: impl std::fmt::Display, url: &str) -> String {
    let mut out = s.to_string().replace(url, "<redacted-url>");
    if let Some((_, query)) = url.split_once('?') {
        if !query.is_empty() {
            out = out.replace(query, "<redacted>");
        }
    }
    out
}

const CANCEL_CHECK_INTERVAL: Duration = Duration::from_secs(1);
/// Cap on the dedup window; the OLDEST key is evicted past it (a wholesale
/// clear let a Slack redelivery right after the wipe be processed twice).
const DEDUP_CAP: usize = 2000;

/// Bounded "seen" set with FIFO eviction: a `HashSet` for lookups plus a
/// `VecDeque` remembering insertion order.
#[derive(Default)]
struct DedupWindow {
    set: HashSet<String>,
    order: std::collections::VecDeque<String>,
}

impl DedupWindow {
    /// `true` when `key` is new (and now remembered), `false` for a repeat.
    fn insert(&mut self, key: String, cap: usize) -> bool {
        if self.set.contains(&key) {
            return false;
        }
        while self.order.len() >= cap.max(1) {
            if let Some(old) = self.order.pop_front() {
                self.set.remove(&old);
            }
        }
        self.set.insert(key.clone());
        self.order.push_back(key);
        true
    }
}

/// Zombie-socket watchdog: Slack pings a Socket Mode connection every few
/// seconds, so a healthy socket NEVER goes this long without a frame. When it
/// does, the TCP connection died silently (laptop sleep, Wi-Fi change, NAT
/// timeout) and `stream.next()` would hang forever looking "connected" while
/// Slack queues undeliverable events — reconnect instead, and Slack redelivers
/// everything pending. Without this, a message sent onto a dead socket sits
/// invisible until something else kills the connection minutes later.
const IDLE_RECONNECT: Duration = Duration::from_secs(75);
/// Client-side probe ping cadence: forces the OS to notice a dead TCP path
/// (the send fails / no pong comes back) well before `IDLE_RECONNECT` fires.
const CLIENT_PING_INTERVAL: Duration = Duration::from_secs(30);

/// Deadline for the Socket Mode WebSocket dial itself (hard-won): unlike the
/// Web API calls above — which ride a `reqwest` client carrying
/// `CONNECT_TIMEOUT` — `tokio_tungstenite::connect_async` has NO built-in
/// timeout, and a TCP/TLS connect that hangs without erroring parks the task
/// inside that single `.await` forever. Everything that would recover the
/// listener (the `'outer` retry, the backoff, the `cancel` check, the
/// `IDLE_RECONNECT` watchdog) lives *after* the dial, so a hung connect is
/// unrecoverable short of a daemon restart. Seen in the wild when the daemon
/// starts seconds after boot, before the network is up: every workspace logged
/// "connecting to socket mode" and then went silent for hours, with no error
/// and no reconnect. Bound the dial so a stuck connect just retries.
const WS_CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

/// How long to wait for a TCP/TLS connection to Slack to establish.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Overall per-request deadline for ordinary Web API calls. A hung Slack
/// endpoint must not block the caller (or the listener loop) indefinitely.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Overall deadline for file-download requests, which can carry large payloads
/// and so are given a more generous budget than ordinary API calls.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(120);
/// Largest inbound Slack attachment Otto downloads for the agent (bytes).
const MAX_DOWNLOAD_BYTES: u64 = 50 * 1024 * 1024;
/// Deadline for one Socket Mode frame write (ping, pong, envelope ack). Like
/// the dial, `sink.send` has no timeout of its own: on a half-dead TCP path
/// with a full send buffer it parks the read loop forever, and with it every
/// recovery path (watchdog, cancel check, reconnect).
const WS_SEND_TIMEOUT: Duration = Duration::from_secs(10);

/// Write one frame to the Socket Mode socket, bounded by [`WS_SEND_TIMEOUT`].
async fn send_frame<S>(sink: &mut S, msg: Message) -> Result<(), String>
where
    S: futures_util::Sink<Message> + Unpin,
    S::Error: std::fmt::Display,
{
    match tokio::time::timeout(WS_SEND_TIMEOUT, sink.send(msg)).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err(format!("timed out after {}s", WS_SEND_TIMEOUT.as_secs())),
    }
}

/// The process-wide HTTP client for Slack Web API calls (connect + overall
/// timeouts). Built once and cloned (a cheap `Arc` bump sharing one connection
/// pool), so a new adapter per inbound message / outbound notification reuses
/// warm TLS connections instead of paying a fresh handshake every time. Falls
/// back to a default client if the builder fails.
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

/// The process-wide HTTP client for downloading attachment files (larger
/// overall budget); shared like [`build_http_client`].
fn build_download_client() -> reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .connect_timeout(CONNECT_TIMEOUT)
                .timeout(DOWNLOAD_TIMEOUT)
                .build()
                .unwrap_or_default()
        })
        .clone()
}

/// Concurrent attachment downloads per inbound message.
const ATTACHMENT_PARALLELISM: usize = 3;
/// Temp-file prefix of downloaded Slack attachments (swept by
/// [`sweep_stale_attachments`]).
const ATTACHMENT_PREFIX: &str = "otto-slack-";
/// Downloaded attachments older than this are deleted when a listener starts.
const ATTACHMENT_MAX_AGE: Duration = Duration::from_secs(24 * 3600);

/// Delete downloaded attachments (`$TMPDIR/otto-slack-*`) older than
/// [`ATTACHMENT_MAX_AGE`] — they were never cleaned up and piled up in the temp
/// dir. Blocking IO; run off the runtime.
fn sweep_stale_attachments(dir: &std::path::Path, max_age: Duration) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut removed = 0;
    for e in entries.flatten() {
        if !e
            .file_name()
            .to_string_lossy()
            .starts_with(ATTACHMENT_PREFIX)
        {
            continue;
        }
        let stale = e
            .metadata()
            .ok()
            .filter(|m| m.is_file())
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > max_age);
        if stale && std::fs::remove_file(e.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// Longest `Retry-After` Otto honours from Slack before giving up on a call.
const MAX_RETRY_AFTER_SECS: u64 = 120;

/// Decode a Web API response into its JSON body, or an error naming `method`.
/// An HTTP 429 becomes `slack <method>: ratelimited (retry after Ns)` with N
/// from the `Retry-After` header — the mirror keys its back-off on
/// `ratelimited` and reads the delay back with `mirror::retry_after_hint`, so a
/// rate-limited final reply is re-posted instead of silently lost. (It used to
/// surface as reqwest's generic "429 Too Many Requests" with no delay.)
async fn api_json(resp: reqwest::Response, method: &str) -> anyhow::Result<serde_json::Value> {
    if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
        let secs = retry_after_secs(
            resp.headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok()),
        );
        anyhow::bail!("slack {method}: ratelimited (retry after {secs}s)");
    }
    let val: serde_json::Value = resp.error_for_status()?.json().await?;
    if !val["ok"].as_bool().unwrap_or(false) {
        let err = val["error"].as_str().unwrap_or("unknown");
        anyhow::bail!("slack {method}: {err}");
    }
    Ok(val)
}

/// Parse a `Retry-After` header (delta-seconds); absent/garbled → 1 s, and
/// capped at [`MAX_RETRY_AFTER_SECS`].
fn retry_after_secs(header: Option<&str>) -> u64 {
    header
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(1)
        .clamp(1, MAX_RETRY_AFTER_SECS)
}

/// `apps.connections.open` errors that retrying can't fix: the app token is
/// wrong, revoked, of the wrong type, or Socket Mode is off for the app. The
/// listener keeps retrying (the user may fix the app in place), but health
/// reports `Failing` with a hint instead of a hopeful "Reconnecting".
fn is_permanent_open_error(code: &str) -> bool {
    matches!(
        code,
        "invalid_auth"
            | "not_authed"
            | "account_inactive"
            | "token_revoked"
            | "token_expired"
            | "not_allowed_token_type"
            | "missing_scope"
            | "no_permission"
            | "app_missing_action_url"
            | "two_factor_setup_required"
    )
}

/// The user-facing sentence for a failed `apps.connections.open`.
fn open_error_detail(code: &str) -> String {
    match code {
        "not_allowed_token_type" => {
            "Slack rejected the app token: it isn't an app-level token. \
            Paste the xapp-… token (Basic Information → App-Level Tokens), not the xoxb- bot token."
                .to_string()
        }
        "missing_scope" | "no_permission" => "Slack rejected the app token: it lacks the \
            connections:write scope. Regenerate it with connections:write."
            .to_string(),
        c if is_permanent_open_error(c) => format!(
            "Slack rejected the app token ({c}). Check that Socket Mode is enabled for the app \
             and paste a current xapp-… token."
        ),
        c => format!("Couldn't open a Socket Mode connection ({c})."),
    }
}

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

/// Message subtypes that announce channel membership changes rather than carry
/// user content. They must never spawn an agent session: a batch invite emits
/// one `channel_join` per user (a session flood), and Slack refuses thread
/// replies on them (`cannot_reply_to_message`).
fn is_membership_subtype(subtype: Option<&str>) -> bool {
    matches!(
        subtype,
        Some("channel_join" | "channel_leave" | "group_join" | "group_leave")
    )
}

/// How far apart (seconds) a `message_changed` event and its message's
/// `edited.ts` may be for the event to BE that human edit.
const EDIT_EVENT_WINDOW_SECS: f64 = 60.0;

/// True when a `message_changed` EVENT is a human edit happening now. Slack
/// stamps `edited: {user, ts}` only on a human edit, and fires the same event
/// with no stamp when it rewrites the message on its own (a link unfurl, a
/// file preview, reaction metadata). The stamp then STAYS on every later
/// rewrite of that message, so its presence alone re-injected an old edited
/// message as a new turn after a restart (or once the dedup window evicted
/// it): the edit must be fresh — `edited.ts` within a minute of the event's
/// own ts. A `hidden` inner message is never user content.
fn is_human_edit(event: &serde_json::Value) -> bool {
    let message = &event["message"];
    if message["hidden"].as_bool() == Some(true) {
        return false;
    }
    let secs = |v: &serde_json::Value| v.as_str().and_then(|t| t.parse::<f64>().ok());
    let Some(edited) = secs(&message["edited"]["ts"]) else {
        return false;
    };
    match secs(&event["event_ts"]).or_else(|| secs(&event["ts"])) {
        Some(at) => (at - edited).abs() <= EDIT_EVENT_WINDOW_SECS,
        // No event timestamp to compare: trust the stamp (old behaviour).
        None => true,
    }
}

/// The dedup identity of an event. A `message_changed` event carries its own
/// (edit) `ts` at the top level while the message it concerns keeps the
/// original under `message.ts` — keying on the outer one made every Slack
/// rewrite (unfurl, preview) look like a brand-new message, so those key on
/// the ORIGINAL ts and dedup onto the post itself.
///
/// A HUMAN edit is new content and must be forwarded (see `process_event`),
/// so it keys on the original ts PLUS its `edited.ts`: distinct from the
/// original post (which used to swallow every edit), yet each specific edit
/// is still delivered at most once (Socket Mode redelivers).
fn dedup_ts(event: &serde_json::Value) -> String {
    let original = event["message"]["ts"]
        .as_str()
        .or_else(|| event["ts"].as_str())
        .unwrap_or("");
    match event["message"]["edited"]["ts"].as_str() {
        Some(edit) if is_human_edit(event) => format!("{original}#edit:{edit}"),
        _ => original.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Adapter implementation
// ---------------------------------------------------------------------------

/// The dedup window for `bot_token`, shared by every listener generation of
/// this bot in the process (see `run`).
fn seen_window(bot_token: &str) -> Arc<Mutex<DedupWindow>> {
    static MAP: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, Arc<Mutex<DedupWindow>>>>,
    > = std::sync::OnceLock::new();
    let map = MAP.get_or_init(Default::default);
    let mut m = map.lock().unwrap_or_else(|p| p.into_inner());
    Arc::clone(m.entry(bot_token.to_string()).or_default())
}

/// Slack bot adapter: post, edit and upload messages via the Web API.
pub struct SlackAdapter {
    bot_token: String,
    http: reqwest::Client,
}

impl SlackAdapter {
    pub fn new(bot_token: impl Into<String>) -> Self {
        Self {
            bot_token: bot_token.into(),
            http: build_http_client(),
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
            .await?;
        let val = api_json(resp, "chat.postMessage").await?;
        let ts = val["ts"].as_str().unwrap_or("").to_string();
        Ok(ts)
    }

    /// Send with Slack mrkdwn rendering enabled. Slack surfaces `*bold*`,
    /// `_italic_`, `` `code` ``, and `<URL|label>` links when `mrkdwn: true`.
    async fn send_formatted(
        &self,
        chat: &str,
        thread: Option<&str>,
        text: &str,
    ) -> anyhow::Result<String> {
        let mut body = serde_json::json!({
            "channel": chat,
            "text": text,
            "mrkdwn": true,
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
            .await?;
        let val = api_json(resp, "chat.postMessage (formatted)").await?;
        Ok(val["ts"].as_str().unwrap_or("").to_string())
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
            .await?;
        api_json(resp, "chat.update").await?;
        Ok(())
    }

    fn channel(&self) -> Channel {
        Channel::Slack
    }

    /// Upload `content` as a file named `filename` to the Slack conversation.
    ///
    /// The legacy `files.upload` endpoint is deprecated/sunset, so this uses the
    /// current external-upload flow:
    ///   1. `files.getUploadURLExternal` → an upload URL + a file id
    ///   2. POST the raw bytes to that URL
    ///   3. `files.completeUploadExternal` → associate the file to the channel
    ///      (and thread, if any)
    ///
    /// The bytes are sent verbatim (no UTF-8 round-trip), so binary files are
    /// uploaded intact.
    async fn upload(
        &self,
        chat: &str,
        thread: Option<&str>,
        filename: &str,
        content: &[u8],
    ) -> anyhow::Result<()> {
        let (upload_url, file_id) = self
            .get_upload_url_external(filename, content.len())
            .await?;
        self.put_bytes_to_upload_url(&upload_url, content).await?;
        self.complete_upload_external(&file_id, filename, chat, thread)
            .await
    }
}

impl SlackAdapter {
    /// Step 1 of the external-upload flow: `files.getUploadURLExternal`.
    ///
    /// Returns `(upload_url, file_id)`. The call is form-encoded with the
    /// `filename` and the byte `length` of the content.
    async fn get_upload_url_external(
        &self,
        filename: &str,
        length: usize,
    ) -> anyhow::Result<(String, String)> {
        let resp = self
            .http
            .post("https://slack.com/api/files.getUploadURLExternal")
            .header("Authorization", format!("Bearer {}", self.bot_token))
            .form(&[
                ("filename", filename.to_string()),
                ("length", length.to_string()),
            ])
            .send()
            .await?
            .error_for_status()?;

        let val: serde_json::Value = resp.json().await?;
        if !val["ok"].as_bool().unwrap_or(false) {
            let err = val["error"].as_str().unwrap_or("unknown").to_string();
            return Err(anyhow::anyhow!("slack files.getUploadURLExternal: {err}"));
        }

        let upload_url = val["upload_url"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("slack getUploadURLExternal: missing upload_url"))?
            .to_string();
        let file_id = val["file_id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("slack getUploadURLExternal: missing file_id"))?
            .to_string();
        Ok((upload_url, file_id))
    }

    /// Step 2 of the external-upload flow: POST the raw bytes to the upload URL.
    ///
    /// The bytes are sent verbatim so binary files are not corrupted.
    async fn put_bytes_to_upload_url(&self, upload_url: &str, bytes: &[u8]) -> anyhow::Result<()> {
        self.http
            .post(upload_url)
            .body(bytes.to_vec())
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("slack upload PUT: {}", redact_url(e, upload_url)))?
            .error_for_status()
            .map_err(|e| anyhow::anyhow!("slack upload PUT: {}", redact_url(e, upload_url)))?;
        Ok(())
    }

    /// Step 3 of the external-upload flow: `files.completeUploadExternal`.
    ///
    /// Associates the uploaded file with `channel_id` (and `thread_ts`, if any).
    async fn complete_upload_external(
        &self,
        file_id: &str,
        filename: &str,
        channel_id: &str,
        thread: Option<&str>,
    ) -> anyhow::Result<()> {
        let files_json = serde_json::json!([{ "id": file_id, "title": filename }]).to_string();

        let mut params = vec![
            ("files", files_json),
            ("channel_id", channel_id.to_string()),
        ];
        if let Some(ts) = thread {
            params.push(("thread_ts", ts.to_string()));
        }

        let resp = self
            .http
            .post("https://slack.com/api/files.completeUploadExternal")
            .header("Authorization", format!("Bearer {}", self.bot_token))
            .form(&params)
            .send()
            .await?
            .error_for_status()?;

        let val: serde_json::Value = resp.json().await?;
        if !val["ok"].as_bool().unwrap_or(false) {
            let err = val["error"].as_str().unwrap_or("unknown").to_string();
            return Err(anyhow::anyhow!("slack files.completeUploadExternal: {err}"));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Socket Mode listener
// ---------------------------------------------------------------------------

/// Sleep `ms`, waking early (in 250 ms slices) once `cancel` is set: a
/// disabled / shut-down listener in a (≤ 60 s) backoff must not linger and
/// make one more `apps.connections.open` call.
async fn sleep_unless_cancelled(ms: u64, cancel: &AtomicBool) {
    let mut left = ms;
    while left > 0 && !cancel.load(Ordering::Relaxed) {
        let step = left.min(250);
        tokio::time::sleep(Duration::from_millis(step)).await;
        left -= step;
    }
}

/// Open and maintain a Slack Socket Mode connection until `cancel` is set.
/// Each inbound `message` event is forwarded to `bridge`. Every connect /
/// drop / failure is reported to `health` (Settings → Channels shows it).
pub async fn run(
    integ: Integration,
    bot_token: String,
    app_token: String,
    bridge: Arc<Bridge>,
    cancel: Arc<AtomicBool>,
    health: Health,
) {
    let http = build_http_client();
    // Downloaded attachments were never deleted; sweep day-old ones off the
    // runtime whenever a listener (re)starts.
    tokio::task::spawn_blocking(|| {
        let n = sweep_stale_attachments(&std::env::temp_dir(), ATTACHMENT_MAX_AGE);
        if n > 0 {
            debug!("slack: removed {n} stale downloaded attachment(s)");
        }
    });
    // In-memory dedup set: keyed by "channel:ts". Shared per bot token
    // across listener generations: a config edit respawns the listener while
    // Slack may still redeliver events the previous generation handled (an
    // unacked envelope, or the old socket's last frames) — a fresh window
    // used to forward those twice.
    let seen = seen_window(&bot_token);

    let mut backoff_ms: u64 = 3_000;
    const BACKOFF_MAX_MS: u64 = 60_000;

    'outer: loop {
        if cancel.load(Ordering::Relaxed) {
            debug!("slack listener stopping (cancel)");
            return;
        }

        // --- Step 1: request a fresh WSS URL from apps.connections.open ---
        let wss_url = match open_socket_mode_connection(&http, &app_token).await {
            Ok(url) => url,
            Err(OpenError { detail, permanent }) => {
                error!("slack: failed to open socket mode connection ({detail}), retrying in {backoff_ms}ms");
                // A rejected token won't heal on a 3 s cadence: go straight
                // to the ceiling instead of hammering Slack.
                if permanent {
                    backoff_ms = BACKOFF_MAX_MS;
                }
                health.failed(&detail, permanent);
                sleep_unless_cancelled(backoff_ms, &cancel).await;
                backoff_ms = (backoff_ms * 2).min(BACKOFF_MAX_MS);
                continue 'outer;
            }
        };

        // --- Step 2: connect to the WSS URL ---
        info!("slack: connecting to socket mode");
        // The dial MUST be bounded — see `WS_CONNECT_TIMEOUT`. A timeout is
        // just another connect failure: fall through to the same backoff.
        let dial = tokio::time::timeout(
            WS_CONNECT_TIMEOUT,
            tokio_tungstenite::connect_async(&wss_url),
        )
        .await;
        let ws_stream = match dial {
            Ok(Ok((stream, _))) => stream,
            Ok(Err(e)) => {
                let why = redact_url(&e, &wss_url);
                error!("slack: websocket connect failed: {why}");
                health.failed(&format!("Socket Mode connect failed: {why}"), false);
                sleep_unless_cancelled(backoff_ms, &cancel).await;
                backoff_ms = (backoff_ms * 2).min(BACKOFF_MAX_MS);
                continue 'outer;
            }
            Err(_elapsed) => {
                error!(
                    "slack: websocket connect timed out after {}s, retrying in {backoff_ms}ms",
                    WS_CONNECT_TIMEOUT.as_secs()
                );
                health.failed(
                    &format!(
                        "Socket Mode connect timed out after {}s (network down or Slack unreachable)",
                        WS_CONNECT_TIMEOUT.as_secs()
                    ),
                    false,
                );
                sleep_unless_cancelled(backoff_ms, &cancel).await;
                backoff_ms = (backoff_ms * 2).min(BACKOFF_MAX_MS);
                continue 'outer;
            }
        };

        let (mut sink, mut stream) = ws_stream.split();

        // --- Step 3: read frames ---
        let mut last_frame = std::time::Instant::now();
        let mut last_ping = std::time::Instant::now();
        // The inner loop yields why it ended (health detail) and whether that
        // was a failure (vs. Slack's routine `disconnect` refresh).
        let (drop_reason, drop_failed): (String, bool) = 'inner: loop {
            if cancel.load(Ordering::Relaxed) {
                debug!("slack listener stopping (cancel) in inner loop");
                return;
            }

            // Zombie-socket watchdog: see IDLE_RECONNECT. Any frame (Slack's
            // own pings included) resets the clock below.
            if last_frame.elapsed() >= IDLE_RECONNECT {
                warn!(
                    "slack: no frames for {}s — connection presumed dead, reconnecting",
                    last_frame.elapsed().as_secs()
                );
                break 'inner (
                    format!(
                        "No frames from Slack for {}s — connection presumed dead",
                        last_frame.elapsed().as_secs()
                    ),
                    true,
                );
            }
            if last_ping.elapsed() >= CLIENT_PING_INTERVAL {
                last_ping = std::time::Instant::now();
                if let Err(e) = send_frame(&mut sink, Message::Ping(Default::default())).await {
                    warn!("slack: probe ping failed ({e}), reconnecting");
                    break 'inner (format!("Probe ping failed: {e}"), true);
                }
            }

            // Use a select with a timeout so we can check `cancel` (and the
            // watchdog above) periodically.
            let maybe_msg = tokio::select! {
                msg = stream.next() => msg,
                _ = tokio::time::sleep(CANCEL_CHECK_INTERVAL) => {
                    continue 'inner;
                }
            };
            if let Some(Ok(_)) = &maybe_msg {
                last_frame = std::time::Instant::now();
            }

            let raw = match maybe_msg {
                Some(Ok(Message::Text(text))) => text,
                Some(Ok(Message::Ping(data))) => {
                    // Respond to WebSocket-level pings. A pong that can't
                    // be written means the socket is gone — reconnect.
                    if let Err(e) = send_frame(&mut sink, Message::Pong(data)).await {
                        warn!("slack: pong failed ({e}), reconnecting");
                        break 'inner (format!("Pong failed: {e}"), true);
                    }
                    continue 'inner;
                }
                Some(Ok(Message::Close(_))) => {
                    info!("slack: server closed websocket, reconnecting");
                    break 'inner ("Slack closed the connection".into(), true);
                }
                Some(Ok(_)) => continue 'inner, // binary / pong frames
                // The backoff pause is taken ONCE, after the inner loop —
                // sleeping here too doubled every reconnect delay.
                Some(Err(e)) => {
                    error!("slack: websocket error: {e}, reconnecting");
                    break 'inner (
                        format!("WebSocket error: {}", redact_url(&e, &wss_url)),
                        true,
                    );
                }
                None => {
                    info!("slack: stream ended, reconnecting");
                    break 'inner ("Connection ended".into(), true);
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
                    backoff_ms = 3_000;
                    health.connected();
                }
                "disconnect" => {
                    // Routine: Slack rotates Socket Mode connections every
                    // few hours (`refresh_requested`), or `link_disabled`
                    // when Socket Mode was switched off for the app.
                    let reason = val["reason"].as_str().unwrap_or("unspecified");
                    info!("slack: disconnect requested by server ({reason}), reconnecting");
                    if reason == "link_disabled" {
                        break 'inner (
                            "Slack disabled Socket Mode for this app (link_disabled) — turn \
                             Socket Mode back on in the app settings"
                                .into(),
                            true,
                        );
                    }
                    break 'inner (format!("Slack asked to reconnect ({reason})"), false);
                }
                "events_api" => {
                    // Always ack immediately.
                    let envelope_id = val["envelope_id"].as_str().unwrap_or("").to_string();
                    if !envelope_id.is_empty() {
                        let ack = format!(r#"{{"envelope_id":"{envelope_id}"}}"#);
                        if let Err(e) = send_frame(&mut sink, Message::Text(ack.into())).await {
                            error!("slack: failed to send ack: {e}");
                            break 'inner (format!("Event ack failed: {e}"), true);
                        }
                    }
                    health.event();

                    // Dedup: build key from (channel, ts).
                    let event = &val["payload"]["event"];
                    let dedup_key = {
                        let ch = event["channel"].as_str().unwrap_or("");
                        format!("{ch}:{}", dedup_ts(event))
                    };
                    {
                        let mut guard = seen.lock().await;
                        if !guard.insert(dedup_key.clone(), DEDUP_CAP) {
                            debug!("slack: duplicate event {dedup_key}, skipping");
                            continue 'inner;
                        }
                    }

                    // Process the event payload OFF the read loop: attachment
                    // downloads + session spawn can take seconds, and a slow
                    // event must never delay reading (and acking) the next
                    // frame. Ordering into a shared session is unaffected —
                    // the bridge's find-or-create lock serializes that, and
                    // PTY submits were already spawned per message.
                    let event = event.clone();
                    let integ = integ.clone();
                    let bot_token = bot_token.clone();
                    let bridge = Arc::clone(&bridge);
                    tokio::spawn(async move {
                        handle_event(&event, &integ, &bot_token, bridge).await;
                    });
                }
                other => {
                    // Ack anything that carries an envelope_id (slash commands, interactive, etc.)
                    if let Some(eid) = val["envelope_id"].as_str() {
                        if !eid.is_empty() {
                            let ack = format!(r#"{{"envelope_id":"{eid}"}}"#);
                            if let Err(e) = send_frame(&mut sink, Message::Text(ack.into())).await {
                                error!("slack: failed to send ack: {e}");
                                break 'inner (format!("Envelope ack failed: {e}"), true);
                            }
                        }
                    }
                    debug!("slack: unhandled envelope type '{other}', ignored");
                }
            }
        };

        if cancel.load(Ordering::Relaxed) {
            return;
        }
        if drop_failed {
            health.failed(&drop_reason, false);
        } else {
            health.reconnecting(&drop_reason);
        }
        // Pause before reconnecting (exponential backoff, reset on successful hello).
        sleep_unless_cancelled(backoff_ms, &cancel).await;
        backoff_ms = (backoff_ms * 2).min(BACKOFF_MAX_MS);
    }
}

/// Why `apps.connections.open` failed: a user-facing `detail`, and whether
/// retrying can fix it (see [`is_permanent_open_error`]).
struct OpenError {
    detail: String,
    permanent: bool,
}

/// POST to `apps.connections.open` and return the WSS URL.
async fn open_socket_mode_connection(
    http: &reqwest::Client,
    app_token: &str,
) -> Result<String, OpenError> {
    let transient = |detail: String| OpenError {
        detail,
        permanent: false,
    };
    let resp = http
        .post("https://slack.com/api/apps.connections.open")
        .header("Authorization", format!("Bearer {app_token}"))
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body("")
        .send()
        .await
        .map_err(|e| {
            error!("slack: apps.connections.open request failed: {e}");
            transient(format!("Couldn't reach Slack (apps.connections.open): {e}"))
        })?;
    if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Err(transient(
            "Slack rate-limited the Socket Mode connect (apps.connections.open)".into(),
        ));
    }
    let val: serde_json::Value = resp.json().await.map_err(|e| {
        error!("slack: apps.connections.open parse failed: {e}");
        transient(format!(
            "Unreadable reply from Slack (apps.connections.open): {e}"
        ))
    })?;
    if !val["ok"].as_bool().unwrap_or(false) {
        let err = val["error"].as_str().unwrap_or("unknown");
        error!("slack: apps.connections.open not ok: {err}");
        return Err(OpenError {
            detail: open_error_detail(err),
            permanent: is_permanent_open_error(err),
        });
    }
    val["url"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| transient("Slack returned no Socket Mode URL".into()))
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
    debug!(event_type, "slack: message-like event received");

    // Loop prevention — the ONLY thing we ever drop. Never forward the bot's
    // own messages, including the nested message of an edit (`message_changed`):
    // the mirror edits its "working…" feed every couple of seconds, and each
    // edit emits a message_changed event authored by the bot. Without this the
    // relay would feed its own output back to the agent in a tight loop.
    if event["bot_id"].is_string() || event["message"]["bot_id"].is_string() {
        debug!(event_type, "slack: bot message skipped (loop prevention)");
        return;
    }

    // Policy: accept ALL user messages. We never drop a message for having an
    // unknown subtype — when in doubt, forward it and let the agent read/act.
    // The exceptions carry no new user content: a deletion is a tombstone, an
    // edit (`message_changed`) carries its content under `message`, and
    // membership notices ("X has joined") are system announcements — a batch
    // invite emits dozens at once, each of which would spawn an agent session,
    // and Slack rejects threading onto them (`cannot_reply_to_message`).
    let subtype = event["subtype"].as_str();
    if subtype == Some("message_deleted") {
        info!(event_type, "slack: deletion skipped (nothing to forward)");
        return;
    }
    if is_membership_subtype(subtype) {
        info!(
            event_type,
            subtype, "slack: membership notice skipped (no user content)"
        );
        return;
    }
    // A `message_changed` with no `edited` stamp is Slack rewriting the message
    // itself, not the user changing it — overwhelmingly a link UNFURL landing a
    // few seconds after the post. The text is identical, so a message that
    // happened to contain a URL got processed twice: two agent sessions, or two
    // runs of the same workflow, from one human message. Only a human edit
    // carries `message.edited`.
    if subtype == Some("message_changed") && !is_human_edit(event) {
        info!(
            event_type,
            "slack: message rewrite skipped (unfurl/attachment, no user edit)"
        );
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

    // The allowed-users gate runs HERE as well as in the bridge: before it,
    // anyone who could message the bot made the daemon download up to 50 MB
    // per attachment into the temp dir — only for the bridge to drop it.
    if !crate::bridge::integration_admits(integ, &user) {
        info!(
            event_type,
            user = %user,
            "slack: sender not in allowed_users, dropped before attachment download"
        );
        return;
    }

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
        user_name: ["display_name", "real_name"]
            .iter()
            .find_map(|k| content["user_profile"][*k].as_str())
            .filter(|n| !n.trim().is_empty())
            .map(str::to_string),
        text: combined,
        edited: subtype == Some("message_changed"),
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
    let client = build_download_client();
    // Up to ATTACHMENT_PARALLELISM downloads at once (a multi-file message
    // used to fetch them strictly one after another); notes keep the
    // message's file order.
    let mut notes: Vec<String> = Vec::with_capacity(files.len());
    for batch in files.chunks(ATTACHMENT_PARALLELISM) {
        notes.extend(
            futures_util::future::join_all(
                batch.iter().map(|f| attachment_note(&client, f, bot_token)),
            )
            .await,
        );
    }
    if notes.is_empty() {
        return String::new();
    }
    format!(
        "[Attachment(s) from the user — read them from these local paths and act on them:]\n{}",
        notes.join("\n")
    )
}

/// Download one attached file and describe it for the agent: its saved path,
/// or (download failed / no URL) its permalink so the agent knows it exists.
async fn attachment_note(
    client: &reqwest::Client,
    f: &serde_json::Value,
    bot_token: &str,
) -> String {
    let name = f["name"].as_str().unwrap_or("file");
    let id = f["id"].as_str().unwrap_or("nofileid");
    let mimetype = f["mimetype"].as_str().unwrap_or("application/octet-stream");
    let url = f["url_private_download"]
        .as_str()
        .or_else(|| f["url_private"].as_str());
    match url {
        Some(u) => match download_slack_file(client, u, bot_token, id, name).await {
            Ok(path) => format!("• {name} ({mimetype}) — saved to: {path}"),
            Err(e) => {
                warn!("slack: failed to download attachment {name}: {e}");
                let link = f["permalink"].as_str().unwrap_or(u);
                format!("• {name} ({mimetype}) — could not download automatically; URL: {link}")
            }
        },
        None => format!("• {name} ({mimetype}) — no download URL available"),
    }
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
    let mut resp = client
        .get(url)
        .header("Authorization", format!("Bearer {bot_token}"))
        .send()
        .await?;
    if !resp.status().is_success() {
        anyhow::bail!("http {}", resp.status());
    }
    // Bounded read: a huge upload must not be buffered whole in daemon memory.
    // Refuse up front on a declared oversize, and stop streaming past the cap
    // when the length is absent/understated.
    if resp
        .content_length()
        .is_some_and(|n| n > MAX_DOWNLOAD_BYTES)
    {
        anyhow::bail!(
            "file is larger than the {} MB limit",
            MAX_DOWNLOAD_BYTES >> 20
        );
    }
    let sanitize = |s: &str| -> String {
        s.chars()
            .map(|c| {
                if c.is_alphanumeric() || matches!(c, '.' | '-' | '_') {
                    c
                } else {
                    '_'
                }
            })
            .collect()
    };
    let safe_name = sanitize(name);
    let safe_id = sanitize(id);
    let path = std::env::temp_dir().join(format!("{ATTACHMENT_PREFIX}{safe_id}-{safe_name}"));
    // Stream chunks straight to disk (a ≤50 MB upload is never buffered whole
    // in daemon memory); a partial file from an oversize/failed stream is
    // removed.
    let mut file = tokio::fs::File::create(&path).await?;
    let mut written: u64 = 0;
    let streamed: anyhow::Result<()> = async {
        use tokio::io::AsyncWriteExt;
        while let Some(chunk) = resp.chunk().await? {
            written += chunk.len() as u64;
            if written > MAX_DOWNLOAD_BYTES {
                anyhow::bail!(
                    "file is larger than the {} MB limit",
                    MAX_DOWNLOAD_BYTES >> 20
                );
            }
            file.write_all(&chunk).await?;
        }
        file.flush().await?;
        Ok(())
    }
    .await;
    if let Err(e) = streamed {
        drop(file);
        let _ = tokio::fs::remove_file(&path).await;
        return Err(e);
    }
    Ok(path.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sweep_removes_only_stale_slack_attachments() {
        let dir = std::env::temp_dir().join(format!(
            "otto-slack-sweep-test-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let old = dir.join(format!("{ATTACHMENT_PREFIX}F1-old.txt"));
        let fresh = dir.join(format!("{ATTACHMENT_PREFIX}F2-new.txt"));
        let other = dir.join("unrelated.txt");
        for p in [&old, &fresh, &other] {
            std::fs::write(p, b"x").unwrap();
        }
        let two_days = std::time::SystemTime::now() - Duration::from_secs(2 * 24 * 3600);
        for p in [&old, &other] {
            std::fs::File::options()
                .write(true)
                .open(p)
                .unwrap()
                .set_modified(two_days)
                .unwrap();
        }
        assert_eq!(sweep_stale_attachments(&dir, ATTACHMENT_MAX_AGE), 1);
        assert!(!old.exists());
        assert!(fresh.exists());
        assert!(other.exists(), "only otto-slack-* files are swept");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dedup_window_evicts_oldest_not_everything() {
        let mut w = DedupWindow::default();
        assert!(w.insert("a".into(), 3));
        assert!(w.insert("b".into(), 3));
        assert!(w.insert("c".into(), 3));
        assert!(!w.insert("c".into(), 3), "repeat inside the window");
        assert!(w.insert("d".into(), 3), "evicts a");
        assert!(!w.insert("b".into(), 3), "b survived the eviction");
        assert!(w.insert("a".into(), 3), "a was the one evicted");
        assert_eq!(w.order.len(), 3);
        assert_eq!(w.set.len(), 3);
    }

    #[test]
    fn an_unfurl_is_not_a_user_edit_and_dedups_onto_the_original() {
        // Slack attaches a link preview a few seconds after the post and emits
        // `message_changed` with the SAME text and a NEW top-level ts. Keying
        // dedup on that ts made one human message start two workflow runs —
        // observed on three consecutive messages, all of which contained a URL.
        let unfurl = serde_json::json!({
            "channel": "C1",
            "ts": "1785255516.000100",              // the rewrite's own ts
            "subtype": "message_changed",
            "message": {
                "ts": "1785255511.983239",          // the message it concerns
                "user": "U1",
                "text": "Action: Workflow\nName: PR Reviewer",
                "attachments": [{"title": "some link"}]
            }
        });
        assert!(!is_human_edit(&unfurl), "an unfurl is not an edit");
        assert_eq!(
            dedup_ts(&unfurl),
            "1785255511.983239",
            "dedup on the original"
        );

        // A real human edit keeps the `edited` stamp — it is new content and
        // is forwarded, so it must NOT dedup onto the original post (that
        // swallowed every edit); a redelivery of the same edit still dedups.
        let edited = serde_json::json!({
            "channel": "C1",
            "ts": "1785255520.000200",
            "subtype": "message_changed",
            "message": {
                "ts": "1785255511.983239",
                "user": "U1",
                "text": "fixed typo",
                "edited": {"user": "U1", "ts": "1785255520.000000"}
            }
        });
        assert!(is_human_edit(&edited));
        // A LATER rewrite (unfurl, parent update) of an edited message keeps
        // the old stamp — it is not a fresh edit and is not re-forwarded.
        let mut stale = edited.clone();
        stale["ts"] = serde_json::json!("1785259999.000100");
        assert!(
            !is_human_edit(&stale),
            "an old edit stamp is not a new edit"
        );
        // A hidden inner message is never user content.
        let mut hidden = edited.clone();
        hidden["message"]["hidden"] = serde_json::json!(true);
        assert!(!is_human_edit(&hidden));
        assert_eq!(
            dedup_ts(&edited),
            "1785255511.983239#edit:1785255520.000000",
            "an edit is distinct from the original post"
        );
        assert_ne!(dedup_ts(&edited), dedup_ts(&unfurl));
        // A second, later edit is distinct again.
        let mut edited2 = edited.clone();
        edited2["message"]["edited"]["ts"] = serde_json::json!("1785255530.000000");
        assert_ne!(dedup_ts(&edited2), dedup_ts(&edited));
        // The dedup window survives a listener respawn on the same bot.
        assert!(Arc::ptr_eq(
            &seen_window("xoxb-dedup-test"),
            &seen_window("xoxb-dedup-test")
        ));

        // A plain new message keys on its own ts, exactly as before.
        let plain = serde_json::json!({"channel": "C1", "ts": "1785255600.5", "text": "hi"});
        assert_eq!(dedup_ts(&plain), "1785255600.5");
    }

    #[test]
    fn retry_after_is_parsed_and_capped() {
        assert_eq!(retry_after_secs(Some("7")), 7);
        assert_eq!(retry_after_secs(Some(" 30 ")), 30);
        assert_eq!(retry_after_secs(None), 1, "no header → retry soon");
        assert_eq!(retry_after_secs(Some("soon")), 1);
        assert_eq!(retry_after_secs(Some("0")), 1);
        assert_eq!(retry_after_secs(Some("86400")), MAX_RETRY_AFTER_SECS);
    }

    #[test]
    fn rejected_app_tokens_are_reported_as_failing_with_a_fix() {
        for code in ["invalid_auth", "token_revoked", "not_allowed_token_type"] {
            assert!(is_permanent_open_error(code), "{code}");
        }
        // Transport-ish / server-side codes are worth retrying quietly.
        for code in ["internal_error", "ratelimited", "fatal_error", "unknown"] {
            assert!(!is_permanent_open_error(code), "{code}");
        }
        assert!(open_error_detail("not_allowed_token_type").contains("xapp-"));
        assert!(open_error_detail("missing_scope").contains("connections:write"));
        assert!(open_error_detail("invalid_auth").contains("invalid_auth"));
        assert!(open_error_detail("internal_error").contains("internal_error"));
    }

    #[test]
    fn membership_subtypes_are_skipped() {
        for s in ["channel_join", "channel_leave", "group_join", "group_leave"] {
            assert!(is_membership_subtype(Some(s)), "{s} must be skipped");
        }
    }

    #[test]
    fn content_subtypes_are_forwarded() {
        // No subtype (plain message) and content-bearing subtypes still flow.
        assert!(!is_membership_subtype(None));
        for s in [
            "message_changed",
            "thread_broadcast",
            "file_share",
            "me_message",
        ] {
            assert!(!is_membership_subtype(Some(s)), "{s} must be forwarded");
        }
    }
}
