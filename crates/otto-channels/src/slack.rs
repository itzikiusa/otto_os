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

/// The Socket Mode WebSocket, as `connect_async` returns it.
type SocketStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Dial the Socket Mode WebSocket, bounded by `deadline` (production passes
/// [`WS_CONNECT_TIMEOUT`]). `Err` is the user-facing reason, URL redacted; a
/// timeout is just another connect failure, so the caller treats every error
/// alike — report it, back off, retry.
async fn dial_socket(url: &str, deadline: Duration) -> Result<SocketStream, String> {
    match tokio::time::timeout(deadline, tokio_tungstenite::connect_async(url)).await {
        Ok(Ok((stream, _))) => Ok(stream),
        Ok(Err(e)) => Err(format!(
            "Socket Mode connect failed: {}",
            redact_url(&e, url)
        )),
        Err(_elapsed) => Err(format!(
            "Socket Mode connect timed out after {}s (network down or Slack unreachable)",
            deadline.as_secs()
        )),
    }
}

/// How long to wait for a TCP/TLS connection to Slack to establish.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Overall per-request deadline for ordinary Web API calls. A hung Slack
/// endpoint must not block the caller (or the listener loop) indefinitely.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Overall deadline for file-download requests, which can carry large payloads
/// and so are given a more generous budget than ordinary API calls.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(120);
/// Overall deadline for the step-2 byte POST of an outbound attachment (up to
/// `attach_guard::MAX_ATTACHMENT_BYTES`): on the 30 s API budget a 20 MiB file
/// needed ~5.6 Mbit/s of uplink and timed out on a VPN (integrations-5-04).
/// Matches Telegram's `sendDocument` budget.
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(120);
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
pub(crate) fn build_http_client() -> reqwest::Client {
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

/// The process-wide HTTP client for attachment uploads ([`UPLOAD_TIMEOUT`]);
/// shared like [`build_http_client`].
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

/// The listener badge while the inbound queue is full and events are left
/// unacknowledged for Slack to redeliver (the socket itself is fine).
const QUEUE_FULL_DETAIL: &str =
    "Otto is busy: new Slack messages are being retried until earlier ones finish";

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

/// Decode a Web API response into its JSON body, or an error naming `method`.
/// An HTTP 429 becomes `slack <method>: ratelimited (retry after Ns)` with N
/// from the `Retry-After` header — the mirror keys its back-off on
/// `ratelimited` and reads the delay back with `mirror::retry_after_hint`, so a
/// rate-limited final reply is re-posted instead of silently lost. (It used to
/// surface as reqwest's generic "429 Too Many Requests" with no delay.)
pub(crate) async fn api_json(
    resp: reqwest::Response,
    method: &str,
) -> anyhow::Result<serde_json::Value> {
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
/// never shortened: it is a server-imposed minimum, not our retry budget.
fn retry_after_secs(header: Option<&str>) -> u64 {
    header
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(1)
        .max(1)
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

/// Turn Slack's wire encoding of a message's `text` back into what the person
/// typed. Slack escapes `&`, `<` and `>` as `&amp;` / `&lt;` / `&gt;` in every
/// message and wraps links, mentions and broadcasts in `<…>` control
/// sequences — forwarded raw, a pasted `if a < b && c` reached the agent as
/// `if a &lt; b &amp;&amp; c`. A raw `<` is therefore always a control
/// sequence: `<url|label>` → the url (with the label when it differs),
/// `<@U1|dana>` → `@dana`, `<#C1|ops>` → `#ops`, `<!here>` → `@here`; the
/// entities are decoded last so an escaped `&amp;lt;` stays a literal `&lt;`.
pub(crate) fn decode_slack_text(text: &str) -> String {
    fn control(inner: &str) -> String {
        let (target, label) = match inner.split_once('|') {
            Some((t, l)) => (t, Some(l)),
            None => (inner, None),
        };
        if let Some(id) = target.strip_prefix('@') {
            return format!("@{}", label.unwrap_or(id));
        }
        if let Some(id) = target.strip_prefix('#') {
            return format!("#{}", label.unwrap_or(id));
        }
        if let Some(special) = target.strip_prefix('!') {
            // `<!subteam^S1|@team>`, `<!date^…|fallback>` carry their text in
            // the label; `<!here>` / `<!channel>` / `<!everyone>` don't.
            return match label {
                Some(l) => l.to_string(),
                None => format!("@{special}"),
            };
        }
        let bare = target.strip_prefix("mailto:").unwrap_or(target);
        match label {
            // Slack's auto-link writes the url (minus scheme) as the label.
            Some(l) if !l.is_empty() && l != bare && !target.ends_with(&format!("//{l}")) => {
                format!("{l} ({bare})")
            }
            _ => bare.to_string(),
        }
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('<') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after.find('>') {
            Some(close) => {
                out.push_str(&control(&after[..close]));
                rest = &after[close + 1..];
            }
            None => {
                out.push_str(&rest[open..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
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
/// A HUMAN edit is new content and must be forwarded (see `handle_event` and `drop_reason`),
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
    /// `_italic_`, `` `code` ``, and `<URL|label>` links when `mrkdwn: true`;
    /// agents answer in GitHub-flavoured Markdown, so the text is converted
    /// first ([`crate::gfm::to_slack_mrkdwn`]).
    async fn send_formatted(
        &self,
        chat: &str,
        thread: Option<&str>,
        text: &str,
    ) -> anyhow::Result<String> {
        let mut body = serde_json::json!({
            "channel": chat,
            "text": crate::gfm::to_slack_mrkdwn(text),
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
            .await?;

        let val = api_json(resp, "files.getUploadURLExternal").await?;
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
        let response = build_upload_client()
            .post(upload_url)
            .body(bytes.to_vec())
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("slack upload PUT: {}", redact_url(e, upload_url)))?;
        if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            let seconds = retry_after_secs(
                response
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|v| v.to_str().ok()),
            );
            anyhow::bail!("slack upload PUT: ratelimited (retry after {seconds}s)");
        }
        response
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
            .await?;

        let val = api_json(resp, "files.completeUploadExternal").await?;
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

/// How long the Socket Mode listener may stay disconnected before the
/// notification centre says so. A routine reconnect (Slack rotates the
/// socket every few hours) takes seconds; two minutes is an outage.
const DOWN_NOTICE_AFTER: Duration = Duration::from_secs(120);

/// One disconnected spell of a listener and whether it was announced. Pure:
/// the caller passes the clock.
#[derive(Debug, Default)]
struct DownSpell {
    since: Option<std::time::Instant>,
    noticed: bool,
}

impl DownSpell {
    /// Not connected at `now`. True exactly once per spell, when it has
    /// lasted [`DOWN_NOTICE_AFTER`].
    fn down(&mut self, now: std::time::Instant) -> bool {
        let since = *self.since.get_or_insert(now);
        if self.noticed || now.saturating_duration_since(since) < DOWN_NOTICE_AFTER {
            return false;
        }
        self.noticed = true;
        true
    }

    /// Connected (`hello`). True when the spell had been announced, so the
    /// recovery is posted too.
    fn up(&mut self) -> bool {
        std::mem::take(self).noticed
    }
}

/// Raise (`down` = why) or clear (`None`) the "Slack disconnected" notice
/// for `integ`'s workspace — one bell row per integration, refreshed in
/// place by the recovery. Spawned, so the frame loop never waits on the DB.
fn post_listener_notice(bridge: &Arc<Bridge>, integ: &Integration, down: Option<String>) {
    use otto_core::domain::NoticeSeverity;
    let bridge = Arc::clone(bridge);
    let ws = integ.workspace_id.clone();
    tokio::spawn(async move {
        let name = match bridge.workspaces.get(&ws).await {
            Ok(w) => w.name,
            Err(_) => ws.clone(),
        };
        let (severity, title, body) = match down {
            Some(why) => (
                NoticeSeverity::Warn,
                "Slack disconnected",
                format!(
                    "Otto's Slack connection for workspace “{name}” has been down for over 2 \
                     minutes ({why}). Otto keeps reconnecting; messages to the bot are not \
                     answered until it is back."
                ),
            ),
            None => (
                NoticeSeverity::Info,
                "Slack reconnected",
                format!("Otto's Slack connection for workspace “{name}” is back."),
            ),
        };
        let key = format!("channel:slack:{ws}:listener");
        bridge
            .manager
            .post_system_notice(&key, severity, title, &body, Some("settings/channels"))
            .await;
    });
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
    let pending_notices = Arc::new(tokio::sync::Mutex::new(HashSet::<String>::new()));

    let mut backoff_ms: u64 = 3_000;
    const BACKOFF_MAX_MS: u64 = 60_000;
    // Down since the listener started until the first `hello`.
    let mut spell = DownSpell::default();
    let mut problem = String::from("not connected yet");

    'outer: loop {
        if cancel.load(Ordering::Relaxed) {
            debug!("slack listener stopping (cancel)");
            return;
        }
        if spell.down(std::time::Instant::now()) {
            post_listener_notice(&bridge, &integ, Some(problem.clone()));
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
                problem = detail;
                sleep_unless_cancelled(backoff_ms, &cancel).await;
                backoff_ms = (backoff_ms * 2).min(BACKOFF_MAX_MS);
                continue 'outer;
            }
        };

        // --- Step 2: connect to the WSS URL ---
        info!("slack: connecting to socket mode");
        // The dial MUST be bounded — see `WS_CONNECT_TIMEOUT`. A timeout is
        // just another connect failure: fall through to the same backoff.
        let ws_stream = match dial_socket(&wss_url, WS_CONNECT_TIMEOUT).await {
            Ok(stream) => stream,
            Err(why) => {
                error!("slack: {why}, retrying in {backoff_ms}ms");
                health.failed(&why, false);
                problem = why;
                sleep_unless_cancelled(backoff_ms, &cancel).await;
                backoff_ms = (backoff_ms * 2).min(BACKOFF_MAX_MS);
                continue 'outer;
            }
        };

        let (mut sink, mut stream) = ws_stream.split();
        let (notice_acks, mut completed_notices) = tokio::sync::mpsc::channel::<String>(8);

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
                Some(envelope_id) = completed_notices.recv() => {
                    let ack = serde_json::json!({"envelope_id": envelope_id}).to_string();
                    if let Err(error) = send_frame(&mut sink, Message::Text(ack.into())).await {
                        break 'inner (format!("Rejection ack failed: {error}"), true);
                    }
                    continue 'inner;
                }
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
                    if spell.up() {
                        post_listener_notice(&bridge, &integ, None);
                    }
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
                    use crate::admission::Admission;
                    let envelope_id = val["envelope_id"].as_str().unwrap_or("").to_string();
                    let event = &val["payload"]["event"];
                    let content = if event["subtype"].as_str() == Some("message_changed") {
                        &event["message"]
                    } else {
                        event
                    };
                    let chat = content["channel"]
                        .as_str()
                        .or_else(|| event["channel"].as_str())
                        .unwrap_or("");
                    let thread = content["thread_ts"]
                        .as_str()
                        .or_else(|| event["thread_ts"].as_str())
                        .or_else(|| content["ts"].as_str())
                        .or_else(|| event["ts"].as_str());
                    let dedup_key = format!("{chat}:{}", dedup_ts(event));
                    if pending_notices.lock().await.contains(&dedup_key) {
                        continue 'inner; // not acknowledged until its notice succeeds
                    }
                    let duplicate = seen.lock().await.set.contains(&dedup_key);
                    // Nothing to forward (bot feed edits, deletions, joins,
                    // unfurls): ACK and remember it, with no admission permit
                    // or ingress ticket (backend-correctness-5-04).
                    let skipped = if duplicate { None } else { drop_reason(event) };
                    if let Some(why) = skipped {
                        debug!("slack: {why}; acknowledged without admission");
                        seen.lock().await.insert(dedup_key.clone(), DEDUP_CAP);
                    }
                    let admission = if duplicate || skipped.is_some() {
                        None
                    } else {
                        Some(bridge.admission.admit_chat(
                            &format!("slack:{}", integ.workspace_id),
                            chat,
                            thread,
                            strip_mention(content["text"].as_str().unwrap_or("")),
                            event["subtype"].as_str() == Some("message_changed"),
                        ))
                    };
                    match admission {
                        Some(Admission::Retry) => {
                            // The socket is fine: say busy, not disconnected.
                            health.busy(QUEUE_FULL_DETAIL);
                            continue 'inner;
                        }
                        Some(Admission::Busy(permit)) => {
                            // `drop_reason` already let only user messages in.
                            let event = event.clone();
                            let integ = integ.clone();
                            let bridge = Arc::clone(&bridge);
                            let adapter = SlackAdapter::new(bot_token.clone());
                            let acks = notice_acks.clone();
                            let seen = seen.clone();
                            let pending = pending_notices.clone();
                            pending.lock().await.insert(dedup_key.clone());
                            tokio::spawn(async move {
                                let _permit = permit;
                                let notice = settle_busy(&event, &integ, &bridge, &adapter);
                                if acknowledge_rejection(notice, &acks, envelope_id).await {
                                    seen.lock().await.insert(dedup_key.clone(), DEDUP_CAP);
                                }
                                pending.lock().await.remove(&dedup_key);
                            });
                            continue 'inner;
                        }
                        Some(Admission::Work(permit)) => {
                            // Reserve work before ACK/dedup. The permit remains
                            // held through attachment I/O and the bridge turn queue.
                            seen.lock().await.insert(dedup_key, DEDUP_CAP);
                            let order = bridge
                                .reserve_ingress(
                                    Channel::Slack,
                                    &integ.workspace_id,
                                    chat,
                                    thread,
                                    strip_mention(content["text"].as_str().unwrap_or("")),
                                    event["subtype"].as_str() == Some("message_changed"),
                                )
                                .await;
                            let event = event.clone();
                            let integ = integ.clone();
                            let bot_token = bot_token.clone();
                            let bridge = Arc::clone(&bridge);
                            tokio::spawn(async move {
                                let _ordered = match order {
                                    Some(q) => Some(q.acquire().await),
                                    None => None,
                                };
                                let adapter = Arc::new(SlackAdapter::new(bot_token.clone()))
                                    as Arc<dyn Adapter>;
                                handle_event(&event, &integ, &bot_token, adapter, bridge, permit)
                                    .await;
                            });
                        }
                        None => {} // already admitted/delivered: ACK the retry
                    }
                    if !envelope_id.is_empty() {
                        let ack = serde_json::json!({"envelope_id": envelope_id}).to_string();
                        if let Err(error) = send_frame(&mut sink, Message::Text(ack.into())).await {
                            break 'inner (format!("Event ack failed: {error}"), true);
                        }
                    }
                    health.event();
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
        problem = drop_reason;
        // Pause before reconnecting (exponential backoff, reset on successful hello).
        sleep_unless_cancelled(backoff_ms, &cancel).await;
        backoff_ms = (backoff_ms * 2).min(BACKOFF_MAX_MS);
    }
}

/// A rejected message can be acknowledged only after its notice is delivered.
async fn acknowledge_rejection(
    notice: impl std::future::Future<Output = anyhow::Result<()>>,
    acknowledgements: &tokio::sync::mpsc::Sender<String>,
    envelope: String,
) -> bool {
    if notice.await.is_err() {
        return false;
    }
    // Delivery is durable even if this socket disconnected before its ACK;
    // remember it so the next generation ACKs the retry without another notice.
    let _ = acknowledgements.send(envelope).await;
    true
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

/// Why an `events_api` event carries nothing to forward, or `None` for a user
/// message. ONE predicate for the listener's admission gate and
/// [`handle_event`] (backend-correctness-5-04): the listener used to admit
/// first, so the mirror's own feed edits each took a conversation permit and
/// an ingress ticket, queued behind a running turn, and after a handful the
/// person's next message was refused as "busy".
fn drop_reason(event: &serde_json::Value) -> Option<&'static str> {
    // Only message + app_mention events.
    if !matches!(event["type"].as_str(), Some("message" | "app_mention")) {
        return Some("non-message event");
    }
    // Loop prevention. Never forward the bot's own messages, including the
    // nested message of an edit (`message_changed`): the mirror edits its
    // "working…" feed every couple of seconds, and each edit emits a
    // message_changed event authored by the bot. Without this the relay would
    // feed its own output back to the agent in a tight loop.
    if event["bot_id"].is_string() || event["message"]["bot_id"].is_string() {
        return Some("bot message (loop prevention)");
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
        return Some("deletion (nothing to forward)");
    }
    if is_membership_subtype(subtype) {
        return Some("membership notice (no user content)");
    }
    // A `message_changed` with no `edited` stamp is Slack rewriting the message
    // itself, not the user changing it — overwhelmingly a link UNFURL landing a
    // few seconds after the post. The text is identical, so a message that
    // happened to contain a URL got processed twice: two agent sessions, or two
    // runs of the same workflow, from one human message. Only a human edit
    // carries `message.edited`.
    if subtype == Some("message_changed") && !is_human_edit(event) {
        return Some("message rewrite (unfurl/attachment, no user edit)");
    }
    None
}

/// The [`Inbound`] of a user message event, its text decoded from Slack's
/// wire format ([`decode_slack_text`], the ONE place Slack text is decoded —
/// everything downstream, workflow triggers included, reads plain text) with
/// the leading bot mention stripped. Attachments are not part of it: only an
/// admitted sender's files are downloaded ([`handle_event`]).
fn slack_inbound(event: &serde_json::Value, integ: &Integration) -> Inbound {
    let edited = event["subtype"].as_str() == Some("message_changed");
    let content = if edited { &event["message"] } else { event };
    let field = |k: &str| content[k].as_str().or_else(|| event[k].as_str());
    Inbound {
        workspace_id: integ.workspace_id.clone(),
        chat: field("channel").unwrap_or("").to_string(),
        // The thread, else the message itself starts one.
        thread: field("thread_ts")
            .or_else(|| field("ts"))
            .map(str::to_string),
        user: field("user").unwrap_or("").to_string(),
        user_name: ["display_name", "real_name"]
            .iter()
            .find_map(|k| content["user_profile"][*k].as_str())
            .filter(|n| !n.trim().is_empty())
            .map(str::to_string),
        text: decode_slack_text(strip_mention(content["text"].as_str().unwrap_or(""))),
        edited,
    }
}

/// A message the inbound queue refused as busy: an admitted sender is told
/// to send it again ([`BUSY_REPLY`](crate::admission::BUSY_REPLY)); a sender
/// the allow-list turns away gets no reply but is recorded as turned away,
/// exactly as when there is room ([`handle_event`]). `Ok` = settled, so the
/// event may be acknowledged.
async fn settle_busy(
    event: &serde_json::Value,
    integ: &Integration,
    bridge: &Bridge,
    adapter: &dyn Adapter,
) -> anyhow::Result<()> {
    let inbound = slack_inbound(event, integ);
    if !crate::bridge::integration_admits(integ, &inbound.user) {
        bridge.record_rejected(Channel::Slack, &inbound).await;
        return Ok(());
    }
    if inbound.chat.is_empty() {
        return Ok(());
    }
    adapter
        .send_notice(
            &inbound.chat,
            inbound.thread.as_deref(),
            crate::admission::BUSY_REPLY,
        )
        .await
        .map(|_| ())
}

/// Inspect a single `events_api` payload event and, if it is a user message,
/// build an `Inbound` and forward it to the bridge through `adapter`.
async fn handle_event(
    event: &serde_json::Value,
    integ: &Integration,
    bot_token: &str,
    adapter: Arc<dyn Adapter>,
    bridge: Arc<Bridge>,
    permit: crate::admission::WorkPermit,
) {
    let event_type = event["type"].as_str().unwrap_or("");
    // The listener already skipped these before admission; kept here so a
    // direct caller can never forward one.
    if let Some(why) = drop_reason(event) {
        debug!(event_type, "slack: skipped: {why}");
        return;
    }
    debug!(event_type, "slack: message-like event received");
    let mut inbound = slack_inbound(event, integ);

    // The allowed-users gate runs HERE as well as in the bridge: before it,
    // anyone who could message the bot made the daemon download up to 50 MB
    // per attachment into the temp dir — only for the bridge to drop it. The
    // turned-away sender is still recorded (Settings → Channels lists them
    // with "Allow"; a workflow trigger they aimed at says so) — this return
    // used to leave no trace at all.
    if !crate::bridge::integration_admits(integ, &inbound.user) {
        info!(
            event_type,
            user = %inbound.user,
            "slack: sender not in allowed_users, dropped before attachment download"
        );
        bridge.record_rejected(Channel::Slack, &inbound).await;
        return;
    }
    if inbound.chat.is_empty() {
        warn!("slack: message event missing channel, skipping");
        return;
    }

    // Download any attached files to local paths so the agent can read them
    // (Slack `url_private` needs the bot token, so the agent can't fetch them
    // itself). Returns a note listing the saved paths, appended to the message.
    let content = if inbound.edited {
        &event["message"]
    } else {
        event
    };
    let files_note = collect_attachments(content, bot_token).await;

    // Only skip when there is genuinely nothing to act on (no text AND no
    // attachments). Everything else is forwarded.
    let text = std::mem::take(&mut inbound.text);
    inbound.text = match (text.is_empty(), files_note.is_empty()) {
        (true, true) => {
            info!(event_type, user = %inbound.user, "slack: no text or attachments, skipping");
            return;
        }
        (false, true) => text,
        (true, false) => format!(
            "[The user sent attachment(s) with no message text — read them and act:]\n{files_note}"
        ),
        (false, false) => format!("{text}\n\n{files_note}"),
    };
    info!(
        workspace = %inbound.workspace_id,
        chat = %inbound.chat,
        thread = ?inbound.thread,
        user = %inbound.user,
        event_type,
        "slack: forwarding inbound event to bridge"
    );
    bridge.handle(integ, adapter, inbound, permit).await;
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
    fn a_listener_down_two_minutes_is_announced_once_and_its_return_once() {
        let t0 = std::time::Instant::now();
        let mut spell = DownSpell::default();
        assert!(!spell.down(t0), "a reconnect starts the clock");
        assert!(!spell.down(t0 + Duration::from_secs(60)));
        assert!(spell.down(t0 + DOWN_NOTICE_AFTER), "two minutes: announce");
        assert!(!spell.down(t0 + Duration::from_secs(600)), "only once");
        assert!(spell.up(), "the recovery is announced");
        assert!(!spell.up());
        // A quick routine reconnect never reaches the bell.
        let t1 = t0 + Duration::from_secs(3600);
        assert!(!spell.down(t1));
        assert!(!spell.down(t1 + Duration::from_secs(5)));
        assert!(!spell.up(), "nothing was announced, nothing to clear");
        // The next spell has its own clock.
        assert!(!spell.down(t1 + Duration::from_secs(10)));
        assert!(spell.down(t1 + Duration::from_secs(10) + DOWN_NOTICE_AFTER));
    }

    #[test]
    fn inbound_text_is_decoded_from_slack_wire_format() {
        // Entities: code pasted into Slack reaches the agent as typed.
        assert_eq!(
            decode_slack_text("if a &lt; b &amp;&amp; c &gt; d"),
            "if a < b && c > d"
        );
        // A literal "&lt;" the person typed arrives as "&amp;lt;" — keep it.
        assert_eq!(decode_slack_text("write &amp;lt; here"), "write &lt; here");
        // Links: auto-links collapse to the url, labelled ones keep both.
        assert_eq!(
            decode_slack_text("see <https://x.io/a?b=1&amp;c=2|x.io/a?b=1&amp;c=2>"),
            "see https://x.io/a?b=1&c=2"
        );
        assert_eq!(decode_slack_text("<https://x.io>"), "https://x.io");
        assert_eq!(
            decode_slack_text("<https://x.io/pr/7|the PR>"),
            "the PR (https://x.io/pr/7)"
        );
        assert_eq!(
            decode_slack_text("mail <mailto:a@b.co|a@b.co>"),
            "mail a@b.co"
        );
        // Mentions, channels, broadcasts, user groups.
        assert_eq!(
            decode_slack_text("ask <@U1|dana> or <@U2> in <#C1|ops> <!here> <!subteam^S1|@devs>"),
            "ask @dana or @U2 in #ops @here @devs"
        );
        // Workflow trigger text, as Slack sends it: decoded HERE, once — the
        // workflow parsers downstream read plain text and decode nothing.
        assert_eq!(
            decode_slack_text("Action: Workflow\nName: A &lt;B&gt;\nMsg: x &lt; 5 and y &gt; 3"),
            "Action: Workflow\nName: A <B>\nMsg: x < 5 and y > 3"
        );
        assert_eq!(decode_slack_text("Name: R&amp;D"), "Name: R&D");
        assert_eq!(decode_slack_text("Vec&lt;String&gt;"), "Vec<String>");
        // No markup: unchanged; an unterminated `<` can't be a sequence.
        assert_eq!(decode_slack_text("plain text"), "plain text");
        assert_eq!(decode_slack_text("odd <tail"), "odd <tail");
        // The leading bot mention is still stripped first.
        assert_eq!(
            decode_slack_text(strip_mention("<@UBOT> run `x &gt; 1`")),
            "run `x > 1`"
        );
    }

    #[tokio::test]
    async fn rejected_work_is_acknowledged_only_after_a_delivered_notice() {
        let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
        let delivered = tokio::sync::Notify::new();
        let notice = async {
            delivered.notified().await;
            Ok(())
        };
        let reject = acknowledge_rejection(notice, &sender, "event".into());
        tokio::pin!(reject);
        assert!(tokio::time::timeout(Duration::from_millis(20), &mut reject)
            .await
            .is_err());
        assert!(
            receiver.try_recv().is_err(),
            "no ACK while rejection delivery waits"
        );
        delivered.notify_one();
        assert!(reject.await);
        assert_eq!(receiver.recv().await.as_deref(), Some("event"));
        assert!(
            !acknowledge_rejection(
                async { Err(anyhow::anyhow!("fixture delivery failure")) },
                &sender,
                "retry".into()
            )
            .await
        );
        assert!(
            receiver.try_recv().is_err(),
            "failed rejection must remain retryable"
        );
        drop(receiver);
        assert!(
            acknowledge_rejection(
                async { Ok(()) },
                &sender,
                "delivered-before-disconnect".into()
            )
            .await,
            "delivered notice stays deduplicated across socket generations"
        );
    }

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
    fn retry_after_is_parsed_without_shortening_the_cooldown() {
        assert_eq!(retry_after_secs(Some("7")), 7);
        assert_eq!(retry_after_secs(Some(" 30 ")), 30);
        assert_eq!(retry_after_secs(None), 1, "no header → retry soon");
        assert_eq!(retry_after_secs(Some("soon")), 1);
        assert_eq!(retry_after_secs(Some("0")), 1);
        assert_eq!(retry_after_secs(Some("86400")), 86400);
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

    /// backend-correctness-5-04: the listener's pre-admission predicate drops
    /// everything `handle_event` would (bot posts and feed edits, deletions,
    /// joins, unfurl rewrites, non-message events) and keeps user messages,
    /// human edits and unknown subtypes.
    #[test]
    fn drop_reason_skips_non_user_events_before_admission() {
        use serde_json::json;
        let dropped = [
            json!({"type": "reaction_added", "user": "U1"}),
            json!({"type": "message", "bot_id": "B1", "text": "working…"}),
            json!({"type": "message", "subtype": "message_changed",
                   "message": {"bot_id": "B1", "text": "working…", "edited": {"ts": "1"}}}),
            json!({"type": "message", "subtype": "message_deleted"}),
            json!({"type": "message", "subtype": "channel_join", "user": "U1"}),
            json!({"type": "message", "subtype": "message_changed",
                   "message": {"user": "U1", "text": "see https://x"}}),
        ];
        for e in &dropped {
            assert!(drop_reason(e).is_some(), "{e} must be dropped");
        }
        let kept = [
            json!({"type": "message", "user": "U1", "text": "hi"}),
            json!({"type": "app_mention", "user": "U1", "text": "<@UBOT> hi"}),
            json!({"type": "message", "subtype": "file_share", "user": "U1"}),
            json!({"type": "message", "subtype": "message_changed",
                   "message": {"user": "U1", "text": "fixed", "edited": {"ts": "2", "user": "U1"}}}),
        ];
        for e in &kept {
            assert_eq!(drop_reason(e), None, "{e} must be forwarded");
        }
    }

    /// Item: Socket Mode dial deadline. A server that accepts the TCP
    /// connection and then never answers the WebSocket handshake (the
    /// boot-time network flap that silenced every workspace for hours) must
    /// fail within the deadline, as an ordinary error the listener backs off
    /// on — never park the task.
    #[tokio::test]
    async fn a_dial_accepted_but_never_answered_fails_within_its_deadline() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            // Accept, then hold the socket open in silence.
            let (_socket, _) = listener.accept().await.unwrap();
            std::future::pending::<()>().await;
        });
        let deadline = Duration::from_millis(300);
        let started = std::time::Instant::now();
        let Err(why) = dial_socket(&format!("ws://{addr}/link/?ticket=t1"), deadline).await else {
            panic!("a silent server must not complete the handshake");
        };
        assert!(why.contains("timed out"), "{why}");
        assert!(
            started.elapsed() < deadline + Duration::from_secs(2),
            "bounded by the deadline: {:?}",
            started.elapsed()
        );
        server.abort();

        // A closed port fails fast as an ordinary connect failure, with the
        // ticket-bearing URL kept out of the reason.
        let closed = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        let Err(why) = dial_socket(&format!("ws://{closed}/link/?ticket=t2"), deadline).await
        else {
            panic!("nothing listens there");
        };
        assert!(why.starts_with("Socket Mode connect failed"), "{why}");
        assert!(!why.contains("ticket=t2"), "{why}");
    }

    /// What the bridge saw from the listener: the hook calls and the replies.
    #[derive(Default)]
    struct Seen {
        noted: std::sync::Mutex<Vec<(String, String)>>,
        started: std::sync::Mutex<Vec<String>>,
        sent: std::sync::atomic::AtomicUsize,
    }

    struct Trigger(Arc<Seen>);

    #[async_trait::async_trait]
    impl crate::workflow_trigger::WorkflowChatTrigger for Trigger {
        async fn try_start(
            &self,
            _workspace_id: &str,
            _channel: &str,
            _chat: &str,
            _thread: Option<&str>,
            _user: &str,
            text: &str,
        ) -> Option<crate::workflow_trigger::WorkflowChatAck> {
            self.0.started.lock().unwrap().push(text.to_string());
            Some(crate::workflow_trigger::WorkflowChatAck {
                reply: "started".into(),
            })
        }

        async fn note_rejected_sender(
            &self,
            _workspace_id: &str,
            channel: &str,
            _chat: &str,
            _thread: Option<&str>,
            user: &str,
            text: &str,
        ) {
            assert_eq!(channel, "slack");
            self.0
                .noted
                .lock()
                .unwrap()
                .push((user.to_string(), text.to_string()));
        }
    }

    /// Never reaches Slack: counts what would have been posted.
    struct Counting(Arc<Seen>);

    #[async_trait::async_trait]
    impl Adapter for Counting {
        async fn send(&self, _c: &str, _t: Option<&str>, _x: &str) -> anyhow::Result<String> {
            self.0.sent.fetch_add(1, Ordering::SeqCst);
            Ok("1.0".into())
        }
        async fn edit(&self, _c: &str, _m: &str, _x: &str) -> anyhow::Result<()> {
            Ok(())
        }
        fn channel(&self) -> Channel {
            Channel::Slack
        }
    }

    fn message(user: &str, ts: &str, text: &str) -> serde_json::Value {
        serde_json::json!({
            "type": "message", "channel": "C1", "user": user, "ts": ts, "text": text,
            "user_profile": {"display_name": format!("name-{user}")}
        })
    }

    /// Item: a Slack sender the allow-list turns away used to leave no trace —
    /// `handle_event` returned before the bridge, so the Settings "turned
    /// away" list stayed empty and a workflow trigger they aimed at never
    /// said why it did nothing. Driven through the listener's real per-event
    /// handler and its busy path: the stranger is listed and the trigger
    /// note fires exactly once, their text is never acted on, and an
    /// admitted sender still flows through to the trigger.
    #[tokio::test]
    async fn a_turned_away_slack_sender_is_recorded_by_the_listener_and_never_acted_on() {
        let dir = std::env::temp_dir().join(format!("otto-slack-rejected-{}", otto_core::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let pool = otto_state::db::open(&dir.join("state.sqlite"))
            .await
            .unwrap();
        let owner = otto_state::UsersRepo::new(pool.clone())
            .create("owner", "", "Owner", true)
            .await
            .unwrap();
        let ws = otto_state::WorkspacesRepo::new(pool.clone())
            .create("WS", "/tmp", &owner.id)
            .await
            .unwrap();
        let (events, _) = tokio::sync::broadcast::channel(16);
        let manager = Arc::new(otto_sessions::SessionManager::new(
            otto_state::SessionsRepo::new(pool.clone()),
            events,
            otto_sessions::providers::ProviderRegistry::new(None),
        ));
        let seen = Arc::new(Seen::default());
        let bridge = Bridge::new_with_workflow_trigger(
            manager.clone(),
            otto_state::WorkspacesRepo::new(pool.clone()),
            otto_state::SettingsRepo::new(pool.clone()),
            crate::mirror::Mirror::new(manager.clone()),
            "root".into(),
            Some(Arc::new(Trigger(seen.clone()))),
            crate::admission::InboundAdmission::default(),
        );
        let integ: Integration = serde_json::from_value(serde_json::json!({
            "workspace_id": ws.id, "channel": "slack", "enabled": true,
            "allowed_users": "U1", "open_to_all": false, "agent_reply": false,
            "reply_instructions": "", "channel_id": "C1", "preferred_cli": "claude",
            "has_bot_token": true, "has_app_token": true, "updated_at": "2026-11-13T00:00:00Z"
        }))
        .unwrap();
        let health = crate::health::Health::begin(&ws.id, Channel::Slack);
        health.connected();
        let adapter = || Arc::new(Counting(seen.clone())) as Arc<dyn Adapter>;
        let permit = |ts: &str| match bridge.admission.admit("slack:t", "C1", Some(ts)) {
            crate::admission::Admission::Work(p) => p,
            _ => panic!("capacity available"),
        };
        let listed = |user: &str| {
            crate::health::snapshot(&ws.id)
                .iter()
                .flat_map(|s| s.rejected_senders.clone())
                .find(|r| r.user == user)
        };

        // A stranger's trigger, as Slack puts it on the wire.
        let stranger = message("U2", "1.1", "<@UBOT> run PR Reviewer: R&amp;D &lt;now&gt;");
        handle_event(
            &stranger,
            &integ,
            "xoxb-test",
            adapter(),
            bridge.clone(),
            permit("1.1"),
        )
        .await;
        let row = listed("U2").expect("the stranger is listed as turned away");
        assert_eq!(row.name.as_deref(), Some("name-U2"));
        assert_eq!(
            *seen.noted.lock().unwrap(),
            [("U2".to_string(), "run PR Reviewer: R&D <now>".to_string())],
            "the trigger note fires once, with the decoded text"
        );
        assert!(seen.started.lock().unwrap().is_empty(), "never acted on");
        assert_eq!(
            seen.sent.load(Ordering::SeqCst),
            0,
            "a stranger gets no reply"
        );
        assert!(manager.list_by_workspace(&ws.id).await.unwrap().is_empty());

        // The busy path records a stranger too — still without a reply.
        settle_busy(
            &message("U3", "1.2", "hello"),
            &integ,
            &bridge,
            &Counting(seen.clone()),
        )
        .await
        .unwrap();
        assert!(listed("U3").is_some());
        assert_eq!(seen.noted.lock().unwrap().len(), 2);
        assert_eq!(seen.sent.load(Ordering::SeqCst), 0);
        // …while an admitted sender is told to resend, and noted nowhere.
        settle_busy(
            &message("U1", "1.3", "hello"),
            &integ,
            &bridge,
            &Counting(seen.clone()),
        )
        .await
        .unwrap();
        assert_eq!(seen.sent.load(Ordering::SeqCst), 1);
        assert_eq!(seen.noted.lock().unwrap().len(), 2);

        // An admitted sender's trigger flows through to the workflow side.
        let owner_msg = message("U1", "1.4", "<@UBOT> run PR Reviewer: go");
        handle_event(
            &owner_msg,
            &integ,
            "xoxb-test",
            adapter(),
            bridge.clone(),
            permit("1.4"),
        )
        .await;
        assert_eq!(*seen.started.lock().unwrap(), ["run PR Reviewer: go"]);
        assert_eq!(seen.noted.lock().unwrap().len(), 2, "no note for the owner");
        assert!(listed("U1").is_none());

        crate::health::remove(&ws.id, Channel::Slack);
        drop((bridge, manager, pool));
        let _ = std::fs::remove_dir_all(dir);
    }
}
