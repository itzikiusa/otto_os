//! Shared report + delivery helpers for the recurring-agent engines
//! (`scheduled_tasks_engine`, `personal_agents_engine`).
//!
//! Extracted verbatim from `scheduled_tasks_engine` so both engines drive the
//! SAME summary extraction, notify-on-change hashing, report writing, and
//! destination delivery (Slack/Telegram/email/webhook) — one implementation,
//! two callers. Behavior is unchanged; the scheduled-tasks engine re-exports
//! these under its old paths.

use otto_core::domain::Channel;
use otto_core::{Error, Result};
use serde_json::Value;

use otto_channels::improve_notify::{build_adapter, send_to};
use otto_channels::{Adapter, GmailSender, WebhookAdapter};
use otto_state::{EmailSendersRepo, IntegrationsRepo};

use crate::state::ServerCtx;

/// Extract the short summary from a report: everything up to the first `---`/`***`
/// horizontal rule, else the first ~800 characters. Always trimmed.
pub fn extract_summary(report: &str) -> String {
    let trimmed = report.trim();
    for sep in ["\n---", "\n***"] {
        if let Some(idx) = trimmed.find(sep) {
            let head = trimmed[..idx].trim();
            if !head.is_empty() {
                return head.to_string();
            }
        }
    }
    if trimmed.chars().count() <= 800 {
        return trimmed.to_string();
    }
    let cut: String = trimmed.chars().take(800).collect();
    format!("{}…", cut.trim_end())
}

/// The text posted alongside the report attachment.
pub fn delivery_message(name: &str, summary: &str) -> String {
    format!("*{name}*\n\n{summary}")
}

/// The destination tag (`none` when absent/blank).
pub fn destination_kind(dest: &Value) -> &str {
    dest.get("type").and_then(Value::as_str).unwrap_or("none")
}

/// Normalised content hash for notify-on-change — collapses whitespace so a
/// re-run with only formatting noise still counts as "unchanged".
pub fn report_hash(report: &str) -> String {
    use std::hash::{Hash, Hasher};
    let normalized: String = report.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut h = std::collections::hash_map::DefaultHasher::new();
    normalized.hash(&mut h);
    format!("{:016x}", h.finish())
}

/// Append the "write your report to FILE" instruction (codex/agy write no
/// transcript, so the file is the reliable capture path; claude's JSONL is a
/// fallback handled by the watcher).
pub fn augment_report_prompt(base: &str, out_path: &str) -> String {
    format!(
        "{base}\n\n---\nWhen you have finished, write your COMPLETE Markdown report (and nothing \
         else) to this absolute file path, overwriting any existing content:\n\n{out_path}\n\n\
         Writing that file is the last thing you do."
    )
}

/// Write the report file, creating parent directories.
pub async fn write_report(abs: &std::path::Path, report: &str) -> Result<()> {
    if let Some(parent) = abs.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| Error::Internal(format!("create report dir: {e}")))?;
    }
    tokio::fs::write(abs, report)
        .await
        .map_err(|e| Error::Internal(format!("write report: {e}")))
}

/// Deliver a report to a `{type: none|slack|telegram|email|webhook, …}`
/// destination on behalf of `owner` (needed to resolve a verified email
/// sender). Returns `(delivered, error?)`. The delivered text + attachment are
/// redacted (the report leaves the machine). Best-effort by contract: the
/// report is stored regardless.
pub async fn deliver_destination(
    ctx: &ServerCtx,
    workspace_id: &str,
    owner: Option<&str>,
    name: &str,
    destination: &Value,
    summary: &str,
    report: &str,
) -> (bool, Option<String>) {
    let kind = destination_kind(destination);
    if kind == "none" {
        return (false, None);
    }
    let msg = otto_core::redact::redact_text(&delivery_message(name, summary)).value;
    let report_bytes = otto_core::redact::redact_text(report).value.into_bytes();
    match kind {
        "slack" | "telegram" => {
            deliver_channel(ctx, workspace_id, destination, kind, &msg, &report_bytes).await
        }
        "email" => deliver_email(ctx, owner, destination, &msg, &report_bytes).await,
        "webhook" => {
            let url = destination.get("url").and_then(Value::as_str).unwrap_or("");
            match deliver_webhook(url, &msg, "report.md", &report_bytes).await {
                Ok(()) => (true, None),
                Err(e) => (false, Some(e.to_string())),
            }
        }
        other => (false, Some(format!("unknown destination type '{other}'"))),
    }
}

async fn deliver_channel(
    ctx: &ServerCtx,
    workspace_id: &str,
    destination: &Value,
    kind: &str,
    msg: &str,
    bytes: &[u8],
) -> (bool, Option<String>) {
    let channel = match kind {
        "slack" => Channel::Slack,
        "telegram" => Channel::Telegram,
        _ => return (false, Some(format!("bad channel '{kind}'"))),
    };
    let integ = match IntegrationsRepo::new(ctx.pool.clone())
        .get(&workspace_id.to_string(), channel)
        .await
    {
        Ok(Some(i)) => i,
        Ok(None) => return (false, Some(format!("no {kind} integration configured for the workspace"))),
        Err(e) => return (false, Some(e.to_string())),
    };
    let chat = destination
        .get("chat_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or(&integ.channel_id)
        .to_string();
    if chat.trim().is_empty() {
        return (false, Some("no destination chat configured".into()));
    }
    if !send_to(&ctx.secrets, &integ, &chat, None, msg).await {
        return (false, Some("channel send failed (bot token missing or API error)".into()));
    }
    if let Some(adapter) = build_adapter(&ctx.secrets, &integ) {
        if let Err(e) = adapter.upload(&chat, None, "report.md", bytes).await {
            return (true, Some(format!("message sent but attachment upload failed: {e}")));
        }
    }
    (true, None)
}

async fn deliver_email(
    ctx: &ServerCtx,
    owner: Option<&str>,
    destination: &Value,
    msg: &str,
    bytes: &[u8],
) -> (bool, Option<String>) {
    let Some(to) = destination
        .get("to")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    else {
        return (false, Some("email destination is missing 'to'".into()));
    };
    let Some(owner) = owner.filter(|s| !s.is_empty()) else {
        return (false, Some("no owner to resolve a verified email sender".into()));
    };
    let sender = match EmailSendersRepo::new(ctx.pool.clone()).get(owner).await {
        Ok(Some(s)) if s.verified_at.is_some() => s,
        Ok(_) => return (false, Some("no verified email sender for the owner".into())),
        Err(e) => return (false, Some(e.to_string())),
    };
    let pw = match ctx.secrets.get(&sender.secret_ref) {
        Ok(Some(p)) => p,
        _ => return (false, Some("email app password unavailable in keychain".into())),
    };
    let subject = destination
        .get("subject")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or("Scheduled task report")
        .to_string();
    let mailer = GmailSender::new(sender.gmail_address, pw);
    match mailer.send_with_attachment(to, &subject, msg, "report.md", bytes).await {
        Ok(()) => (true, None),
        Err(e) => (false, Some(e.to_string())),
    }
}

/// POST the report to a user-supplied URL via `WebhookAdapter`, which runs the
/// `otto_netguard` SSRF check + redirect policy before every request.
pub async fn deliver_webhook(url: &str, text: &str, filename: &str, bytes: &[u8]) -> Result<()> {
    if url.trim().is_empty() {
        return Err(Error::Invalid("webhook destination is missing 'url'".into()));
    }
    let adapter = WebhookAdapter::new(Some(url.to_string()));
    adapter
        .send_formatted("scheduled-task", None, text)
        .await
        .map_err(|e| Error::Upstream(format!("webhook delivery: {e}")))?;
    adapter
        .upload("scheduled-task", None, filename, bytes)
        .await
        .map_err(|e| Error::Upstream(format!("webhook attachment: {e}")))?;
    Ok(())
}
