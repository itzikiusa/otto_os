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
const RETRY_SLEEP: Duration = Duration::from_secs(3);

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
    message_thread_id: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct TgUser {
    id: i64,
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
    http: reqwest::Client,
}

impl TelegramAdapter {
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
            http: reqwest::Client::new(),
        }
    }

    fn api_url(&self, method: &str) -> String {
        format!("{API_BASE}/bot{}/{method}", self.token)
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
        }
        let resp = self
            .http
            .post(self.api_url("sendMessage"))
            .json(&body)
            .send()
            .await?
            .error_for_status()?;

        let tg: TgResponse<TgMessage> = resp.json().await?;
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

    async fn edit(&self, chat: &str, message_id: &str, text: &str) -> anyhow::Result<()> {
        let body = serde_json::json!({
            "chat_id": chat,
            "message_id": message_id.parse::<i64>().unwrap_or(0),
            "text": text,
        });
        let resp = self
            .http
            .post(self.api_url("editMessageText"))
            .json(&body)
            .send()
            .await?
            .error_for_status()?;

        let tg: TgResponse<serde_json::Value> = resp.json().await?;
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
        content: &str,
    ) -> anyhow::Result<()> {
        let file_part = reqwest::multipart::Part::bytes(content.as_bytes().to_vec())
            .file_name(filename.to_string())
            .mime_str("text/markdown")?;

        let mut form = reqwest::multipart::Form::new()
            .text("chat_id", chat.to_string())
            .part("document", file_part);

        if let Some(t) = thread {
            if let Ok(mid) = t.parse::<i64>() {
                form = form.text("reply_to_message_id", mid.to_string());
            }
        }

        let resp = self
            .http
            .post(self.api_url("sendDocument"))
            .multipart(form)
            .send()
            .await?
            .error_for_status()?;

        let tg: TgResponse<serde_json::Value> = resp.json().await?;
        if !tg.ok {
            return Err(anyhow::anyhow!(
                "Telegram sendDocument failed: {}",
                tg.description.unwrap_or_default()
            ));
        }
        Ok(())
    }

    /// Send a "typing" chat action so Telegram shows "Bot is typing…".
    async fn typing(&self, chat: &str) -> anyhow::Result<()> {
        let body = serde_json::json!({
            "chat_id": chat,
            "action": "typing",
        });
        let resp = self
            .http
            .post(self.api_url("sendChatAction"))
            .json(&body)
            .send()
            .await?
            .error_for_status()?;

        let tg: TgResponse<bool> = resp.json().await?;
        if !tg.ok {
            return Err(anyhow::anyhow!(
                "Telegram sendChatAction failed: {}",
                tg.description.unwrap_or_default()
            ));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Long-poll listener
// ---------------------------------------------------------------------------

/// Long-poll until `cancel` is set. Each incoming text message is forwarded
/// to `bridge`.
pub async fn run(integ: Integration, token: String, bridge: Arc<Bridge>, cancel: Arc<AtomicBool>) {
    let adapter = Arc::new(TelegramAdapter::new(token.clone()));
    let http = reqwest::Client::new();
    let mut offset: i64 = 0;
    info!(workspace = %integ.workspace_id, "telegram: listener loop started");

    loop {
        if cancel.load(Ordering::Relaxed) {
            debug!("telegram listener stopping (cancel)");
            return;
        }

        let url = format!(
            "{API_BASE}/bot{}/getUpdates?timeout={LONG_POLL_TIMEOUT}&offset={offset}",
            token
        );

        let resp = match http.get(&url).send().await {
            Ok(r) => r,
            Err(e) => {
                error!("telegram getUpdates: {e}");
                tokio::time::sleep(RETRY_SLEEP).await;
                continue;
            }
        };

        let tg: TgResponse<Vec<TgUpdate>> = match resp.json().await {
            Ok(v) => v,
            Err(e) => {
                error!("telegram getUpdates parse: {e}");
                tokio::time::sleep(RETRY_SLEEP).await;
                continue;
            }
        };

        if !tg.ok {
            error!(
                "telegram getUpdates not ok: {}",
                tg.description.unwrap_or_default()
            );
            tokio::time::sleep(RETRY_SLEEP).await;
            continue;
        }

        let updates = tg.result.unwrap_or_default();
        for update in &updates {
            if let Some(msg) = &update.message {
                if let Some(text) = &msg.text {
                    let user = msg
                        .from
                        .as_ref()
                        .map(|u| u.id.to_string())
                        .unwrap_or_default();
                    let chat = msg.chat.id.to_string();
                    let thread = msg.message_thread_id.map(|t| t.to_string());

                    let inbound = Inbound {
                        workspace_id: integ.workspace_id.clone(),
                        chat,
                        thread,
                        user,
                        text: text.clone(),
                    };
                    info!(
                        workspace = %inbound.workspace_id,
                        chat = %inbound.chat,
                        thread = ?inbound.thread,
                        user = %inbound.user,
                        update_id = update.update_id,
                        "telegram: inbound text update received"
                    );
                    bridge
                        .handle(&integ, Arc::clone(&adapter) as Arc<dyn Adapter>, inbound)
                        .await;
                }
            }
            // Advance offset past this update so we don't re-process it.
            offset = update.update_id + 1;
        }

        // If we got no updates, yield briefly to avoid a hot spin.
        if updates.is_empty() {
            tokio::task::yield_now().await;
        }
    }
}
